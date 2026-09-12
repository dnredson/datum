//! Stage 107B-A (ADR-0013): OCI Distribution v2 registry resolution.
//!
//! Resolves a mutable image reference (e.g. `eclipse-mosquitto:2`) into an
//! immutable, multi-platform `ResolvedImageIdentityV1` by talking to the
//! real registry (or, in tests, a deterministic fake -- no live registry is
//! ever contacted by a test in this module). Fail-closed at every branch: an
//! unresolvable, ambiguous, or malformed response is always an error, never
//! a silent fallback (e.g. defaulting to "the first platform entry" or "amd64").
//!
//! Both Docker Hub and `ghcr.io` require the anonymous OCI Distribution v2
//! bearer-token challenge flow even for public images (401 +
//! `WWW-Authenticate: Bearer realm=...,service=...,scope=...` -> anonymous
//! token GET -> retry with `Authorization: Bearer <token>`); this is
//! implemented below, not optional.
//!
//! Transport is deliberately SYNCHRONOUS (mirrors
//! `DATUM/src/dispatch/container_executor.rs`'s `ContainerRuntime` trait +
//! `FakeRuntime` pattern): no `async_trait` dependency exists anywhere in
//! this workspace, and native async-fn-in-traits is not `dyn`-object-safe on
//! this toolchain without hand-written boxed futures. `ReqwestOciHttpTransport`
//! uses `reqwest::blocking::Client` (the existing `reqwest` dependency's
//! `"blocking"` Cargo feature, no new crate); the one async call site
//! (`dserver/src/api/service_artifacts.rs::post_resolve_image_identity`)
//! runs `pin_image_identity` inside `tokio::task::spawn_blocking`.
//!
//! **Trust boundary (v5 review P1-4, CORRECTED by v6 review P0-2).** The
//! `POST /image-identity/resolve` endpoint is NOT, today, a "separately
//! authorized administrative action" in any meaningful sense: DServer has
//! no application-layer authentication/authorization middleware at all, on
//! this route or on the ordinary `service-artifacts` POST/PUT routes, and it
//! binds `0.0.0.0` by default. `X-Datum-Project` is request scoping, not
//! authorization. Earlier revisions of this module's doc comment (and
//! ADR-0013) incorrectly implied otherwise -- do not repeat that claim
//! anywhere in this codebase until a real authentication/authorization layer
//! exists.
//!
//! Given that, the actual safety boundary preventing this feature from being
//! an unbounded outbound-HTTPS-request primitive for anyone who can reach
//! DServer is the explicit allow-list below (`ALLOWED_REGISTRY_HOSTS`,
//! `ALLOWED_TOKEN_REALM_HOSTS`), enforced BEFORE the first outbound request
//! in both `resolve_image_identity` (registry host) and
//! `fetch_anonymous_bearer_token` (token realm host, in addition to the
//! existing `https://`-only requirement -- HTTPS alone does not make an
//! arbitrary target safe, since an attacker-controlled host can terminate
//! TLS perfectly well). This is deliberately a small, hardcoded, default-deny
//! list, not a general configuration subsystem: extend it only when a real
//! catalog registry is added, and never treat it as a substitute for real
//! authentication/authorization, which this stage does not implement. Bearer
//! tokens are never logged.

use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

use serde_json::Value;

use crate::core::sha256::sha256_hex;
use crate::core::service_artifact_registry::{ResolvedImageIdentityV1, ResolvedPlatformManifestV1};

/// v6 review P0-2: the only registry API hosts this resolver will ever
/// target with a FIRST-HOP request. Matches the real catalog's two
/// registries exactly (Docker Hub's API host and GHCR). Default-deny: any
/// other host is refused before any outbound request is made.
///
/// v7 review P1-3 correction: this list does NOT, by itself, bound every
/// host this resolver's requests can ever reach a response from -- a
/// redirect issued BY one of these hosts (e.g. a blob GET 307-redirected to
/// provider-controlled CDN/object-storage infrastructure, which both Docker
/// Hub and GHCR do routinely) is followed under the separate, explicit
/// redirect policy (`evaluate_oci_redirect`), which does not re-check the
/// redirect target against this list. See that function's doc comment for
/// the full rationale; do not describe this list alone as bounding "every
/// host this resolver will ever contact."
pub const ALLOWED_REGISTRY_HOSTS: &[&str] = &["registry-1.docker.io", "ghcr.io"];

/// v6 review P0-2: the only bearer-token realm hosts this resolver will ever
/// follow a `WWW-Authenticate` challenge to. `auth.docker.io` is Docker
/// Hub's real token issuer; `ghcr.io` issues its own tokens at the same
/// host. Checked in addition to (not instead of) the `https://`-only
/// requirement on the realm. See the `ALLOWED_REGISTRY_HOSTS` note (v7 P1-3)
/// for the same caveat regarding a subsequent redirect from this host.
pub const ALLOWED_TOKEN_REALM_HOSTS: &[&str] = &["auth.docker.io", "ghcr.io"];

/// Extracts the host (and port, if present) from an `https://host[:port]/...`
/// URL. Returns `None` for anything that doesn't start with `https://`,
/// which callers should already have rejected before calling this.
fn url_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let end = rest.find('/').unwrap_or(rest.len());
    let host = &rest[..end];
    if host.is_empty() {
        None
    } else {
        Some(host)
    }
}

