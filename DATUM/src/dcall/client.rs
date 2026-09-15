//! Control-plane HTTP client for D-Call (ADR-0022), mirroring
//! `dcode::client`'s established pattern exactly.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::dcall::grant::{DCallDeliveryGrantV1, RequestGrantResponse, SourceExecutionBindingV1};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Only these known deterministic rejections may become persistent codes.
/// Arbitrary server text is represented by an exact-byte digest, never copied.
#[derive(Debug)]
pub struct GrantRequestRejection {
    pub code: Option<crate::dcall::evidence::DCallDeliveryErrorCode>,
    status: u16,
    body_sha256: String,
}

impl GrantRequestRejection {
    fn from_response(status: u16, body: &str) -> Self {
        use crate::dcall::evidence::DCallDeliveryErrorCode::*;
        let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
        let code = match parsed.as_ref().and_then(|value| value.get("error")).and_then(|value| value.as_str()) {
            Some("dcall_source_execution_stale") => Some(DCallSourceExecutionStale),
            Some("dcall_payload_exceeds_source_message_limit") => Some(DCallPayloadExceedsSourceMessageLimit),
            Some("dcall_payload_exceeds_destination_message_limit") => Some(DCallPayloadExceedsDestinationMessageLimit),
            _ => None,
        };
        Self { code, status, body_sha256: crate::dcode::identity::sha256_hex(body.as_bytes()) }
    }
}

impl std::fmt::Display for GrantRequestRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "D-Call grant request rejected: status={} code={} body_sha256={}", self.status,
            self.code.map(|code| code.as_str()).unwrap_or("dcall_grant_request_failed"), self.body_sha256)
    }
}

impl std::error::Error for GrantRequestRejection {}

fn client() -> Result<Client> {
    Client::builder().timeout(REQUEST_TIMEOUT).build().context("build blocking reqwest client")
}

/// Lists the `call_id`s of every D-Call edge sourced from
/// `source_service_id` in the application's CURRENT active D-Graph. The
/// caller never supplies a destination -- only which SOURCE service/emit it
/// is dispatching for.
pub fn list_calls_for_source_service(base_url: &str, project_id: &str, application_id: &str, source_service_id: &str) -> Result<Vec<String>> {
    let url = format!("{}/api/v1/dcall/calls/{project_id}/{application_id}/{source_service_id}", base_url.trim_end_matches('/'));
    let response = client()?.get(&url).send().with_context(|| format!("GET {url}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("list D-Calls for source service failed: status={status} body={body}"));
    }
    serde_json::from_str(&body).with_context(|| format!("parse call_id list JSON: {body}"))
}

#[derive(Debug, Serialize)]
struct RequestGrantPayload<'a> {
    project_id: &'a str,
    application_id: &'a str,
    call_id: &'a str,
    source_node_id: &'a str,
    source_output_port: &'a str,
    message_id: &'a str,
    correlation_id: &'a str,
    payload_sha256: &'a str,
    payload_size: u64,
    /// Independent review fix v4 (K2A): the authority the real source
    /// D-Code invocation actually executed under, derived by the caller
    /// exclusively from that invocation's own evidence.
    source_execution: &'a SourceExecutionBindingV1,
    /// Independent review fix v5 (section 9/11): opaque digest reference to
    /// the exact source `DCodeInvocationEvidenceV1` this emit came from.
    source_dcode_evidence_sha256: &'a str,
}

/// `POST /api/v1/dcall/grants`. `source_node_id` is the CALLER's own
/// resolved local D-Node identity (never a destination claim -- there is no
/// destination field on this request at all, matching the server-side
/// `deny_unknown_fields` source guard).
#[allow(clippy::too_many_arguments)]
pub fn request_grant(
    base_url: &str,
    project_id: &str,
    application_id: &str,
    call_id: &str,
    source_node_id: &str,
    source_output_port: &str,
    message_id: &str,
    correlation_id: &str,
    payload_sha256: &str,
    payload_size: u64,
    source_execution: &SourceExecutionBindingV1,
    source_dcode_evidence_sha256: &str,
) -> Result<RequestGrantResponse> {
    let url = format!("{}/api/v1/dcall/grants", base_url.trim_end_matches('/'));
    let payload = RequestGrantPayload {
        project_id,
        application_id,
        call_id,
        source_node_id,
        source_output_port,
        message_id,
        correlation_id,
        payload_sha256,
        payload_size,
        source_execution,
        source_dcode_evidence_sha256,
    };
    let response = client()?.post(&url).json(&payload).send().with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(GrantRequestRejection::from_response(status.as_u16(), &body).into());
    }
    serde_json::from_str(&body).with_context(|| format!("parse RequestGrantResponse JSON: {body}"))
}

#[derive(Debug, Serialize)]
struct RedeemGrantPayload<'a> {
    token: &'a str,
    destination_node_id: &'a str,
    payload_sha256: &'a str,
    payload_size: u64,
}

