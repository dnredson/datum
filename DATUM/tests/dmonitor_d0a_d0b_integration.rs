//! Phase 114D0A + 114D0B integration proof: builds a REAL DMap/2 + DForward
//! + DIoT placement topology and a REAL DIoT runtime binding through the
//! production HTTP proposal/acceptance/registration flow against a REAL
//! (freshly `cargo build`-ed) DServer process, fetches the CURRENT runtime
//! binding through the SAME production `dcall::client::fetch_current_runtime_binding`
//! seam `run_mqtt_dcall_listener` itself uses, builds ONE canonical
//! `Healthy` DIoT DMonitor observation through the PRODUCTION Phase-114D0B
//! builder (`agent::diot_runtime_dmonitor`), publishes it through the
//! PRODUCTION `DMonitorHttpOutput`, then evaluates Phase-114D0A DIoT
//! convergence through the REAL server-side interpretation endpoint and
//! proves `Converged`.
//!
//! Deliberately does NOT require a real MQTT broker or the
//! `smartsentinel-dcall-mqtt-listen` subprocess: MQTT session-readiness
//! classification (ConnAck/SubAck) is proven at the unit level
//! (`dcall::mqtt`'s own state-machine tests, deterministic, no broker
//! needed); this test instead proves the D0A+D0B EVIDENCE CHAIN --
//! authority registration -> production observation construction ->
//! production HTTP publication -> real server-side exact correlation --
//! which is the piece that actually spans both phases and both crates.
//!
//! `#[ignore]`d for the same reason `dmonitor_b2_integration.rs` and
//! `dcall_mqtt_governed_e2e.rs` are: it builds and spawns a real DServer
//! subprocess. Run explicitly:
//!
//!   cargo test --locked --test dmonitor_d0a_d0b_integration -- --ignored --test-threads=1

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use datum::agent::canonical_dcontinuum::{
    CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
    CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
};
use datum::agent::canonical_dgraph::{
    CanonicalDCallV1, CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
    CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
};
use datum::agent::canonical_dmonitor::DMonitorHealthV1;
use datum::agent::diot_runtime_dmonitor::{
    DiotRuntimeBindingIdentity, DiotRuntimeMonitorEmitter, DiotRuntimeState,
};
use datum::agent::dmonitor_emission::DMonitorEmissionContext;
use datum::dcall::client;
use datum::outputs::dmonitor_http::{DMonitorHttpOutput, DMonitorPublishOutcome};

const PROJECT: &str = "proj:dmonitor-d0a-d0b-integration";
const APP: &str = "app:pipeline";
const SOURCE_SVC: &str = "sensor";
const DEST_SVC: &str = "decoder";
const CALL_ID: &str = "sensor-to-decoder";
const SOURCE_NODE: &str = "fog-01";
const DEST_NODE: &str = "cloud-01";
const DIOT_ID: &str = "mqtt-broker";
const DIOT_ARTIFACT_ID: &str = "artifact:mqtt-broker";
const DIOT_PLACEMENT_ID: &str = "diotplace:mqtt-broker-cloud01";
const DFORWARD_ID: &str = "dforward:d0a-d0b-integration";
const DCALL_CHANNEL: &str = "iot/sensor-to-decoder";
const DCALL_REALIZATION_INTERFACE: &str = "mqtt.pubsub/3.1.1";