/// True when `s` is `sha256:` followed by exactly 64 lowercase hex
/// characters -- the OCI content-digest domain. Deliberately a SEPARATE
/// validator from `jcs::is_canonical_digest_format` even though the string
/// shape is coincidentally identical: an OCI content digest (over raw
/// registry manifest/config bytes) and a canonical DATUM contract digest
/// (over JCS-canonicalized JSON) are different digest domains per ADR-0006,
/// and must never be treated as interchangeable or have one validator reused
/// for the other.
pub fn is_oci_digest_format(s: &str) -> bool {
    match s.strip_prefix("sha256:") {
        Some(hex) => {
            hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSpec {
    pub os: String,
    pub architecture: String,
    pub variant: Option<String>,
}

/// v6 review P1-2: a caller-controlled input (ultimately from the
/// `POST /image-identity/resolve` request body) must be validated at the
/// resolver boundary BEFORE it drives any network call or ends up echoed
/// into a persisted `ResolvedImageIdentityV1` -- an empty `os`/`architecture`
/// here would otherwise happily coexist with (and be indistinguishable from)
/// a malformed manifest's own missing platform fields, and duplicate
/// requested platforms would silently produce a duplicate persisted platform
/// set. Fail-closed on: emptiness, a blank (but present) `os`/`architecture`,
/// an explicitly-empty-string `variant`, a duplicate `(os, architecture,
/// variant)` tuple, or more than a small, sane maximum.
const MAX_TARGET_PLATFORMS: usize = 16;

pub fn validate_target_platforms(target_platforms: &[PlatformSpec]) -> Result<(), OciResolutionError> {
    if target_platforms.is_empty() {
        return Err(OciResolutionError::InvalidReference(
            "at least one target platform is required to resolve an image".to_string(),
        ));
    }
    if target_platforms.len() > MAX_TARGET_PLATFORMS {
        return Err(OciResolutionError::InvalidTargetPlatform(format!(
            "at most {MAX_TARGET_PLATFORMS} target platforms may be requested at once, got {}",
            target_platforms.len()
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for platform in target_platforms {
        if platform.os.trim().is_empty() {
            return Err(OciResolutionError::InvalidTargetPlatform("os must not be empty".to_string()));
        }
        if platform.architecture.trim().is_empty() {
            return Err(OciResolutionError::InvalidTargetPlatform("architecture must not be empty".to_string()));
        }
        if let Some(variant) = &platform.variant {
            if variant.trim().is_empty() {
                return Err(OciResolutionError::InvalidTargetPlatform(
                    "variant, when present, must not be an empty string (omit it entirely instead)".to_string(),
                ));
            }
        }
        let key = (platform.os.clone(), platform.architecture.clone(), platform.variant.clone());
        if !seen.insert(key) {
            return Err(OciResolutionError::InvalidTargetPlatform(format!(
                "duplicate requested platform {}/{}{}",
                platform.os,
                platform.architecture,
                platform.variant.as_deref().map(|v| format!("/{v}")).unwrap_or_default()
            )));
        }
    }
    Ok(())
}

/// Maximum response body size accepted from a registry/token endpoint (v5
/// review P1-4, basic hardening -- not a full streaming-cap subsystem).
/// Manifests, indexes, image configs and token responses are all small
/// (kilobytes); this is generous headroom, not a tuned production limit.
const MAX_RESPONSE_BODY_BYTES: usize = 32 * 1024 * 1024;

/// v7 review P1-3: the maximum number of redirect hops this resolver will
/// follow for any single request. Real OCI registries commonly redirect
/// blob (and occasionally manifest) GETs to provider-controlled object
/// storage or CDN infrastructure -- both Docker Hub and GHCR routinely
/// 307-redirect a blob fetch to a signed, time-limited URL on a completely
/// different host. Disabling redirects outright would break real
/// resolution against both of this resolver's allowed registries, so this
/// bound is deliberately small but non-zero.
const MAX_OCI_REDIRECTS: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OciResolutionError {
    InvalidReference(String),
    RegistryUnreachable(String),
    AuthenticationChallengeFailed(String),
    ManifestNotFound(String),
    UnsupportedMediaType(String),
    MalformedDigest(String),
    MalformedImageConfig(String),
    DigestMismatch { expected: String, computed: String },
    PlatformNotFoundInIndex { requested: PlatformSpec, available: Vec<PlatformSpec> },
    AmbiguousPlatformMatchInIndex { requested: PlatformSpec, count: usize },
    PlatformMismatch { requested: PlatformSpec, actual: PlatformSpec },
    SinglePlatformManifestPlatformCountMismatch { requested: usize },
    ResponseTooLarge { bytes: u64 },
    RegistryHostNotAllowed(String),
    TokenRealmHostNotAllowed(String),
    InvalidTargetPlatform(String),
    RedirectRejected(String),
}

impl std::fmt::Display for OciResolutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidReference(m) => write!(f, "invalid image reference: {m}"),
            Self::RegistryUnreachable(m) => write!(f, "registry unreachable: {m}"),
            Self::AuthenticationChallengeFailed(m) => write!(f, "authentication challenge failed: {m}"),
            Self::ManifestNotFound(m) => write!(f, "manifest not found: {m}"),
            Self::UnsupportedMediaType(m) => write!(f, "unsupported manifest media type: {m}"),
            Self::MalformedDigest(m) => write!(f, "malformed OCI digest: {m}"),
            Self::MalformedImageConfig(m) => write!(f, "malformed image config: {m}"),
            Self::DigestMismatch { expected, computed } => {
                write!(f, "digest mismatch: expected {expected}, computed {computed}")
            }
            Self::PlatformNotFoundInIndex { requested, available } => write!(
                f,
                "requested platform {}/{}{} not found in index; available: {:?}",
                requested.os,
                requested.architecture,
                requested.variant.as_deref().map(|v| format!("/{v}")).unwrap_or_default(),
                available
            ),
            Self::AmbiguousPlatformMatchInIndex { requested, count } => write!(
                f,
                "requested platform {}/{}{} matched {count} index entries; ambiguous, refusing to guess",
                requested.os,
                requested.architecture,
                requested.variant.as_deref().map(|v| format!("/{v}")).unwrap_or_default(),
            ),
            Self::PlatformMismatch { requested, actual } => write!(
                f,
                "requested platform {}/{}{} but the image config actually declares {}/{}{}",
                requested.os,
                requested.architecture,
                requested.variant.as_deref().map(|v| format!("/{v}")).unwrap_or_default(),
                actual.os,
                actual.architecture,
                actual.variant.as_deref().map(|v| format!("/{v}")).unwrap_or_default(),
            ),
            Self::SinglePlatformManifestPlatformCountMismatch { requested } => write!(
                f,
                "a single (non-index) manifest disambiguates exactly one platform, but {requested} were requested"
            ),
            Self::ResponseTooLarge { bytes } => write!(f, "response body too large ({bytes} bytes)"),
            Self::RegistryHostNotAllowed(host) => {
                write!(f, "registry host '{host}' is not in the allowed-registry-host list (ALLOWED_REGISTRY_HOSTS)")
            }
            Self::TokenRealmHostNotAllowed(host) => {
                write!(f, "bearer token realm host '{host}' is not in the allowed-token-realm-host list (ALLOWED_TOKEN_REALM_HOSTS)")
            }
            Self::InvalidTargetPlatform(message) => write!(f, "invalid target platform: {message}"),
            Self::RedirectRejected(message) => write!(f, "redirect rejected: {message}"),
        }
    }
}
impl std::error::Error for OciResolutionError {}

// --- HTTP transport abstraction (sync, mirrors ContainerRuntime) ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OciHttpMethod {
    Get,
}

#[derive(Debug, Clone)]
pub struct OciHttpRequest {
    pub method: OciHttpMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct OciHttpResponse {
    pub status: u16,
    /// Header names are matched case-insensitively by callers; stored as
    /// received.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

pub trait OciHttpTransport {
    fn execute(&self, request: &OciHttpRequest) -> Result<OciHttpResponse, OciResolutionError>;
}

pub struct ReqwestOciHttpTransport {
    client: reqwest::blocking::Client,
}

/// v7 review P1-3: the actual, testable redirect-trust decision, independent
/// of `reqwest`'s own `redirect::Policy` machinery (whose `Attempt` type
/// cannot be constructed directly in a unit test, since it is only ever
/// produced by `reqwest` itself mid-request). `ReqwestOciHttpTransport`
/// wires this into a `reqwest::redirect::Policy::custom` closure below; the
/// tests in this module call it directly against synthetic hop
/// counts/locations, with no HTTP involved.
///
/// Deliberate design choice (option B from the review): a redirect issued BY
/// an already-allowed registry or token host (the only kind of request this
/// resolver ever makes, per `ALLOWED_REGISTRY_HOSTS` / `ALLOWED_TOKEN_REALM_HOSTS`)
/// is treated as within that provider's own trust boundary and is followed
/// -- the redirect TARGET (typically a CDN or object-storage host) is
/// provider-controlled, not catalog-controlled, and is deliberately NOT
/// re-checked against either allow-list; requiring that would break real
/// resolution against both real registries this resolver targets. What IS
/// enforced, regardless of which allowed provider issued the redirect:
///   - HTTPS-only (a location that isn't `https://` is rejected outright,
///     which also rules out an HTTPS -> HTTP downgrade by construction);
///   - a bounded hop count (`MAX_OCI_REDIRECTS`);
///   - a well-formed location with an extractable host (rejects an
///     unparseable/malformed `Location`).
fn evaluate_oci_redirect(previous_hop_count: usize, location: &str) -> Result<(), OciResolutionError> {
    if previous_hop_count >= MAX_OCI_REDIRECTS {
        return Err(OciResolutionError::RedirectRejected(format!(
            "exceeded the maximum of {MAX_OCI_REDIRECTS} redirect hops"
        )));
    }
    if !location.starts_with("https://") {
        return Err(OciResolutionError::RedirectRejected(format!(
            "redirect target must be an https:// URL (no downgrade to plain http), got: {location}"
        )));
    }
    if url_host(location).is_none() {
        return Err(OciResolutionError::RedirectRejected(format!(
            "redirect target has no extractable host: {location}"
        )));
    }
    Ok(())
}

impl ReqwestOciHttpTransport {
    pub fn new() -> Self {
        // v5 review P1-4: finite connect/request timeouts -- a hung or
        // slow-loris registry/token endpoint must not block the pin
        // operation (and the write lock it eventually takes, see
        // oci_image_pinning.rs's CAS design) indefinitely.
        //
        // v7 review P1-3: an explicit redirect policy -- `reqwest`'s default
        // follows up to 10 redirects to ANY host with no scheme constraint,
        // which is stronger than this module's own documented trust
        // boundary actually guaranteed. `evaluate_oci_redirect` is the real,
        // testable policy; see its doc comment for the full rationale.
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                match evaluate_oci_redirect(attempt.previous().len(), attempt.url().as_str()) {
                    Ok(()) => attempt.follow(),
                    Err(error) => attempt.error(error),
                }
            }))
            .build()
            .expect("failed to build the OCI registry HTTP client (TLS backend initialization failed)");
        Self { client }
    }
}

impl Default for ReqwestOciHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl OciHttpTransport for ReqwestOciHttpTransport {
    fn execute(&self, request: &OciHttpRequest) -> Result<OciHttpResponse, OciResolutionError> {
        let mut builder = match request.method {
            OciHttpMethod::Get => self.client.get(&request.url),
        };
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = builder
            .send()
            .map_err(|error| OciResolutionError::RegistryUnreachable(error.to_string()))?;
        let status = response.status().as_u16();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        // v6 review P1-1: reject an oversized response before reading the
        // body when the server declares its size up front -- a fast-path
        // optimization only. The REAL guarantee comes from the bounded read
        // below, which never accumulates more than MAX_RESPONSE_BODY_BYTES + 1
        // bytes regardless of whether Content-Length is present, absent, or
        // dishonest (e.g. a chunked response with no declared length).
        if let Some(declared) = find_header(&headers, "content-length").and_then(|v| v.parse::<u64>().ok()) {
            if declared > MAX_RESPONSE_BODY_BYTES as u64 {
                return Err(OciResolutionError::ResponseTooLarge { bytes: declared });
            }
        }
        let body = read_bounded_body(response, MAX_RESPONSE_BODY_BYTES)?;
        Ok(OciHttpResponse { status, headers, body })
    }
}

/// v6 review P1-1: reads AT MOST `limit + 1` bytes from `reader` (via
/// `Read::take`, which never accumulates more into memory regardless of the
/// stream's real length or any Content-Length header) and rejects with
/// `ResponseTooLarge` if that extra byte was actually present -- i.e. if the
/// stream turned out to hold more than `limit` bytes. Extracted as a
/// standalone, transport-agnostic function so it is unit-testable against a
/// plain in-memory reader, with no HTTP involved.
fn read_bounded_body(reader: impl std::io::Read, limit: usize) -> Result<Vec<u8>, OciResolutionError> {
    let mut buf = Vec::with_capacity(std::cmp::min(limit.saturating_add(1), 1 << 20));
    let read_limit = (limit as u64).saturating_add(1);
    reader
        .take(read_limit)
        .read_to_end(&mut buf)
        .map_err(|error| OciResolutionError::RegistryUnreachable(error.to_string()))?;
    if buf.len() > limit {
        return Err(OciResolutionError::ResponseTooLarge { bytes: buf.len() as u64 });
    }
    Ok(buf)
}

fn find_header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

// --- Reference parsing ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceSelector {
    Tag(String),
    Digest(String),
}

/// v5 review P0-3: a reference resolves to FOUR distinct concepts, which
/// must never be conflated:
/// - `registry_host`: the actual API host this resolver talks to over HTTP
///   (e.g. `registry-1.docker.io` -- Docker Hub's real API endpoint).
/// - `canonical_registry`: the registry name as it appears in a canonical,
///   human/Docker-facing reference (e.g. `docker.io` -- NOT
///   `registry-1.docker.io`, which is an API-only implementation detail).
/// - `repository_path`: the path segment used in the HTTP API
///   (`/v2/<repository_path>/...`), e.g. `library/eclipse-mosquitto`.
/// - `fully_qualified_repository()` = `"{canonical_registry}/{repository_path}"`,
///   e.g. `docker.io/library/eclipse-mosquitto`, `ghcr.io/absmach/atom` --
///   THIS is what `ResolvedImageIdentityV1.repository` and
///   `execution_reference` must be built from, never the bare
///   `repository_path` alone (an unqualified `absmach/atom@sha256:...`
///   reference is a DIFFERENT, ambiguous image identity: Docker would
///   resolve it against its own default registry, not necessarily GHCR).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedImageReference {
    pub registry_host: String,
    pub canonical_registry: String,
    pub repository_path: String,
    pub selector: ReferenceSelector,
}

impl ParsedImageReference {
    pub fn fully_qualified_repository(&self) -> String {
        format!("{}/{}", self.canonical_registry, self.repository_path)
    }
}