/// `POST /api/v1/dcall/grants/:grant_id/redeem`. Returns `Err` for ANY
/// non-2xx response -- the caller must treat redemption failure as "do not
/// execute", never optimistically.
pub fn redeem_grant(
    base_url: &str,
    grant_id: &str,
    token: &str,
    destination_node_id: &str,
    payload_sha256: &str,
    payload_size: u64,
) -> Result<DCallDeliveryGrantV1> {
    let url = format!("{}/api/v1/dcall/grants/{grant_id}/redeem", base_url.trim_end_matches('/'));
    let payload = RedeemGrantPayload { token, destination_node_id, payload_sha256, payload_size };
    let response = client()?.post(&url).json(&payload).send().with_context(|| format!("POST {url}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("D-Call grant redemption failed: status={status} body={body}"));
    }
    serde_json::from_str(&body).with_context(|| format!("parse redeemed DCallDeliveryGrantV1 JSON: {body}"))
}

/// `PUT /api/v1/dcall/endpoints/:node_id/:transport` -- used only by
/// `smartsentinel-dcall-listen` to register/update its own reachable
/// address at startup.
pub fn declare_endpoint(base_url: &str, node_id: &str, transport: &str, endpoint_base_url: &str) -> Result<()> {
    let url = format!("{}/api/v1/dcall/endpoints/{node_id}/{transport}", base_url.trim_end_matches('/'));
    let response = client()?
        .put(&url)
        .json(&serde_json::json!({"base_url": endpoint_base_url}))
        .send()
        .with_context(|| format!("PUT {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        return Err(anyhow!("D-Call endpoint declaration failed: status={status} body={body}"));
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct EndpointResponse {
    pub node_id: String,
    pub transport: String,
    pub endpoint_id: String,
    pub revision: u64,
    pub base_url: String,
    pub descriptor_digest: String,
}

/// `GET /api/v1/dcall/endpoints/:node_id/:transport` -- fetches the
/// destination's CURRENT reachable address. Called by the source dispatcher
/// only AFTER a grant has already been issued for a `route_kind=remote`
/// delivery (the grant itself deliberately carries only the endpoint's
/// id/revision/digest, not its raw URL, mirroring the evidence-payload
/// discipline elsewhere in this codebase).
pub fn fetch_endpoint(base_url: &str, node_id: &str, transport: &str) -> Result<EndpointResponse> {
    let url = format!("{}/api/v1/dcall/endpoints/{node_id}/{transport}", base_url.trim_end_matches('/'));
    let response = client()?.get(&url).send().with_context(|| format!("GET {url}"))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("D-Call endpoint fetch failed: status={status} body={body}"));
    }
    serde_json::from_str(&body).with_context(|| format!("parse endpoint JSON: {body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn serve_one_http_response(status_line: &'static str, body: String) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
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
    fn list_calls_parses_a_real_http_success_response() {
        let base_url = serve_one_http_response("HTTP/1.1 200 OK", serde_json::json!(["call-a", "call-b"]).to_string());
        let calls = list_calls_for_source_service(&base_url, "proj:lab", "app:a", "svc").unwrap();
        assert_eq!(calls, vec!["call-a".to_string(), "call-b".to_string()]);
    }

    #[test]
    fn request_grant_surfaces_a_real_http_error_response() {
        let source_execution = SourceExecutionBindingV1 {
            service_id: "svc:a".to_string(),
            node_id: "fog-01".to_string(),
            dgraph_id: "dgraph:a".to_string(),
            dgraph_revision: 1,
            dgraph_digest: format!("sha256:{}", "a".repeat(64)),
            dmap_id: "dmap:a".to_string(),
            dmap_revision: 1,
            dmap_digest: format!("sha256:{}", "a".repeat(64)),
            dserv_artifact_id: "artifact:a".to_string(),
            dserv_artifact_digest: "a".repeat(64),
            dserv_artifact_descriptor_digest: format!("sha256:{}", "a".repeat(64)),
        };
        for code in ["dcall_source_execution_stale", "dcall_payload_exceeds_source_message_limit", "dcall_payload_exceeds_destination_message_limit", "arbitrary-secret-server-code"] {
            let base_url = serve_one_http_response("HTTP/1.1 422 Unprocessable Entity",
                serde_json::json!({"error":code, "message":"secret-server-message"}).to_string());
            let error = request_grant(&base_url, "proj", "app", "call", "fog-01", "out", "m1", "c1", &"a".repeat(64), 4, &source_execution, &"b".repeat(64)).unwrap_err();
            let rejection = error.downcast_ref::<GrantRequestRejection>().unwrap();
            assert_eq!(rejection.code.map(|value| value.as_str()), (!code.starts_with("arbitrary")).then_some(code));
            assert!(error.to_string().contains("422"));
            assert!(!error.to_string().contains("secret"));
        }
    }
}
