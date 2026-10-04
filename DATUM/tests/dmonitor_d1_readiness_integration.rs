//! Phase 114D1 integration proof: builds a REAL DMap/2 + DForward + DIoT
//! placement topology and a REAL DIoT runtime binding through the
//! production HTTP proposal/acceptance/registration flow against a REAL
//! (freshly `cargo build`-ed) DServer process, publishes a real Healthy
//! DIoT DMonitor observation through the production Phase-114D0B builder,
//! and proves the Phase-114D1 server-owned readiness endpoint
//! (`GET /api/v1/dmonitor/readiness/:project_id/:application_id`):
//!
//! 1. before any canonical DServ realization evidence exists, readiness is
//!    `NotReady` even though the DIoT placement is already exact-current
//!    and Healthy -- proving readiness genuinely requires every required
//!    dimension, not just one;
//! 2. once exact-current Healthy `DServRealization` evidence is ingested
//!    for every required D-Serv placement, readiness becomes `Ready`;
//! 3. a fresh exact-current `Unhealthy` DIoT observation then flips
//!    readiness back to `NotReady` while DIoT convergence itself remains
//!    `Converged` -- proving readiness is a genuinely separate fifth
//!    dimension, not an alias for convergence or health.
//!
//! Per Phase 114D1's own documented limitation: the generic SmartSentinel
//! B2 bridge deliberately does not manufacture `DServRealization` identity
//! from container/process names, so this test explicitly constructs and
//! publishes a canonical `DServRealization` fixture directly (via
//! `agent::canonical_dmonitor` + `outputs::dmonitor_http`) rather than
//! faking a production SmartSentinel producer that does not exist yet --
//! this is testing GOVERNANCE, not inventing a D-Serv evidence producer.
//!
//! `#[ignore]`d for the same reason `dmonitor_d0a_d0b_integration.rs` is:
//! it builds and spawns a real DServer subprocess. Run explicitly:
//!
//!   cargo test --locked --test dmonitor_d1_readiness_integration -- --ignored --test-threads=1

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
use datum::agent::canonical_dmonitor::{
    CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, DMonitorSubjectV1,
    CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
};
use datum::agent::diot_runtime_dmonitor::{DiotRuntimeBindingIdentity, DiotRuntimeMonitorEmitter, DiotRuntimeState};
use datum::agent::dmonitor_emission::DMonitorEmissionContext;
use datum::dcall::client;
use datum::outputs::dmonitor_http::{DMonitorHttpOutput, DMonitorPublishOutcome};

const PROJECT: &str = "proj:dmonitor-d1-readiness-integration";
const APP: &str = "app:pipeline";
const SOURCE_SVC: &str = "sensor";
const DEST_SVC: &str = "decoder";
const CALL_ID: &str = "sensor-to-decoder";
const NODE: &str = "cloud-01";
const DIOT_ID: &str = "mqtt-broker";
const DIOT_ARTIFACT_ID: &str = "artifact:mqtt-broker";
const DIOT_PLACEMENT_ID: &str = "diotplace:mqtt-broker-cloud01";
const DFORWARD_ID: &str = "dforward:d1-readiness-integration";
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
            "[server]\nbind = \"127.0.0.1:{port}\"\ndebug = true\n\n[firebase]\nproject_id = \"dmonitor-d1-unused\"\ncredentials_path = \"unused.json\"\n\n[cors]\nenabled = false\n"
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
    let response = http().post(format!("{base_url}/api/v1/service-artifacts")).header("X-Datum-Project", PROJECT).json(artifact).send().unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "declare_service_artifact failed: {status} {text}");
}

fn dgraph() -> CanonicalDGraphV1 {
    CanonicalDGraphV1 {
        schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
        application_id: APP.to_string(),
        dgraph_id: "dgraph:d1-readiness-integration".to_string(),
        revision: 1,
        services: vec![
            CanonicalDServV1 { service_id: SOURCE_SVC.to_string(), service_definition: None, execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 } },
            CanonicalDServV1 { service_id: DEST_SVC.to_string(), service_definition: None, execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 } },
        ],
        calls: vec![CanonicalDCallV1 { call_id: CALL_ID.to_string(), source_service_id: SOURCE_SVC.to_string(), target_service_id: DEST_SVC.to_string() }],
    }
}