/// Parses exactly the reference shapes the real catalog uses: bare
/// (`postgres:16`), Docker-Hub-namespaced (`chirpstack/chirpstack:4`),
/// explicit `docker.io/...`, `ghcr.io/...`, a custom host (optionally with a
/// port), `@sha256:...` digest-form references, and the combined
/// `name:tag@sha256:...` form (v5 review P1-2: the digest is authoritative;
/// any `:tag` before `@` is stripped rather than leaking into the
/// repository name). Applies Docker Hub's implicit-registry-host and
/// implicit-`library/`-namespace rules (both for a bare name and for an
/// explicit `docker.io/<name>` reference with no further namespace segment).
pub fn parse_image_reference(reference: &str) -> Result<ParsedImageReference, OciResolutionError> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Err(OciResolutionError::InvalidReference("image reference must not be empty".to_string()));
    }

    // Digest-form ("@sha256:<hex>") takes precedence over any trailing
    // ":tag" if both somehow appear, matching Docker's own reference
    // grammar (a digest is the authoritative selector when present).
    let (name_part, selector) = if let Some(at_index) = reference.rfind('@') {
        let (before_at, digest_part) = reference.split_at(at_index);
        let digest = &digest_part[1..];
        if !is_oci_digest_format(digest) {
            return Err(OciResolutionError::MalformedDigest(digest.to_string()));
        }
        // P1-2: `image:tag@digest` is legal Docker/OCI grammar. The digest
        // is authoritative; a trailing ":tag" before '@' must NOT leak into
        // the repository name (searched only after the last '/', since a
        // registry host's port, e.g. "localhost:5000/name", also contains
        // ':' before the last '/').
        let last_slash = before_at.rfind('/');
        let search_from = last_slash.map(|i| i + 1).unwrap_or(0);
        let name = match before_at[search_from..].rfind(':') {
            Some(colon_index) => &before_at[..search_from + colon_index],
            None => before_at,
        };
        (name, ReferenceSelector::Digest(digest.to_string()))
    } else {
        // Look for a ":tag" AFTER the last '/', since a registry host's
        // port (e.g. "localhost:5000/name") also contains ':' before the
        // last '/'.
        let last_slash = reference.rfind('/');
        let search_from = last_slash.map(|i| i + 1).unwrap_or(0);
        if let Some(colon_index) = reference[search_from..].rfind(':') {
            let absolute_colon = search_from + colon_index;
            let (name, tag_part) = reference.split_at(absolute_colon);
            let tag = &tag_part[1..];
            if tag.is_empty() {
                return Err(OciResolutionError::InvalidReference(format!("malformed reference '{reference}'")));
            }
            (name, ReferenceSelector::Tag(tag.to_string()))
        } else {
            (reference, ReferenceSelector::Tag("latest".to_string()))
        }
    };

    if name_part.is_empty() {
        return Err(OciResolutionError::InvalidReference(format!("malformed reference '{reference}'")));
    }

    let segments: Vec<&str> = name_part.split('/').collect();
    let first_looks_like_host =
        segments.len() > 1 && (segments[0].contains('.') || segments[0].contains(':') || segments[0] == "localhost");

    let (registry_host, canonical_registry, mut repository_path) = if first_looks_like_host {
        let host = segments[0].to_string();
        let repo = segments[1..].join("/");
        if host == "docker.io" {
            ("registry-1.docker.io".to_string(), "docker.io".to_string(), repo)
        } else {
            (host.clone(), host, repo)
        }
    } else if segments.len() == 1 {
        ("registry-1.docker.io".to_string(), "docker.io".to_string(), segments[0].to_string())
    } else {
        ("registry-1.docker.io".to_string(), "docker.io".to_string(), name_part.to_string())
    };

    if canonical_registry == "docker.io" && !repository_path.contains('/') {
        repository_path = format!("library/{repository_path}");
    }

    if repository_path.is_empty() {
        return Err(OciResolutionError::InvalidReference(format!("malformed reference '{reference}'")));
    }

    Ok(ParsedImageReference { registry_host, canonical_registry, repository_path, selector })
}

// --- Bearer challenge / token flow -----------------------------------------

fn parse_bearer_challenge(challenge: &str) -> Result<HashMap<String, String>, OciResolutionError> {
    let rest = challenge.trim().strip_prefix("Bearer ").ok_or_else(|| {
        OciResolutionError::AuthenticationChallengeFailed(format!(
            "WWW-Authenticate challenge is not a Bearer challenge: {challenge}"
        ))
    })?;
    let mut params = HashMap::new();
    for part in split_challenge_params(rest) {
        if let Some((key, value)) = part.split_once('=') {
            let value = value.trim().trim_matches('"').to_string();
            params.insert(key.trim().to_string(), value);
        }
    }
    Ok(params)
}

/// Comma-separated `key="value"` pairs; values are always quoted per RFC
/// 7235, so a naive split on `,` outside quotes is sufficient for the
/// registries this resolver targets.
fn split_challenge_params(rest: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in rest.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => parts.push(std::mem::take(&mut current)),
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Minimal percent-encoding sufficient for the values that appear in a
/// Distribution token request's `service`/`scope` query parameters (which
/// are themselves colon/slash-shaped, e.g. `repository:library/nginx:pull`,
/// and must reach the token endpoint with those characters intact).
fn urlencode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn fetch_anonymous_bearer_token(
    transport: &dyn OciHttpTransport,
    challenge: &str,
) -> Result<String, OciResolutionError> {
    let params = parse_bearer_challenge(challenge)?;
    let realm = params.get("realm").ok_or_else(|| {
        OciResolutionError::AuthenticationChallengeFailed(format!("WWW-Authenticate challenge missing realm: {challenge}"))
    })?;
    // v5 review P1-4 (SSRF hardening): `realm` comes from a REMOTE server's
    // own response header -- less trusted than catalog data. Refuse to
    // follow it anywhere but `https://`, so a malicious/compromised
    // registry cannot redirect this server-side request to an arbitrary
    // internal `http://` endpoint.
    if !realm.starts_with("https://") {
        return Err(OciResolutionError::AuthenticationChallengeFailed(format!(
            "WWW-Authenticate realm must be an https:// URL, got: {realm}"
        )));
    }
    // v6 review P0-2: HTTPS alone does not make an arbitrary target safe --
    // an attacker-controlled host can terminate TLS. The realm host must
    // also be in the explicit allow-list.
    let realm_host = url_host(realm).ok_or_else(|| {
        OciResolutionError::AuthenticationChallengeFailed(format!("could not extract a host from realm: {realm}"))
    })?;
    if !ALLOWED_TOKEN_REALM_HOSTS.contains(&realm_host) {
        return Err(OciResolutionError::TokenRealmHostNotAllowed(realm_host.to_string()));
    }
    let mut query_parts = Vec::new();
    if let Some(service) = params.get("service") {
        query_parts.push(format!("service={}", urlencode(service)));
    }
    if let Some(scope) = params.get("scope") {
        query_parts.push(format!("scope={}", urlencode(scope)));
    }
    let mut url = format!("{realm}?");
    url.push_str(&query_parts.join("&"));

    let request = OciHttpRequest { method: OciHttpMethod::Get, url, headers: Vec::new() };
    let response = transport.execute(&request)?;
    if response.status != 200 {
        return Err(OciResolutionError::AuthenticationChallengeFailed(format!(
            "token endpoint responded with status {}",
            response.status
        )));
    }
    let document: Value = serde_json::from_slice(&response.body).map_err(|error| {
        OciResolutionError::AuthenticationChallengeFailed(format!("malformed token response: {error}"))
    })?;
    // Never log `document`/the extracted token anywhere in this module.
    document
        .get("token")
        .or_else(|| document.get("access_token"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            OciResolutionError::AuthenticationChallengeFailed(
                "token response carried neither 'token' nor 'access_token'".to_string(),
            )
        })
}

// --- Manifest / blob fetch ---------------------------------------------------

const OCI_INDEX_MEDIA_TYPE: &str = "application/vnd.oci.image.index.v1+json";
const DOCKER_MANIFEST_LIST_MEDIA_TYPE: &str = "application/vnd.docker.distribution.manifest.list.v2+json";
const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";
const DOCKER_MANIFEST_MEDIA_TYPE: &str = "application/vnd.docker.distribution.manifest.v2+json";

struct FetchedManifest {
    body: Vec<u8>,
    docker_content_digest_header: Option<String>,
}

/// Performs one GET against `url`, transparently completing the anonymous
/// bearer-token challenge on a 401, and returns the final 200 response.
/// Shared by manifest and blob fetches -- both need the identical
/// challenge/retry flow.
fn get_with_bearer_challenge(
    transport: &dyn OciHttpTransport,
    url: &str,
    accept: Option<&str>,
) -> Result<OciHttpResponse, OciResolutionError> {
    let mut headers = Vec::new();
    if let Some(accept) = accept {
        headers.push(("Accept".to_string(), accept.to_string()));
    }
    let request = OciHttpRequest { method: OciHttpMethod::Get, url: url.to_string(), headers: headers.clone() };
    let response = transport.execute(&request)?;
    if response.status == 401 {
        let challenge = find_header(&response.headers, "www-authenticate").ok_or_else(|| {
            OciResolutionError::AuthenticationChallengeFailed("401 response carried no WWW-Authenticate header".to_string())
        })?;
        let token = fetch_anonymous_bearer_token(transport, &challenge)?;
        headers.push(("Authorization".to_string(), format!("Bearer {token}")));
        let authorized_request = OciHttpRequest { method: OciHttpMethod::Get, url: url.to_string(), headers };
        transport.execute(&authorized_request)
    } else {
        Ok(response)
    }
}

fn fetch_manifest_bytes(
    transport: &dyn OciHttpTransport,
    registry_host: &str,
    repository_path: &str,
    selector: &str,
) -> Result<FetchedManifest, OciResolutionError> {
    let accept = [
        OCI_INDEX_MEDIA_TYPE,
        DOCKER_MANIFEST_LIST_MEDIA_TYPE,
        OCI_MANIFEST_MEDIA_TYPE,
        DOCKER_MANIFEST_MEDIA_TYPE,
    ]
    .join(", ");
    let url = format!("https://{registry_host}/v2/{repository_path}/manifests/{selector}");
    let response = get_with_bearer_challenge(transport, &url, Some(&accept))?;
    if response.status != 200 {
        return Err(OciResolutionError::ManifestNotFound(format!(
            "registry responded with status {}",
            response.status
        )));
    }
    let docker_content_digest_header = find_header(&response.headers, "docker-content-digest");
    Ok(FetchedManifest { body: response.body, docker_content_digest_header })
}

/// v5 review P0-4: a manifest's `platform` field (index case) or the
/// CALLER'S requested `PlatformSpec` (single-manifest case) is never trusted
/// on its own -- the image CONFIG blob is fetched, its SHA-256 verified
/// against `config_digest`, and its own declared `os`/`architecture`/
/// `variant` compared against `requested`. Fails closed on any mismatch,
/// fetch failure, or malformed config JSON.
/// v6 review P2: OCI image config's `variant` field is optional. Two
/// distinct trust situations exist for it here:
///   - `variant_authority = IndexDescriptorAlreadyConfirmed` (the index
///     path): the index descriptor's own `platform.variant` was ALREADY
///     matched exactly against `requested` before this function is ever
///     called (see `resolve_from_index`), so an absent `variant` in the
///     config blob is accepted -- the index is the authority for variant
///     identity here, and real-world config blobs commonly omit it even for
///     e.g. `arm64/v8`. `os`/`architecture` are never relaxed.
///   - `variant_authority = NoOtherAuthority` (the single-manifest path): no
///     index descriptor exists at all, so the config blob is the ONLY
///     source of truth; an absent `variant` when `requested.variant` is
///     `Some(..)` is treated as a mismatch, fail-closed, exactly like an
///     absent/wrong `os` or `architecture` would be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariantAuthority {
    IndexDescriptorAlreadyConfirmed,
    NoOtherAuthority,
}

