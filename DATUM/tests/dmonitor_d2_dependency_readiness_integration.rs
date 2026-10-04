//! Phase 114D2 integration proof: builds a REAL DMap/2 + DForward topology
//! with a consumer DIoT (`cloud-01`) requiring `interface = I` with
//! `scope = any_element`, connected by a canonical chain hop to a provider
//! DIoT (`fog-01`) that provides `I`, through the production HTTP
//! proposal/acceptance/registration flow against a REAL (freshly
//! `cargo build`-ed) DServer process. Proves the Phase 114D2 server-owned
//! dependency-readiness endpoint
//! (`GET /api/v1/dforward/dependency-readiness/:project_id/:application_id/:node_id`):
//!
//! 1. before any provider DMonitor evidence exists, the dependency is
//!    `Blocked`;
//! 2. once exact-current Healthy D0B-style provider DIoT evidence is
//!    published, the dependency becomes `Satisfied` -- WHILE an unrelated
//!    required D-Serv placement is deliberately left without canonical
//!    evidence, so whole-application Phase 114D1 readiness is `NotReady`.
//!    This proves `any_element` depends on the PROVIDER PLACEMENT's own
//!    readiness, never on whole-application readiness;
//! 3. a fresh exact-current `Unhealthy` provider observation then flips the
//!    dependency back to `Blocked`, while the provider's own DIoT
//!    convergence may remain `Converged` -- proving the dependency consumes
//!    readiness, not convergence or raw health alone.
//!
//! `#[ignore]`d for the same reason `dmonitor_d0a_d0b_integration.rs`/
//! `dmonitor_d1_readiness_integration.rs` are: it builds and spawns a real
//! DServer subprocess. Run explicitly:
//!
//!   cargo test --locked --test dmonitor_d2_dependency_readiness_integration -- --ignored --test-threads=1

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use datum::agent::canonical_dcontinuum::{
    CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
    CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
};
use datum::agent::canonical_dgraph::{CanonicalDGraphV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0};
use datum::agent::canonical_dmonitor::DMonitorHealthV1;
use datum::agent::diot_runtime_dmonitor::{DiotRuntimeBindingIdentity, DiotRuntimeMonitorEmitter, DiotRuntimeState};
use datum::agent::dmonitor_emission::DMonitorEmissionContext;
use datum::dcall::client;
use datum::outputs::dmonitor_http::{DMonitorHttpOutput, DMonitorPublishOutcome};

const PROJECT: &str = "proj:dmonitor-d2-dependency-integration";
const APP: &str = "app:pipeline";
const NODE_PROVIDER: &str = "fog-01";
const NODE_CONSUMER: &str = "cloud-01";
const CONSUMER_DIOT_ID: &str = "diot:consumer";
const CONSUMER_PLACEMENT_ID: &str = "diotplace:consumer";
const CONSUMER_ARTIFACT_ID: &str = "artifact:consumer";
const PROVIDER_DIOT_ID: &str = "mqtt-broker";
const PROVIDER_ARTIFACT_ID: &str = "artifact:mqtt-broker";
const PROVIDER_PLACEMENT_ID: &str = "diotplace:mqtt-broker-fog01";
const DFORWARD_ID: &str = "dforward:d2-dependency-integration";
const INTERFACE: &str = "sensor-bus";
// An unrelated required D-Serv placement, deliberately left without
// canonical evidence -- proves whole-application readiness stays NotReady
// while the any_element dependency itself is Satisfied.
const UNRELATED_SVC: &str = "unrelated-service";

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
            "[server]\nbind = \"127.0.0.1:{port}\"\ndebug = true\n\n[firebase]\nproject_id = \"dmonitor-d2-unused\"\ncredentials_path = \"unused.json\"\n\n[cors]\nenabled = false\n"
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

fn minimal_service_artifact(artifact_id: &str, service_id: &str, target_node: &str) -> serde_json::Value {
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
        dgraph_id: "dgraph:d2-dependency-integration".to_string(),
        revision: 1,
        services: vec![datum::agent::canonical_dgraph::CanonicalDServV1 {
            service_id: UNRELATED_SVC.to_string(),
            service_definition: None,
            execution_requirement: datum::agent::canonical_dgraph::CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
            resource_requirement: datum::agent::canonical_dgraph::CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
        }],
        calls: Vec::new(),
    }
}

fn dcontinuum() -> CanonicalDContinuumV1 {
    CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:d2-dependency-integration".to_string(),
        revision: 1,
        nodes: vec![node(NODE_PROVIDER), node(NODE_CONSUMER)],
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
                "diot_id": CONSUMER_DIOT_ID,
                "diot_kind": "consumer",
                "artifact_requirement": { "artifact_id": CONSUMER_ARTIFACT_ID },
                "provides": [],
                "requires": [{ "interface": INTERFACE, "scope": "any_element" }],
            },
            {
                "diot_id": PROVIDER_DIOT_ID,
                "diot_kind": "mqtt_broker",
                "artifact_requirement": { "artifact_id": PROVIDER_ARTIFACT_ID },
                "provides": [{ "interface": INTERFACE }],
                "requires": [],
            }
        ],
        "external_endpoints": [],
        "chains": [
            { "chain_id": "c1", "hops": [{ "from": CONSUMER_DIOT_ID, "to": PROVIDER_DIOT_ID, "interface": INTERFACE }] }
        ],
        "support_jobs": [],
    })
}

