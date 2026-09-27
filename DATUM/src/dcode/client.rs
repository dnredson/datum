//! Control-plane preflight HTTP client (ADR-0021, independent review v2 R1/
//! R2C; Phase 117H). The GOVERNED invocation path calls
//! `fetch_dcode_authorization` and then (Phase 117H)
//! `fetch_dserv_artifact_revision` -- keyed by the EXACT
//! `descriptor_digest` the fresh authorization itself names -- directly
//! against DServer immediately before every invocation. An authorization is
//! ALWAYS freshly derived server-side
//! (`dserver::core::dcode_authorization::authorize_dcode_invocation`, never
//! cached there), so calling this again immediately before execution IS the
//! preflight/re-verification step; no separate `/verify` endpoint is needed
//! for that purpose (see `control-plane-preflight-audit.txt`).
//!
//! Phase 117H closes the exact-revision selection gap Phase 117G's own
//! server-side hardening intentionally deferred: DServer's D-Code registry
//! is now versioned and immutable per revision
//! (`dserver::core::dcode_registry::DCodeArtifactRegistryStateV2`), and a
//! service's active binding may point at any one of several coexisting
//! registered revisions. The OLD logical fetch (`fetch_dserv_artifact`,
//! `GET /api/v1/dcode/artifacts/:application_id/:dserv_id`) can only ever
//! resolve unambiguously while exactly one revision is registered -- it
//! fails closed (never guesses) the moment a second revision exists, so it
//! is SAFE but cannot execute the valid governed single-service,
//! multiple-revision scenario Phase 117G intentionally enabled (an
//! authorization naming a specific `descriptor_digest` while other
//! revisions of the same logical service coexist in the registry). The
//! governed path below always selects by the exact digest instead, so it
//! works identically whether one or many revisions are registered.
//!
//! Documented, honest residual limitation: a control-plane state change can
//! still theoretically occur between this HTTP response and the guest
//! actually executing (distributed TOCTOU) -- this module does not, and
//! cannot, claim distributed atomicity. Phase 117G's own immutable-revision
//! design DOES eliminate one class of that TOCTOU window: once an
//! authorization names `descriptor_digest` D1, D1's own registered content
//! can never change out from under this fetch (the registry has no
//! in-place-mutation/deletion path at all) -- registering a NEW D2
//! elsewhere cannot retroactively alter D1. The remaining residual concern
//! is ACTIVE AUTHORITY moving from D1 to D2 (a new accepted D-Deploy
//! proposal) between this preflight and actual execution, not D1's own
//! content changing in place. What this module guarantees overall is that a
//! real invocation can no longer be driven purely from a locally-fabricated
//! authorization/artifact file with no DServer involved at all (the v1 gap
//! the original R1 fix closed), AND (Phase 117H) that the governed path
//! never falls back to logical-service-only selection once multiple
//! revisions exist.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};

use crate::dcode::authorization::DCodeInvocationAuthorizationV1;
use crate::dcode::dserv_artifact::{recompute_dserv_artifact_digest, CanonicalDServArtifactV1};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn authorize_endpoint_url(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse DServer base URL: {base_url}"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("DServer base URL cannot be a base URL"))?
        .clear()
        .extend(["api", "v1", "dcode", "authorize"]);
    Ok(url)
}

/// Legacy, LOGICAL `(application_id, dserv_id)` endpoint URL builder
/// (pre-Phase-117G shape). Kept only for `fetch_dserv_artifact`'s
/// compatibility/debug use -- see that function's own doc comment. NOT
/// suitable for governed invocation once a service has more than one
/// registered D-Code revision; use `artifact_revision_endpoint_url` instead.
pub fn artifact_endpoint_url(base_url: &str, application_id: &str, dserv_id: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse DServer base URL: {base_url}"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("DServer base URL cannot be a base URL"))?
        .clear()
        .extend(["api", "v1", "dcode", "artifacts", application_id, dserv_id]);
    Ok(url)
}