fn platform_matches_with_variant_policy(actual: &PlatformSpec, requested: &PlatformSpec, authority: VariantAuthority) -> bool {
    if actual.os != requested.os || actual.architecture != requested.architecture {
        return false;
    }
    match authority {
        VariantAuthority::IndexDescriptorAlreadyConfirmed if actual.variant.is_none() => true,
        _ => actual.variant == requested.variant,
    }
}

fn verify_platform_via_config_blob(
    transport: &dyn OciHttpTransport,
    registry_host: &str,
    repository_path: &str,
    config_digest: &str,
    requested: &PlatformSpec,
    variant_authority: VariantAuthority,
) -> Result<(), OciResolutionError> {
    let url = format!("https://{registry_host}/v2/{repository_path}/blobs/{config_digest}");
    let response = get_with_bearer_challenge(transport, &url, None)?;
    if response.status != 200 {
        return Err(OciResolutionError::ManifestNotFound(format!(
            "config blob fetch responded with status {}",
            response.status
        )));
    }
    let computed = format!("sha256:{}", sha256_hex(&response.body));
    if computed != config_digest {
        return Err(OciResolutionError::DigestMismatch { expected: config_digest.to_string(), computed });
    }
    let document: Value = serde_json::from_slice(&response.body)
        .map_err(|error| OciResolutionError::MalformedImageConfig(format!("malformed image config JSON: {error}")))?;
    let actual = PlatformSpec {
        os: document.get("os").and_then(Value::as_str).unwrap_or("").to_string(),
        architecture: document.get("architecture").and_then(Value::as_str).unwrap_or("").to_string(),
        variant: document.get("variant").and_then(Value::as_str).map(str::to_string),
    };
    if !platform_matches_with_variant_policy(&actual, requested, variant_authority) {
        return Err(OciResolutionError::PlatformMismatch { requested: requested.clone(), actual });
    }
    Ok(())
}

// --- Resolution entry point --------------------------------------------------

/// Fail-closed at every branch: an unparseable reference, a malformed
/// digest, an unrecognized media type, a digest self-check mismatch, an
/// ambiguous or absent platform match, or an image-config platform mismatch
/// are all errors -- never a silent fallback (never "use the first index
/// entry", never "assume amd64", never "trust the caller's platform claim").
pub fn resolve_image_identity(
    transport: &dyn OciHttpTransport,
    reference: &str,
    target_platforms: &[PlatformSpec],
) -> Result<ResolvedImageIdentityV1, OciResolutionError> {
    let parsed = parse_image_reference(reference)?;
    // v6 review P0-2: applied BEFORE the first outbound request. This is the
    // actual safety boundary for this unauthenticated-beyond-project-scoping
    // endpoint -- see the module-level trust-boundary doc comment.
    if !ALLOWED_REGISTRY_HOSTS.contains(&parsed.registry_host.as_str()) {
        return Err(OciResolutionError::RegistryHostNotAllowed(parsed.registry_host.clone()));
    }
    validate_target_platforms(target_platforms)?;

    let selector_text = match &parsed.selector {
        ReferenceSelector::Tag(tag) => tag.clone(),
        ReferenceSelector::Digest(digest) => digest.clone(),
    };

    let fetched = fetch_manifest_bytes(transport, &parsed.registry_host, &parsed.repository_path, &selector_text)?;
    let computed_digest = format!("sha256:{}", sha256_hex(&fetched.body));

    // Self-verify: never trust a header or a request selector alone -- the
    // digest actually governing this content is always the one computed
    // over the exact bytes received.
    if let ReferenceSelector::Digest(requested_digest) = &parsed.selector {
        if requested_digest != &computed_digest {
            return Err(OciResolutionError::DigestMismatch {
                expected: requested_digest.clone(),
                computed: computed_digest,
            });
        }
    }
    if let Some(header_digest) = &fetched.docker_content_digest_header {
        if header_digest != &computed_digest {
            return Err(OciResolutionError::DigestMismatch {
                expected: computed_digest,
                computed: header_digest.clone(),
            });
        }
    }

    let document: Value = serde_json::from_slice(&fetched.body)
        .map_err(|error| OciResolutionError::ManifestNotFound(format!("malformed manifest JSON: {error}")))?;
    let media_type = document.get("mediaType").and_then(Value::as_str).unwrap_or("");

    let mut resolved = match media_type {
        t if t == OCI_INDEX_MEDIA_TYPE || t == DOCKER_MANIFEST_LIST_MEDIA_TYPE => {
            resolve_from_index(transport, &parsed, &document, &computed_digest, target_platforms)
        }
        t if t == OCI_MANIFEST_MEDIA_TYPE || t == DOCKER_MANIFEST_MEDIA_TYPE => {
            resolve_from_single_manifest(transport, &parsed, &document, &computed_digest, target_platforms)
        }
        other => Err(OciResolutionError::UnsupportedMediaType(other.to_string())),
    }?;

    // v6 review P1-4: `platforms` is semantically a SET of concrete platform
    // bindings, not an ordered sequence -- two pins requesting the same
    // platforms in a different order must produce byte-identical JCS output.
    // Canonicalize by sorting on (os, architecture, variant) regardless of
    // the caller's requested order.
    resolved
        .platforms
        .sort_by(|a, b| (a.os.as_str(), a.architecture.as_str(), a.variant.as_deref()).cmp(&(b.os.as_str(), b.architecture.as_str(), b.variant.as_deref())));

    Ok(resolved)
}

fn platform_from_entry(entry: &Value) -> PlatformSpec {
    let platform = entry.get("platform");
    PlatformSpec {
        os: platform.and_then(|p| p.get("os")).and_then(Value::as_str).unwrap_or("").to_string(),
        architecture: platform.and_then(|p| p.get("architecture")).and_then(Value::as_str).unwrap_or("").to_string(),
        variant: platform.and_then(|p| p.get("variant")).and_then(Value::as_str).map(str::to_string),
    }
}

fn resolve_from_index(
    transport: &dyn OciHttpTransport,
    parsed: &ParsedImageReference,
    document: &Value,
    index_digest: &str,
    target_platforms: &[PlatformSpec],
) -> Result<ResolvedImageIdentityV1, OciResolutionError> {
    let manifests = document
        .get("manifests")
        .and_then(Value::as_array)
        .ok_or_else(|| OciResolutionError::ManifestNotFound("index carried no 'manifests' array".to_string()))?;

    let available_platforms: Vec<PlatformSpec> = manifests.iter().map(platform_from_entry).collect();
    let fully_qualified_repository = parsed.fully_qualified_repository();

    let mut resolved_platforms = Vec::with_capacity(target_platforms.len());
    for requested in target_platforms {
        // v5 review P1-3: exactly one matching descriptor is required.
        // Zero is unresolvable; more than one is ambiguous and must not be
        // silently resolved by picking the first.
        let matched: Vec<&Value> = manifests.iter().filter(|entry| &platform_from_entry(entry) == requested).collect();
        let matched = match matched.len() {
            0 => {
                return Err(OciResolutionError::PlatformNotFoundInIndex {
                    requested: requested.clone(),
                    available: available_platforms.clone(),
                })
            }
            1 => matched[0],
            count => {
                return Err(OciResolutionError::AmbiguousPlatformMatchInIndex { requested: requested.clone(), count })
            }
        };

        // v5 review P1-3: verify the matched descriptor is actually an
        // image manifest before fetching it as one -- an index entry could
        // in principle carry `platform` fields on a non-manifest referrer
        // (e.g. an attestation/SBOM) that must not be blindly dereferenced.
        let entry_media_type = matched.get("mediaType").and_then(Value::as_str).unwrap_or("");
        if entry_media_type != OCI_MANIFEST_MEDIA_TYPE && entry_media_type != DOCKER_MANIFEST_MEDIA_TYPE {
            return Err(OciResolutionError::UnsupportedMediaType(entry_media_type.to_string()));
        }

        let entry_digest = matched.get("digest").and_then(Value::as_str).unwrap_or("");
        if !is_oci_digest_format(entry_digest) {
            return Err(OciResolutionError::MalformedDigest(entry_digest.to_string()));
        }

        let fetched = fetch_manifest_bytes(transport, &parsed.registry_host, &parsed.repository_path, entry_digest)?;
        let computed = format!("sha256:{}", sha256_hex(&fetched.body));
        if computed != entry_digest {
            return Err(OciResolutionError::DigestMismatch { expected: entry_digest.to_string(), computed });
        }
        if let Some(header_digest) = &fetched.docker_content_digest_header {
            if header_digest != entry_digest {
                return Err(OciResolutionError::DigestMismatch {
                    expected: entry_digest.to_string(),
                    computed: header_digest.clone(),
                });
            }
        }

        let platform_document: Value = serde_json::from_slice(&fetched.body).map_err(|error| {
            OciResolutionError::ManifestNotFound(format!("malformed platform manifest JSON: {error}"))
        })?;
        let config_digest = platform_document.get("config").and_then(|c| c.get("digest")).and_then(Value::as_str).unwrap_or("");
        if !is_oci_digest_format(config_digest) {
            return Err(OciResolutionError::MalformedDigest(config_digest.to_string()));
        }

        // v5 review P0-4: defense in depth -- the index descriptor already
        // declares `platform`, matched exactly above, but the config blob
        // is independently fetched and verified anyway rather than trusted
        // twice-removed. v6 review P2: the index descriptor is the variant
        // authority here, so an absent config-blob variant is accepted.
        verify_platform_via_config_blob(
            transport,
            &parsed.registry_host,
            &parsed.repository_path,
            config_digest,
            requested,
            VariantAuthority::IndexDescriptorAlreadyConfirmed,
        )?;

        resolved_platforms.push(ResolvedPlatformManifestV1 {
            os: requested.os.clone(),
            architecture: requested.architecture.clone(),
            variant: requested.variant.clone(),
            manifest_digest: entry_digest.to_string(),
            config_digest: config_digest.to_string(),
            execution_reference: format!("{fully_qualified_repository}@{entry_digest}"),
        });
    }

    Ok(ResolvedImageIdentityV1 {
        repository: fully_qualified_repository,
        index_digest: Some(index_digest.to_string()),
        platforms: resolved_platforms,
        resolved_at_utc: String::new(),
        resolver_tool_version: String::new(),
        resolved_by: String::new(),
        registry_host: parsed.registry_host.clone(),
    })
}

