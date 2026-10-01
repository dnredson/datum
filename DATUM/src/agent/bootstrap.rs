use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::resource_discovery::{
    collect_unbound_resource_discovery, derive_resource_lookup_id,
    SmartSentinelResourceDiscovery,
};
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapState {
    KnownBoundNode,
    KnownUnboundResource,
    UnknownResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartSentinelBootstrapReport {
    pub bootstrap_state: BootstrapState,
    pub resource_discovery: SmartSentinelResourceDiscovery,
    pub control_plane: ControlPlaneBootstrapContext,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPlaneBootstrapContext {
    pub dserver_url: String,
    pub project_id: String,
    pub resource_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_node_id: Option<String>,
    pub node_known: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_record: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_snapshot: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_capabilities: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentinel_registration: Option<Value>,
}

pub fn run_bootstrap(
    cfg: &Config,
    requested_node_id: Option<&str>,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
) -> Result<SmartSentinelBootstrapReport> {
    let resource_discovery = collect_unbound_resource_discovery(cfg, hostname_override)?;
    run_bootstrap_with_resource_discovery(
        resource_discovery,
        requested_node_id,
        dserver_url,
        project_id,
        timeout_s,
    )
}

pub fn run_bootstrap_with_resource_discovery(
    resource_discovery: SmartSentinelResourceDiscovery,
    requested_node_id: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
) -> Result<SmartSentinelBootstrapReport> {
    if project_id.trim().is_empty() {
        bail!("SmartSentinel bootstrap project_id must not be empty");
    }
    if dserver_url.trim().is_empty() {
        bail!("SmartSentinel bootstrap dserver_url must not be empty");
    }
    if timeout_s == 0 {
        bail!("SmartSentinel bootstrap timeout_s must be greater than zero");
    }
    resource_discovery.validate()?;

    let requested_node_id = requested_node_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let base = dserver_url.trim().trim_end_matches('/');
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_s))
        .build()
        .context("build SmartSentinel bootstrap HTTP client")?;

    let resource_id = derive_resource_lookup_id(&resource_discovery);
    let resource_url = format!("{base}/api/v1/resources/{resource_id}");
    let resource_response = client
        .get(&resource_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver resource context {resource_url}"))?;
    let resource_record = optional_json_response(
        resource_response,
        "resource registry",
        StatusCode::NOT_FOUND,
    )?;

    let registry_assigned_node_id = resource_record
        .as_ref()
        .and_then(|value| value.get("assigned_node_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    if let (Some(requested), Some(assigned)) = (
        requested_node_id.as_deref(),
        registry_assigned_node_id.as_deref(),
    ) {
        if requested != assigned {
            bail!(
                "resource registry binds local resource '{}' to node '{}' but bootstrap requested node '{}'; refusing conflicting role",
                resource_id,
                assigned,
                requested
            );
        }
    }

    let node_candidate = registry_assigned_node_id
        .clone()
        .or_else(|| requested_node_id.clone());
    let node_context_query = match node_candidate.as_deref() {
        Some(node_id) => query_node_context(&client, base, project_id, node_id),
        None => Ok((false, None)),
    };
    // Phase 119H1 (Phase 119H0 decision B): a legacy Firebase-backed
    // `/nodes/:id/latest` failure must not abort bootstrap once the
    // Resource Registry ALREADY supplies an authoritative
    // `assigned_node_id` -- see `resolve_node_context_enrichment`'s own
    // doc comment for the full reasoning.
    let (node_known, latest_snapshot) =
        resolve_node_context_enrichment(registry_assigned_node_id.is_some(), node_context_query)?;

    let bootstrap_state = classify_bootstrap_state(
        node_known,
        resource_record.is_some(),
        registry_assigned_node_id.is_some(),
    );

    let resolved_node_id = if node_known {
        node_candidate.clone()
    } else {
        registry_assigned_node_id.clone()
    };

    // Phase 119H1: `expected_capabilities` is legacy-Firebase-backed and
    // confirmed unused by reconciliation (Phase 119H0 §6) -- its failure
    // must never abort bootstrap, regardless of binding state. Sentinel
    // registration (`query_optional_registration`) is unchanged and
    // deliberately left load-bearing (Phase 119H0 found it Firebase-free;
    // Phase 119H1 does not broaden scope to it, per its own task's
    // explicit instruction).
    let (expected_capabilities, sentinel_registration) = match resolved_node_id.as_deref() {
        Some(node_id) => (
            capability_enrichment_or_none(query_optional_capabilities(&client, base, project_id, node_id)),
            query_optional_registration(&client, base, node_id)?,
        ),
        None => (None, None),
    };

    let control_plane = ControlPlaneBootstrapContext {
        dserver_url: base.to_string(),
        project_id: project_id.trim().to_string(),
        resource_id,
        requested_node_id,
        resolved_node_id,
        node_known,
        resource_record,
        latest_snapshot,
        expected_capabilities,
        sentinel_registration,
    };

    Ok(SmartSentinelBootstrapReport {
        bootstrap_state,
        resource_discovery,
        control_plane,
        side_effects_performed: false,
    })
}

/// Phase 119H1 (Phase 119H0 decision B, §9 point 1): the exact
/// graceful-degradation policy for `query_node_context`'s result
/// (`GET /api/v1/nodes/:id/latest`, legacy-Firebase-backed server-side).
///
/// - `registry_binding_present == true`: the Resource Registry (already
///   Firebase-free, already queried above) supplies an authoritative
///   `assigned_node_id` independent of this call entirely --
///   `classify_bootstrap_state` already treats that alone as sufficient
///   for `KnownBoundNode`, and `resolved_node_id` already falls back to
///   it whenever `node_known` is `false`. A failure here (Firebase
///   outage surfaced as a 500, a decode error, a transport error --
///   anything, not merely a specific status code) therefore degrades to
///   `(false, None)`: the enrichment fields (`node_known`,
///   `latest_snapshot`) are simply lost, nothing authoritative is.
/// - `registry_binding_present == false`: this call is the ONLY
///   Firebase-free-or-not `node_known`-equivalent signal wired up today
///   (Phase 119H0 §7/§9) -- it remains fully load-bearing, and its
///   failure propagates exactly as before. This is deliberate and
///   required, not an oversight: converting a requested `--node-id` plus
///   a failed/absent lookup into `KnownBoundNode` would make
///   caller-supplied identity authoritative, which is never permitted
///   (Phase 119H0 §8; Phase 119H1 task §6/§15).
fn resolve_node_context_enrichment(
    registry_binding_present: bool,
    query_result: Result<(bool, Option<Value>)>,
) -> Result<(bool, Option<Value>)> {
    match query_result {
        Ok(result) => Ok(result),
        Err(_) if registry_binding_present => Ok((false, None)),
        Err(error) => Err(error),
    }
}

/// Phase 119H1 (Phase 119H0 decision B, §9 point 2): `expected_capabilities`
/// (`GET /api/v1/nodes/:id/capabilities`, also legacy-Firebase-backed
/// server-side) is confirmed, by exhaustive search, never consumed by
/// operational reconciliation in any case (Phase 119H0 §6) -- ANY failure
/// (404, 500, decode error, transport error) therefore degrades to `None`
/// unconditionally, regardless of Resource Registry binding state. Unlike
/// `resolve_node_context_enrichment` above, there is no load-bearing case
/// for this call at all.
fn capability_enrichment_or_none(query_result: Result<Option<Value>>) -> Option<Value> {
    query_result.unwrap_or(None)
}

pub fn classify_bootstrap_state(
    node_known: bool,
    resource_known: bool,
    resource_has_assigned_node: bool,
) -> BootstrapState {
    if node_known || resource_has_assigned_node {
        BootstrapState::KnownBoundNode
    } else if resource_known {
        BootstrapState::KnownUnboundResource
    } else {
        BootstrapState::UnknownResource
    }
}

fn query_node_context(
    client: &Client,
    base: &str,
    project_id: &str,
    node_id: &str,
) -> Result<(bool, Option<Value>)> {
    let node_url = format!("{base}/api/v1/nodes/{}/latest", node_id.trim());
    let response = client
        .get(&node_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver node context {node_url}"))?;
    match response.status() {
        StatusCode::OK => Ok((
            true,
            Some(
                response
                    .json::<Value>()
                    .context("decode known-node latest snapshot response")?,
            ),
        )),
        StatusCode::NOT_FOUND => Ok((false, None)),
        other => {
            let body = response.text().unwrap_or_default();
            bail!(
                "dserver node lookup returned unexpected status {other}; refusing to guess bootstrap state: {body}"
            )
        }
    }
}

fn query_optional_capabilities(
    client: &Client,
    base: &str,
    project_id: &str,
    node_id: &str,
) -> Result<Option<Value>> {
    let url = format!("{base}/api/v1/nodes/{}/capabilities", node_id.trim());
    let response = client
        .get(&url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("query dserver node capabilities {url}"))?;
    optional_json_response(response, "node capabilities", StatusCode::NOT_FOUND)
}

fn query_optional_registration(
    client: &Client,
    base: &str,
    node_id: &str,
) -> Result<Option<Value>> {
    let url = format!("{base}/api/v1/sentinels/{}", node_id.trim());
    let response = client
        .get(&url)
        .send()
        .with_context(|| format!("query dserver Sentinel registration {url}"))?;
    optional_json_response(response, "Sentinel registration", StatusCode::NOT_FOUND)
}

fn optional_json_response(
    response: reqwest::blocking::Response,
    label: &str,
    absent_status: StatusCode,
) -> Result<Option<Value>> {
    let status = response.status();
    if status == absent_status {
        return Ok(None);
    }
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("dserver {label} query failed with status {status}: {body}");
    }
    response
        .json::<Value>()
        .map(Some)
        .with_context(|| format!("decode dserver {label} response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_classification_distinguishes_three_resource_states() {
        assert_eq!(
            classify_bootstrap_state(true, true, true),
            BootstrapState::KnownBoundNode
        );
        assert_eq!(
            classify_bootstrap_state(false, true, true),
            BootstrapState::KnownBoundNode
        );
        assert_eq!(
            classify_bootstrap_state(false, true, false),
            BootstrapState::KnownUnboundResource
        );
        assert_eq!(
            classify_bootstrap_state(false, false, false),
            BootstrapState::UnknownResource
        );
    }

    // ---- Phase 119H1: resolve_node_context_enrichment (B1, B3, B4, B6) -- //

    #[test]
    fn b1_binding_present_plus_successful_query_preserves_node_known_and_snapshot() {
        let snapshot = serde_json::json!({"observed_at_utc": "2026-01-01T00:00:00Z"});
        let result = resolve_node_context_enrichment(true, Ok((true, Some(snapshot.clone()))));
        assert_eq!(result.unwrap(), (true, Some(snapshot)));
    }

    #[test]
    fn b3_binding_present_plus_query_error_degrades_to_false_none_without_erroring() {
        let result = resolve_node_context_enrichment(true, Err(anyhow::anyhow!("dserver node lookup returned unexpected status 500 Internal Server Error: firestore unavailable")));
        assert_eq!(result.unwrap(), (false, None));
    }

    #[test]
    fn b4_no_binding_plus_successful_query_preserves_existing_behavior() {
        let snapshot = serde_json::json!({"observed_at_utc": "2026-01-01T00:00:00Z"});
        let result = resolve_node_context_enrichment(false, Ok((true, Some(snapshot.clone()))));
        assert_eq!(result.unwrap(), (true, Some(snapshot)));
    }

    #[test]
    fn b6_no_binding_plus_query_error_propagates_the_error() {
        let result = resolve_node_context_enrichment(false, Err(anyhow::anyhow!("dserver node lookup returned unexpected status 500 Internal Server Error: firestore unavailable")));
        assert!(result.is_err(), "without a Resource Registry binding, query_node_context remains load-bearing and its failure must propagate");
    }

    // B2/B5: the same policy threaded through `classify_bootstrap_state`,
    // proving the END-TO-END classification outcome, not merely the raw
    // tuple `resolve_node_context_enrichment` returns.

    #[test]
    fn b2_binding_present_plus_404_context_still_yields_known_bound_node() {
        // A 404 from query_node_context is already Ok((false, None)) --
        // this proves that outcome, combined with an existing binding,
        // reaches KnownBoundNode via the binding alone, never via node_known.
        let (node_known, _snapshot) = resolve_node_context_enrichment(true, Ok((false, None))).unwrap();
        assert!(!node_known);
        assert_eq!(
            classify_bootstrap_state(node_known, true, true),
            BootstrapState::KnownBoundNode
        );
    }

    #[test]
    fn b5_no_binding_plus_404_context_never_reaches_known_bound_node_from_requested_node_alone() {
        let (node_known, _snapshot) = resolve_node_context_enrichment(false, Ok((false, None))).unwrap();
        assert!(!node_known);
        // resource_known=true (the resource itself is registered) but
        // resource_has_assigned_node=false (no binding) -- a requested
        // --node-id that the (Firebase-backed) lookup could not confirm
        // must never, by itself, become KnownBoundNode.
        assert_eq!(
            classify_bootstrap_state(node_known, true, false),
            BootstrapState::KnownUnboundResource
        );
    }

    // ---- Phase 119H1: capability_enrichment_or_none (B8, B9, B10) ------- //

    #[test]
    fn b8_capability_success_yields_some() {
        let caps = serde_json::json!({"docker": true});
        assert_eq!(capability_enrichment_or_none(Ok(Some(caps.clone()))), Some(caps));
    }

    #[test]
    fn b9_capability_404_yields_none() {
        // query_optional_capabilities already maps 404 to Ok(None) itself;
        // the degradation helper must pass that through unchanged.
        assert_eq!(capability_enrichment_or_none(Ok(None)), None);
    }

    #[test]
    fn b10_capability_error_yields_none_never_propagates() {
        let result = capability_enrichment_or_none(Err(anyhow::anyhow!("dserver node capabilities query failed with status 500: firestore unavailable")));
        assert_eq!(result, None);
    }

    // ---- Phase 119H1: end-to-end mock-DServer proof (task §21/§22) ------ //

    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A minimal, local (no external mocking crate), raw-socket mock
    /// DServer, adapted from the established `MockDServer` pattern already
    /// used by `outputs::dmonitor_http::tests`/`dcode::client::tests` --
    /// extended here to route by exact request PATH (a real bootstrap
    /// cycle hits several distinct endpoints with different required
    /// responses), not a single canned response for every request.
    struct PathRoutedMockDServer {
        url: String,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl PathRoutedMockDServer {
        /// `routes` maps an exact request path (e.g.
        /// `"/api/v1/resources/resource:os:aaaa..."`) to `(status, body)`.
        /// Any unmatched path replies 404 with an empty JSON object.
        fn start(routes: HashMap<String, (u16, String)>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();

            let thread = std::thread::spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream.set_nonblocking(false).unwrap();
                            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                            let mut bytes = Vec::new();
                            let mut buffer = [0u8; 4096];
                            let mut request_line = String::new();
                            loop {
                                let count = match stream.read(&mut buffer) {
                                    Ok(0) => break,
                                    Ok(count) => count,
                                    Err(_) => break,
                                };
                                bytes.extend_from_slice(&buffer[..count]);
                                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&bytes[..end]).to_string();
                                    request_line = headers.lines().next().unwrap_or("").to_string();
                                    let length = headers
                                        .lines()
                                        .find_map(|line| {
                                            let (name, value) = line.split_once(':')?;
                                            name.eq_ignore_ascii_case("content-length")
                                                .then(|| value.trim().parse::<usize>().unwrap_or(0))
                                        })
                                        .unwrap_or(0);
                                    if bytes.len() >= end + 4 + length {
                                        break;
                                    }
                                }
                            }
                            let path = request_line
                                .split_whitespace()
                                .nth(1)
                                .unwrap_or("")
                                .to_string();
                            let (status, body) = routes
                                .get(&path)
                                .cloned()
                                .unwrap_or((404, "{}".to_string()));
                            let reason = match status {
                                200 => "OK",
                                404 => "Not Found",
                                500 => "Internal Server Error",
                                _ => "Status",
                            };
                            let _ = write!(
                                stream,
                                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            );
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                        Err(_) => break,
                    }
                }
            });

            Self { url, stop, thread: Some(thread) }
        }
    }

    impl Drop for PathRoutedMockDServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// Deterministic resource discovery fixture: `hardware_fingerprint_id`
    /// is `None` and `os_installation_id` is a fixed 64-`a` digest, so
    /// `derive_resource_lookup_id` always computes the same
    /// `"resource:os:aaaa...a"` id -- required so the mock server's routes
    /// can be set up ahead of time.
    fn deterministic_discovery() -> SmartSentinelResourceDiscovery {
        use crate::agent::discovery::{
            CpuObservation, IdentityConfidence, IdentityObservation, MemoryObservation,
            OsIdentityBasis, PlatformObservation, ResourceObservation, SwapObservation,
        };
        SmartSentinelResourceDiscovery {
            schema_version: "0.1.0".to_string(),
            observed_at_utc: chrono::Utc::now(),
            instance_id: "instance-119h1".to_string(),
            observation_host_id: "host:probe-119h1".to_string(),
            stage_hint: None,
            identity: IdentityObservation {
                hostname: "host-119h1".to_string(),
                sentinel_version: "0.1.0".to_string(),
                hardware_fingerprint_id: None,
                hardware_identity_basis: Vec::new(),
                hardware_identity_confidence: IdentityConfidence::Unknown,
                hardware_identity_attested: false,
                os_installation_id: Some(format!("osinst:{}", "a".repeat(64))),
                os_identity_basis: OsIdentityBasis::MachineId,
                role_binding_id: None,
                predecessor_binding_id: None,
            },
            platform: PlatformObservation {
                os_family: "linux".to_string(),
                os_name: "Ubuntu".to_string(),
                os_version: "25.10".to_string(),
                arch: "x86_64".to_string(),
            },
            resources: ResourceObservation {
                capacity_profile_id: "capacity:test".to_string(),
                uptime_seconds: Some(1),
                cpu: CpuObservation { logical_cores: 4, usage_percent: Some(1.0) },
                memory: MemoryObservation { total_bytes: 1024, used_bytes: 1, available_bytes: 1023 },
                swap: SwapObservation { total_bytes: 0, used_bytes: 0, available_bytes: 0 },
                filesystems: Vec::new(),
            },
            capabilities: Vec::new(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    const RESOURCE_ID: &str = "resource:os:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn task_section_21_binding_present_with_firebase_backed_endpoints_failing_still_yields_known_bound_node() {
        let mut routes = HashMap::new();
        routes.insert(
            format!("/api/v1/resources/{RESOURCE_ID}"),
            (200u16, serde_json::json!({"resource_id": RESOURCE_ID, "assigned_node_id": "fog-119h1"}).to_string()),
        );
        routes.insert(
            "/api/v1/nodes/fog-119h1/latest".to_string(),
            (500u16, "{\"error\":\"firestore unavailable\"}".to_string()),
        );
        routes.insert(
            "/api/v1/nodes/fog-119h1/capabilities".to_string(),
            (500u16, "{\"error\":\"firestore unavailable\"}".to_string()),
        );
        routes.insert(
            "/api/v1/sentinels/fog-119h1".to_string(),
            (404u16, "{}".to_string()),
        );
        let server = PathRoutedMockDServer::start(routes);

        let report = run_bootstrap_with_resource_discovery(
            deterministic_discovery(),
            None,
            &server.url,
            "proj-119h1",
            5,
        )
        .expect("bootstrap must succeed even though both Firebase-backed endpoints return 500");

        assert_eq!(report.bootstrap_state, BootstrapState::KnownBoundNode);
        assert_eq!(report.control_plane.resolved_node_id.as_deref(), Some("fog-119h1"));
        assert_eq!(report.control_plane.resource_id, RESOURCE_ID);
        assert!(!report.control_plane.node_known, "node_known must have degraded to false, not caused a failure");
        assert!(report.control_plane.latest_snapshot.is_none());
        assert!(report.control_plane.expected_capabilities.is_none());
        assert!(!report.side_effects_performed);
    }

    #[test]
    fn b7_requested_node_id_conflicting_with_registry_binding_still_fails_closed() {
        let mut routes = HashMap::new();
        routes.insert(
            format!("/api/v1/resources/{RESOURCE_ID}"),
            (200u16, serde_json::json!({"resource_id": RESOURCE_ID, "assigned_node_id": "fog-119h1"}).to_string()),
        );
        let server = PathRoutedMockDServer::start(routes);

        let result = run_bootstrap_with_resource_discovery(
            deterministic_discovery(),
            Some("rpi-119h1"),
            &server.url,
            "proj-119h1",
            5,
        );
        assert!(result.is_err(), "a requested node_id conflicting with the Resource Registry's assigned_node_id must still fail closed, unchanged");
        assert!(
            format!("{}", result.unwrap_err()).contains("conflicting role"),
            "the existing conflict message must be preserved"
        );
    }

    #[test]
    fn no_binding_plus_firebase_backed_node_lookup_failure_still_propagates_as_an_error() {
        let mut routes = HashMap::new();
        routes.insert(
            format!("/api/v1/resources/{RESOURCE_ID}"),
            (404u16, "{}".to_string()),
        );
        routes.insert(
            "/api/v1/nodes/rpi-119h1/latest".to_string(),
            (500u16, "{\"error\":\"firestore unavailable\"}".to_string()),
        );
        let server = PathRoutedMockDServer::start(routes);

        let result = run_bootstrap_with_resource_discovery(
            deterministic_discovery(),
            Some("rpi-119h1"),
            &server.url,
            "proj-119h1",
            5,
        );
        assert!(
            result.is_err(),
            "without any Resource Registry binding, a failed node-context lookup for an explicitly requested node must remain fatal, never silently becoming KnownBoundNode from caller input alone"
        );
    }
}
