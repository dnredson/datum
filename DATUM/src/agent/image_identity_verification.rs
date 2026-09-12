//! Stage 107B-B (ADR-0014): runtime image identity verification.
//!
//! Given a server-resolved, pinned `OperationalResolvedImageIdentity`
//! (Stage 107B-A) and the local Docker daemon's own reported state, decides
//! whether the *actual local content* this node would execute genuinely
//! matches the *authorized immutable identity* DServer resolved before
//! authorization -- never merely whether a tag string looks right.
//!
//! Deliberately narrow and Docker-only: this module knows nothing about
//! leases, mutation boundaries (M1-M8), or `execution_pinning_mode`. Those
//! remain entirely owned by `operational_reconciliation.rs`, which
//! orchestrates calls into this module and decides, from the *outcome*, when
//! a lease-gated `docker pull` (M5) may proceed and whether a mismatch/
//! unverified outcome blocks in the current effective mode. Digest domains
//! are never conflated here: `manifest_digest`/`config_digest` are OCI
//! content digests (`sha256:<64 lowercase hex>`), validated with the exact
//! same `is_oci_digest_format` the rest of Stage 107B-A/B uses -- never
//! re-prefixed, so a value that already carries `sha256:` is never turned
//! into `sha256:sha256:<hex>`.
//!
//! Testability: mirrors this exact codebase's own established pattern for
//! `operational_reconciliation.rs` (pure comparison functions fed synthetic
//! `serde_json::Value`, e.g. `inspected_container_matches_declared_attachments`)
//! rather than introducing the heavier trait/fake pattern used elsewhere
//! (`ContainerRuntime`/`FakeRuntime`, `OciHttpTransport`/`FakeOciHttpTransport`).
//! The one exception is `DockerImageRuntime`, a narrow interface (inspect
//! image / pull image / server platform only) needed because these three
//! operations combine a Docker invocation with a decision (pull-or-not,
//! selection) that must be exercised deterministically without a live
//! daemon; every other function in this module is a pure comparison over
//! already-fetched data, unit-tested directly.

use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::operational_reconciliation::{
    OperationalResolvedImageIdentity, OperationalResolvedPlatformManifest,
};
use crate::agent::stage107a_enforcement::is_oci_digest_format;

// ---------------------------------------------------------------------- //
// reason_kind vocabulary (structured detail, never a separate top-level
// stable code -- see ADR-0014 / stage107a_enforcement's two new
// BLOCKED_IMAGE_IDENTITY_* constants).
// ---------------------------------------------------------------------- //
pub const REASON_NO_MATCHING_PLATFORM: &str = "no_matching_platform";
pub const REASON_AMBIGUOUS_PLATFORM: &str = "ambiguous_platform";
pub const REASON_INSPECT_UNAVAILABLE: &str = "inspect_unavailable";
pub const REASON_MANIFEST_MISMATCH: &str = "manifest_mismatch";
pub const REASON_CONFIG_MISMATCH: &str = "config_mismatch";
pub const REASON_PLATFORM_MISMATCH: &str = "platform_mismatch";
pub const REASON_CONTAINER_BINDING_MISMATCH: &str = "container_binding_mismatch";
/// v8 review P1-1: a container is known to exist (Running/Stopped), but its
/// image-binding evidence could not be obtained (inspect command failure,
/// non-zero exit, malformed JSON, or a missing `.Image` field) -- distinct
/// from `NotApplicable` (no container exists at all, e.g. Missing state).
/// Never collapsed into "nothing to verify"; always surfaces as unverified.
pub const REASON_CONTAINER_BINDING_UNAVAILABLE: &str = "container_binding_unavailable";

// ---------------------------------------------------------------------- //
// pull_disposition vocabulary. v8 review P1-3: these are MECHANICAL FACTS
// about what was done (was the image already present, or was a pull
// performed), never a verification-outcome claim -- `verified`/`reason_kind`
// on the same evidence record carry the actual result, so evidence can never
// contradict itself (e.g. `pull_disposition: already_present_verified` next
// to `verified: false` in the v1 bundle was exactly this contradiction).
// ---------------------------------------------------------------------- //
pub const PULL_DISPOSITION_ALREADY_PRESENT: &str = "already_present";
pub const PULL_DISPOSITION_PULL_PERFORMED: &str = "pull_performed";
pub const PULL_DISPOSITION_NOT_APPLICABLE_UNPINNED_SOURCE: &str = "not_applicable_unpinned_source";

// ---------------------------------------------------------------------- //
// v9 review P1-4: reason_kind values for artifacts this module was never
// asked to verify at all -- distinct from every "we tried and could not
// confirm" reason above. Their presence lets an observer (and, later,
// D-Monitor/GNN evidence consumers) distinguish "verified" from "pinnable
// but unverified" from "not even pinnable" without ever inferring the
// answer from an ABSENT field, which would be misreadable as "nothing was
// checked because everything was fine."
// ---------------------------------------------------------------------- //
pub const REASON_LOCAL_BUILD_NOT_PINNABLE: &str = "local_build_not_pinnable";
pub const REASON_LEGACY_UNCLASSIFIED_NOT_PINNABLE: &str = "legacy_unclassified_not_pinnable";
pub const REASON_REGISTRY_UNRESOLVED: &str = "registry_unresolved";
pub const REASON_REGISTRY_PIN_STRUCTURALLY_INVALID: &str = "registry_pin_structurally_invalid";

// ---------------------------------------------------------------------- //
// v10 review P1-1: explicit, string-valued EXISTING CONTAINER binding
// status carried on the serialized evidence itself -- so a downstream
// consumer (D-Monitor snapshots, later GNN training) can distinguish these
// four cases WITHOUT inferring anything from an absent
// `actual_container_image` field, which could otherwise mean "not
// applicable," "inspection failed," or "container disappeared between
// observations" indistinguishably. Mirrors `ContainerBindingObservation`
// one-to-one (`ContainerBindingObservation::as_status_str` is the single
// place that maps one to the other).
// ---------------------------------------------------------------------- //
pub const CONTAINER_BINDING_STATUS_NOT_APPLICABLE: &str = "not_applicable";
pub const CONTAINER_BINDING_STATUS_MATCHES: &str = "matches";
pub const CONTAINER_BINDING_STATUS_MISMATCHES: &str = "mismatches";
pub const CONTAINER_BINDING_STATUS_UNAVAILABLE: &str = "unavailable";

// ---------------------------------------------------------------------- //
// Evidence (runtime-only; must never enter ArtifactExecutionProjectionV1 --
// see ADR-0014's evidence-isolation section and the regression test in
// `execution_projection.rs` that proves it).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedImageIdentity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_execution_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_manifest_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorized_config_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_platform: Option<String>,
    #[serde(default)]
    pub actual_repo_digests: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_image_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_container_image: Option<String>,
    /// TARGET IMAGE verification only (manifest/config/platform) --
    /// renamed from `verified` (v10 review P1-1) specifically so it can
    /// never be misread as covering EXISTING CONTAINER binding too. See
    /// `container_binding_status` for that separate dimension, and
    /// `runtime_identity_verified()` for the derived combination of both.
    pub target_verified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_kind: Option<String>,
    /// EXISTING CONTAINER binding observation, as an explicit string
    /// (v10 review P1-1) -- one of `CONTAINER_BINDING_STATUS_NOT_APPLICABLE`
    /// / `_MATCHES` / `_MISMATCHES` / `_UNAVAILABLE`. ALWAYS set, never
    /// left to be inferred from `actual_container_image` being absent
    /// (which could otherwise mean "not applicable," "inspection failed,"
    /// or "container disappeared between observations" indistinguishably).
    pub container_binding_status: String,
    /// Evidence-only (ADR-0014): never a factor in any content-identity
    /// decision, and must never perturb `execution_projection_digest` /
    /// `execution_generation`.
    pub checked_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_disposition: Option<String>,
}

impl VerifiedImageIdentity {
    /// Convenience only -- v10 review P1-1 explicitly warns against
    /// "confusing duplicate truth fields," so this is a derived METHOD,
    /// never a stored/serialized field: true only when the TARGET IMAGE
    /// verified AND the EXISTING CONTAINER binding is proven to match it.
    /// `false` for every other combination, including "not applicable"
    /// (an unpinned/local_build/legacy_unclassified source, or a Missing
    /// container, has no runtime container-binding proof to combine with
    /// target verification at all).
    pub fn runtime_identity_verified(&self) -> bool {
        self.target_verified && self.container_binding_status == CONTAINER_BINDING_STATUS_MATCHES
    }
}