fn resolve_from_single_manifest(
    transport: &dyn OciHttpTransport,
    parsed: &ParsedImageReference,
    document: &Value,
    manifest_digest: &str,
    target_platforms: &[PlatformSpec],
) -> Result<ResolvedImageIdentityV1, OciResolutionError> {
    if target_platforms.len() != 1 {
        // A bare (non-index) manifest carries no per-platform
        // disambiguation of its own -- resolving it against more than one
        // requested platform would silently apply the same content to every
        // platform, which is exactly the ambiguity Amendment 3 forbids.
        return Err(OciResolutionError::SinglePlatformManifestPlatformCountMismatch { requested: target_platforms.len() });
    }
    let requested = &target_platforms[0];
    let config_digest = document.get("config").and_then(|c| c.get("digest")).and_then(Value::as_str).unwrap_or("");
    if !is_oci_digest_format(config_digest) {
        return Err(OciResolutionError::MalformedDigest(config_digest.to_string()));
    }

    // v5 review P0-4 (mandatory here, unlike the index path's defense in
    // depth): a bare manifest proves NOTHING about the platform on its own
    // -- the caller's `requested` platform is never trusted without
    // independently verifying the image config blob's own declared
    // os/architecture/variant. v6 review P2: no index descriptor exists on
    // this path, so the config blob is the ONLY authority -- variant is
    // compared strictly, with no leniency for an absent value.
    verify_platform_via_config_blob(
        transport,
        &parsed.registry_host,
        &parsed.repository_path,
        config_digest,
        requested,
        VariantAuthority::NoOtherAuthority,
    )?;

    let fully_qualified_repository = parsed.fully_qualified_repository();
    Ok(ResolvedImageIdentityV1 {
        repository: fully_qualified_repository.clone(),
        index_digest: None,
        platforms: vec![ResolvedPlatformManifestV1 {
            os: requested.os.clone(),
            architecture: requested.architecture.clone(),
            variant: requested.variant.clone(),
            manifest_digest: manifest_digest.to_string(),
            config_digest: config_digest.to_string(),
            execution_reference: format!("{fully_qualified_repository}@{manifest_digest}"),
        }],
        resolved_at_utc: String::new(),
        resolver_tool_version: String::new(),
        resolved_by: String::new(),
        registry_host: parsed.registry_host.clone(),
    })
}

// --- P1-5: structural validation, not merely Option::is_some() -------------