fn diot_placements() -> serde_json::Value {
    serde_json::json!([
        {
            "placement_id": CONSUMER_PLACEMENT_ID,
            "diot_id": CONSUMER_DIOT_ID,
            "node_id": NODE_CONSUMER,
            "artifact_binding": { "artifact_id": CONSUMER_ARTIFACT_ID, "descriptor_digest": format!("sha256:{}", "d".repeat(64)), "content_digest": serde_json::Value::Null },
        },
        {
            "placement_id": PROVIDER_PLACEMENT_ID,
            "diot_id": PROVIDER_DIOT_ID,
            "node_id": NODE_PROVIDER,
            "artifact_binding": { "artifact_id": PROVIDER_ARTIFACT_ID, "descriptor_digest": format!("sha256:{}", "a".repeat(64)), "content_digest": serde_json::Value::Null },
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
            {"service_id": UNRELATED_SVC, "node_id": NODE_PROVIDER},
        ],
        "diot_placements": diot_placements(),
        "dcall_realizations": [],
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
            "accepted_by": "dmonitor-d2-dependency-integration-test",
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
        .put(format!("{base_url}/api/v1/diot/runtime-bindings/{PROJECT}/{APP}/{PROVIDER_PLACEMENT_ID}"))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({ "broker_uri": broker_uri }))
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "register_runtime_binding failed: {status} {text}");
}

fn get_dependency_readiness(base_url: &str) -> serde_json::Value {
    let response = http()
        .get(format!("{base_url}/api/v1/dforward/dependency-readiness/{PROJECT}/{APP}/{NODE_CONSUMER}"))
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "get dependency readiness failed: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

fn get_application_readiness(base_url: &str) -> serde_json::Value {
    let response = http().get(format!("{base_url}/api/v1/dmonitor/readiness/{PROJECT}/{APP}")).send().unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "get application readiness failed: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

#[test]
#[ignore = "builds and spawns a real DServer subprocess; run explicitly via \
            `cargo test --locked --test dmonitor_d2_dependency_readiness_integration -- --ignored --test-threads=1`"]
fn d2_any_element_dependency_consumes_provider_placement_readiness_not_application_readiness() {
    let tmp = std::env::temp_dir().join(format!("datum-dmonitor-d2-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();
    let (dserver_url, _guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(NODE_PROVIDER));
    declare_node(&dserver_url, &node(NODE_CONSUMER));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(&format!("artifact:{UNRELATED_SVC}"), UNRELATED_SVC, NODE_PROVIDER));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(CONSUMER_ARTIFACT_ID, CONSUMER_DIOT_ID, NODE_CONSUMER));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(PROVIDER_ARTIFACT_ID, PROVIDER_DIOT_ID, NODE_PROVIDER));
    accept_a_new_v2_proposal(&dserver_url);
    register_runtime_binding(&dserver_url, "mqtt://broker.invalid:1883");

    let binding: client::CurrentRuntimeBindingResponse =
        client::fetch_current_runtime_binding(&dserver_url, PROJECT, APP, PROVIDER_PLACEMENT_ID).expect("fetch current runtime binding");

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
    let mut emitter = DiotRuntimeMonitorEmitter::new(DMonitorEmissionContext::new_process_epoch(&binding.node_id), binding_identity);
    let output = DMonitorHttpOutput::new(dserver_url.clone(), PROJECT.to_string(), 10).unwrap();

    // Step 1: no provider evidence at all yet -> Blocked.
    let dependency_readiness = get_dependency_readiness(&dserver_url);
    let deps = dependency_readiness["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 1, "{dependency_readiness:#?}");
    assert_eq!(deps[0]["state"], "blocked", "{dependency_readiness:#?}");

    // Step 2: publish exact-current Healthy provider DIoT evidence.
    // Deliberately leave UNRELATED_SVC without any canonical D-Serv
    // evidence, so whole-application readiness stays NotReady.
    let healthy_observation = emitter.emit(&chrono::Utc::now().to_rfc3339(), DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
    assert!(healthy_observation.validate().is_empty());
    let outcome = output.publish(&healthy_observation.observation_id, &serde_json::to_value(&healthy_observation).unwrap()).unwrap();
    assert_eq!(outcome, DMonitorPublishOutcome::Accepted);

    let app_readiness = get_application_readiness(&dserver_url);
    assert_eq!(app_readiness["readiness"], "not_ready", "test premise violated -- unrelated D-Serv must leave application readiness NotReady: {app_readiness:#?}");

    let dependency_readiness = get_dependency_readiness(&dserver_url);
    let dep = &dependency_readiness["dependencies"][0];
    assert_eq!(dep["state"], "satisfied", "any_element must be Satisfied from the provider's OWN placement readiness, independent of whole-application NotReady: {dependency_readiness:#?}");
    assert_eq!(dep["provider_diot_id"], PROVIDER_DIOT_ID);
    assert_eq!(dep["provider_placement_id"], PROVIDER_PLACEMENT_ID);
    assert_eq!(dep["provider_readiness"]["readiness"], "ready");

    // Step 3: a fresh exact-current Unhealthy provider observation flips the
    // dependency back to Blocked, while DIoT convergence may remain
    // Converged.
    let unhealthy_observation = emitter.emit(&chrono::Utc::now().to_rfc3339(), DMonitorHealthV1::Unhealthy, DiotRuntimeState::Subscribed, true);
    let outcome = output.publish(&unhealthy_observation.observation_id, &serde_json::to_value(&unhealthy_observation).unwrap()).unwrap();
    assert_eq!(outcome, DMonitorPublishOutcome::Accepted);

    let dependency_readiness = get_dependency_readiness(&dserver_url);
    let dep = &dependency_readiness["dependencies"][0];
    assert_eq!(dep["state"], "blocked", "{dependency_readiness:#?}");
    assert_eq!(dep["provider_readiness"]["convergence"], "converged", "DIoT convergence must remain Converged even though the dependency is Blocked: {dependency_readiness:#?}");
    assert_eq!(dep["provider_readiness"]["readiness"], "not_ready");
}