fn empty_evidence(reason_kind: &str, checked_at_utc: &str) -> VerifiedImageIdentity {
    VerifiedImageIdentity {
        authorized_execution_reference: None,
        authorized_manifest_digest: None,
        authorized_config_digest: None,
        selected_platform: None,
        actual_repo_digests: Vec::new(),
        actual_image_id: None,
        actual_container_image: None,
        target_verified: false,
        reason_kind: Some(reason_kind.to_string()),
        // Every call site of `empty_evidence` (platform-selection failure,
        // image-inspect failure, and `not_applicable_pinning_evidence`) has
        // no existing container binding to report at all.
        container_binding_status: CONTAINER_BINDING_STATUS_NOT_APPLICABLE.to_string(),
        checked_at_utc: checked_at_utc.to_string(),
        pull_disposition: None,
    }
}

/// v9 review P1-4: truthful evidence for an artifact this module was never
/// asked to verify -- `local_build`/`legacy_unclassified` sources, an
/// unresolved registry artifact (no `resolved_image_identity` yet), or a
/// registry artifact whose pin failed the structural-validity check before
/// any Docker call was ever made. `pull_disposition` is explicitly
/// `not_applicable_unpinned_source`, never left absent, so an observer can
/// never read "no evidence" as "verified and fine."
pub fn not_applicable_pinning_evidence(reason_kind: &str, checked_at_utc: &str) -> VerifiedImageIdentity {
    let mut evidence = empty_evidence(reason_kind, checked_at_utc);
    evidence.pull_disposition = Some(PULL_DISPOSITION_NOT_APPLICABLE_UNPINNED_SOURCE.to_string());
    evidence
}

// ---------------------------------------------------------------------- //
// Docker runtime abstraction -- narrow, image-identity-only (review
// decision: "the smallest abstraction necessary", not a wholesale executor
// rewrite).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformIdentity {
    pub os: String,
    pub architecture: String,
    /// `None` whenever the daemon does not authoritatively report a variant
    /// for this query -- Docker's CLI/API does not reliably expose ARM
    /// variant (v6/v7/v8) disambiguation via `docker version`, and this must
    /// be represented honestly as "unknown," never guessed. See
    /// `select_authorized_platform`'s doc comment for how this is used.
    pub variant: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInspectRecord {
    pub id: String,
    pub repo_digests: Vec<String>,
    pub os: String,
    pub architecture: String,
    /// Absent whenever the daemon/image predates the `.Variant` field
    /// (Docker API < 1.30) -- represented as `None`, never inferred.
    pub variant: Option<String>,
}

pub trait DockerImageRuntime {
    /// `Ok(None)` means the image is RECOGNIZED as absent (Docker reported
    /// "no such image" or equivalent) -- the only outcome that may lead to a
    /// pull. `Err(_)` means anything else (daemon unreachable, permission
    /// error, malformed response, command execution failure) and must NEVER
    /// be treated as proof of absence (review decision #6).
    fn inspect_image(&self, reference: &str) -> Result<Option<ImageInspectRecord>, String>;
    fn pull_image(&self, reference: &str) -> Result<(), String>;
    fn server_platform(&self) -> Result<PlatformIdentity, String>;
    /// v8 review P1-1: the narrow, fakeable surface for obtaining a
    /// container's top-level `.Image` binding (NOT `.Config.Image`).
    /// `Ok(Some(image_id))` -- container found, binding evidence obtained.
    /// `Ok(None)` -- container recognized-missing (defensive; callers only
    /// invoke this when they already believe the container exists).
    /// `Err(_)` -- inspect command failure, non-zero exit, malformed JSON, or
    /// a missing `.Image` field -- ALWAYS surfaced as unverifiable evidence
    /// by the caller, never silently treated as "no container to check."
    fn inspect_container_image_binding(&self, container_name: &str) -> Result<Option<String>, String>;
}

#[derive(Debug, Default)]
pub struct CliDockerImageRuntime;

fn image_inspect_reports_missing(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("no such image") || normalized.contains("no such object")
}

fn container_inspect_reports_missing(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("no such object") || normalized.contains("no such container")
}

fn parse_image_inspect_record(value: &Value) -> Result<ImageInspectRecord, String> {
    let id = value
        .get("Id")
        .and_then(Value::as_str)
        .ok_or_else(|| "docker image inspect response is missing .Id".to_string())?
        .to_string();
    let repo_digests = value
        .get("RepoDigests")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let os = value
        .get("Os")
        .and_then(Value::as_str)
        .ok_or_else(|| "docker image inspect response is missing .Os".to_string())?
        .to_string();
    let architecture = value
        .get("Architecture")
        .and_then(Value::as_str)
        .ok_or_else(|| "docker image inspect response is missing .Architecture".to_string())?
        .to_string();
    let variant = value
        .get("Variant")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty());
    Ok(ImageInspectRecord { id, repo_digests, os, architecture, variant })
}

/// v8 review P1-4: the pure decision half of a SUCCESSFUL `docker image
/// inspect` invocation (status success, valid JSON already decoded) --
/// extracted so it is directly unit-testable with a synthetic `Vec<Value>`,
/// with no `Command::new` involved. `Ok(None)` is reserved EXCLUSIVELY for
/// the "no such image" case, handled by the caller BEFORE JSON parsing is
/// even attempted (a non-zero exit status) -- this function is only ever
/// reached after a successful exit, so an EMPTY array here is a
/// malformed/unexpected response, never proof of absence.
fn interpret_successful_image_inspect_json(parsed: Vec<Value>, reference: &str) -> Result<Option<ImageInspectRecord>, String> {
    match parsed.into_iter().next() {
        Some(record) => parse_image_inspect_record(&record)
            .map(Some)
            .map_err(|error| format!("malformed docker image inspect response for '{reference}': {error}")),
        None => Err(format!(
            "docker image inspect returned a successful but empty response for '{reference}'"
        )),
    }
}