struct KillOnDrop(Child);
impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_for_port(port: u16, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        if Instant::now() > deadline {
            panic!("nothing listening on 127.0.0.1:{port} after {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn dserver_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("dserver")
}

fn dserver_binary_path() -> PathBuf {
    dserver_dir().join("target").join("debug").join(if cfg!(windows) { "DServer.exe" } else { "DServer" })
}

/// Mirrors `dmonitor_b2_integration.rs`/`dcall_mqtt_governed_e2e.rs`'s own
/// `spawn_real_dserver` exactly -- this project's established pattern.
fn spawn_real_dserver(tmp: &std::path::Path) -> (String, KillOnDrop) {
    let build_status = Command::new("cargo")
        .args(["build", "--locked", "--bin", "DServer"])
        .current_dir(dserver_dir())
        .status()
        .expect("run cargo build for DServer");
    assert!(build_status.success(), "cargo build --bin DServer failed");

    let port = free_port();
    let config_path = tmp.join("dserver-config.toml");
    std::fs::write(
        &config_path,
        format!(
            "[server]\nbind = \"127.0.0.1:{port}\"\ndebug = true\n\n[firebase]\nproject_id = \"dmonitor-d0a-d0b-unused\"\ncredentials_path = \"unused.json\"\n\n[cors]\nenabled = false\n"
        ),
    )
    .unwrap();

    let isolated_binary = tmp.join(if cfg!(windows) { "DServer.exe" } else { "DServer" });
    std::fs::copy(dserver_binary_path(), &isolated_binary).unwrap();
    let child = Command::new(isolated_binary)
        .env("DATUM_DSERVER_CONFIG_PATH", &config_path)
        .env("DATUM_CONTROL_STATE_DB_PATH", tmp.join("control-plane.db"))
        .env("DATUM_OPERATIONAL_DGRAPH_STATE_PATH", tmp.join("operational-dgraph-state.json"))
        .current_dir(tmp)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real DServer process");

    wait_for_port(port, Duration::from_secs(20));
    (format!("http://127.0.0.1:{port}"), KillOnDrop(child))
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder().timeout(Duration::from_secs(10)).build().unwrap()
}

fn node(node_id: &str) -> CanonicalDNodeV1 {
    CanonicalDNodeV1 {
        node_id: node_id.to_string(),
        platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "x86_64".to_string() },
        capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_000_000_000 },
        capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
    }
}

fn declare_node(base_url: &str, node: &CanonicalDNodeV1) {
    let response = http().put(format!("{base_url}/api/v1/dnodes/{}", node.node_id)).json(node).send().unwrap();
    assert!(response.status().is_success(), "declare_node failed: {}", response.text().unwrap_or_default());
}

fn minimal_service_artifact(artifact_id_override: Option<&str>, service_id: &str, target_node: &str) -> serde_json::Value {
    let artifact_id = artifact_id_override.map(|s| s.to_string()).unwrap_or_else(|| format!("artifact:{service_id}"));
    serde_json::json!({
        "schema_version": "0.2.0",
        "artifact_id": artifact_id,
        "service_id": service_id,
        "runtime": {
            "kind": "container",
            "container": { "container_name": service_id, "image": format!("example.invalid/{service_id}:test"), "image_source_kind": "local_build" }
        },
        "probes": { "execution": {"kind": "process_running"} },
        "control_plane": {
            "project_id": PROJECT,
            "name": service_id,
            "version": "1",
            "artifact_kind": "container",
            "target_nodes": [target_node],
            "acquisition": {"kind": "image"},
            "wasm_decision": {
                "explicit_invocation_required": true,
                "module_id": "reconcile_service",
                "function_name": "decide",
                "host_imports_allowed": false,
                "side_effects_allowed": false,
                "safe_to_auto_execute": false,
                "max_fuel": 1000000,
                "timeout_ms": 1000
            },
        }
    })
}

fn declare_service_artifact(base_url: &str, artifact: &serde_json::Value) {
    let response = http()
        .post(format!("{base_url}/api/v1/service-artifacts"))
        .header("X-Datum-Project", PROJECT)
        .json(artifact)
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "declare_service_artifact failed: {status} {text}");
}

fn dgraph() -> CanonicalDGraphV1 {
    CanonicalDGraphV1 {
        schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
        application_id: APP.to_string(),
        dgraph_id: "dgraph:d0a-d0b-integration".to_string(),
        revision: 1,
        services: vec![
            CanonicalDServV1 {
                service_id: SOURCE_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
            },
            CanonicalDServV1 {
                service_id: DEST_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
            },
        ],
        calls: vec![CanonicalDCallV1 { call_id: CALL_ID.to_string(), source_service_id: SOURCE_SVC.to_string(), target_service_id: DEST_SVC.to_string() }],
    }
}

fn dcontinuum() -> CanonicalDContinuumV1 {
    CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:d0a-d0b-integration".to_string(),
        revision: 1,
        nodes: vec![node(SOURCE_NODE), node(DEST_NODE)],
        links: Vec::new(),
    }
}