/// v5 review P1-5: `required_completeness_gaps` must VALIDATE a
/// `resolved_image_identity`, not merely check it is `Some(..)` -- after the
/// P0-1 fix (server-owned write path), this is defense in depth against a
/// bug in that write path or a tampered wire assignment, not the primary
/// trust boundary. Checks: the declared `image` reference's fully-qualified
/// repository agrees with the resolved one; all digests are valid OCI
/// digests; the platform set is non-empty and free of duplicate
/// `(os, architecture, variant)` entries; every platform's
/// `execution_reference` is exactly `"{repository}@{manifest_digest}"`.
pub fn validate_resolved_image_identity(
    image: &str,
    resolved: &ResolvedImageIdentityV1,
) -> Result<(), String> {
    let parsed = parse_image_reference(image).map_err(|error| error.to_string())?;
    let expected_repository = parsed.fully_qualified_repository();
    if resolved.repository != expected_repository {
        return Err(format!(
            "resolved_image_identity.repository '{}' does not match the declared image's repository '{expected_repository}'",
            resolved.repository
        ));
    }
    if let Some(index_digest) = &resolved.index_digest {
        if !is_oci_digest_format(index_digest) {
            return Err("resolved_image_identity.index_digest is not a valid OCI digest".to_string());
        }
    }
    if resolved.platforms.is_empty() {
        return Err("resolved_image_identity.platforms must not be empty".to_string());
    }
    // v6 review P1-3 (originally required, but missed, in the v1 review):
    // when the DECLARED image reference itself carries a digest selector
    // (repo@sha256:...), that digest must be consistent with the resolved
    // ROOT content, not just internally self-consistent among the
    // resolved fields:
    //   - if the root resolved as an index, the declared digest must equal
    //     index_digest;
    //   - if the root resolved as a single manifest (index_digest is None),
    //     the declared digest must equal the one resolved platform's
    //     manifest_digest (a bare manifest disambiguates exactly one
    //     platform by construction -- see SinglePlatformManifestPlatformCountMismatch).
    if let ReferenceSelector::Digest(declared) = &parsed.selector {
        match &resolved.index_digest {
            Some(index_digest) => {
                if declared != index_digest {
                    return Err(format!(
                        "declared digest selector '{declared}' does not match the resolved index_digest '{index_digest}'"
                    ));
                }
            }
            None => {
                if resolved.platforms.len() != 1 || &resolved.platforms[0].manifest_digest != declared {
                    return Err(format!(
                        "declared digest selector '{declared}' does not match the resolved single-manifest content"
                    ));
                }
            }
        }
    }
    // v7 review P2-3: harden this reusable validator to reject the same
    // shape problems `validate_target_platforms` already rejects on the
    // REQUEST side, so it does not silently accept a malformed platform
    // entry just because the resolver itself happens to be its only caller
    // today -- this is the general-purpose completeness validator, reused
    // by `apply_resolved_image_identity`'s defensive check and by
    // `required_completeness_gaps`, and must stay correct independent of
    // who else might call it in the future.
    let keys: Vec<(String, String, Option<String>)> = resolved
        .platforms
        .iter()
        .map(|platform| (platform.os.clone(), platform.architecture.clone(), platform.variant.clone()))
        .collect();
    let mut sorted_keys = keys.clone();
    sorted_keys.sort();
    if keys != sorted_keys {
        return Err(
            "resolved_image_identity.platforms must be in canonical (os, architecture, variant) sorted order".to_string(),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for platform in &resolved.platforms {
        if platform.os.trim().is_empty() {
            return Err("resolved_image_identity.platforms entries require a non-empty os".to_string());
        }
        if platform.architecture.trim().is_empty() {
            return Err("resolved_image_identity.platforms entries require a non-empty architecture".to_string());
        }
        if let Some(variant) = &platform.variant {
            if variant.trim().is_empty() {
                return Err(
                    "resolved_image_identity.platforms entries must not declare an explicitly-empty variant".to_string(),
                );
            }
        }
        let key = (platform.os.clone(), platform.architecture.clone(), platform.variant.clone());
        if !seen.insert(key) {
            return Err("resolved_image_identity.platforms must not contain duplicate (os, architecture, variant) entries".to_string());
        }
        if !is_oci_digest_format(&platform.manifest_digest) || !is_oci_digest_format(&platform.config_digest) {
            return Err("resolved_image_identity.platforms entries require valid OCI manifest_digest and config_digest".to_string());
        }
        let expected_reference = format!("{}@{}", resolved.repository, platform.manifest_digest);
        if platform.execution_reference != expected_reference {
            return Err(format!(
                "resolved_image_identity.platforms entries require execution_reference == '{{repository}}@{{manifest_digest}}', got '{}'",
                platform.execution_reference
            ));
        }
    }
    Ok(())
}

// --- Test-only fake transport -----------------------------------------------
//
// Promoted to module scope (rather than nested inside this file's own `mod
// tests`, unlike `ContainerRuntime`'s `FakeRuntime`) because
// `oci_image_pinning.rs`'s tests need the SAME fake transport type, not a
// second, duplicate implementation.

#[cfg(test)]
pub struct FakeOciHttpTransport {
    responses: std::cell::RefCell<HashMap<String, std::collections::VecDeque<Result<OciHttpResponse, OciResolutionError>>>>,
    calls: std::cell::RefCell<Vec<OciHttpRequest>>,
}

#[cfg(test)]
impl FakeOciHttpTransport {
    pub fn new() -> Self {
        Self { responses: std::cell::RefCell::new(HashMap::new()), calls: std::cell::RefCell::new(Vec::new()) }
    }

    /// Queue a response (or error) for the exact request URL. Multiple
    /// `queue` calls for the same URL are served in FIFO order, so a test
    /// can script "401 challenge, then a 200 body" against the SAME
    /// transport instance and URL.
    pub fn queue(&self, url: &str, response: Result<OciHttpResponse, OciResolutionError>) {
        self.responses.borrow_mut().entry(url.to_string()).or_default().push_back(response);
    }

    pub fn calls(&self) -> Vec<OciHttpRequest> {
        self.calls.borrow().clone()
    }
}

#[cfg(test)]
impl Default for FakeOciHttpTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl OciHttpTransport for FakeOciHttpTransport {
    fn execute(&self, request: &OciHttpRequest) -> Result<OciHttpResponse, OciResolutionError> {
        self.calls.borrow_mut().push(request.clone());
        match self.responses.borrow_mut().get_mut(&request.url).and_then(|queue| queue.pop_front()) {
            Some(result) => result,
            None => Err(OciResolutionError::RegistryUnreachable(format!(
                "no fake response queued for {} -- this proves no live registry is ever contacted by a test",
                request.url
            ))),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! Shared fixture-building helpers for this module's tests AND
    //! `oci_image_pinning.rs`'s tests -- both need to build self-consistent
    //! (manifest, config blob) pairs whose digests actually match.
    use super::*;

    pub fn digest_of(fill: char) -> String {
        format!("sha256:{}", fill.to_string().repeat(64))
    }

    pub fn amd64() -> PlatformSpec {
        PlatformSpec { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None }
    }

    pub fn arm64() -> PlatformSpec {
        PlatformSpec { os: "linux".to_string(), architecture: "arm64".to_string(), variant: None }
    }

    /// A real (bytes hash to their own claimed digest) image-config blob
    /// declaring exactly `platform`.
    pub fn config_blob_body(platform: &PlatformSpec) -> Vec<u8> {
        let mut obj = serde_json::json!({
            "os": platform.os,
            "architecture": platform.architecture,
        });
        if let Some(variant) = &platform.variant {
            obj["variant"] = serde_json::json!(variant);
        }
        obj.to_string().into_bytes()
    }

    pub fn config_digest_of(platform: &PlatformSpec) -> String {
        format!("sha256:{}", sha256_hex(&config_blob_body(platform)))
    }

    pub fn single_manifest_body(config_digest: &str) -> Vec<u8> {
        serde_json::json!({
            "schemaVersion": 2,
            "mediaType": DOCKER_MANIFEST_MEDIA_TYPE,
            "config": { "mediaType": "application/vnd.docker.container.image.v1+json", "digest": config_digest },
        })
        .to_string()
        .into_bytes()
    }

    pub fn ok(body: Vec<u8>) -> Result<OciHttpResponse, OciResolutionError> {
        Ok(OciHttpResponse { status: 200, headers: Vec::new(), body })
    }

    /// Queues a self-consistent single-manifest resolution at
    /// `https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2`
    /// (manifest body + its own config blob), for `platform`. Returns the
    /// expected `manifest_digest`.
    pub fn queue_mosquitto_single_manifest(transport: &FakeOciHttpTransport, platform: &PlatformSpec) -> String {
        let config_digest = config_digest_of(platform);
        let manifest_body = single_manifest_body(&config_digest);
        let manifest_digest = format!("sha256:{}", sha256_hex(&manifest_body));
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(manifest_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{config_digest}"),
            ok(config_blob_body(platform)),
        );
        manifest_digest
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn parses_the_real_catalogs_reference_shapes() {
        assert_eq!(
            parse_image_reference("eclipse-mosquitto:2").unwrap(),
            ParsedImageReference {
                registry_host: "registry-1.docker.io".to_string(),
                canonical_registry: "docker.io".to_string(),
                repository_path: "library/eclipse-mosquitto".to_string(),
                selector: ReferenceSelector::Tag("2".to_string()),
            }
        );
        assert_eq!(
            parse_image_reference("chirpstack/chirpstack:4").unwrap(),
            ParsedImageReference {
                registry_host: "registry-1.docker.io".to_string(),
                canonical_registry: "docker.io".to_string(),
                repository_path: "chirpstack/chirpstack".to_string(),
                selector: ReferenceSelector::Tag("4".to_string()),
            }
        );
        assert_eq!(
            parse_image_reference("docker.io/nginx:1.31.3-alpine3.24").unwrap(),
            ParsedImageReference {
                registry_host: "registry-1.docker.io".to_string(),
                canonical_registry: "docker.io".to_string(),
                repository_path: "library/nginx".to_string(),
                selector: ReferenceSelector::Tag("1.31.3-alpine3.24".to_string()),
            }
        );
        assert_eq!(
            parse_image_reference("ghcr.io/absmach/atom:v1.0.0").unwrap(),
            ParsedImageReference {
                registry_host: "ghcr.io".to_string(),
                canonical_registry: "ghcr.io".to_string(),
                repository_path: "absmach/atom".to_string(),
                selector: ReferenceSelector::Tag("v1.0.0".to_string()),
            }
        );
        assert_eq!(
            parse_image_reference("myregistry.example.com:5000/foo:tag").unwrap(),
            ParsedImageReference {
                registry_host: "myregistry.example.com:5000".to_string(),
                canonical_registry: "myregistry.example.com:5000".to_string(),
                repository_path: "foo".to_string(),
                selector: ReferenceSelector::Tag("tag".to_string()),
            }
        );
        let digest = digest_of('c');
        assert_eq!(
            parse_image_reference(&format!("chirpstack/chirpstack-gateway-bridge@{digest}")).unwrap(),
            ParsedImageReference {
                registry_host: "registry-1.docker.io".to_string(),
                canonical_registry: "docker.io".to_string(),
                repository_path: "chirpstack/chirpstack-gateway-bridge".to_string(),
                selector: ReferenceSelector::Digest(digest),
            }
        );
    }

    // ---- P0-3: fully-qualified repository / execution_reference -------- //
    #[test]
    fn docker_hub_resolution_carries_the_fully_qualified_docker_io_repository() {
        let transport = FakeOciHttpTransport::new();
        let manifest_digest = queue_mosquitto_single_manifest(&transport, &amd64());
        let resolved = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap();
        assert_eq!(resolved.repository, "docker.io/library/eclipse-mosquitto");
        assert_eq!(
            resolved.platforms[0].execution_reference,
            format!("docker.io/library/eclipse-mosquitto@{manifest_digest}")
        );
    }

    #[test]
    fn ghcr_resolution_carries_the_fully_qualified_ghcr_io_repository() {
        let transport = FakeOciHttpTransport::new();
        let config_digest = config_digest_of(&amd64());
        let manifest_body = single_manifest_body(&config_digest);
        let manifest_digest = format!("sha256:{}", sha256_hex(&manifest_body));
        transport.queue("https://ghcr.io/v2/absmach/atom/manifests/v1.0.0", ok(manifest_body));
        transport.queue(&format!("https://ghcr.io/v2/absmach/atom/blobs/{config_digest}"), ok(config_blob_body(&amd64())));

        let resolved = resolve_image_identity(&transport, "ghcr.io/absmach/atom:v1.0.0", &[amd64()]).unwrap();
        assert_eq!(resolved.repository, "ghcr.io/absmach/atom");
        assert_eq!(resolved.platforms[0].execution_reference, format!("ghcr.io/absmach/atom@{manifest_digest}"));
    }

    #[test]
    fn custom_registry_host_is_preserved_in_the_fully_qualified_repository() {
        // v6 review P0-2: `myregistry.example.com:5000` is not in
        // ALLOWED_REGISTRY_HOSTS, so an end-to-end `resolve_image_identity`
        // call against it is correctly refused (see
        // `disallowed_registry_host_is_refused_before_any_outbound_request`
        // below) -- this test is scoped to `parse_image_reference` alone,
        // proving the P0-3 fully-qualified-repository CONSTRUCTION logic
        // handles an arbitrary custom host correctly, independent of the
        // separate allow-list policy decision.
        let parsed = parse_image_reference("myregistry.example.com:5000/foo:tag").unwrap();
        assert_eq!(parsed.fully_qualified_repository(), "myregistry.example.com:5000/foo");
        let manifest_digest = format!("sha256:{}", "1".repeat(64));
        assert_eq!(
            format!("{}@{manifest_digest}", parsed.fully_qualified_repository()),
            format!("myregistry.example.com:5000/foo@{manifest_digest}")
        );
    }

    // ---- test 1: single-manifest resolution ---------------------------- //
    #[test]
    fn single_manifest_resolution_without_index() {
        let transport = FakeOciHttpTransport::new();
        let manifest_digest = queue_mosquitto_single_manifest(&transport, &amd64());

        let resolved = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap();
        assert_eq!(resolved.repository, "docker.io/library/eclipse-mosquitto");
        assert_eq!(resolved.index_digest, None);
        assert_eq!(resolved.platforms.len(), 1);
        assert_eq!(resolved.platforms[0].manifest_digest, manifest_digest);
        assert_eq!(resolved.platforms[0].config_digest, config_digest_of(&amd64()));
    }

    // ---- test 2: multi-arch index -> correct platform selected --------- //
    #[test]
    fn multi_arch_index_selects_the_correct_platform_manifest() {
        let transport = FakeOciHttpTransport::new();
        let amd64_config_digest = config_digest_of(&amd64());
        let arm64_config_digest = config_digest_of(&arm64());
        let amd64_body = single_manifest_body(&amd64_config_digest);
        let arm64_body = single_manifest_body(&arm64_config_digest);
        let amd64_digest = format!("sha256:{}", sha256_hex(&amd64_body));
        let arm64_digest = format!("sha256:{}", sha256_hex(&arm64_body));

        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": amd64_digest, "platform": { "os": "linux", "architecture": "amd64" } },
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": arm64_digest, "platform": { "os": "linux", "architecture": "arm64" } },
            ],
        })
        .to_string()
        .into_bytes();
        let expected_index_digest = format!("sha256:{}", sha256_hex(&index_body));

        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(index_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{amd64_digest}"),
            ok(amd64_body),
        );
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{amd64_config_digest}"),
            ok(config_blob_body(&amd64())),
        );

        let resolved = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap();
        assert_eq!(resolved.index_digest, Some(expected_index_digest));
        assert_eq!(resolved.platforms.len(), 1);
        assert_eq!(resolved.platforms[0].manifest_digest, amd64_digest);
        assert_eq!(resolved.platforms[0].config_digest, amd64_config_digest);

        // The arm64 manifest was never fetched -- only queried entries the
        // resolver actually matched should ever hit the transport.
        let arm64_url = format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{arm64_digest}");
        assert!(!transport.calls().iter().any(|call| call.url == arm64_url));
    }

    // ---- test 3: platform not found in index -> fail closed ------------ //
    #[test]
    fn platform_not_found_in_index_fails_closed_without_silent_fallback() {
        let transport = FakeOciHttpTransport::new();
        let amd64_config_digest = config_digest_of(&amd64());
        let amd64_body = single_manifest_body(&amd64_config_digest);
        let amd64_digest = format!("sha256:{}", sha256_hex(&amd64_body));
        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": amd64_digest, "platform": { "os": "linux", "architecture": "amd64" } },
            ],
        })
        .to_string()
        .into_bytes();
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(index_body));

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[arm64()]).unwrap_err();
        match error {
            OciResolutionError::PlatformNotFoundInIndex { requested, available } => {
                assert_eq!(requested.architecture, "arm64");
                assert_eq!(available.len(), 1);
                assert_eq!(available[0].architecture, "amd64");
            }
            other => panic!("expected PlatformNotFoundInIndex, got {other:?}"),
        }
    }

    // ---- P1-3: ambiguous index match -> fail closed --------------------- //
    #[test]
    fn duplicate_platform_entries_in_index_are_ambiguous_and_fail_closed() {
        let transport = FakeOciHttpTransport::new();
        let config_digest_a = config_digest_of(&amd64());
        let body_a = single_manifest_body(&config_digest_a);
        let digest_a = format!("sha256:{}", sha256_hex(&body_a));
        // A SECOND, distinct manifest digest that happens to declare the
        // SAME (os, architecture) as the first -- a malformed/duplicated
        // index the resolver must refuse to disambiguate by guessing.
        let config_digest_b = format!("sha256:{}", "9".repeat(64));
        let body_b = single_manifest_body(&config_digest_b);
        let digest_b = format!("sha256:{}", sha256_hex(&body_b));
        assert_ne!(digest_a, digest_b);

        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": digest_a, "platform": { "os": "linux", "architecture": "amd64" } },
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": digest_b, "platform": { "os": "linux", "architecture": "amd64" } },
            ],
        })
        .to_string()
        .into_bytes();
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(index_body));

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::AmbiguousPlatformMatchInIndex { count: 2, .. }), "{error:?}");
    }

    #[test]
    fn index_entry_with_a_non_manifest_media_type_is_refused() {
        let transport = FakeOciHttpTransport::new();
        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [
                {
                    "mediaType": "application/vnd.oci.image.manifest.v1+json+attestation-not-a-real-manifest-type",
                    "digest": format!("sha256:{}", "1".repeat(64)),
                    "platform": { "os": "linux", "architecture": "amd64" },
                },
            ],
        })
        .to_string()
        .into_bytes();
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(index_body));

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::UnsupportedMediaType(_)), "{error:?}");
    }

    // ---- test 4: malformed digest -> reject ----------------------------- //
    #[test]
    fn malformed_config_digest_is_rejected() {
        let transport = FakeOciHttpTransport::new();
        let body = single_manifest_body("sha256:not-valid-hex");
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(body));

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::MalformedDigest(_)), "{error:?}");
    }

    // ---- test 5: anonymous bearer challenge/response flow --------------- //
    #[test]
    fn anonymous_bearer_challenge_flow_is_completed_before_the_manifest_is_returned() {
        let transport = FakeOciHttpTransport::new();
        let manifest_url = "https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2";
        transport.queue(
            manifest_url,
            Ok(OciHttpResponse {
                status: 401,
                headers: vec![(
                    "www-authenticate".to_string(),
                    "Bearer realm=\"https://auth.docker.io/token\",service=\"registry.docker.io\",scope=\"repository:library/eclipse-mosquitto:pull\""
                        .to_string(),
                )],
                body: Vec::new(),
            }),
        );
        let token_url = "https://auth.docker.io/token?service=registry.docker.io&scope=repository:library/eclipse-mosquitto:pull";
        transport.queue(token_url, ok(br#"{"token":"anonymous-test-token"}"#.to_vec()));
        let config_digest = config_digest_of(&amd64());
        let manifest_body = single_manifest_body(&config_digest);
        transport.queue(manifest_url, ok(manifest_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{config_digest}"),
            ok(config_blob_body(&amd64())),
        );

        let resolved = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]);
        assert!(resolved.is_ok(), "{resolved:?}");

        let calls = transport.calls();
        let retried = calls
            .iter()
            .rev()
            .find(|call| call.url == manifest_url && call.headers.iter().any(|(name, _)| name == "Authorization"))
            .expect("a retried, authorized manifest request is expected");
        assert!(
            retried.headers.iter().any(|(name, value)| name == "Authorization" && value == "Bearer anonymous-test-token"),
            "{retried:?}"
        );
    }

    #[test]
    fn a_non_https_bearer_realm_is_refused() {
        let transport = FakeOciHttpTransport::new();
        let manifest_url = "https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2";
        transport.queue(
            manifest_url,
            Ok(OciHttpResponse {
                status: 401,
                headers: vec![(
                    "www-authenticate".to_string(),
                    "Bearer realm=\"http://attacker.example/token\",service=\"registry.docker.io\",scope=\"pull\"".to_string(),
                )],
                body: Vec::new(),
            }),
        );

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::AuthenticationChallengeFailed(_)), "{error:?}");
    }

    // ---- test 6: no test ever contacts a live registry ------------------ //
    #[test]
    fn resolution_with_no_queued_fake_response_fails_closed_and_never_touches_the_network() {
        let transport = FakeOciHttpTransport::new();
        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::RegistryUnreachable(_)), "{error:?}");
    }

    #[test]
    fn single_manifest_resolution_requires_exactly_one_target_platform() {
        let transport = FakeOciHttpTransport::new();
        let config_digest = config_digest_of(&amd64());
        let body = single_manifest_body(&config_digest);
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(body));

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64(), arm64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::SinglePlatformManifestPlatformCountMismatch { requested: 2 }), "{error:?}");
    }

    // ---- P0-4: single-manifest platform must be verified, not assumed --- //
    #[test]
    fn single_manifest_actual_amd64_and_requested_amd64_succeeds() {
        let transport = FakeOciHttpTransport::new();
        queue_mosquitto_single_manifest(&transport, &amd64());
        assert!(resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).is_ok());
    }

    #[test]
    fn single_manifest_actual_amd64_but_requested_arm64_fails_closed() {
        let transport = FakeOciHttpTransport::new();
        // The config blob actually declares amd64; the caller asks for arm64.
        queue_mosquitto_single_manifest(&transport, &amd64());
        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[arm64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::PlatformMismatch { .. }), "{error:?}");
    }

    #[test]
    fn single_manifest_config_blob_digest_mismatch_fails_closed() {
        let transport = FakeOciHttpTransport::new();
        let claimed_config_digest = config_digest_of(&amd64());
        let manifest_body = single_manifest_body(&claimed_config_digest);
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(manifest_body));
        // Serve a DIFFERENT blob than the one whose digest was claimed.
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{claimed_config_digest}"),
            ok(config_blob_body(&arm64())),
        );
        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::DigestMismatch { .. }), "{error:?}");
    }

    #[test]
    fn single_manifest_malformed_config_json_fails_closed() {
        let transport = FakeOciHttpTransport::new();
        let malformed = b"not json".to_vec();
        let config_digest = format!("sha256:{}", sha256_hex(&malformed));
        let manifest_body = single_manifest_body(&config_digest);
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(manifest_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{config_digest}"),
            ok(malformed),
        );
        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::MalformedImageConfig(_)), "{error:?}");
    }

    #[test]
    fn single_manifest_variant_mismatch_fails_closed_when_variant_is_present() {
        let transport = FakeOciHttpTransport::new();
        let requested = PlatformSpec { os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v7".to_string()) };
        let actual = PlatformSpec { os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v6".to_string()) };
        let config_digest = config_digest_of(&actual);
        let manifest_body = single_manifest_body(&config_digest);
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(manifest_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{config_digest}"),
            ok(config_blob_body(&actual)),
        );
        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[requested]).unwrap_err();
        assert!(matches!(error, OciResolutionError::PlatformMismatch { .. }), "{error:?}");
    }

    // ---- P1-2: tag@digest reference parsing ----------------------------- //
    #[test]
    fn tag_and_digest_combined_reference_does_not_leak_the_tag_into_the_repository() {
        let digest = digest_of('7');
        let parsed = parse_image_reference(&format!("eclipse-mosquitto:2@{digest}")).unwrap();
        assert_eq!(parsed.repository_path, "library/eclipse-mosquitto");
        assert_eq!(parsed.selector, ReferenceSelector::Digest(digest));
    }

    // ---- P1-5: structural validation, not Option::is_some() ------------- //
    #[test]
    fn validate_resolved_image_identity_accepts_a_consistent_identity() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_ok());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_a_repository_mismatch() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "ghcr.io/absmach/atom".to_string(), // does not match the declared image below
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("ghcr.io/absmach/atom@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_a_bare_unqualified_execution_reference() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                // Missing the "docker.io/" qualifier -- must be rejected.
                execution_reference: format!("library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_duplicate_platform_entries() {
        let platform = ResolvedPlatformManifestV1 {
            os: "linux".to_string(),
            architecture: "amd64".to_string(),
            variant: None,
            manifest_digest: digest_of('1'),
            config_digest: digest_of('2'),
            execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
        };
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![platform.clone(), platform],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    // ---- v7 review P2-3: hardened validate_resolved_image_identity ---- //
    #[test]
    fn validate_resolved_image_identity_rejects_an_empty_platform_os() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_an_empty_platform_architecture() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "  ".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_an_explicitly_empty_variant() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "arm64".to_string(),
                variant: Some("".to_string()),
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_a_non_canonically_ordered_platform_set() {
        // amd64 then arm64 would be the CANONICAL (sorted) order; this
        // candidate presents them reversed and must be rejected, mirroring
        // the ordering `resolve_image_identity` itself always produces (see
        // `resolved_platform_order_is_canonicalized_regardless_of_requested_order`).
        let repository = "docker.io/library/eclipse-mosquitto".to_string();
        let resolved = ResolvedImageIdentityV1 {
            repository: repository.clone(),
            index_digest: None,
            platforms: vec![
                ResolvedPlatformManifestV1 {
                    os: "linux".to_string(),
                    architecture: "arm64".to_string(),
                    variant: None,
                    manifest_digest: digest_of('1'),
                    config_digest: digest_of('2'),
                    execution_reference: format!("{repository}@{}", digest_of('1')),
                },
                ResolvedPlatformManifestV1 {
                    os: "linux".to_string(),
                    architecture: "amd64".to_string(),
                    variant: None,
                    manifest_digest: digest_of('3'),
                    config_digest: digest_of('4'),
                    execution_reference: format!("{repository}@{}", digest_of('3')),
                },
            ],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        assert!(validate_resolved_image_identity("eclipse-mosquitto:2", &resolved).is_err());
    }

    // ---- v7 review P1-3: redirect policy -------------------------------- //
    #[test]
    fn redirect_to_an_https_url_within_the_hop_bound_is_allowed() {
        assert!(evaluate_oci_redirect(0, "https://production.cloudflare.docker.com/blob/abc").is_ok());
        assert!(evaluate_oci_redirect(MAX_OCI_REDIRECTS - 1, "https://objects.githubusercontent.com/blob/abc").is_ok());
    }

    #[test]
    fn redirect_exceeding_the_hop_bound_is_rejected() {
        let error = evaluate_oci_redirect(MAX_OCI_REDIRECTS, "https://cdn.example.com/blob").unwrap_err();
        assert!(matches!(error, OciResolutionError::RedirectRejected(_)), "{error:?}");
    }

    #[test]
    fn redirect_to_a_plain_http_url_is_rejected_regardless_of_hop_count() {
        let error = evaluate_oci_redirect(0, "http://cdn.example.com/blob").unwrap_err();
        assert!(matches!(error, OciResolutionError::RedirectRejected(_)), "{error:?}");
    }

    #[test]
    fn redirect_to_a_malformed_location_with_no_extractable_host_is_rejected() {
        let error = evaluate_oci_redirect(0, "https://").unwrap_err();
        assert!(matches!(error, OciResolutionError::RedirectRejected(_)), "{error:?}");
    }

    #[test]
    fn redirect_target_host_is_not_re_checked_against_the_registry_or_token_allow_lists() {
        // Deliberate design choice (v7 P1-3, option B): a redirect issued by
        // an already-allowed registry/token host is followed to ANY https
        // host within the hop bound -- the target is provider-controlled
        // (e.g. a CDN), not catalog-controlled, and is not itself required
        // to appear in ALLOWED_REGISTRY_HOSTS / ALLOWED_TOKEN_REALM_HOSTS.
        assert!(!ALLOWED_REGISTRY_HOSTS.contains(&"production.cloudflare.docker.com"));
        assert!(!ALLOWED_TOKEN_REALM_HOSTS.contains(&"production.cloudflare.docker.com"));
        assert!(evaluate_oci_redirect(0, "https://production.cloudflare.docker.com/blob/abc").is_ok());
    }

    // ---- v6 review P0-2: registry-host / token-realm-host allow-list --- //
    #[test]
    fn disallowed_registry_host_is_refused_before_any_outbound_request() {
        let transport = FakeOciHttpTransport::new();
        let error = resolve_image_identity(&transport, "attacker.example.com/foo:tag", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::RegistryHostNotAllowed(ref host) if host == "attacker.example.com"), "{error:?}");
        assert!(transport.calls().is_empty(), "a disallowed registry host must never reach the transport at all");
    }

    #[test]
    fn disallowed_token_realm_host_is_refused() {
        let transport = FakeOciHttpTransport::new();
        let manifest_url = "https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2";
        transport.queue(
            manifest_url,
            Ok(OciHttpResponse {
                status: 401,
                headers: vec![(
                    "www-authenticate".to_string(),
                    "Bearer realm=\"https://attacker.example/token\",service=\"registry.docker.io\",scope=\"pull\"".to_string(),
                )],
                body: Vec::new(),
            }),
        );
        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[amd64()]).unwrap_err();
        assert!(matches!(error, OciResolutionError::TokenRealmHostNotAllowed(ref host) if host == "attacker.example"), "{error:?}");
    }

    // ---- v6 review P1-1: bounded body reader (transport-agnostic) ------ //
    #[test]
    fn bounded_read_accepts_a_body_of_exactly_the_limit() {
        let body = vec![b'x'; 10];
        let result = read_bounded_body(std::io::Cursor::new(body.clone()), 10).unwrap();
        assert_eq!(result, body);
    }

    #[test]
    fn bounded_read_rejects_limit_plus_one() {
        let body = vec![b'x'; 11];
        let error = read_bounded_body(std::io::Cursor::new(body), 10).unwrap_err();
        assert!(matches!(error, OciResolutionError::ResponseTooLarge { bytes: 11 }), "{error:?}");
    }

    #[test]
    fn bounded_read_is_immune_to_a_dishonest_declared_length_because_it_never_consults_one() {
        // read_bounded_body takes no declared-length/Content-Length
        // parameter at all -- there is structurally no way for an
        // externally-declared value (honest or not) to influence its
        // behavior. A stream far larger than the limit, with nothing
        // "declaring" anything, is still rejected purely from its own byte
        // count -- exactly the property that closes the "chunked response
        // with no/lying Content-Length" gap the reviewer flagged.
        let body = vec![b'x'; 1_000_000];
        let error = read_bounded_body(std::io::Cursor::new(body), 10).unwrap_err();
        assert!(matches!(error, OciResolutionError::ResponseTooLarge { bytes: 11 }), "{error:?}");
    }

    #[test]
    fn bounded_read_never_allocates_more_than_limit_plus_one_regardless_of_stream_length() {
        let body = vec![b'x'; 5_000_000];
        let result = read_bounded_body(std::io::Cursor::new(body), 10);
        // Whether accepted or rejected is not the point of this test (a
        // 5,000,000-byte stream against a 10-byte limit is rejected); the
        // point is that `Read::take` bounds what is ever read into `buf` to
        // at most limit + 1 bytes, which the ResponseTooLarge{bytes: 11}
        // value itself proves (if the whole 5,000,000 bytes had been
        // buffered first, `bytes` would report 5,000,000, not 11).
        match result {
            Err(OciResolutionError::ResponseTooLarge { bytes }) => assert_eq!(bytes, 11),
            other => panic!("expected ResponseTooLarge{{bytes: 11}}, got {other:?}"),
        }
    }

    // ---- v6 review P1-2: target platform request validation ------------ //
    #[test]
    fn target_platform_with_empty_os_is_rejected() {
        let platform = PlatformSpec { os: String::new(), architecture: "amd64".to_string(), variant: None };
        assert!(matches!(validate_target_platforms(&[platform]), Err(OciResolutionError::InvalidTargetPlatform(_))));
    }

    #[test]
    fn target_platform_with_empty_architecture_is_rejected() {
        let platform = PlatformSpec { os: "linux".to_string(), architecture: String::new(), variant: None };
        assert!(matches!(validate_target_platforms(&[platform]), Err(OciResolutionError::InvalidTargetPlatform(_))));
    }

    #[test]
    fn target_platform_with_empty_string_variant_is_rejected() {
        let platform = PlatformSpec { os: "linux".to_string(), architecture: "arm".to_string(), variant: Some(String::new()) };
        assert!(matches!(validate_target_platforms(&[platform]), Err(OciResolutionError::InvalidTargetPlatform(_))));
    }

    #[test]
    fn duplicate_requested_platforms_are_rejected() {
        assert!(matches!(validate_target_platforms(&[amd64(), amd64()]), Err(OciResolutionError::InvalidTargetPlatform(_))));
    }

    #[test]
    fn too_many_target_platforms_is_rejected() {
        let platforms: Vec<PlatformSpec> = (0..17)
            .map(|i| PlatformSpec { os: "linux".to_string(), architecture: format!("arch{i}"), variant: None })
            .collect();
        assert!(matches!(validate_target_platforms(&platforms), Err(OciResolutionError::InvalidTargetPlatform(_))));
    }

    #[test]
    fn a_valid_target_platform_list_is_accepted() {
        assert!(validate_target_platforms(&[amd64(), arm64()]).is_ok());
    }

    // ---- v6 review P1-3: digest-selector consistency -------------------- //
    #[test]
    fn validate_resolved_image_identity_accepts_a_digest_selector_matching_the_index_digest() {
        let index_digest = digest_of('3');
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: Some(index_digest.clone()),
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        let image = format!("eclipse-mosquitto@{index_digest}");
        assert!(validate_resolved_image_identity(&image, &resolved).is_ok());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_a_digest_selector_not_matching_the_index_digest() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: Some(digest_of('3')),
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        // Declared digest ('9') disagrees with the resolved index_digest ('3').
        let image = format!("eclipse-mosquitto@{}", digest_of('9'));
        assert!(validate_resolved_image_identity(&image, &resolved).is_err());
    }

    #[test]
    fn validate_resolved_image_identity_accepts_a_digest_selector_matching_the_single_manifest_digest() {
        let manifest_digest = digest_of('1');
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: manifest_digest.clone(),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{manifest_digest}"),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        let image = format!("eclipse-mosquitto@{manifest_digest}");
        assert!(validate_resolved_image_identity(&image, &resolved).is_ok());
    }

    #[test]
    fn validate_resolved_image_identity_rejects_a_digest_selector_not_matching_the_single_manifest_digest() {
        let resolved = ResolvedImageIdentityV1 {
            repository: "docker.io/library/eclipse-mosquitto".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: digest_of('1'),
                config_digest: digest_of('2'),
                execution_reference: format!("docker.io/library/eclipse-mosquitto@{}", digest_of('1')),
            }],
            resolved_at_utc: String::new(),
            resolver_tool_version: String::new(),
            resolved_by: String::new(),
            registry_host: "registry-1.docker.io".to_string(),
        };
        let image = format!("eclipse-mosquitto@{}", digest_of('9'));
        assert!(validate_resolved_image_identity(&image, &resolved).is_err());
    }

    // ---- v6 review P1-4: platform-set ordering is canonical ------------- //
    #[test]
    fn resolved_platform_order_is_canonicalized_regardless_of_requested_order() {
        let amd64_config_digest = config_digest_of(&amd64());
        let arm64_config_digest = config_digest_of(&arm64());
        let amd64_body = single_manifest_body(&amd64_config_digest);
        let arm64_body = single_manifest_body(&arm64_config_digest);
        let amd64_digest = format!("sha256:{}", sha256_hex(&amd64_body));
        let arm64_digest = format!("sha256:{}", sha256_hex(&arm64_body));
        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": amd64_digest, "platform": { "os": "linux", "architecture": "amd64" } },
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": arm64_digest, "platform": { "os": "linux", "architecture": "arm64" } },
            ],
        })
        .to_string()
        .into_bytes();

        let build = |target_platforms: &[PlatformSpec]| {
            let transport = FakeOciHttpTransport::new();
            transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(index_body.clone()));
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{amd64_digest}"),
                ok(amd64_body.clone()),
            );
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{arm64_digest}"),
                ok(arm64_body.clone()),
            );
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{amd64_config_digest}"),
                ok(config_blob_body(&amd64())),
            );
            transport.queue(
                &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{arm64_config_digest}"),
                ok(config_blob_body(&arm64())),
            );
            resolve_image_identity(&transport, "eclipse-mosquitto:2", target_platforms).unwrap()
        };

        let a_then_b = build(&[amd64(), arm64()]);
        let b_then_a = build(&[arm64(), amd64()]);
        assert_eq!(
            a_then_b.platforms, b_then_a.platforms,
            "requesting the same platform set in a different order must produce identically-ordered resolved platforms"
        );
    }

    // ---- v6 review P2: variant policy ----------------------------------- //
    #[test]
    fn index_path_accepts_a_config_blob_that_omits_variant_when_the_index_already_confirmed_it() {
        let requested = PlatformSpec { os: "linux".to_string(), architecture: "arm64".to_string(), variant: Some("v8".to_string()) };
        // Config blob declares os/architecture but NO variant key at all --
        // realistic real-world behavior for many published images.
        let config_body_no_variant = serde_json::json!({ "os": "linux", "architecture": "arm64" }).to_string().into_bytes();
        let config_digest = format!("sha256:{}", sha256_hex(&config_body_no_variant));
        let manifest_body = single_manifest_body(&config_digest);
        let manifest_digest = format!("sha256:{}", sha256_hex(&manifest_body));
        let index_body = serde_json::json!({
            "schemaVersion": 2,
            "mediaType": OCI_INDEX_MEDIA_TYPE,
            "manifests": [
                { "mediaType": DOCKER_MANIFEST_MEDIA_TYPE, "digest": manifest_digest, "platform": { "os": "linux", "architecture": "arm64", "variant": "v8" } },
            ],
        })
        .to_string()
        .into_bytes();

        let transport = FakeOciHttpTransport::new();
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(index_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/{manifest_digest}"),
            ok(manifest_body),
        );
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{config_digest}"),
            ok(config_body_no_variant),
        );

        let resolved = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[requested]).unwrap();
        assert_eq!(resolved.platforms[0].architecture, "arm64");
    }

    #[test]
    fn single_manifest_path_rejects_an_absent_config_variant_when_requested_is_some() {
        let requested = PlatformSpec { os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v7".to_string()) };
        // No index exists on this path -- the config blob is the ONLY
        // authority, so an absent variant must NOT be leniently accepted.
        let config_body_no_variant = serde_json::json!({ "os": "linux", "architecture": "arm" }).to_string().into_bytes();
        let config_digest = format!("sha256:{}", sha256_hex(&config_body_no_variant));
        let manifest_body = single_manifest_body(&config_digest);
        let transport = FakeOciHttpTransport::new();
        transport.queue("https://registry-1.docker.io/v2/library/eclipse-mosquitto/manifests/2", ok(manifest_body));
        transport.queue(
            &format!("https://registry-1.docker.io/v2/library/eclipse-mosquitto/blobs/{config_digest}"),
            ok(config_body_no_variant),
        );

        let error = resolve_image_identity(&transport, "eclipse-mosquitto:2", &[requested]).unwrap_err();
        assert!(matches!(error, OciResolutionError::PlatformMismatch { .. }), "{error:?}");
    }
}