impl DockerImageRuntime for CliDockerImageRuntime {
    fn inspect_image(&self, reference: &str) -> Result<Option<ImageInspectRecord>, String> {
        let output = Command::new("docker")
            .args(["image", "inspect", reference])
            .output()
            .map_err(|error| format!("execute docker image inspect for '{reference}': {error}"))?;
        if !output.status.success() {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if image_inspect_reports_missing(&combined) {
                return Ok(None);
            }
            return Err(format!("docker image inspect failed for '{reference}': {}", combined.trim()));
        }
        let parsed: Vec<Value> = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("decode docker image inspect for '{reference}': {error}"))?;
        interpret_successful_image_inspect_json(parsed, reference)
    }

    fn pull_image(&self, reference: &str) -> Result<(), String> {
        let output = Command::new("docker")
            .args(["pull", reference])
            .output()
            .map_err(|error| format!("execute docker pull for '{reference}': {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "docker pull failed for '{reference}': {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(())
    }

    fn server_platform(&self) -> Result<PlatformIdentity, String> {
        let output = Command::new("docker")
            .args(["version", "--format", "{{.Server.Os}}/{{.Server.Arch}}"])
            .output()
            .map_err(|error| format!("execute docker version: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "docker version failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let mut parts = text.splitn(2, '/');
        let os = parts.next().unwrap_or_default().trim().to_string();
        let architecture = parts.next().unwrap_or_default().trim().to_string();
        if os.is_empty() || architecture.is_empty() {
            return Err(format!("unexpected docker version output: '{text}'"));
        }
        // Docker does not reliably expose an authoritative ARM variant via
        // `docker version` -- never guess one. See ADR-0014's variant-policy
        // section: `None` here is an honest, documented limitation, not a
        // placeholder that should be "improved" into a guess.
        Ok(PlatformIdentity { os, architecture, variant: None })
    }

    fn inspect_container_image_binding(&self, container_name: &str) -> Result<Option<String>, String> {
        let output = Command::new("docker")
            .args(["inspect", container_name])
            .output()
            .map_err(|error| format!("execute docker inspect for container '{container_name}': {error}"))?;
        if !output.status.success() {
            let combined = format!(
                "{} {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if container_inspect_reports_missing(&combined) {
                return Ok(None);
            }
            return Err(format!("docker inspect failed for container '{container_name}': {}", combined.trim()));
        }
        let parsed: Vec<Value> = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("decode docker inspect for container '{container_name}': {error}"))?;
        let Some(record) = parsed.into_iter().next() else {
            return Err(format!(
                "docker inspect returned a successful but empty response for container '{container_name}'"
            ));
        };
        record
            .get("Image")
            .and_then(Value::as_str)
            .map(|image_id| Ok(Some(image_id.to_string())))
            .unwrap_or_else(|| {
                Err(format!("docker inspect response for container '{container_name}' is missing top-level .Image"))
            })
    }
}

// ---------------------------------------------------------------------- //
// Test-only fake transport (mirrors `oci_registry_resolver::FakeOciHttpTransport`
// / `container_executor::FakeRuntime` precedent).
// ---------------------------------------------------------------------- //

#[cfg(test)]
pub struct FakeDockerImageRuntime {
    inspect_responses: std::cell::RefCell<std::collections::HashMap<String, std::collections::VecDeque<Result<Option<ImageInspectRecord>, String>>>>,
    pull_responses: std::cell::RefCell<std::collections::HashMap<String, std::collections::VecDeque<Result<(), String>>>>,
    container_binding_responses: std::cell::RefCell<std::collections::HashMap<String, std::collections::VecDeque<Result<Option<String>, String>>>>,
    platform: std::cell::RefCell<Result<PlatformIdentity, String>>,
    pub inspect_calls: std::cell::RefCell<Vec<String>>,
    pub pull_calls: std::cell::RefCell<Vec<String>>,
    pub container_binding_calls: std::cell::RefCell<Vec<String>>,
}

#[cfg(test)]
impl FakeDockerImageRuntime {
    pub fn new(platform: PlatformIdentity) -> Self {
        Self {
            inspect_responses: std::cell::RefCell::new(std::collections::HashMap::new()),
            pull_responses: std::cell::RefCell::new(std::collections::HashMap::new()),
            container_binding_responses: std::cell::RefCell::new(std::collections::HashMap::new()),
            platform: std::cell::RefCell::new(Ok(platform)),
            inspect_calls: std::cell::RefCell::new(Vec::new()),
            pull_calls: std::cell::RefCell::new(Vec::new()),
            container_binding_calls: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub fn with_platform_error(error: &str) -> Self {
        Self {
            inspect_responses: std::cell::RefCell::new(std::collections::HashMap::new()),
            pull_responses: std::cell::RefCell::new(std::collections::HashMap::new()),
            container_binding_responses: std::cell::RefCell::new(std::collections::HashMap::new()),
            platform: std::cell::RefCell::new(Err(error.to_string())),
            inspect_calls: std::cell::RefCell::new(Vec::new()),
            pull_calls: std::cell::RefCell::new(Vec::new()),
            container_binding_calls: std::cell::RefCell::new(Vec::new()),
        }
    }

    pub fn queue_inspect(&self, reference: &str, response: Result<Option<ImageInspectRecord>, String>) {
        self.inspect_responses.borrow_mut().entry(reference.to_string()).or_default().push_back(response);
    }

    pub fn queue_pull(&self, reference: &str, response: Result<(), String>) {
        self.pull_responses.borrow_mut().entry(reference.to_string()).or_default().push_back(response);
    }

    pub fn queue_container_binding(&self, container_name: &str, response: Result<Option<String>, String>) {
        self.container_binding_responses.borrow_mut().entry(container_name.to_string()).or_default().push_back(response);
    }
}

#[cfg(test)]
impl DockerImageRuntime for FakeDockerImageRuntime {
    fn inspect_image(&self, reference: &str) -> Result<Option<ImageInspectRecord>, String> {
        self.inspect_calls.borrow_mut().push(reference.to_string());
        self.inspect_responses
            .borrow_mut()
            .get_mut(reference)
            .and_then(|queue| queue.pop_front())
            .unwrap_or_else(|| Err(format!("no fake inspect response queued for '{reference}'")))
    }

    fn pull_image(&self, reference: &str) -> Result<(), String> {
        self.pull_calls.borrow_mut().push(reference.to_string());
        self.pull_responses
            .borrow_mut()
            .get_mut(reference)
            .and_then(|queue| queue.pop_front())
            .unwrap_or_else(|| Err(format!("no fake pull response queued for '{reference}'")))
    }

    fn inspect_container_image_binding(&self, container_name: &str) -> Result<Option<String>, String> {
        self.container_binding_calls.borrow_mut().push(container_name.to_string());
        self.container_binding_responses
            .borrow_mut()
            .get_mut(container_name)
            .and_then(|queue| queue.pop_front())
            .unwrap_or_else(|| Err(format!("no fake container-binding response queued for '{container_name}'")))
    }

    fn server_platform(&self) -> Result<PlatformIdentity, String> {
        self.platform.borrow().clone()
    }
}

// ---------------------------------------------------------------------- //
// Platform selection (pure). Never picks the first candidate; never
// silently infers a variant to make execution proceed (review decision #5).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformSelection<'a> {
    Selected(&'a OperationalResolvedPlatformManifest),
    NoMatch,
    Ambiguous,
}

/// Selects exactly one authorized platform matching the local daemon's own
/// reported platform, or fails closed. Rules (review decision #5):
/// - filter to platforms whose `os`/`architecture` match the daemon exactly;
/// - zero matches -> `NoMatch`;
/// - exactly one match -> selected UNLESS the daemon authoritatively reports
///   a variant that actively disagrees with that one candidate's variant (a
///   real conflict, not an ambiguity -- caught here as `NoMatch` since no
///   authorized platform actually fits this daemon); a daemon variant of
///   `None` never blocks the single-candidate case (nothing to disambiguate
///   against; mirrors ADR-0013's "index already confirmed it" leniency);
/// - two or more matches -> the daemon's variant must be known and must
///   narrow it to exactly one; unknown daemon variant with 2+ candidates, or
///   a known variant matching zero or 2+ candidates (the latter should not
///   occur given ADR-0013's platform-uniqueness constraint, but is handled
///   defensively) -> `Ambiguous`. Never picks an arbitrary first candidate.
pub fn select_authorized_platform<'a>(
    platforms: &'a [OperationalResolvedPlatformManifest],
    daemon: &PlatformIdentity,
) -> PlatformSelection<'a> {
    let candidates: Vec<&OperationalResolvedPlatformManifest> = platforms
        .iter()
        .filter(|platform| platform.os == daemon.os && platform.architecture == daemon.architecture)
        .collect();
    match candidates.len() {
        0 => PlatformSelection::NoMatch,
        1 => {
            let candidate = candidates[0];
            match (&daemon.variant, &candidate.variant) {
                (Some(daemon_variant), Some(candidate_variant)) if daemon_variant != candidate_variant => {
                    PlatformSelection::NoMatch
                }
                _ => PlatformSelection::Selected(candidate),
            }
        }
        _ => match &daemon.variant {
            None => PlatformSelection::Ambiguous,
            Some(daemon_variant) => {
                let matches: Vec<&&OperationalResolvedPlatformManifest> = candidates
                    .iter()
                    .filter(|candidate| candidate.variant.as_deref() == Some(daemon_variant.as_str()))
                    .collect();
                match matches.len() {
                    0 => PlatformSelection::NoMatch,
                    1 => PlatformSelection::Selected(matches[0]),
                    _ => PlatformSelection::Ambiguous,
                }
            }
        },
    }
}

// ---------------------------------------------------------------------- //
// RepoDigest canonicalization (review decision #4). Docker's observed
// RepoDigest entries use the "familiar" reference form (Docker Hub official
// images and the implicit-registry rules drop the `docker.io`/`library/`
// prefix); this must be normalized to the SAME fully-qualified form
// `resolved_image_identity.repository` already uses before comparison --
// never a fragile raw-string match. Deliberately duplicates (does not
// import) dserver's `oci_registry_resolver::parse_image_reference` logic:
// no server dependency is introduced into this agent crate.
// ---------------------------------------------------------------------- //

fn canonicalize_repository_name(name_part: &str) -> Option<(String, String)> {
    if name_part.is_empty() {
        return None;
    }
    let segments: Vec<&str> = name_part.split('/').collect();
    let first_looks_like_host =
        segments.len() > 1 && (segments[0].contains('.') || segments[0].contains(':') || segments[0] == "localhost");
    if first_looks_like_host {
        let host = segments[0].to_string();
        let repo = segments[1..].join("/");
        if repo.is_empty() {
            return None;
        }
        Some((host, repo))
    } else {
        Some(("docker.io".to_string(), name_part.to_string()))
    }
}

/// Parses one observed `RepoDigests` entry (`"<name>@sha256:<hex>"`) into
/// `(fully_qualified_repository, digest)`, applying the same Docker-Hub
/// implicit-registry / implicit-`library/`-namespace normalization
/// `resolved_image_identity.repository` was built with server-side, so the
/// two become directly comparable regardless of which "familiar" spelling
/// Docker chose to print.
pub fn canonicalize_repo_digest_entry(entry: &str) -> Option<(String, String)> {
    let (name_part, digest_part) = entry.rsplit_once('@')?;
    if !is_oci_digest_format(digest_part) {
        return None;
    }
    let (canonical_registry, mut repository_path) = canonicalize_repository_name(name_part)?;
    if canonical_registry == "docker.io" && !repository_path.contains('/') {
        repository_path = format!("library/{repository_path}");
    }
    if repository_path.is_empty() {
        return None;
    }
    Some((format!("{canonical_registry}/{repository_path}"), digest_part.to_string()))
}

/// True when at least one `RepoDigests` entry canonicalizes to exactly the
/// authorized `(repository, manifest_digest)` pair. An unrelated repository
/// that happens to share the same digest string does NOT match (the
/// repository half is compared too, not just the digest) -- and additional,
/// irrelevant `RepoDigests` entries (other repos/re-tags) never disqualify a
/// match on their own.
pub fn repo_digests_contain(entries: &[String], authorized_repository: &str, authorized_manifest_digest: &str) -> bool {
    entries.iter().any(|entry| {
        canonicalize_repo_digest_entry(entry)
            .is_some_and(|(repository, digest)| repository == authorized_repository && digest == authorized_manifest_digest)
    })
}

// ---------------------------------------------------------------------- //
// Comparison (pure).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageComparisonResult {
    pub verified: bool,
    pub reason_kind: Option<String>,
}

/// Compares an already-fetched `ImageInspectRecord` against ONE selected
/// authorized platform. Never re-prefixes a digest (review decision #3):
/// `manifest_digest`/`config_digest` are validated as already-well-formed
/// `sha256:<64hex>` OCI digests and compared directly against Docker's own
/// `RepoDigests`/`.Id` (also already `sha256:...`-shaped), never wrapped in
/// an extra `format!("sha256:{...}")`.
pub fn compare_inspected_image(
    record: &ImageInspectRecord,
    authorized_repository: &str,
    platform: &OperationalResolvedPlatformManifest,
) -> ImageComparisonResult {
    if !is_oci_digest_format(&platform.manifest_digest) || !is_oci_digest_format(&platform.config_digest) {
        // A malformed authorized digest is a data-shape problem, not runtime
        // evidence of a mismatch -- but there is nothing safe to verify
        // against, so this is reported the same as an active mismatch here
        // (defense in depth; `validate_resolved_image_identity`-equivalent
        // structural checks upstream should already have prevented this).
        return ImageComparisonResult { verified: false, reason_kind: Some(REASON_CONFIG_MISMATCH.to_string()) };
    }
    if !repo_digests_contain(&record.repo_digests, authorized_repository, &platform.manifest_digest) {
        return ImageComparisonResult { verified: false, reason_kind: Some(REASON_MANIFEST_MISMATCH.to_string()) };
    }
    if !record.id.eq_ignore_ascii_case(&platform.config_digest) {
        return ImageComparisonResult { verified: false, reason_kind: Some(REASON_CONFIG_MISMATCH.to_string()) };
    }
    if record.os != platform.os || record.architecture != platform.architecture {
        return ImageComparisonResult { verified: false, reason_kind: Some(REASON_PLATFORM_MISMATCH.to_string()) };
    }
    if let (Some(expected_variant), Some(actual_variant)) = (&platform.variant, &record.variant) {
        if expected_variant != actual_variant {
            return ImageComparisonResult { verified: false, reason_kind: Some(REASON_PLATFORM_MISMATCH.to_string()) };
        }
    }
    // An absent `record.variant` when `platform.variant` is `Some(..)` is
    // lenient by design (ADR-0013 precedent: the platform was already
    // selected/confirmed by digest above; an image config's own `variant`
    // field is optional per the OCI spec and its absence here is not itself
    // contradicting evidence).
    ImageComparisonResult { verified: true, reason_kind: None }
}

/// Compares the top-level `.Image` field of an already-fetched
/// `docker inspect <container>` object (NOT `.Config.Image`, which is only
/// the create-time argument string and proves nothing about content) against
/// the verified local image's own `.Id`.
pub fn container_image_binding_matches(inspected_container: &Value, verified_image_id: &str) -> bool {
    inspected_container.get("Image").and_then(Value::as_str) == Some(verified_image_id)
}

// ---------------------------------------------------------------------- //
// Top-level, read-only evaluation. NEVER calls `pull_image` -- safe to
// invoke unconditionally (e.g. for convergence observation in a cycle that
// is not even authorized to mutate), mirroring how `docker image inspect`/
// `docker inspect <container>` are already unconditional, read-only calls
// elsewhere in this codebase. `operational_reconciliation.rs` owns the
// separate, lease-gated decision of whether to call `pull_image` when this
// reports the image recognized-absent.
// ---------------------------------------------------------------------- //

/// v9 review P0-2: the EXISTING CONTAINER's binding to the verified target
/// image, kept STRICTLY SEPARATE from the target image's own verification
/// (`ImageIdentityEvaluation.verified`/`evidence.reason_kind`). A mismatch
/// here is drift (an old/different deployment bound to different content),
/// never a statement that the TARGET candidate image itself is unsafe --
/// conflating the two (v2's bug) meant a legitimate re-pin could never be
/// repaired: the verified new target was permanently blocked merely because
/// the OLD container hadn't been recreated onto it yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerBindingObservation {
    /// No existing container to check (Missing state, or none was passed).
    NotApplicable,
    /// The existing container's top-level `.Image` equals the verified
    /// target's own `.Id` -- the running/stopped container IS the verified
    /// content.
    Matches,
    /// The existing container is bound to DIFFERENT content than the
    /// verified target -- drift, not a candidate-image safety problem. The
    /// caller must treat this as "needs a governed recreate," never as a
    /// terminal `blocked_image_identity_mismatch`.
    Mismatches { actual_image_id: String },
    /// A container is known to exist, but its binding evidence could not be
    /// obtained (`inspect_container_image_binding` returned `Ok(None)` or
    /// `Err(_)`) -- never silently treated as `NotApplicable`.
    Unavailable,
}

impl ContainerBindingObservation {
    /// v10 review P1-1: the single place that maps this internal enum onto
    /// the explicit, serialized `CONTAINER_BINDING_STATUS_*` string carried
    /// on `VerifiedImageIdentity.container_binding_status`.
    pub fn as_status_str(&self) -> &'static str {
        match self {
            ContainerBindingObservation::NotApplicable => CONTAINER_BINDING_STATUS_NOT_APPLICABLE,
            ContainerBindingObservation::Matches => CONTAINER_BINDING_STATUS_MATCHES,
            ContainerBindingObservation::Mismatches { .. } => CONTAINER_BINDING_STATUS_MISMATCHES,
            ContainerBindingObservation::Unavailable => CONTAINER_BINDING_STATUS_UNAVAILABLE,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageIdentityEvaluation {
    pub evidence: VerifiedImageIdentity,
    /// `Some(..)` exactly when a platform was successfully selected --
    /// regardless of whether the subsequent inspection/comparison verified,
    /// mismatched, or was itself unavailable. Absent only when platform
    /// selection itself failed (`no_matching_platform` / `ambiguous_platform`).
    pub execution_reference: Option<String>,
    /// True exactly when `inspect_image` returned `Ok(Some(..))` this call --
    /// i.e. the image was found locally (whether or not it then verified).
    pub found_locally: bool,
    /// TARGET IMAGE verification only (manifest/config/platform) -- NEVER
    /// affected by container-binding drift (v9 review P0-2).
    pub verified: bool,
    /// EXISTING CONTAINER binding, kept separate from `verified` -- see
    /// `ContainerBindingObservation`'s own doc comment.
    pub container_binding: ContainerBindingObservation,
}

/// Read-only: selects a platform, inspects the resulting immutable reference
/// (never pulls), and -- when `existing_container_name` is provided -- ALSO
/// obtains and checks the container-binding dimension via
/// `DockerImageRuntime::inspect_container_image_binding` (v8 review P1-1: no
/// longer an unabstracted `Command::new` call in `operational_reconciliation.rs`,
/// and no longer collapsed to "nothing to check" on failure). Reused by both
/// the unconditional convergence-time check and as the read-only preflight
/// phase of the lease-gated, may-pull orchestration in
/// `operational_reconciliation.rs` (v8 review P0-1: this function itself
/// never mutates anything, so it is always safe to call before any
/// governance gate has been evaluated).
pub fn evaluate_image_identity(
    runtime: &dyn DockerImageRuntime,
    resolved: &OperationalResolvedImageIdentity,
    existing_container_name: Option<&str>,
    checked_at_utc: &str,
) -> ImageIdentityEvaluation {
    let daemon = match runtime.server_platform() {
        Ok(platform) => platform,
        Err(_error) => {
            return ImageIdentityEvaluation {
                evidence: empty_evidence(REASON_INSPECT_UNAVAILABLE, checked_at_utc),
                execution_reference: None,
                found_locally: false,
                verified: false,
                container_binding: ContainerBindingObservation::NotApplicable,
            };
        }
    };
    let platform = match select_authorized_platform(&resolved.platforms, &daemon) {
        PlatformSelection::Selected(platform) => platform,
        PlatformSelection::NoMatch => {
            return ImageIdentityEvaluation {
                evidence: empty_evidence(REASON_NO_MATCHING_PLATFORM, checked_at_utc),
                execution_reference: None,
                found_locally: false,
                verified: false,
                container_binding: ContainerBindingObservation::NotApplicable,
            };
        }
        PlatformSelection::Ambiguous => {
            return ImageIdentityEvaluation {
                evidence: empty_evidence(REASON_AMBIGUOUS_PLATFORM, checked_at_utc),
                execution_reference: None,
                found_locally: false,
                verified: false,
                container_binding: ContainerBindingObservation::NotApplicable,
            };
        }
    };
    let execution_reference = platform.execution_reference.clone();
    let selected_platform_label = format!(
        "{}/{}{}",
        platform.os,
        platform.architecture,
        platform.variant.as_deref().map(|variant| format!("/{variant}")).unwrap_or_default()
    );
    let base_evidence = |verified: bool, reason_kind: Option<String>, repo_digests: Vec<String>, image_id: Option<String>, container_image: Option<String>, container_binding_status: &str| {
        VerifiedImageIdentity {
            authorized_execution_reference: Some(execution_reference.clone()),
            authorized_manifest_digest: Some(platform.manifest_digest.clone()),
            authorized_config_digest: Some(platform.config_digest.clone()),
            selected_platform: Some(selected_platform_label.clone()),
            actual_repo_digests: repo_digests,
            actual_image_id: image_id,
            actual_container_image: container_image,
            target_verified: verified,
            reason_kind,
            container_binding_status: container_binding_status.to_string(),
            checked_at_utc: checked_at_utc.to_string(),
            pull_disposition: None,
        }
    };

    match runtime.inspect_image(&execution_reference) {
        Err(_error) => ImageIdentityEvaluation {
            // The target image itself could not even be inspected -- no
            // container-binding check was attempted either.
            evidence: base_evidence(false, Some(REASON_INSPECT_UNAVAILABLE.to_string()), Vec::new(), None, None, CONTAINER_BINDING_STATUS_NOT_APPLICABLE),
            execution_reference: Some(execution_reference),
            found_locally: false,
            verified: false,
            container_binding: ContainerBindingObservation::NotApplicable,
        },
        Ok(None) => ImageIdentityEvaluation {
            // Recognized absent: NOT itself a reason_kind (absence isn't a
            // verification failure by itself) -- the caller decides whether
            // a pull may follow. Nothing locally to compare an existing
            // container's binding against either.
            evidence: base_evidence(false, None, Vec::new(), None, None, CONTAINER_BINDING_STATUS_NOT_APPLICABLE),
            execution_reference: Some(execution_reference),
            found_locally: false,
            verified: false,
            container_binding: ContainerBindingObservation::NotApplicable,
        },
        Ok(Some(record)) => {
            // v9 review P0-2: TARGET IMAGE verification only -- `verified`/
            // `reason_kind` NEVER reflect container-binding drift anymore.
            let comparison = compare_inspected_image(&record, &resolved.repository, platform);
            let (verified, reason_kind) = if comparison.verified { (true, None) } else { (false, comparison.reason_kind) };
            // EXISTING CONTAINER binding is a SEPARATE observation, computed
            // regardless of whether the target itself verified (evidence is
            // still useful even for a mismatching target), but never folded
            // back into `verified`/`reason_kind`.
            let (container_binding, container_image) = match existing_container_name {
                None => (ContainerBindingObservation::NotApplicable, None),
                Some(container_name) => match runtime.inspect_container_image_binding(container_name) {
                    Ok(Some(actual_image_id)) if actual_image_id == record.id => {
                        (ContainerBindingObservation::Matches, Some(actual_image_id))
                    }
                    Ok(Some(actual_image_id)) => (
                        ContainerBindingObservation::Mismatches { actual_image_id: actual_image_id.clone() },
                        Some(actual_image_id),
                    ),
                    // v8 review P1-1: a container is known to exist (the
                    // caller only passes `Some(name)` for Running/Stopped),
                    // so `Ok(None)` here (recognized-missing) is just as
                    // much "could not obtain binding evidence" as `Err(_)`
                    // -- both are UNAVAILABLE, never silently treated as
                    // "no container to check."
                    Ok(None) | Err(_) => (ContainerBindingObservation::Unavailable, None),
                },
            };
            ImageIdentityEvaluation {
                evidence: base_evidence(verified, reason_kind, record.repo_digests.clone(), Some(record.id.clone()), container_image, container_binding.as_status_str()),
                execution_reference: Some(execution_reference),
                found_locally: true,
                verified,
                container_binding,
            }
        }
    }
}

// ---------------------------------------------------------------------- //
// Preflight classification (v8 review P0-1/P0-2/P1-3): turns a read-only
// `ImageIdentityEvaluation` into an explicit decision the caller can act on
// WITHOUT itself inspecting evidence internals -- in particular, without
// ever needing to special-case "found_locally=false, reason_kind=None" as
// "go ahead and pull" at the call site. `NoSelectableReference` and
// `KnownMismatch` are terminal (no reference is ever safe to fall back to);
// `UnverifiedWithReference` carries the immutable reference forward
// specifically so a permitted `legacy_partial` continuation never has to
// fall back to the mutable declared tag (v8 review P0-2).
// ---------------------------------------------------------------------- //

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightOutcome {
    /// Fully verified now (image present-and-matching, container binding
    /// confirmed where applicable).
    Verified(String),
    /// Evidence available and it actively CONTRADICTS the authorized
    /// identity -- always terminal, regardless of effective mode.
    KnownMismatch(String),
    /// No execution_reference could even be selected (no matching platform /
    /// ambiguous platform / daemon platform query failed) -- always
    /// terminal: there is no immutable reference to safely fall back to,
    /// regardless of effective mode (v8 review P0-2, case B).
    NoSelectableReference(String),
    /// A platform WAS selected (an execution_reference is known) but the
    /// image/container evidence could not be confirmed (daemon inspect
    /// error, not "recognized absent"; or container-binding evidence
    /// unavailable). Mode-dependent: `required` treats this as terminal;
    /// `legacy_partial` MAY continue, but only using this execution_reference
    /// (v8 review P0-2, case A) -- never the mutable declared tag.
    UnverifiedWithReference { execution_reference: String, reason_kind: String },
    /// A platform was selected and the image is recognized ABSENT locally --
    /// acquisition (M5 pull) may proceed if the caller's own governance
    /// gates otherwise permit it (v8 review P0-1: this outcome by itself
    /// grants no permission to mutate).
    NeedsPull { execution_reference: String },
}

/// Pure classification of an already-computed `ImageIdentityEvaluation` --
/// no I/O, no mutation, safe to call from anywhere (including the
/// unconditional, non-executing-mode convergence path).
pub fn classify_evaluation(evaluation: &ImageIdentityEvaluation) -> PreflightOutcome {
    let Some(execution_reference) = evaluation.execution_reference.clone() else {
        return PreflightOutcome::NoSelectableReference(
            evaluation.evidence.reason_kind.clone().unwrap_or_else(|| REASON_INSPECT_UNAVAILABLE.to_string()),
        );
    };
    if evaluation.verified {
        return PreflightOutcome::Verified(execution_reference);
    }
    if !evaluation.found_locally {
        return match &evaluation.evidence.reason_kind {
            None => PreflightOutcome::NeedsPull { execution_reference },
            Some(reason_kind) => PreflightOutcome::UnverifiedWithReference {
                execution_reference,
                reason_kind: reason_kind.clone(),
            },
        };
    }
    match &evaluation.evidence.reason_kind {
        // v9 review P0-2: container-binding drift is intentionally NEVER a
        // `reason_kind` here anymore (see `ContainerBindingObservation`) --
        // this match is TARGET-IMAGE-only by construction, not merely by
        // convention.
        Some(reason_kind)
            if reason_kind == REASON_MANIFEST_MISMATCH
                || reason_kind == REASON_CONFIG_MISMATCH
                || reason_kind == REASON_PLATFORM_MISMATCH =>
        {
            PreflightOutcome::KnownMismatch(reason_kind.clone())
        }
        Some(reason_kind) => {
            PreflightOutcome::UnverifiedWithReference { execution_reference, reason_kind: reason_kind.clone() }
        }
        // Defensive: found_locally=true, verified=false, no reason_kind
        // should not occur given this module's own invariants, but never
        // silently treated as verified.
        None => PreflightOutcome::UnverifiedWithReference {
            execution_reference,
            reason_kind: REASON_INSPECT_UNAVAILABLE.to_string(),
        },
    }
}

/// The acquisition (M5) phase: pulls `execution_reference`, then re-evaluates
/// read-only exactly like `evaluate_image_identity`, and re-classifies.
/// `Err(_)` means the `docker pull` COMMAND ITSELF genuinely failed
/// (registry/network problem) -- NOT an identity-verification outcome; the
/// caller surfaces that as a per-service hard failure, never as a
/// `blocked_image_identity_*` code (v8 review P0-1/P1-2). The caller MUST
/// have already confirmed every other non-image governance gate passes
/// before calling this -- this function itself has no knowledge of leases,
/// generation/projection checks, or `execution_pinning_mode`.
pub fn acquire_pinned_image(
    runtime: &dyn DockerImageRuntime,
    resolved: &OperationalResolvedImageIdentity,
    existing_container_name: Option<&str>,
    execution_reference: &str,
    checked_at_utc: &str,
) -> Result<(ImageIdentityEvaluation, PreflightOutcome), String> {
    runtime.pull_image(execution_reference)?;
    let mut evaluation = evaluate_image_identity(runtime, resolved, existing_container_name, checked_at_utc);
    evaluation.evidence.pull_disposition = Some(PULL_DISPOSITION_PULL_PERFORMED.to_string());
    if !evaluation.found_locally && evaluation.evidence.reason_kind.is_none() {
        // Anomalous: a reported-successful pull, still recognized-absent
        // immediately afterward. Absent evidence is not CONTRADICTING
        // evidence -- unverified, never "mismatch"; and never re-classified
        // as another NeedsPull (no second pull is ever attempted).
        evaluation.evidence.reason_kind = Some(REASON_INSPECT_UNAVAILABLE.to_string());
    }
    let outcome = classify_evaluation(&evaluation);
    Ok((evaluation, outcome))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn platform(os: &str, architecture: &str, variant: Option<&str>, manifest: char, config: char, repository: &str) -> OperationalResolvedPlatformManifest {
        OperationalResolvedPlatformManifest {
            os: os.to_string(),
            architecture: architecture.to_string(),
            variant: variant.map(str::to_string),
            manifest_digest: digest(manifest),
            config_digest: digest(config),
            execution_reference: format!("{repository}@{}", digest(manifest)),
        }
    }

    fn resolved(repository: &str, platforms: Vec<OperationalResolvedPlatformManifest>) -> OperationalResolvedImageIdentity {
        OperationalResolvedImageIdentity {
            repository: repository.to_string(),
            index_digest: None,
            platforms,
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        }
    }

    fn inspect_record(id: char, repo_digests: Vec<String>, os: &str, architecture: &str, variant: Option<&str>) -> ImageInspectRecord {
        ImageInspectRecord {
            id: digest(id),
            repo_digests,
            os: os.to_string(),
            architecture: architecture.to_string(),
            variant: variant.map(str::to_string),
        }
    }

    // ---- decision #4: RepoDigest canonicalization ---------------------- //

    #[test]
    fn canonicalizes_docker_hub_official_image_short_form() {
        let entry = format!("eclipse-mosquitto@{}", digest('1'));
        let (repository, digest_out) = canonicalize_repo_digest_entry(&entry).unwrap();
        assert_eq!(repository, "docker.io/library/eclipse-mosquitto");
        assert_eq!(digest_out, digest('1'));
    }

    #[test]
    fn canonicalizes_docker_hub_namespaced_short_form() {
        let entry = format!("chirpstack/chirpstack@{}", digest('1'));
        let (repository, _) = canonicalize_repo_digest_entry(&entry).unwrap();
        assert_eq!(repository, "docker.io/chirpstack/chirpstack");
    }

    #[test]
    fn canonicalizes_explicit_docker_io_form_identically_to_short_form() {
        let short = canonicalize_repo_digest_entry(&format!("eclipse-mosquitto@{}", digest('1'))).unwrap();
        let explicit = canonicalize_repo_digest_entry(&format!("docker.io/library/eclipse-mosquitto@{}", digest('1'))).unwrap();
        assert_eq!(short, explicit);
    }

    #[test]
    fn canonicalizes_ghcr_repository() {
        let entry = format!("ghcr.io/absmach/atom@{}", digest('1'));
        let (repository, _) = canonicalize_repo_digest_entry(&entry).unwrap();
        assert_eq!(repository, "ghcr.io/absmach/atom");
    }

    #[test]
    fn canonicalizes_custom_registry_host_with_port() {
        let entry = format!("registry.example.com:5000/team/app@{}", digest('1'));
        let (repository, _) = canonicalize_repo_digest_entry(&entry).unwrap();
        assert_eq!(repository, "registry.example.com:5000/team/app");
    }

    #[test]
    fn rejects_malformed_digest_suffix() {
        assert!(canonicalize_repo_digest_entry("eclipse-mosquitto@not-a-digest").is_none());
        assert!(canonicalize_repo_digest_entry("eclipse-mosquitto-no-at-sign").is_none());
    }

    // ---- v8 review P1-4: successful-but-empty inspect is NOT absence ---- //

    #[test]
    fn interpret_successful_image_inspect_json_rejects_an_empty_array() {
        let result = interpret_successful_image_inspect_json(Vec::new(), "docker.io/library/eclipse-mosquitto@sha256:abc");
        assert!(result.is_err(), "an empty-but-successful response must never be treated as recognized absence");
    }

    #[test]
    fn interpret_successful_image_inspect_json_parses_a_single_element_array() {
        let record = serde_json::json!({
            "Id": digest('2'),
            "RepoDigests": [format!("eclipse-mosquitto@{}", digest('1'))],
            "Os": "linux",
            "Architecture": "amd64",
        });
        let result = interpret_successful_image_inspect_json(vec![record], "ref").unwrap();
        assert_eq!(result.unwrap().id, digest('2'));
    }

    #[test]
    fn interpret_successful_image_inspect_json_rejects_a_malformed_element() {
        let record = serde_json::json!({ "Os": "linux" }); // missing .Id
        let result = interpret_successful_image_inspect_json(vec![record], "ref");
        assert!(result.is_err());
    }

    #[test]
    fn repo_digests_contain_matches_across_spellings() {
        let entries = vec![format!("eclipse-mosquitto@{}", digest('1'))];
        assert!(repo_digests_contain(&entries, "docker.io/library/eclipse-mosquitto", &digest('1')));
    }

    #[test]
    fn repo_digests_contain_rejects_unrelated_repository_with_same_digest() {
        // Same digest string, but a DIFFERENT repository -- must be rejected;
        // matching on digest alone would let unrelated content pass.
        let entries = vec![format!("some/other-repo@{}", digest('1'))];
        assert!(!repo_digests_contain(&entries, "docker.io/library/eclipse-mosquitto", &digest('1')));
    }

    #[test]
    fn repo_digests_contain_accepts_multiple_entries_with_one_correct() {
        let entries = vec![
            format!("some/other-repo@{}", digest('9')),
            format!("eclipse-mosquitto@{}", digest('1')),
        ];
        assert!(repo_digests_contain(&entries, "docker.io/library/eclipse-mosquitto", &digest('1')));
    }

    // ---- platform selection ---------------------------------------------//

    #[test]
    fn selects_the_single_matching_platform_when_daemon_variant_is_unknown() {
        let platforms = vec![platform("linux", "amd64", None, '1', '2', "repo")];
        let daemon = PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None };
        assert!(matches!(select_authorized_platform(&platforms, &daemon), PlatformSelection::Selected(_)));
    }

    #[test]
    fn single_candidate_conflicting_with_a_known_daemon_variant_is_no_match() {
        let platforms = vec![platform("linux", "arm", Some("v7"), '1', '2', "repo")];
        let daemon = PlatformIdentity { os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v8".to_string()) };
        assert_eq!(select_authorized_platform(&platforms, &daemon), PlatformSelection::NoMatch);
    }

    #[test]
    fn no_candidates_for_os_and_architecture_is_no_match() {
        let platforms = vec![platform("linux", "amd64", None, '1', '2', "repo")];
        let daemon = PlatformIdentity { os: "linux".to_string(), architecture: "arm64".to_string(), variant: None };
        assert_eq!(select_authorized_platform(&platforms, &daemon), PlatformSelection::NoMatch);
    }

    #[test]
    fn multiple_variants_of_the_same_architecture_with_unknown_daemon_variant_is_ambiguous() {
        let platforms = vec![
            platform("linux", "arm", Some("v6"), '1', '2', "repo"),
            platform("linux", "arm", Some("v7"), '3', '4', "repo"),
        ];
        let daemon = PlatformIdentity { os: "linux".to_string(), architecture: "arm".to_string(), variant: None };
        assert_eq!(select_authorized_platform(&platforms, &daemon), PlatformSelection::Ambiguous);
    }

    #[test]
    fn multiple_variants_with_a_known_daemon_variant_selects_exactly_the_matching_one() {
        let platforms = vec![
            platform("linux", "arm", Some("v6"), '1', '2', "repo"),
            platform("linux", "arm", Some("v7"), '3', '4', "repo"),
        ];
        let daemon = PlatformIdentity { os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v7".to_string()) };
        match select_authorized_platform(&platforms, &daemon) {
            PlatformSelection::Selected(selected) => assert_eq!(selected.manifest_digest, digest('3')),
            other => panic!("expected Selected, got {other:?}"),
        }
    }

    #[test]
    fn distinct_architectures_amd64_and_arm64_never_pick_the_first_candidate_by_position() {
        let platforms = vec![
            platform("linux", "arm64", None, '1', '2', "repo"),
            platform("linux", "amd64", None, '3', '4', "repo"),
        ];
        let daemon = PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None };
        match select_authorized_platform(&platforms, &daemon) {
            PlatformSelection::Selected(selected) => assert_eq!(selected.manifest_digest, digest('3')),
            other => panic!("expected Selected, got {other:?}"),
        }
    }

    // ---- comparison ------------------------------------------------------//

    #[test]
    fn compare_inspected_image_accepts_a_fully_matching_record() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let plat = platform("linux", "amd64", None, '1', '2', repository);
        let record = inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None);
        let result = compare_inspected_image(&record, repository, &plat);
        assert!(result.verified, "{result:?}");
    }

    #[test]
    fn compare_inspected_image_rejects_manifest_mismatch() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let plat = platform("linux", "amd64", None, '1', '2', repository);
        let record = inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('9'))], "linux", "amd64", None);
        let result = compare_inspected_image(&record, repository, &plat);
        assert_eq!(result.reason_kind.as_deref(), Some(REASON_MANIFEST_MISMATCH));
    }

    #[test]
    fn compare_inspected_image_rejects_config_id_mismatch() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let plat = platform("linux", "amd64", None, '1', '2', repository);
        let record = inspect_record('9', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None);
        let result = compare_inspected_image(&record, repository, &plat);
        assert_eq!(result.reason_kind.as_deref(), Some(REASON_CONFIG_MISMATCH));
    }

    #[test]
    fn compare_inspected_image_rejects_architecture_mismatch() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let plat = platform("linux", "amd64", None, '1', '2', repository);
        let record = inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "arm64", None);
        let result = compare_inspected_image(&record, repository, &plat);
        assert_eq!(result.reason_kind.as_deref(), Some(REASON_PLATFORM_MISMATCH));
    }

    #[test]
    fn compare_inspected_image_rejects_a_disagreeing_variant() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let plat = platform("linux", "arm", Some("v7"), '1', '2', repository);
        let record = inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "arm", Some("v6"));
        let result = compare_inspected_image(&record, repository, &plat);
        assert_eq!(result.reason_kind.as_deref(), Some(REASON_PLATFORM_MISMATCH));
    }

    #[test]
    fn compare_inspected_image_is_lenient_when_the_image_config_omits_variant() {
        // ADR-0013 precedent: the platform was already selected/confirmed by
        // digest; an absent config-level variant is not contradicting
        // evidence.
        let repository = "docker.io/library/eclipse-mosquitto";
        let plat = platform("linux", "arm", Some("v7"), '1', '2', repository);
        let record = inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "arm", None);
        let result = compare_inspected_image(&record, repository, &plat);
        assert!(result.verified, "{result:?}");
    }

    #[test]
    fn container_image_binding_matches_the_top_level_image_field_only() {
        let inspected = serde_json::json!({ "Image": digest('2'), "Config": { "Image": "eclipse-mosquitto:2" } });
        assert!(container_image_binding_matches(&inspected, &digest('2')));
        assert!(!container_image_binding_matches(&inspected, &digest('9')));
        // `.Config.Image` (the mutable declared string) must NEVER be treated
        // as content-identity proof by this function.
        assert!(!container_image_binding_matches(&inspected, "eclipse-mosquitto:2"));
    }

    // ---- evaluate_image_identity (via FakeDockerImageRuntime) -----------//

    #[test]
    fn evaluate_reports_inspect_unavailable_when_the_daemon_platform_cannot_be_queried() {
        let runtime = FakeDockerImageRuntime::with_platform_error("daemon unreachable");
        let identity = resolved("docker.io/library/eclipse-mosquitto", vec![platform("linux", "amd64", None, '1', '2', "docker.io/library/eclipse-mosquitto")]);
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert!(!evaluation.verified);
        assert_eq!(evaluation.evidence.reason_kind.as_deref(), Some(REASON_INSPECT_UNAVAILABLE));
        assert!(evaluation.execution_reference.is_none());
    }

    #[test]
    fn evaluate_reports_no_matching_platform() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "arm64".to_string(), variant: None });
        let identity = resolved("docker.io/library/eclipse-mosquitto", vec![platform("linux", "amd64", None, '1', '2', "docker.io/library/eclipse-mosquitto")]);
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert_eq!(evaluation.evidence.reason_kind.as_deref(), Some(REASON_NO_MATCHING_PLATFORM));
        assert!(evaluation.execution_reference.is_none());
        assert!(runtime.inspect_calls.borrow().is_empty(), "must never inspect an image without a selected platform");
    }

    #[test]
    fn evaluate_reports_recognized_absent_with_no_reason_kind_and_never_pulls() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(None));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert!(!evaluation.found_locally);
        assert!(!evaluation.verified);
        assert!(evaluation.evidence.reason_kind.is_none(), "absence alone is not a reason_kind -- the caller decides pull-or-not");
        assert_eq!(evaluation.execution_reference.as_deref(), Some(reference.as_str()));
        assert!(runtime.pull_calls.borrow().is_empty(), "evaluate_image_identity must never pull");
    }

    #[test]
    fn evaluate_reports_verified_for_a_fully_matching_present_image() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert!(evaluation.found_locally);
        assert!(evaluation.verified);
        assert_eq!(evaluation.evidence.actual_image_id.as_deref(), Some(digest('2').as_str()));
    }

    #[test]
    fn evaluate_reports_container_binding_mismatch_as_a_separate_observation_from_target_verification() {
        // v9 review P0-2: an existing container bound to a DIFFERENT image
        // object (e.g. after a legitimate re-pin) must never contaminate
        // TARGET IMAGE verification -- `verified`/`reason_kind` stay `true`/
        // `None` here; the drift is reported ONLY via `container_binding`,
        // so the caller can force a governed recreate without ever treating
        // this as a terminal "wrong candidate" mismatch.
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        runtime.queue_container_binding("mqtt", Ok(Some(digest('9'))));
        let evaluation = evaluate_image_identity(&runtime, &identity, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(evaluation.verified, "target image identity must verify independently of container binding: {evaluation:?}");
        assert!(evaluation.evidence.reason_kind.is_none());
        assert_eq!(evaluation.container_binding, ContainerBindingObservation::Mismatches { actual_image_id: digest('9') });
    }

    #[test]
    fn evaluate_reports_verified_when_container_binding_matches() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        runtime.queue_container_binding("mqtt", Ok(Some(digest('2'))));
        let evaluation = evaluate_image_identity(&runtime, &identity, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(evaluation.verified, "{evaluation:?}");
        assert_eq!(evaluation.evidence.actual_container_image.as_deref(), Some(digest('2').as_str()));
    }

    // ---- v8 review P1-1: container-binding failure is UNAVAILABLE, never silently ignored ----

    #[test]
    fn evaluate_reports_container_binding_unavailable_on_inspect_error_target_still_verified() {
        // v9 review P0-2: container-binding UNAVAILABLE must never be folded
        // into target-image `verified`/`reason_kind` either -- it lives only
        // in `container_binding`, distinct from a KNOWN mismatch.
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        runtime.queue_container_binding("mqtt", Err("docker inspect failed: permission denied".to_string()));
        let evaluation = evaluate_image_identity(&runtime, &identity, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(evaluation.verified, "target image identity must verify independently of container-binding availability: {evaluation:?}");
        assert_eq!(evaluation.container_binding, ContainerBindingObservation::Unavailable);
    }

    #[test]
    fn evaluate_reports_container_binding_unavailable_when_container_recognized_missing() {
        // The caller only ever passes `Some(name)` when it already believes
        // the container exists (Running/Stopped); `Ok(None)` here is exactly
        // as much "could not obtain binding evidence" as `Err(_)`.
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        runtime.queue_container_binding("mqtt", Ok(None));
        let evaluation = evaluate_image_identity(&runtime, &identity, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(evaluation.verified, "target image identity must verify independently of container-binding availability: {evaluation:?}");
        assert_eq!(evaluation.container_binding, ContainerBindingObservation::Unavailable);
    }

    // ---- v8 review P0-1/P0-2/P1-3: PreflightOutcome classification ------ //

    #[test]
    fn classify_no_selectable_reference_when_platform_selection_fails() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "arm64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert_eq!(classify_evaluation(&evaluation), PreflightOutcome::NoSelectableReference(REASON_NO_MATCHING_PLATFORM.to_string()));
    }

    #[test]
    fn classify_needs_pull_when_recognized_absent() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(None));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert_eq!(classify_evaluation(&evaluation), PreflightOutcome::NeedsPull { execution_reference: reference });
    }

    #[test]
    fn classify_unverified_with_reference_when_inspect_errors_but_platform_selected() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Err("daemon unreachable".to_string()));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert_eq!(
            classify_evaluation(&evaluation),
            PreflightOutcome::UnverifiedWithReference { execution_reference: reference, reason_kind: REASON_INSPECT_UNAVAILABLE.to_string() }
        );
    }

    #[test]
    fn classify_known_mismatch_when_manifest_disagrees() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('9'))], "linux", "amd64", None))));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert_eq!(classify_evaluation(&evaluation), PreflightOutcome::KnownMismatch(REASON_MANIFEST_MISMATCH.to_string()));
    }

    #[test]
    fn classify_verified_when_everything_matches() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert_eq!(classify_evaluation(&evaluation), PreflightOutcome::Verified(reference));
    }

    // ---- v8 review P0-1: acquire_pinned_image (the M5 acquisition phase) //

    #[test]
    fn acquire_pinned_image_pulls_once_reinspects_and_verifies() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_pull(&reference, Ok(()));
        runtime.queue_inspect(&reference, Ok(Some(inspect_record('2', vec![format!("eclipse-mosquitto@{}", digest('1'))], "linux", "amd64", None))));
        let (evaluation, outcome) = acquire_pinned_image(&runtime, &identity, None, &reference, "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(outcome, PreflightOutcome::Verified(reference));
        assert_eq!(evaluation.evidence.pull_disposition.as_deref(), Some(PULL_DISPOSITION_PULL_PERFORMED));
        assert_eq!(runtime.pull_calls.borrow().len(), 1);
    }

    #[test]
    fn acquire_pinned_image_propagates_a_genuine_pull_command_failure() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_pull(&reference, Err("registry unreachable".to_string()));
        let result = acquire_pinned_image(&runtime, &identity, None, &reference, "2026-01-01T00:00:00Z");
        assert!(result.is_err(), "a genuine pull command failure must propagate as Err, not as an identity outcome");
        assert!(runtime.inspect_calls.borrow().is_empty(), "must not re-inspect after a failed pull");
    }

    #[test]
    fn acquire_pinned_image_still_absent_after_a_reported_successful_pull_is_unverified_not_needs_pull_again() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_pull(&reference, Ok(()));
        runtime.queue_inspect(&reference, Ok(None));
        let (_evidence, outcome) = acquire_pinned_image(&runtime, &identity, None, &reference, "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(
            outcome,
            PreflightOutcome::UnverifiedWithReference { execution_reference: reference, reason_kind: REASON_INSPECT_UNAVAILABLE.to_string() }
        );
        assert_eq!(runtime.pull_calls.borrow().len(), 1, "never a second pull attempt");
    }

    #[test]
    fn evaluate_reports_inspect_error_distinctly_from_recognized_absence() {
        let runtime = FakeDockerImageRuntime::new(PlatformIdentity { os: "linux".to_string(), architecture: "amd64".to_string(), variant: None });
        let repository = "docker.io/library/eclipse-mosquitto";
        let identity = resolved(repository, vec![platform("linux", "amd64", None, '1', '2', repository)]);
        let reference = format!("{repository}@{}", digest('1'));
        runtime.queue_inspect(&reference, Err("docker daemon unreachable".to_string()));
        let evaluation = evaluate_image_identity(&runtime, &identity, None, "2026-01-01T00:00:00Z");
        assert!(!evaluation.found_locally);
        assert_eq!(evaluation.evidence.reason_kind.as_deref(), Some(REASON_INSPECT_UNAVAILABLE));
    }

    // ---- v9 review P1-4: not_applicable_pinning_evidence is always explicit ----

    #[test]
    fn not_applicable_pinning_evidence_is_never_verified_and_always_carries_a_reason_and_disposition() {
        for reason in [
            REASON_LOCAL_BUILD_NOT_PINNABLE,
            REASON_LEGACY_UNCLASSIFIED_NOT_PINNABLE,
            REASON_REGISTRY_UNRESOLVED,
            REASON_REGISTRY_PIN_STRUCTURALLY_INVALID,
        ] {
            let evidence = not_applicable_pinning_evidence(reason, "2026-01-01T00:00:00Z");
            assert!(!evidence.target_verified, "not-applicable evidence must never claim verified: {reason}");
            assert_eq!(evidence.container_binding_status, CONTAINER_BINDING_STATUS_NOT_APPLICABLE);
            assert!(!evidence.runtime_identity_verified());
            assert_eq!(evidence.reason_kind.as_deref(), Some(reason));
            assert_eq!(
                evidence.pull_disposition.as_deref(),
                Some(PULL_DISPOSITION_NOT_APPLICABLE_UNPINNED_SOURCE),
                "pull_disposition must never be left absent for {reason}"
            );
            assert!(evidence.authorized_execution_reference.is_none());
            assert!(evidence.actual_image_id.is_none());
        }
    }
}