fn dforward() -> serde_json::Value {
    serde_json::json!({
        "schema": "datum.dforward/1",
        "project_id": PROJECT,
        "dforward_id": DFORWARD_ID,
        "revision": 1,
        "diots": [
            {
                "diot_id": DIOT_ID,
                "diot_kind": "mqtt_broker",
                "artifact_requirement": { "artifact_id": DIOT_ARTIFACT_ID },
                "provides": [{ "interface": DCALL_REALIZATION_INTERFACE }],
                "requires": [],
            }
        ],
        "external_endpoints": [],
        "chains": [],
        "support_jobs": [],
    })
}

fn diot_placements() -> serde_json::Value {
    serde_json::json!([
        {
            "placement_id": DIOT_PLACEMENT_ID,
            "diot_id": DIOT_ID,
            "node_id": DEST_NODE,
            "artifact_binding": {
                "artifact_id": DIOT_ARTIFACT_ID,
                "descriptor_digest": format!("sha256:{}", "a".repeat(64)),
                "content_digest": serde_json::Value::Null,
            }
        }
    ])
}

fn dcall_realizations() -> serde_json::Value {
    serde_json::json!([
        {
            "call_id": CALL_ID,
            "realization_kind": "via_diot_pubsub",
            "diot_placement_id": DIOT_PLACEMENT_ID,
            "interface": DCALL_REALIZATION_INTERFACE,
            "channel": DCALL_CHANNEL,
        }
    ])
}

/// Real Phase-113A production proposal/acceptance HTTP flow (never a
/// hand-injected active DMap).
fn accept_a_new_v2_proposal(base_url: &str) {
    let payload = serde_json::json!({
        "application_id": APP,
        "source": "operator",
        "dgraph": dgraph(),
        "dcontinuum": dcontinuum(),
        "dforward": dforward(),
        "placements": [
            {"service_id": SOURCE_SVC, "node_id": SOURCE_NODE},
            {"service_id": DEST_SVC, "node_id": DEST_NODE},
        ],
        "diot_placements": diot_placements(),
        "dcall_realizations": dcall_realizations(),
        "external_port_bindings": [],
    });
    let response = http()
        .post(format!("{base_url}/api/v1/ddeploy/v2/proposals"))
        .header("X-Datum-Project", PROJECT)
        .json(&payload)
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "create v2 proposal failed: {status} {text}");
    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
    let proposal_id = body["proposal"]["proposal_id"].as_str().expect("proposal_id").to_string();

    let accept = http()
        .post(format!("{base_url}/api/v1/ddeploy/v2/proposals/{proposal_id}/accept"))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({
            "accepted_by": "dmonitor-d0a-d0b-integration-test",
            "expected_proposal_digest": body["proposal"]["proposal_digest"],
        }))
        .send()
        .unwrap();
    let accept_status = accept.status();
    let accept_text = accept.text().unwrap_or_default();
    assert!(accept_status.is_success(), "accept v2 proposal failed: {accept_status} {accept_text}");
}

fn register_runtime_binding(base_url: &str, broker_uri: &str) {
    let response = http()
        .put(format!("{base_url}/api/v1/diot/runtime-bindings/{PROJECT}/{APP}/{DIOT_PLACEMENT_ID}"))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({ "broker_uri": broker_uri }))
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "register_runtime_binding failed: {status} {text}");
}