/// Phase 117G/117H: the EXACT-REVISION endpoint URL builder --
/// `GET /api/v1/dcode/artifacts/:application_id/:dserv_id/:descriptor_digest`.
/// This is the ONLY artifact endpoint the governed invocation path may use.
/// Path segments (including `descriptor_digest`, which contains a literal
/// `:` in its canonical `sha256:<64 lowercase hex>` form) are appended via
/// `Url::path_segments_mut`, which performs standard percent-encoding of
/// each segment -- never hand-built/concatenated string interpolation, so a
/// URL-sensitive character in any identifier can never be misinterpreted as
/// a path separator or otherwise corrupt the resulting URL.
pub fn artifact_revision_endpoint_url(
    base_url: &str,
    application_id: &str,
    dserv_id: &str,
    descriptor_digest: &str,
) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse DServer base URL: {base_url}"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("DServer base URL cannot be a base URL"))?
        .clear()
        .extend(["api", "v1", "dcode", "artifacts", application_id, dserv_id, descriptor_digest]);
    Ok(url)
}

/// Calls `POST /api/v1/dcode/authorize` -- a fresh, server-derived
/// authorization proving the CURRENT active-DMap -> D-Graph -> D-Node ->
/// artifact chain at the moment of this call.
pub fn fetch_dcode_authorization(
    base_url: &str,
    project_id: &str,
    application_id: &str,
    service_id: &str,
    node_id: &str,
) -> Result<DCodeInvocationAuthorizationV1> {
    let endpoint = authorize_endpoint_url(base_url)?;
    let client = Client::builder().timeout(REQUEST_TIMEOUT).build().context("build blocking reqwest client")?;
    let response = client
        .post(endpoint.clone())
        .json(&serde_json::json!({
            "project_id": project_id,
            "application_id": application_id,
            "service_id": service_id,
            "node_id": node_id,
        }))
        .send()
        .with_context(|| format!("POST {endpoint} for D-Code authorization"))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("D-Code authorization request failed: status={status} body={body}"));
    }
    serde_json::from_str(&body)
        .with_context(|| format!("parse DCodeInvocationAuthorizationV1 response JSON: {body}"))
}

/// LEGACY / LOGICAL / UNAMBIGUOUS-ONLY. Calls
/// `GET /api/v1/dcode/artifacts/:application_id/:dserv_id` -- DServer
/// resolves this by LOGICAL `(application_id, dserv_id)` identity alone,
/// which only ever succeeds while at most one D-Code revision is
/// registered for that service; it fails closed (409, never a guess) the
/// moment a second revision exists (see
/// `dserver::core::dcode_registry::DCodeArtifactRegistryStateV2::get_logical_if_unambiguous`).
///
/// **NOT suitable for governed invocation under Phase 117G's versioned
/// registry.** It cannot execute the valid governed scenario where a
/// service's active binding names one specific revision while other
/// revisions of the same logical service also exist. Kept only for
/// compatibility/debugging/one-revision-only callers; the governed
/// production path (`fetch_governed_dcode_execution_material`,
/// `fetch_dserv_artifact_revision`) never calls this.
pub fn fetch_dserv_artifact(
    base_url: &str,
    application_id: &str,
    dserv_id: &str,
) -> Result<CanonicalDServArtifactV1> {
    let endpoint = artifact_endpoint_url(base_url, application_id, dserv_id)?;
    let client = Client::builder().timeout(REQUEST_TIMEOUT).build().context("build blocking reqwest client")?;
    let response = client
        .get(endpoint.clone())
        .send()
        .with_context(|| format!("GET {endpoint} for canonical D-Serv artifact (logical, unambiguous-only)"))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("D-Serv artifact fetch failed: application_id={application_id} dserv_id={dserv_id} status={status} body={body}"));
    }
    serde_json::from_str(&body).with_context(|| format!("parse CanonicalDServArtifactV1 response JSON: {body}"))
}

/// Phase 117G/117H: the GOVERNED fetch. Calls
/// `GET /api/v1/dcode/artifacts/:application_id/:dserv_id/:descriptor_digest`
/// -- the exact, immutable descriptor revision named by `descriptor_digest`
/// (normally a fresh authorization's own
/// `dserv_artifact_descriptor_digest`), never a "latest"/"only"/logical
/// lookup. This is the ONLY artifact-fetch function the governed invocation
/// path may use.
pub fn fetch_dserv_artifact_revision(
    base_url: &str,
    application_id: &str,
    dserv_id: &str,
    descriptor_digest: &str,
) -> Result<CanonicalDServArtifactV1> {
    let endpoint = artifact_revision_endpoint_url(base_url, application_id, dserv_id, descriptor_digest)?;
    let client = Client::builder().timeout(REQUEST_TIMEOUT).build().context("build blocking reqwest client")?;
    let response = client
        .get(endpoint.clone())
        .send()
        .with_context(|| format!("GET {endpoint} for exact D-Code descriptor revision"))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "exact D-Code descriptor revision fetch failed: application_id={application_id} dserv_id={dserv_id} descriptor_digest={descriptor_digest} status={status} body={body}"
        ));
    }
    serde_json::from_str(&body)
        .with_context(|| format!("parse CanonicalDServArtifactV1 response JSON for descriptor_digest={descriptor_digest}: {body}"))
}

