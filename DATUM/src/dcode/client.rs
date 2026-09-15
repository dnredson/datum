//! Control-plane preflight HTTP client (ADR-0021, independent review v2 R1/
//! R2C). The GOVERNED invocation path calls `fetch_dcode_authorization` and
//! `fetch_dserv_artifact` directly against DServer immediately before every
//! invocation -- an authorization is ALWAYS freshly derived server-side
//! (`dserver::core::dcode_authorization::authorize_dcode_invocation`, never
//! cached there), so calling this again immediately before execution IS the
//! preflight/re-verification step; no separate `/verify` endpoint is needed
//! for that purpose (see `control-plane-preflight-audit.txt`).
//!
//! Documented, honest residual limitation: a control-plane state change can
//! still theoretically occur between this HTTP response and the guest
//! actually executing (distributed TOCTOU) -- this module does not, and
//! cannot, claim distributed atomicity. What it DOES guarantee is that a
//! real invocation can no longer be driven purely from a locally-fabricated
//! authorization/artifact file with no DServer involved at all (the v1 gap
//! this whole R1 fix closes).

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::{blocking::Client, Url};

use crate::dcode::authorization::DCodeInvocationAuthorizationV1;
use crate::dcode::dserv_artifact::CanonicalDServArtifactV1;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub fn authorize_endpoint_url(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse DServer base URL: {base_url}"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("DServer base URL cannot be a base URL"))?
        .clear()
        .extend(["api", "v1", "dcode", "authorize"]);
    Ok(url)
}

pub fn artifact_endpoint_url(base_url: &str, application_id: &str, dserv_id: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).with_context(|| format!("parse DServer base URL: {base_url}"))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("DServer base URL cannot be a base URL"))?
        .clear()
        .extend(["api", "v1", "dcode", "artifacts", application_id, dserv_id]);
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

/// Calls `GET /api/v1/dcode/artifacts/:application_id/:dserv_id` -- the
/// authoritative canonical D-Serv artifact descriptor DServer currently has
/// registered.
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
        .with_context(|| format!("GET {endpoint} for canonical D-Serv artifact"))?;

    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("D-Serv artifact fetch failed: status={status} body={body}"));
    }
    serde_json::from_str(&body).with_context(|| format!("parse CanonicalDServArtifactV1 response JSON: {body}"))
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
}