fn evaluate_convergence(base_url: &str) -> serde_json::Value {
    let response = http()
        .post(format!("{base_url}/api/v1/dmonitor/convergence/{PROJECT}/{APP}/evaluate"))
        .json(&serde_json::json!({
            "max_observed_age_seconds": 300,
            "max_received_age_seconds": 300,
            "max_future_clock_skew_seconds": 30,
        }))
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "evaluate convergence failed: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

#[test]
#[ignore = "builds and spawns a real DServer subprocess; run explicitly via \
            `cargo test --locked --test dmonitor_d0a_d0b_integration -- --ignored --test-threads=1`"]
fn d0a_d0b_end_to_end_diot_evidence_converges() {
    let tmp = std::env::temp_dir().join(format!("datum-dmonitor-d0a-d0b-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (dserver_url, _guard) = spawn_real_dserver(&tmp);

    // 1. Real D-Node declarations.
    declare_node(&dserver_url, &node(SOURCE_NODE));
    declare_node(&dserver_url, &node(DEST_NODE));

    // 2. Real ServiceArtifact declarations (including the DIoT's own
    // backing artifact, resolved through the same production
    // `resolve_diot_artifact_authority` path as any other DIoT).
    declare_service_artifact(&dserver_url, &minimal_service_artifact(None, SOURCE_SVC, SOURCE_NODE));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(None, DEST_SVC, DEST_NODE));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(Some(DIOT_ARTIFACT_ID), DIOT_ID, DEST_NODE));

    // 3. Real DMap/2 + DForward + DIoT placement + via_diot_pubsub
    // realization, via the real production proposal/acceptance flow.
    accept_a_new_v2_proposal(&dserver_url);

    // 4. Real DIoT runtime binding registration (broker URI is never
    // actually dialed by this test -- only registered/fetched authority).
    register_runtime_binding(&dserver_url, "mqtt://broker.invalid:1883");

    // 5. Fetch the CURRENT runtime binding through the EXACT SAME
    // production seam `run_mqtt_dcall_listener` itself calls.
    let binding: client::CurrentRuntimeBindingResponse =
        client::fetch_current_runtime_binding(&dserver_url, PROJECT, APP, DIOT_PLACEMENT_ID)
            .expect("fetch current runtime binding");
    assert_eq!(binding.node_id, DEST_NODE);
    assert_eq!(binding.diot_id, DIOT_ID);

    // 6. Build ONE Healthy DIoT DMonitor observation through the
    // PRODUCTION Phase-114D0B builder -- the exact code path
    // `run_mqtt_dcall_listener_loop` uses after confirmed session
    // readiness.
    let binding_identity = DiotRuntimeBindingIdentity {
        project_id: binding.project_id.clone(),
        application_id: binding.application_id.clone(),
        diot_id: binding.diot_id.clone(),
        diot_placement_id: binding.placement_id.clone(),
        node_id: binding.node_id.clone(),
        dmap_id: binding.dmap_id.clone(),
        dmap_revision: binding.dmap_revision,
        dmap_digest: binding.dmap_digest.clone(),
        dforward_id: binding.dforward_id.clone(),
        dforward_revision: binding.dforward_revision,
        dforward_digest: binding.dforward_digest.clone(),
        binding_id: binding.binding_id.clone(),
        binding_revision: binding.binding_revision,
        descriptor_digest: binding.descriptor_digest.clone(),
    };
    let mut emitter = DiotRuntimeMonitorEmitter::new(
        DMonitorEmissionContext::new_process_epoch(&binding.node_id),
        binding_identity,
    );
    let observed_at_utc = chrono::Utc::now().to_rfc3339();
    let observation = emitter.emit(&observed_at_utc, DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
    assert!(observation.validate().is_empty(), "{:?}", observation.validate());

    // 7. Publish through the PRODUCTION DMonitor HTTP output.
    let output = DMonitorHttpOutput::new(dserver_url.clone(), PROJECT.to_string(), 10).unwrap();
    let observation_json = serde_json::to_value(&observation).unwrap();
    let outcome = output.publish(&observation.observation_id, &observation_json).unwrap();
    assert_eq!(outcome, DMonitorPublishOutcome::Accepted);

    // 8. Evaluate through the REAL Phase-114D0A convergence endpoint and
    // prove the exact correlation chain holds.
    let result = evaluate_convergence(&dserver_url);
    let diot = &result["diot"];
    assert_eq!(diot["convergence"], "converged", "full diot result: {diot:#?}");
    let placement = &diot["placements"][0];
    assert_eq!(placement["diot_id"], DIOT_ID);
    assert_eq!(placement["diot_placement_id"], DIOT_PLACEMENT_ID);
    assert_eq!(placement["convergence"], "converged");
    let admitted = &placement["admitted_evidence"][0];
    assert_eq!(admitted["health"], "healthy");
    assert_eq!(admitted["dmap_correlation"], "exact_match");
    assert_eq!(admitted["dforward_correlation"], "exact_match");
    assert_eq!(admitted["runtime_binding_correlation"], "exact_match");
    assert_eq!(admitted["runtime_binding_current"], true);
    assert_eq!(admitted["freshness"], "fresh");
}