/// Phase 117H (task Sections 27/28): the paired result of a governed
/// preflight -- a fresh authorization plus the EXACT immutable descriptor
/// revision it names, already cross-checked against each other and against
/// the caller's own governed request. Not a wire type (no `serde`, no new
/// protocol schema) -- purely an internal pairing so every governed caller
/// (the `invoke`/`dcall-dispatch` CLIs, `dcall::delivery`'s destination
/// execution) shares ONE implementation of "authorize, then fetch the
/// exact authorized revision, then verify the relationship" rather than
/// each repeating (and risking silently diverging from) that sequence.
#[derive(Debug)]
pub struct GovernedDCodePreflight {
    pub authorization: DCodeInvocationAuthorizationV1,
    pub artifact: CanonicalDServArtifactV1,
}

/// The ONE governed resolution sequence (task Section 4):
/// fresh authorization -> `authorization.dserv_artifact_descriptor_digest`
/// -> exact-revision fetch -> local relational verification -> ready to
/// invoke. No caller-supplied artifact_id/version/local file/"latest" ever
/// participates in selecting which revision this returns.
///
/// Cross-checks performed here, and why (task Sections 10/11):
///
/// - `authorization.{project_id,application_id,service_id,node_id}` are
///   compared against the CALLER'S OWN request parameters (never merely
///   asserted against themselves). This is real, non-duplicated defense:
///   nothing else in this crate re-derives "does the HTTP response we
///   actually received correspond to the request we actually sent" --
///   `invoke_dcode` only ever checks internal consistency BETWEEN the
///   `authorization` and `artifact` values it is handed, so a misrouted
///   proxy/cache or a server-side routing bug that silently returned an
///   authorization for a DIFFERENT application/service/node would
///   otherwise go completely undetected until (at best) a much more
///   confusing failure deep inside invocation. `node_id` in particular is
///   later re-checked again inside `invoke_dcode` against `this_node_id`
///   (necessarily -- that is the actual execution-time boundary); the copy
///   here catches the same class of problem earlier, with a clearer error.
/// - `artifact.application_id`/`artifact.dserv_id`/recomputed descriptor
///   digest/`artifact.dcode.sha256` are compared against `authorization`'s
///   own fields. `invoke_dcode` performs this exact set of checks too
///   (necessarily, as the final execution-boundary defense -- see its own
///   doc comment) -- duplicating them here is deliberate, for early, clear,
///   preflight-stage error reporting distinct from an in-invocation
///   failure, using the SAME shared `recompute_dserv_artifact_digest`
///   function both sides call, never a hand-rolled second digest rule.
pub fn fetch_governed_dcode_execution_material(
    base_url: &str,
    project_id: &str,
    application_id: &str,
    service_id: &str,
    node_id: &str,
) -> Result<GovernedDCodePreflight> {
    let authorization = fetch_dcode_authorization(base_url, project_id, application_id, service_id, node_id)
        .context("fetch D-Code authorization from DServer (governed path)")?;

    if authorization.project_id != project_id
        || authorization.application_id != application_id
        || authorization.service_id != service_id
        || authorization.node_id != node_id
    {
        return Err(anyhow!(
            "D-Code authorization response does not correspond to the requested context: requested (project_id={project_id}, application_id={application_id}, service_id={service_id}, node_id={node_id}), authorization named (project_id={}, application_id={}, service_id={}, node_id={})",
            authorization.project_id, authorization.application_id, authorization.service_id, authorization.node_id
        ));
    }

    let artifact = fetch_dserv_artifact_revision(
        base_url,
        application_id,
        service_id,
        &authorization.dserv_artifact_descriptor_digest,
    )
    .context("fetch exact authorized D-Code descriptor revision from DServer (governed path)")?;

    if artifact.application_id != authorization.application_id {
        return Err(anyhow!(
            "fetched D-Code artifact application_id '{}' does not match authorization application_id '{}'",
            artifact.application_id, authorization.application_id
        ));
    }
    if artifact.dserv_id != authorization.service_id {
        return Err(anyhow!(
            "fetched D-Code artifact dserv_id '{}' does not match authorization service_id '{}'",
            artifact.dserv_id, authorization.service_id
        ));
    }
    let recomputed_descriptor_digest = recompute_dserv_artifact_digest(&artifact)
        .map_err(|error| anyhow!("recompute descriptor digest of fetched D-Code artifact: {error}"))?;
    if recomputed_descriptor_digest != authorization.dserv_artifact_descriptor_digest {
        return Err(anyhow!(
            "fetched D-Code artifact's recomputed descriptor digest '{recomputed_descriptor_digest}' does not match authorization's descriptor digest '{}' -- refusing to invoke a revision other than the one actually authorized",
            authorization.dserv_artifact_descriptor_digest
        ));
    }
    if artifact.dcode.sha256 != authorization.dserv_artifact_digest {
        return Err(anyhow!(
            "fetched D-Code artifact module digest '{}' does not match authorization module digest '{}'",
            artifact.dcode.sha256, authorization.dserv_artifact_digest
        ));
    }

    Ok(GovernedDCodePreflight { authorization, artifact })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn authorize_endpoint_url_is_correct() {
        let url = authorize_endpoint_url("http://localhost:8080").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/v1/dcode/authorize");
    }

    #[test]
    fn artifact_endpoint_url_is_correct() {
        let url = artifact_endpoint_url("http://localhost:8080", "app:sensor-pipeline", "decode").unwrap();
        assert_eq!(url.as_str(), "http://localhost:8080/api/v1/dcode/artifacts/app:sensor-pipeline/decode");
    }

    /// Minimal hand-rolled single-request HTTP mock (standard library only,
    /// no mocking crate/dev-dependency) proving a REAL HTTP round trip: a
    /// TCP listener on an OS-assigned loopback port responds with a fixed
    /// status/body, and `fetch_dcode_authorization` is exercised against it
    /// exactly as it would be against a real DServer.
    fn serve_one_http_response(status_line: &'static str, body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf); // drain the request, ignore its content
                let response = format!(
                    "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn fetch_dcode_authorization_parses_a_real_http_success_response() {
        let authorization_json = serde_json::json!({
            "authorization_id": "dcode-auth:proj:app:svc:node:1",
            "project_id": "proj:lab", "application_id": "app:a", "service_id": "svc", "node_id": "node-1",
            "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
            "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
            "dserv_artifact_id": "dserv:a:svc",
            "dserv_artifact_digest": "a".repeat(64),
            "dserv_artifact_descriptor_digest": format!("sha256:{}", "3".repeat(64)),
            "dnode_abi": "datum-dnode/0",
        })
        .to_string();
        let base_url = serve_one_http_response("HTTP/1.1 200 OK", authorization_json);
        let authorization =
            fetch_dcode_authorization(&base_url, "proj:lab", "app:a", "svc", "node-1").expect("should succeed");
        assert_eq!(authorization.node_id, "node-1");
        assert_eq!(authorization.dserv_artifact_descriptor_digest, format!("sha256:{}", "3".repeat(64)));
    }

    #[test]
    fn fetch_dcode_authorization_surfaces_a_real_http_error_response() {
        let base_url = serve_one_http_response(
            "HTTP/1.1 422 Unprocessable Entity",
            serde_json::json!({"error": "dcode_application_not_active", "message": "no active DMap"}).to_string(),
        );
        let error = fetch_dcode_authorization(&base_url, "proj:lab", "app:a", "svc", "node-1").unwrap_err();
        assert!(format!("{error}").contains("422"));
    }

    #[test]
    fn fetch_dserv_artifact_parses_a_real_http_success_response() {
        let artifact_json = serde_json::json!({
            "schema": "datum.dserv-artifact/1", "artifact_id": "dserv:a:svc", "version": "0.1.0",
            "application_id": "app:a", "dserv_id": "svc",
            "dcode": {"uri": format!("datum-blob://sha256/{}", "a".repeat(64)), "sha256": "a".repeat(64), "content_id": format!("sha256:{}", "a".repeat(64)), "size_bytes": 10},
            "abi": {"name": "datum-dnode", "version": "0", "required_exports": ["memory","datum_abi_version","datum_alloc","datum_dealloc","datum_handle"], "allowed_imports": []},
            "ports": {"inputs": [], "outputs": []},
            "limits": {"fuel_per_invocation": 1, "timeout_ms": 1, "max_memory_pages": 1, "max_message_bytes": 1, "max_emits_per_invocation": 1},
            "safety": {"filesystem_access": false, "network_access": false, "host_command_access": false, "docker_access": false, "side_effects_allowed": false},
            "state_model": "stateless", "instantiation": "fresh_instance_per_invocation",
        })
        .to_string();
        let base_url = serve_one_http_response("HTTP/1.1 200 OK", artifact_json);
        let artifact = fetch_dserv_artifact(&base_url, "app:a", "svc").expect("should succeed");
        assert_eq!(artifact.artifact_id, "dserv:a:svc");
    }

    // =======================================================================
    // Phase 117H: exact-revision endpoint/fetch/governed-preflight test
    // matrix (task Sections 21-26/38).
    // =======================================================================

    #[test]
    fn artifact_revision_endpoint_url_is_correct() {
        // Task Section 21: the descriptor digest's literal `:` must be
        // percent-encoded by `Url::path_segments_mut`, never hand-built.
        let digest = format!("sha256:{}", "a".repeat(64));
        let url = artifact_revision_endpoint_url("http://localhost:8080", "app:sensor-pipeline", "decode", &digest).unwrap();
        let segments: Vec<&str> = url.path_segments().unwrap().collect();
        assert_eq!(segments, ["api", "v1", "dcode", "artifacts", "app:sensor-pipeline", "decode", digest.as_str()]);
        // Url::path_segments() already decodes each segment back to its
        // original text -- confirming round-trip correctness independent of
        // exactly how `:` happens to be percent-encoded on the wire.
    }

    fn sample_artifact_json(application_id: &str, dserv_id: &str, timeout_ms: u64) -> serde_json::Value {
        serde_json::json!({
            "schema": "datum.dserv-artifact/1", "artifact_id": format!("dserv:{application_id}:{dserv_id}"), "version": "0.1.0",
            "application_id": application_id, "dserv_id": dserv_id,
            "dcode": {"uri": format!("datum-blob://sha256/{}", "a".repeat(64)), "sha256": "a".repeat(64), "content_id": format!("sha256:{}", "a".repeat(64)), "size_bytes": 10},
            "abi": {"name": "datum-dnode", "version": "0", "required_exports": ["memory","datum_abi_version","datum_alloc","datum_dealloc","datum_handle"], "allowed_imports": []},
            "ports": {"inputs": [], "outputs": []},
            "limits": {"fuel_per_invocation": 1, "timeout_ms": timeout_ms, "max_memory_pages": 1, "max_message_bytes": 1, "max_emits_per_invocation": 1},
            "safety": {"filesystem_access": false, "network_access": false, "host_command_access": false, "docker_access": false, "side_effects_allowed": false},
            "state_model": "stateless", "instantiation": "fresh_instance_per_invocation",
        })
    }

    fn parsed_artifact(json: &serde_json::Value) -> CanonicalDServArtifactV1 {
        serde_json::from_value(json.clone()).unwrap()
    }

    #[test]
    fn fetch_dserv_artifact_revision_parses_a_real_http_success_response() {
        let artifact_json = sample_artifact_json("app:a", "svc", 1).to_string();
        let base_url = serve_one_http_response("HTTP/1.1 200 OK", artifact_json);
        let artifact = fetch_dserv_artifact_revision(&base_url, "app:a", "svc", &format!("sha256:{}", "1".repeat(64)))
            .expect("should succeed");
        assert_eq!(artifact.artifact_id, "dserv:app:a:svc");
    }

    #[test]
    fn fetch_dserv_artifact_revision_surfaces_a_real_http_404() {
        let base_url = serve_one_http_response(
            "HTTP/1.1 404 Not Found",
            serde_json::json!({"error": "no such revision"}).to_string(),
        );
        let error = fetch_dserv_artifact_revision(&base_url, "app:a", "svc", &format!("sha256:{}", "9".repeat(64))).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("404"), "error should surface the HTTP status: {message}");
        assert!(message.contains("svc"), "error should name the failing service: {message}");
    }

    /// Serves exactly TWO sequential HTTP responses over the SAME loopback
    /// listener, in order -- `fetch_governed_dcode_execution_material` makes
    /// exactly two requests (POST /authorize, then GET the exact revision)
    /// against the same base URL, and this reproduces that real two-request
    /// sequence over a genuine TCP round trip (no mocking crate).
    fn serve_two_http_responses(
        first: (&'static str, String),
        second: (&'static str, String),
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (status_line, body) in [first, second] {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = stream.read(&mut buf);
                    let response = format!(
                        "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
            }
        });
        format!("http://{addr}")
    }

    fn authorization_json(
        project_id: &str,
        application_id: &str,
        service_id: &str,
        node_id: &str,
        dserv_artifact_digest: &str,
        dserv_artifact_descriptor_digest: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "authorization_id": format!("dcode-auth:{project_id}:{application_id}:{service_id}:{node_id}:1"),
            "project_id": project_id, "application_id": application_id, "service_id": service_id, "node_id": node_id,
            "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
            "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
            "dserv_artifact_id": format!("dserv:{application_id}:{service_id}"),
            "dserv_artifact_digest": dserv_artifact_digest,
            "dserv_artifact_descriptor_digest": dserv_artifact_descriptor_digest,
            "dnode_abi": "datum-dnode/0",
        })
    }

    /// Task Section 38: the exact scenario the OLD logical fetch could not
    /// support -- an authorization names ONE specific revision (D2) while
    /// ANOTHER revision of the same logical service (D1, different content,
    /// same module digest) also legitimately exists server-side. The
    /// governed preflight must succeed and return exactly D2, regardless of
    /// D1's coexistence (this mock never even needs to serve D1's content --
    /// the point is the client never asks for "the" artifact by logical
    /// identity at all, only by D2's own exact digest).
    #[test]
    fn governed_preflight_succeeds_in_the_multi_revision_scenario() {
        let d2_json = sample_artifact_json("app:a", "svc", 2);
        let d2_digest = recompute_dserv_artifact_digest(&parsed_artifact(&d2_json)).unwrap();
        let auth_json = authorization_json("proj:lab", "app:a", "svc", "node-1", "a".repeat(64).as_str(), &d2_digest).to_string();
        let base_url = serve_two_http_responses(
            ("HTTP/1.1 200 OK", auth_json),
            ("HTTP/1.1 200 OK", d2_json.to_string()),
        );
        let preflight = fetch_governed_dcode_execution_material(&base_url, "proj:lab", "app:a", "svc", "node-1")
            .expect("governed preflight must succeed when the authorized revision is fetched by its own exact digest, regardless of how many other revisions coexist");
        assert_eq!(preflight.authorization.dserv_artifact_descriptor_digest, d2_digest);
        assert_eq!(preflight.artifact.limits.timeout_ms, 2);
    }

    /// Task Section 24: critical. Authorization names descriptor D1; the
    /// exact-digest GET unexpectedly returns descriptor D2 instead (a
    /// server route bug, a misconfigured proxy/cache, or a malicious
    /// response). The governed preflight must fail BEFORE any invocation is
    /// attempted -- it must never silently proceed with whatever content the
    /// GET happened to return.
    #[test]
    fn governed_preflight_fails_closed_on_wrong_descriptor_response() {
        let d1_json = sample_artifact_json("app:a", "svc", 1);
        let d1_digest = recompute_dserv_artifact_digest(&parsed_artifact(&d1_json)).unwrap();
        let d2_json = sample_artifact_json("app:a", "svc", 2); // different content -> different digest
        let auth_json = authorization_json("proj:lab", "app:a", "svc", "node-1", "a".repeat(64).as_str(), &d1_digest).to_string();
        let base_url = serve_two_http_responses(
            ("HTTP/1.1 200 OK", auth_json),
            ("HTTP/1.1 200 OK", d2_json.to_string()), // wrong revision returned
        );
        let error = fetch_governed_dcode_execution_material(&base_url, "proj:lab", "app:a", "svc", "node-1").unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("does not match authorization's descriptor digest"),
            "expected a descriptor-digest-mismatch failure, got: {message}"
        );
    }

    /// Task Section 25: same module bytes (`dcode.sha256` identical), but a
    /// different descriptor overall (different `limits`) -- proves the
    /// governed preflight binds to the WHOLE descriptor revision, not merely
    /// the module bytes. Authorization names D1 (timeout_ms=1); the server
    /// returns D2 (timeout_ms=2, same module sha256). Must fail closed.
    #[test]
    fn governed_preflight_fails_closed_on_same_module_different_descriptor() {
        let d1_json = sample_artifact_json("app:a", "svc", 1);
        let d1_digest = recompute_dserv_artifact_digest(&parsed_artifact(&d1_json)).unwrap();
        let d2_json = sample_artifact_json("app:a", "svc", 2);
        assert_eq!(d1_json["dcode"]["sha256"], d2_json["dcode"]["sha256"], "test setup: module digest must be identical between D1 and D2");
        let auth_json = authorization_json("proj:lab", "app:a", "svc", "node-1", "a".repeat(64).as_str(), &d1_digest).to_string();
        let base_url = serve_two_http_responses(
            ("HTTP/1.1 200 OK", auth_json),
            ("HTTP/1.1 200 OK", d2_json.to_string()),
        );
        let error = fetch_governed_dcode_execution_material(&base_url, "proj:lab", "app:a", "svc", "node-1").unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("does not match authorization's descriptor digest"),
            "expected a descriptor-digest-mismatch failure (same module, different policy), got: {message}"
        );
    }

    /// Task Section 26: an authorization whose `dserv_artifact_digest`
    /// (module digest) is internally inconsistent with its own
    /// `dserv_artifact_descriptor_digest` (i.e. does not match the module
    /// digest embedded in the descriptor that digest actually names) must
    /// be rejected -- proving the governed preflight independently checks
    /// the module-digest relation too, not only the whole-descriptor digest.
    #[test]
    fn governed_preflight_fails_closed_on_inconsistent_module_digest() {
        let artifact_json = sample_artifact_json("app:a", "svc", 1);
        let real_descriptor_digest = recompute_dserv_artifact_digest(&parsed_artifact(&artifact_json)).unwrap();
        // The authorization's own descriptor digest correctly names this
        // real artifact, but its module digest field is a LIE relative to
        // that same artifact's real `dcode.sha256`.
        let wrong_module_digest = "9".repeat(64);
        let auth_json = authorization_json("proj:lab", "app:a", "svc", "node-1", &wrong_module_digest, &real_descriptor_digest).to_string();
        let base_url = serve_two_http_responses(
            ("HTTP/1.1 200 OK", auth_json),
            ("HTTP/1.1 200 OK", artifact_json.to_string()),
        );
        let error = fetch_governed_dcode_execution_material(&base_url, "proj:lab", "app:a", "svc", "node-1").unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("module digest"),
            "expected a module-digest-mismatch failure, got: {message}"
        );
    }

    /// A misrouted/malicious authorization response naming a DIFFERENT
    /// application/service/node than what was actually requested must be
    /// rejected before any artifact fetch is even attempted (task Section
    /// 10) -- this is the one relational check nothing downstream
    /// (`invoke_dcode`) performs, since it only ever checks internal
    /// consistency between whatever `authorization`/`artifact` it is
    /// handed, never against the caller's original intent.
    #[test]
    fn governed_preflight_fails_closed_on_authorization_context_mismatch() {
        // Authorization claims to be for a DIFFERENT service than requested.
        let auth_json = authorization_json("proj:lab", "app:a", "svc-OTHER", "node-1", &"a".repeat(64), &format!("sha256:{}", "5".repeat(64))).to_string();
        let base_url = serve_one_http_response("HTTP/1.1 200 OK", auth_json);
        let error = fetch_governed_dcode_execution_material(&base_url, "proj:lab", "app:a", "svc", "node-1").unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("does not correspond to the requested context"),
            "expected a context-mismatch failure, got: {message}"
        );
    }

    #[test]
    fn governed_preflight_surfaces_authorization_http_failure() {
        let base_url = serve_one_http_response(
            "HTTP/1.1 422 Unprocessable Entity",
            serde_json::json!({"error": "dcode_application_not_active", "message": "no active DMap"}).to_string(),
        );
        let error = fetch_governed_dcode_execution_material(&base_url, "proj:lab", "app:a", "svc", "node-1").unwrap_err();
        assert!(format!("{error:#}").contains("422"));
    }
}