fn dcontinuum() -> CanonicalDContinuumV1 {
    CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:d1-readiness-integration".to_string(),
        revision: 1,
        nodes: vec![node(NODE)],
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
            "node_id": NODE,
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

fn accept_a_new_v2_proposal(base_url: &str) {
    let payload = serde_json::json!({
        "application_id": APP,
        "source": "operator",
        "dgraph": dgraph(),
        "dcontinuum": dcontinuum(),
        "dforward": dforward(),
        "placements": [
            {"service_id": SOURCE_SVC, "node_id": NODE},
            {"service_id": DEST_SVC, "node_id": NODE},
        ],
        "diot_placements": diot_placements(),
        "dcall_realizations": dcall_realizations(),
        "external_port_bindings": [],
    });
    let response = http().post(format!("{base_url}/api/v1/ddeploy/v2/proposals")).header("X-Datum-Project", PROJECT).json(&payload).send().unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "create v2 proposal failed: {status} {text}");
    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
    let proposal_id = body["proposal"]["proposal_id"].as_str().expect("proposal_id").to_string();

    let accept = http()
        .post(format!("{base_url}/api/v1/ddeploy/v2/proposals/{proposal_id}/accept"))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({
            "accepted_by": "dmonitor-d1-readiness-integration-test",
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

fn get_readiness(base_url: &str) -> serde_json::Value {
    let response = http().get(format!("{base_url}/api/v1/dmonitor/readiness/{PROJECT}/{APP}")).send().unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "get readiness failed: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

fn dserv_observation(monitor_instance_id: &str, sequence: u64, service_id: &str, dmap: &DMonitorContractRefV1, health: DMonitorHealthV1, observed_at_utc: &str) -> CanonicalDMonitorObservationV1 {
    // `agent::dmonitor_emission::DMonitorEmissionContext::allocate_sequence`
    // is crate-private (pub(crate)) and therefore unreachable from this
    // external test binary -- this test's own D-Serv fixture is not
    // required to go through that shared context (only the DIoT side, via
    // `DiotRuntimeMonitorEmitter`, needs to per section 37's own scope), so
    // a plain explicit sequence counter is used here instead, matching the
    // exact deterministic `dmonitor-obs:<monitor_instance_id>:<sequence>`
    // id convention by construction.
    let observation_id = format!("dmonitor-obs:{monitor_instance_id}:{sequence}");
    CanonicalDMonitorObservationV1 {
        schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
        observation_id,
        project_id: Some(PROJECT.to_string()),
        node_id: NODE.to_string(),
        monitor_instance_id: monitor_instance_id.to_string(),
        sequence,
        observed_at_utc: observed_at_utc.to_string(),
        cycle_id: None,
        collector: "test_dserv_realization".to_string(),
        subject: DMonitorSubjectV1::DServRealization {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            service_id: service_id.to_string(),
            node_id: NODE.to_string(),
            dmap_ref: Some(dmap.clone()),
        },
        health,
        facts: serde_json::json!({}),
    }
}

#[test]
#[ignore = "builds and spawns a real DServer subprocess; run explicitly via \
            `cargo test --locked --test dmonitor_d1_readiness_integration -- --ignored --test-threads=1`"]
fn d1_readiness_requires_every_required_dimension_ready() {
    let tmp = std::env::temp_dir().join(format!("datum-dmonitor-d1-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (dserver_url, _guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(NODE));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(None, SOURCE_SVC, NODE));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(None, DEST_SVC, NODE));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(Some(DIOT_ARTIFACT_ID), DIOT_ID, NODE));
    accept_a_new_v2_proposal(&dserver_url);
    register_runtime_binding(&dserver_url, "mqtt://broker.invalid:1883");

    let binding: client::CurrentRuntimeBindingResponse =
        client::fetch_current_runtime_binding(&dserver_url, PROJECT, APP, DIOT_PLACEMENT_ID).expect("fetch current runtime binding");

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
    let mut diot_emitter = DiotRuntimeMonitorEmitter::new(DMonitorEmissionContext::new_process_epoch(&binding.node_id), binding_identity);
    let output = DMonitorHttpOutput::new(dserver_url.clone(), PROJECT.to_string(), 10).unwrap();

    // Step 1: publish the DIoT Healthy observation. Readiness must still be
    // NotReady -- no canonical DServ realization evidence exists yet.
    let diot_observation = diot_emitter.emit(&chrono::Utc::now().to_rfc3339(), DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
    assert!(diot_observation.validate().is_empty());
    let outcome = output.publish(&diot_observation.observation_id, &serde_json::to_value(&diot_observation).unwrap()).unwrap();
    assert_eq!(outcome, DMonitorPublishOutcome::Accepted);

    let readiness = get_readiness(&dserver_url);
    assert_eq!(readiness["readiness"], "not_ready", "missing canonical DServ evidence must block readiness: {readiness:#?}");
    assert!(
        readiness["findings"].as_array().unwrap().iter().any(|f| f["code"] == "dserv_not_converged"),
        "{readiness:#?}"
    );

    // Step 2: ingest exact-current Healthy DServRealization evidence for
    // BOTH required D-Serv placements.
    let dmap_ref = DMonitorContractRefV1 { id: binding.dmap_id.clone(), revision: binding.dmap_revision, digest: binding.dmap_digest.clone() };
    let dserv_monitor_instance_id = format!("dmonitor-instance:{NODE}:{}", uuid::Uuid::new_v4());
    let source_observation = dserv_observation(&dserv_monitor_instance_id, 1, SOURCE_SVC, &dmap_ref, DMonitorHealthV1::Healthy, &chrono::Utc::now().to_rfc3339());
    let dest_observation = dserv_observation(&dserv_monitor_instance_id, 2, DEST_SVC, &dmap_ref, DMonitorHealthV1::Healthy, &chrono::Utc::now().to_rfc3339());
    for observation in [&source_observation, &dest_observation] {
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let outcome = output.publish(&observation.observation_id, &serde_json::to_value(observation).unwrap()).unwrap();
        assert_eq!(outcome, DMonitorPublishOutcome::Accepted);
    }

    let readiness = get_readiness(&dserver_url);
    assert_eq!(readiness["readiness"], "ready", "{readiness:#?}");
    assert_eq!(readiness["policy"]["policy_id"], "dmonitor-readiness-v1");
    assert_eq!(readiness["dserv"]["placements"][0]["readiness"], "ready");
    assert_eq!(readiness["diot"]["placements"][0]["readiness"], "ready");

    // Step 3: a fresh exact-current Unhealthy DIoT observation flips
    // readiness back to NotReady, while DIoT convergence itself remains
    // Converged -- proving readiness is a genuinely separate dimension.
    let unhealthy_diot_observation = diot_emitter.emit(&chrono::Utc::now().to_rfc3339(), DMonitorHealthV1::Unhealthy, DiotRuntimeState::Subscribed, true);
    let outcome = output.publish(&unhealthy_diot_observation.observation_id, &serde_json::to_value(&unhealthy_diot_observation).unwrap()).unwrap();
    assert_eq!(outcome, DMonitorPublishOutcome::Accepted);

    let readiness = get_readiness(&dserver_url);
    assert_eq!(readiness["readiness"], "not_ready", "{readiness:#?}");
    assert_eq!(readiness["diot"]["placements"][0]["convergence"], "converged", "DIoT convergence must remain Converged even though readiness is NotReady: {readiness:#?}");
    assert_eq!(readiness["diot"]["placements"][0]["readiness"], "not_ready");
}
