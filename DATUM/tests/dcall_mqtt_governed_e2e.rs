//! Phase 113B4A (ADR-0023): the full governed real-broker MQTT D-Call E2E.
//! This is deliberately NOT mocked and NOT a shortcut: it builds and
//! launches the REAL `DServer` binary against isolated temp storage, builds
//! a real canonical DMap/2 + DForward + DIoT placement topology through the
//! REAL Phase-113A proposal/acceptance HTTP flow, registers the DIoT runtime
//! binding through the REAL Phase-113B1 `PUT` API pointing at a REAL
//! external MQTT broker, launches the REAL `smartsentinel-dcall-mqtt-listen`
//! binary as the destination's subprocess, and drives the REAL
//! `dcall::delivery::dispatch_validated_emit` seam end to end: real source
//! D-Code invocation -> real grant issuance carrying typed DMap/2
//! `via_diot_pubsub` transport authority -> real CURRENT runtime-binding
//! fetch/comparison -> a real MQTT publish/subscribe round trip against the
//! real broker -> real grant redemption -> a REAL isolated-worker D-Code/WASM
//! invocation on the destination -> a real MQTT response -> real
//! source-side destination-evidence validation -> `DCallDeliveryOutcome::Success`.
//!
//! B3 already proved real MQTT transport + a broker round-trip seam
//! (`dcall_mqtt_real_broker.rs`, kept unchanged). B4A proves the FULL
//! governed chain end to end, through production entrypoints only -- no
//! internal-function shortcut stands in for any production boundary here.
//! It does NOT attempt fog/cloud multi-host topology, DMonitor, placement
//! migration, or Stage-115 reconciliation -- those remain later phases.
//!
//! Phase 113B4A.1 adds four real-broker FAILURE-mode scenarios alongside the
//! primary success case: a legitimately-stale runtime binding, an
//! unreachable broker, a broker that is reachable but has no destination
//! listener running, and a physical duplicate MQTT delivery of the exact
//! same governed request. Two of these (stale binding, duplicate delivery)
//! cannot be exercised through the single public `dispatch_validated_emit`
//! entrypoint alone, because that function issues its own fresh grant and
//! delivers in one call -- there is no public seam to hold an
//! already-issued grant and deliver it later. Both scenarios therefore use
//! the OTHER real public production seams directly: `dcall::client::request_grant`
//! (the exact same HTTP call `dispatch_one_call` makes internally) to obtain
//! a real grant+token, and the real `dcall::mqtt` primitives
//! (`RumqttcPubSub`, `request_topic`/`reply_topic`) to publish the exact
//! envelope shape production code publishes -- so the REAL destination
//! listener subprocess and REAL DServer redemption path are still exercised
//! for real, with no shortcut in EITHER of those two pieces. This is
//! disclosed explicitly rather than silently claimed as testing the
//! source-side pre-publish check (see the doc comments on each new test).
//!
//! Every test here is `#[ignore]`d and additionally requires
//! `DATUM_TEST_MQTT_BROKER_URI` (a real reachable `mqtt://host:port`
//! broker); ordinary `cargo test` runs never require Docker/Mosquitto.
//!
//! Run against a real Mosquitto instance, e.g. (PowerShell):
//!   docker run --rm -d -p 1883:1883 --name datum-it-mosquitto eclipse-mosquitto
//!   $env:DATUM_TEST_MQTT_BROKER_URI = "mqtt://127.0.0.1:1883"
//!   $env:DATUM_PROOF_PYTHON = "C:/Users/dnred/AppData/Local/Python/pythoncore-3.14-64/python.exe"
//!   cargo test --locked --features dcall-test-support --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1
//!
//! See `documentation/how-to/run-governed-mqtt-dcall-e2e.md` for the full
//! standalone prerequisites and run instructions.

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
use datum::dcall::delivery::{dispatch_validated_emit, SourceExecutionContext};
use datum::dcall::evidence::DCallDeliveryOutcome;
use datum::dcall::grant::{RequestGrantResponse, SourceExecutionBindingV1};
use datum::dcall::mqtt::{
    MqttDCallRequestEnvelopeV1, MqttPubSub, MqttQos, RumqttcPubSub, REQUEST_ENVELOPE_SCHEMA,
};
use datum::dcode::abi::DNODE_ABI_V0_REQUIRED_EXPORTS;
use datum::dcode::authorization::DCodeInvocationAuthorizationV1;
use datum::dcode::dserv_artifact::{
    CanonicalDServArtifactV1, DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1,
    DServArtifactLimitsV1, DServArtifactPortV1, DServArtifactPortsV1, DServArtifactSafetyV1,
    CANONICAL_DSERV_ARTIFACT_SCHEMA,
};
use datum::dcode::identity::sha256_hex;
use datum::dcode::invoke::{invoke_dcode_governed_capturing_emits, GovernedDCodeInvocationResult};
use datum::dcode::local_identity::init_local_identity;
use datum::dcode::module_store::DCodeModuleStore;

const TRIGGER_PORT: &str = "trigger";
const EMITTED_PAYLOAD: &[u8] = b"governed-mqtt-e2e-emitted-payload";

const PROJECT: &str = "proj:dcall-mqtt-governed-e2e";
const APP: &str = "app:pipeline";
const SOURCE_SVC: &str = "sensor";
const DEST_SVC: &str = "decoder";
const CALL_ID: &str = "sensor-to-decoder";
const OUTPUT_PORT: &str = "raw";
const INPUT_PORT: &str = "in";
const PAYLOAD_SCHEMA: &str = "raw-bytes/1";
const SOURCE_NODE: &str = "fog-01";
const DEST_NODE: &str = "cloud-01";
const DIOT_ID: &str = "mqtt-broker";
const DIOT_ARTIFACT_ID: &str = "artifact:mqtt-broker";
const DIOT_PLACEMENT_ID: &str = "diotplace:mqtt-broker-cloud01";
const DFORWARD_ID: &str = "dforward:governed-e2e";
const DCALL_CHANNEL: &str = "iot/sensor-to-decoder";
const DCALL_REALIZATION_INTERFACE: &str = "mqtt.pubsub/3.1.1";

fn broker_uri_from_env() -> Option<String> {
    std::env::var("DATUM_TEST_MQTT_BROKER_URI").ok()
}

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
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("dserver")
}

fn dserver_binary_path() -> PathBuf {
    dserver_dir()
        .join("target")
        .join("debug")
        .join(if cfg!(windows) { "DServer.exe" } else { "DServer" })
}

fn install_real_sibling_worker() {
    let installed = datum::dcode::worker::trusted_sibling_worker_binary_path();
    let built = PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"));
    std::fs::copy(&built, &installed)
        .expect("provision real Cargo-built sibling worker for test installation");
}

/// Spawns a REAL, freshly `cargo build`-ed `DServer` binary against fully
/// isolated temp storage -- honestly whatever is currently checked out on
/// this branch, never a stale artifact. Mirrors `dcall_two_node_integration.rs`'s
/// own `spawn_real_dserver` exactly (section 4: reuse the established
/// subprocess test pattern, do not invent a new one).
fn spawn_real_dserver(tmp: &std::path::Path) -> (String, KillOnDrop) {
    install_real_sibling_worker();
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
            "[server]\nbind = \"127.0.0.1:{port}\"\ndebug = true\n\n[firebase]\nproject_id = \"dcall-mqtt-e2e-unused\"\ncredentials_path = \"unused.json\"\n\n[cors]\nenabled = false\n"
        ),
    )
    .unwrap();

    let isolated_binary = tmp.join(if cfg!(windows) { "DServer.exe" } else { "DServer" });
    std::fs::copy(dserver_binary_path(), &isolated_binary).unwrap();
    let child = Command::new(isolated_binary)
        .env("DATUM_DSERVER_CONFIG_PATH", &config_path)
        .env("DATUM_CONTROL_STATE_DB_PATH", tmp.join("control-plane.db"))
        .env(
            "DATUM_OPERATIONAL_DGRAPH_STATE_PATH",
            tmp.join("operational-dgraph-state.json"),
        )
        .current_dir(tmp)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real DServer process");

    wait_for_port(port, Duration::from_secs(20));
    (format!("http://127.0.0.1:{port}"), KillOnDrop(child))
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn node(node_id: &str) -> CanonicalDNodeV1 {
    CanonicalDNodeV1 {
        node_id: node_id.to_string(),
        platform: CanonicalNodePlatformV1 {
            os: "linux".to_string(),
            architecture: "x86_64".to_string(),
        },
        capacity: CanonicalNodeCapacityV1 {
            cpu_cores: 4,
            memory_bytes: 4_000_000_000,
        },
        capabilities: CanonicalNodeCapabilitiesV1 {
            dnode_abi_versions: vec![DNODE_ABI_V0.to_string()],
        },
    }
}

fn declare_node(base_url: &str, node: &CanonicalDNodeV1) {
    let response = http()
        .put(format!("{base_url}/api/v1/dnodes/{}", node.node_id))
        .json(node)
        .send()
        .unwrap();
    assert!(response.status().is_success(), "declare_node failed: {}", response.text().unwrap_or_default());
}

fn declare_artifact(base_url: &str, artifact: &CanonicalDServArtifactV1) {
    let response = http()
        .post(format!("{base_url}/api/v1/dcode/artifacts"))
        .json(artifact)
        .send()
        .unwrap();
    assert!(response.status().is_success(), "declare_artifact failed: {}", response.text().unwrap_or_default());
}

fn minimal_service_artifact(artifact_id_override: Option<&str>, service_id: &str, target_node: &str) -> serde_json::Value {
    let artifact_id = artifact_id_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("artifact:{service_id}"));
    serde_json::json!({
        "schema_version": "0.2.0",
        "artifact_id": artifact_id,
        "service_id": service_id,
        "runtime": {
            "kind": "container",
            "container": {
                "container_name": service_id,
                "image": format!("example.invalid/{service_id}:test"),
                "image_source_kind": "local_build",
            }
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
        dgraph_id: "dgraph:governed-e2e".to_string(),
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
        calls: vec![CanonicalDCallV1 {
            call_id: CALL_ID.to_string(),
            source_service_id: SOURCE_SVC.to_string(),
            target_service_id: DEST_SVC.to_string(),
        }],
    }
}

fn dcontinuum() -> CanonicalDContinuumV1 {
    CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:governed-e2e".to_string(),
        revision: 1,
        nodes: vec![node(SOURCE_NODE), node(DEST_NODE)],
        links: Vec::new(),
    }
}

/// One DForward with one DIoT (a `mqtt-broker` logical identity, backed by
/// its own ServiceArtifact -- section 5/6: this is exact production
/// governance data flowing through `resolve_diot_artifact_authority`, never
/// hand-injected) and one `via_diot_pubsub` D-Call realization for the real
/// `sensor-to-decoder` call declared above. `provides` MUST declare the
/// realization's own `interface` -- `canonical_dmap_v2.rs`'s `interface_unknown`
/// validation rejects a realization whose interface is absent from the
/// DIoT's own provided/required semantics.
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

/// Real Phase-113A production proposal/acceptance HTTP flow -- section 6:
/// never hand-injects an active DMap into persisted storage.
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
            "accepted_by": "dcall-mqtt-governed-e2e-test",
            "expected_proposal_digest": body["proposal"]["proposal_digest"],
        }))
        .send()
        .unwrap();
    let accept_status = accept.status();
    let accept_text = accept.text().unwrap_or_default();
    assert!(accept_status.is_success(), "accept v2 proposal failed: {accept_status} {accept_text}");
}

fn register_runtime_binding(base_url: &str, broker_uri: &str) -> serde_json::Value {
    let response = http()
        .put(format!(
            "{base_url}/api/v1/diot/runtime-bindings/{PROJECT}/{APP}/{DIOT_PLACEMENT_ID}"
        ))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({ "broker_uri": broker_uri }))
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "register_runtime_binding failed: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

fn fetch_current_runtime_binding(base_url: &str) -> serde_json::Value {
    let response = http()
        .get(format!(
            "{base_url}/api/v1/diot/runtime-bindings/{PROJECT}/{APP}/{DIOT_PLACEMENT_ID}/current"
        ))
        .header("X-Datum-Project", PROJECT)
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(status.is_success(), "fetch current runtime binding failed: {status} {text}");
    serde_json::from_str(&text).unwrap()
}

fn respond_ok_wat(body: &str) -> Vec<u8> {
    let escaped: String = body.bytes().map(|b| format!("\\{:02x}", b)).collect();
    let module = format!(
        r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 0) "{escaped}")
  (func (export "datum_abi_version") (result i32) (i32.const 0))
  (func (export "datum_alloc") (param $len i32) (result i32) (i32.const 4096))
  (func (export "datum_dealloc") (param $ptr i32) (param $len i32))
  (func (export "datum_handle") (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const {}))))
"#,
        body.len()
    );
    wat::parse_str(&module).unwrap()
}

fn source_module_with_emit() -> Vec<u8> {
    use base64::Engine;
    let payload_b64 = base64::engine::general_purpose::STANDARD.encode(EMITTED_PAYLOAD);
    respond_ok_wat(&format!(
        r#"{{"status":"ok","emits":[{{"port":"{OUTPUT_PORT}","payload_b64":"{payload_b64}"}}]}}"#
    ))
}

fn artifact(service_id: &str, ports: DServArtifactPortsV1, module_bytes: &[u8]) -> CanonicalDServArtifactV1 {
    let digest = sha256_hex(module_bytes);
    CanonicalDServArtifactV1 {
        schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
        artifact_id: format!("dserv:governed-e2e:{service_id}"),
        version: "0.1.0".to_string(),
        application_id: APP.to_string(),
        dserv_id: service_id.to_string(),
        dcode: DServArtifactDCodeV1 {
            uri: format!("datum-blob://sha256/{digest}"),
            sha256: digest.clone(),
            content_id: format!("sha256:{digest}"),
            size_bytes: module_bytes.len() as u64,
        },
        abi: DServArtifactAbiV1 {
            name: "datum-dnode".to_string(),
            version: "0".to_string(),
            required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
            allowed_imports: Vec::new(),
        },
        ports,
        limits: DServArtifactLimitsV1 {
            fuel_per_invocation: 10_000_000,
            timeout_ms: 5_000,
            max_memory_pages: 4,
            max_message_bytes: 4096,
            max_emits_per_invocation: 1,
        },
        safety: DServArtifactSafetyV1::default(),
        state_model: "stateless".to_string(),
        instantiation: "fresh_instance_per_invocation".to_string(),
        constraints: DServArtifactConstraintsV1::default(),
        qos: serde_json::Value::Null,
        provenance: serde_json::Value::Null,
        probes: serde_json::Value::Null,
        signature_ref: None,
    }
}

fn run_real_source_invocation(
    dserver_url: &str,
    node_root: &std::path::Path,
    node_id: &str,
) -> (GovernedDCodeInvocationResult, usize) {
    // Phase 117H: mirror the real governed CLI/DCall-dispatch sequence
    // exactly -- exact-authorized-revision preflight, never the old logical
    // fetch, so this integration test keeps accurately modeling production
    // behavior.
    let preflight = datum::dcode::client::fetch_governed_dcode_execution_material(
        dserver_url, PROJECT, APP, SOURCE_SVC, node_id,
    )
    .expect("governed D-Code preflight (authorization + exact descriptor revision)");
    let authorization: DCodeInvocationAuthorizationV1 = preflight.authorization;
    let artifact: CanonicalDServArtifactV1 = preflight.artifact;
    let module_store = DCodeModuleStore::new(node_root);
    let result = invoke_dcode_governed_capturing_emits(
        &module_store,
        &authorization,
        &artifact,
        node_root,
        TRIGGER_PORT,
        b"trigger-input-ignored-by-source-module",
    )
    .unwrap();
    assert_eq!(
        result.evidence().outcome,
        datum::dcode::evidence::DCodeInvocationOutcome::Success,
        "real source invocation did not succeed: {:?}",
        result.evidence()
    );
    let emit_index = result
        .validated_emits()
        .iter()
        .position(|emit| emit.port() == OUTPUT_PORT)
        .expect("source module must have emitted on OUTPUT_PORT");
    (result, emit_index)
}

// ---------------------------------------------------------------------
// Shared governed-topology setup (113B4A.1): every scenario in this file
// -- the primary success case and all four new failure-mode scenarios --
// needs the SAME real canonical topology (DServer, DGraph/DContinuum,
// DMap/2, DForward, DIoT placement, runtime binding, both node roots with
// their real installed WASM modules). Extracting this once means every new
// negative scenario reuses the EXACT same production setup path as the
// already-passing primary test, rather than a second, subtly different
// construction that could hide a divergence.
// ---------------------------------------------------------------------

struct GovernedTopology {
    dserver_url: String,
    _dserver_guard: KillOnDrop,
    #[allow(dead_code)]
    tmp: PathBuf,
    node_a_root: PathBuf,
    node_b_root: PathBuf,
    registered_binding: serde_json::Value,
    current_binding: serde_json::Value,
}

fn setup_governed_topology(broker_uri: &str) -> GovernedTopology {
    let tmp = std::env::temp_dir().join(format!("datum-dcall-mqtt-governed-e2e-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();

    let (dserver_url, dserver_guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(SOURCE_NODE));
    declare_node(&dserver_url, &node(DEST_NODE));

    let source_module = source_module_with_emit();
    let dest_module = respond_ok_wat(r#"{"status":"ok","reason":null,"emits":[]}"#);

    declare_artifact(
        &dserver_url,
        &artifact(
            SOURCE_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 { port: TRIGGER_PORT.to_string(), payload_schema: "trigger/1".to_string(), external: true }],
                outputs: vec![DServArtifactPortV1 { port: OUTPUT_PORT.to_string(), payload_schema: PAYLOAD_SCHEMA.to_string(), external: false }],
            },
            &source_module,
        ),
    );
    declare_artifact(
        &dserver_url,
        &artifact(
            DEST_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 { port: INPUT_PORT.to_string(), payload_schema: PAYLOAD_SCHEMA.to_string(), external: false }],
                outputs: Vec::new(),
            },
            &dest_module,
        ),
    );

    declare_service_artifact(&dserver_url, &minimal_service_artifact(None, SOURCE_SVC, SOURCE_NODE));
    declare_service_artifact(&dserver_url, &minimal_service_artifact(None, DEST_SVC, DEST_NODE));
    // The DIoT's own backing artifact -- section 5/6: resolved through the
    // exact same production `resolve_diot_artifact_authority` path used for
    // every other DIoT artifact requirement, never fabricated.
    declare_service_artifact(&dserver_url, &minimal_service_artifact(Some(DIOT_ARTIFACT_ID), DIOT_ID, DEST_NODE));

    // Real DMap/2 + DForward + DIoT placement + via_diot_pubsub realization,
    // via the real production proposal/acceptance flow (section 6).
    accept_a_new_v2_proposal(&dserver_url);

    // Real runtime-binding registration (section 7), pointed at the given
    // broker URI (which may be a real reachable broker, or a deliberately
    // unreachable one for the broker-unavailable scenario), then fetched
    // back through the CURRENT-authority endpoint and asserted to match
    // exactly what was registered.
    let registered_binding = register_runtime_binding(&dserver_url, broker_uri);
    let current_binding = fetch_current_runtime_binding(&dserver_url);

    // Destination: its own runtime root, local D-Node identity, installed
    // module -- exactly the same production installation pattern as the
    // Phase-112 HTTP listener test.
    let node_b_root = tmp.join("node-cloud-01");
    std::fs::create_dir_all(&node_b_root).unwrap();
    init_local_identity(&node_b_root, DEST_NODE).unwrap();
    let dest_digest = sha256_hex(&dest_module);
    DCodeModuleStore::new(&node_b_root).install(&dest_module, &dest_digest).unwrap();

    // Source: its own runtime root, local identity, installed source module.
    let node_a_root = tmp.join("node-fog-01");
    std::fs::create_dir_all(&node_a_root).unwrap();
    init_local_identity(&node_a_root, SOURCE_NODE).unwrap();
    let source_digest = sha256_hex(&source_module);
    DCodeModuleStore::new(&node_a_root).install(&source_module, &source_digest).unwrap();

    GovernedTopology {
        dserver_url,
        _dserver_guard: dserver_guard,
        tmp,
        node_a_root,
        node_b_root,
        registered_binding,
        current_binding,
    }
}

/// Real production MQTT listener subprocess (section 10): only
/// authority-derived startup inputs -- no manual channel/broker/service.
fn start_listener(topology: &GovernedTopology) -> KillOnDrop {
    let listener_child = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcall-mqtt-listen"))
        .args([
            "--dserver-url", &topology.dserver_url,
            "--dnode-root", topology.node_b_root.to_str().unwrap(),
            "--project-id", PROJECT,
            "--application-id", APP,
            "--placement-id", DIOT_PLACEMENT_ID,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real smartsentinel-dcall-mqtt-listen");
    let guard = KillOnDrop(listener_child);
    // The listener has no HTTP port to poll for readiness (unlike the HTTP
    // listener); give it a bounded moment to connect+subscribe to the real
    // broker before the source publishes.
    std::thread::sleep(Duration::from_millis(1500));
    guard
}

/// Directly calls the SAME real `POST /api/v1/dcall/grants` production
/// endpoint `dcall::delivery::dispatch_one_call` calls internally (via the
/// public `dcall::client::request_grant`), constructing
/// `SourceExecutionBindingV1` from the real source invocation's own
/// evidence exactly as production code does. Used by the two scenarios
/// (stale binding, duplicate delivery) that need to hold a real, already-
/// issued grant+token separately from delivery -- there is no public seam
/// to do this through `dispatch_validated_emit` alone, since that function
/// issues its own fresh grant and delivers in one call.
fn manually_request_v2_grant(
    dserver_url: &str,
    source_result: &GovernedDCodeInvocationResult,
    emit_index: usize,
    message_id: &str,
) -> RequestGrantResponse {
    let evidence = source_result.evidence();
    let source_execution = SourceExecutionBindingV1 {
        service_id: evidence.service_id.clone(),
        node_id: evidence.node_id.clone(),
        dgraph_id: evidence.dgraph_id.clone(),
        dgraph_revision: evidence.dgraph_revision,
        dgraph_digest: evidence.dgraph_digest.clone(),
        dmap_id: evidence.dmap_id.clone(),
        dmap_revision: evidence.dmap_revision,
        dmap_digest: evidence.dmap_digest.clone(),
        dserv_artifact_id: evidence.dserv_artifact_id.clone(),
        dserv_artifact_digest: evidence.dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: evidence.dserv_artifact_descriptor_digest.clone(),
    };
    let source_dcode_evidence_sha256 = datum::dcode::evidence::dcode_evidence_sha256(evidence);
    let emitted = &source_result.validated_emits()[emit_index];
    let payload = emitted.payload();
    let payload_sha256 = sha256_hex(payload);
    datum::dcall::client::request_grant(
        dserver_url,
        &evidence.project_id,
        &evidence.application_id,
        CALL_ID,
        &evidence.node_id,
        OUTPUT_PORT,
        message_id,
        &evidence.correlation_id,
        &payload_sha256,
        payload.len() as u64,
        &source_execution,
        &source_dcode_evidence_sha256,
    )
    .expect("manual real production grant request must succeed")
}

/// Builds the exact strict MQTT request envelope production code publishes,
/// for a grant already known to carry `via_diot_pubsub` transport authority.
fn build_request_envelope(grant: &datum::dcall::grant::DCallDeliveryGrantV1, token: &str, payload: &[u8]) -> MqttDCallRequestEnvelopeV1 {
    use base64::Engine;
    MqttDCallRequestEnvelopeV1 {
        schema: REQUEST_ENVELOPE_SCHEMA.to_string(),
        grant_id: grant.grant_id.clone(),
        token: token.to_string(),
        payload_b64: base64::engine::general_purpose::STANDARD.encode(payload),
        payload_sha256: sha256_hex(payload),
        payload_size: payload.len() as u64,
    }
}

/// Section 3/30: the primary governed E2E. `#[ignore]`d; requires a real
/// reachable MQTT broker via `DATUM_TEST_MQTT_BROKER_URI`. On a missing
/// prerequisite this prints a clear message and returns WITHOUT having done
/// any partial setup (the check happens before any process is spawned).
#[test]
#[ignore = "spawns real DServer + smartsentinel-dcall-mqtt-listen subprocesses and requires a real MQTT broker; run explicitly via DATUM_TEST_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 cargo test --locked --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1"]
fn governed_mqtt_dcall_delivery_end_to_end() {
    let Some(broker_uri) = broker_uri_from_env() else {
        eprintln!(
            "PREREQUISITE MISSING: DATUM_TEST_MQTT_BROKER_URI is not set. \
             This test requires a real reachable MQTT broker (e.g. \
             `docker run --rm -d -p 1883:1883 eclipse-mosquitto`) and \
             `DATUM_TEST_MQTT_BROKER_URI=mqtt://127.0.0.1:1883`. Skipping \
             before any process was spawned -- NOT a partial run."
        );
        return;
    };

    let topology = setup_governed_topology(&broker_uri);
    assert_eq!(topology.current_binding["binding_id"], topology.registered_binding["binding_id"]);
    assert_eq!(topology.current_binding["transport"], "mqtt");
    assert_eq!(topology.current_binding["broker_uri"], broker_uri);
    assert_eq!(topology.current_binding["project_id"], PROJECT);
    assert_eq!(topology.current_binding["application_id"], APP);
    assert_eq!(topology.current_binding["placement_id"], DIOT_PLACEMENT_ID);
    assert_eq!(topology.current_binding["diot_id"], DIOT_ID);
    assert_eq!(topology.current_binding["node_id"], DEST_NODE);
    assert!(topology.current_binding["dmap_id"].as_str().unwrap().starts_with("dmap:"));
    assert_eq!(topology.current_binding["dforward_id"], DFORWARD_ID);
    assert_eq!(topology.current_binding["artifact_id"], DIOT_ARTIFACT_ID);
    assert!(topology.current_binding["execution_generation"].as_u64().unwrap() >= 1);
    assert!(!topology.current_binding["execution_projection_digest"].as_str().unwrap().is_empty());

    let _listener_guard = start_listener(&topology);

    let (source_result, emit_index) = run_real_source_invocation(&topology.dserver_url, &topology.node_a_root, SOURCE_NODE);
    let emitted_payload = source_result.validated_emits()[emit_index].payload().to_vec();
    assert_eq!(emitted_payload, EMITTED_PAYLOAD, "dispatched bytes must be the REAL source emit");

    let expected_topic = datum::dcall::mqtt::request_topic(PROJECT, APP, DIOT_PLACEMENT_ID, DEST_NODE, DCALL_CHANNEL);

    let context = SourceExecutionContext {
        dserver_url: topology.dserver_url.clone(),
        dnode_root: topology.node_a_root.to_string_lossy().to_string(),
    };
    // Real production dispatch path (section 11/13): `dispatch_validated_emit`
    // internally requests the grant, recognizes the typed DMap/2
    // `via_diot_pubsub` transport authority, and routes through the real
    // `deliver_via_diot_pubsub` -> real `RumqttcPubSub` -> real broker --
    // no MQTT envelope is injected by this test for this primary case.
    let results = dispatch_validated_emit(&context, &source_result, emit_index);

    assert_eq!(results.len(), 1, "expected exactly one D-Call edge to resolve, got: {results:?}");
    let evidence = &results[0];

    // Section 13: exact required success assertions.
    assert!(
        matches!(evidence.outcome, DCallDeliveryOutcome::Success),
        "governed MQTT delivery did not succeed: {evidence:?}"
    );
    assert_eq!(evidence.call_id, CALL_ID);
    assert_eq!(evidence.source.node_id, SOURCE_NODE);
    assert_eq!(evidence.destination.service_id, DEST_SVC);
    assert_eq!(evidence.destination.node_id, DEST_NODE);
    assert_eq!(evidence.payload_sha256, sha256_hex(&emitted_payload));
    assert!(evidence.error_code.is_none());
    assert!(
        evidence.destination_dcode_evidence_sha256.is_some(),
        "destination WASM must have actually executed and produced evidence"
    );
    let recomputed_source_digest = datum::dcode::evidence::dcode_evidence_sha256(source_result.evidence());
    assert_eq!(
        evidence.source_dcode_evidence_sha256.as_deref(),
        Some(recomputed_source_digest.as_str()),
        "DCall evidence must bind the REAL source evidence digest"
    );

    // Section 13/38: raw payload/token/broker URI absent from durable
    // evidence -- checked by inspecting the actual serialized evidence, not
    // merely by construction.
    let serialized = serde_json::to_string(evidence).unwrap();
    assert!(!serialized.contains(std::str::from_utf8(EMITTED_PAYLOAD).unwrap()), "raw payload leaked into durable evidence");
    assert!(!serialized.contains(&broker_uri), "broker URI leaked into durable evidence");
    assert!(!serialized.to_lowercase().contains("token"), "a token-shaped field leaked into durable evidence");

    // Section 12/17: the request topic actually used was the exact expected
    // deterministic topic (recomputed independently here from the grant's
    // own identity fields, not merely asserted equal to itself).
    // The listener's own redemption already independently re-derived and
    // matched this topic server-side (B3/B3.1, `observed_mqtt_topic`) --
    // this local recomputation additionally proves the TEST's expectation
    // agrees with production, not just that production agreed with itself.
    assert!(!expected_topic.contains(DCALL_CHANNEL), "topic must not carry the raw canonical channel");
    assert!(!expected_topic.contains('+') && !expected_topic.contains('#'));
}

/// 113B4A.1 section 3: a legitimately stale runtime binding must fail
/// closed against a real broker, with zero destination execution.
///
/// HONEST DISCLOSURE: this proves the REDEMPTION-side stale-binding
/// protection for real, over a real broker and a real destination listener
/// subprocess -- not the source-side pre-publish check. The source-side
/// check lives inside the private `deliver_via_diot_pubsub`, reachable only
/// via the public `dispatch_validated_emit`, which issues its OWN fresh
/// grant bound to whatever binding is CURRENT at the moment it is called --
/// there is no public seam to hold an already-issued grant across a
/// legitimate binding change and then hand it to that same function later,
/// so the source-side race is not reachable deterministically without
/// adding a new test-only production seam (out of scope: "do not redesign
/// the implementation"). This test therefore obtains a real grant bound to
/// binding revision N via the same real `request_grant` HTTP call
/// production code makes, legitimately re-registers the binding (bumping it
/// to revision N+1 through the real `PUT` API, exactly as an operator
/// changing the broker declaration would), and then publishes the exact
/// production envelope shape directly to the real broker so the REAL
/// destination listener subprocess attempts REAL redemption against
/// DServer. That redemption is required to fail closed via the existing
/// Phase 113B1 `verify_runtime_binding_still_current` staleness check
/// (already unit-tested; this proves it for real, end-to-end, over a real
/// broker). Because a redemption failure causes the listener to publish NO
/// response at all (`handle_mqtt_dcall_request` returns `None` -- see its
/// doc comment), "no reply arrives within a bounded window" is the correct
/// externally observable proof of "fail closed, no destination execution,
/// no false success".
#[test]
#[ignore = "spawns real DServer + smartsentinel-dcall-mqtt-listen subprocesses and requires a real MQTT broker; run explicitly via DATUM_TEST_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 cargo test --locked --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1"]
fn real_broker_stale_runtime_binding_after_legitimate_change_fails_closed() {
    let Some(broker_uri) = broker_uri_from_env() else {
        eprintln!("PREREQUISITE MISSING: DATUM_TEST_MQTT_BROKER_URI is not set -- skipping before any process was spawned.");
        return;
    };
    let topology = setup_governed_topology(&broker_uri);
    let binding_revision_n = topology.registered_binding["binding_revision"].clone();
    let _listener_guard = start_listener(&topology);

    let (source_result, emit_index) = run_real_source_invocation(&topology.dserver_url, &topology.node_a_root, SOURCE_NODE);
    let issued = manually_request_v2_grant(&topology.dserver_url, &source_result, emit_index, "m-stale-binding");
    assert!(issued.grant.v2_dcall_transport_authority.is_some());
    let v2_authority = issued.grant.v2_dcall_transport_authority.as_ref().unwrap();
    assert_eq!(
        v2_authority.binding_revision.to_string(),
        binding_revision_n.to_string(),
        "sanity: the grant must be bound to the ORIGINAL binding revision N"
    );

    // Legitimate re-registration, through the real production PUT API, with
    // a genuinely different broker_uri -- NOT a hand-mutation of persisted
    // state. `register_runtime_binding`'s own idempotency rule (B1.1) means
    // a byte-identical re-registration would NOT bump the revision; this
    // re-registration is deliberately non-identical.
    let unreachable_port = free_port();
    let new_broker_uri = format!("mqtt://127.0.0.1:{unreachable_port}");
    let re_registered = register_runtime_binding(&topology.dserver_url, &new_broker_uri);
    assert_ne!(
        re_registered["binding_revision"], binding_revision_n,
        "a genuinely different broker_uri must bump the binding revision (B1.1 idempotency does not apply here)"
    );

    let emitted_payload = source_result.validated_emits()[emit_index].payload().to_vec();
    let envelope = build_request_envelope(&issued.grant, &issued.token, &emitted_payload);
    let envelope_bytes = serde_json::to_vec(&envelope).unwrap();

    let broker = datum::dcall::mqtt::parse_broker_uri_for_delivery(&broker_uri).unwrap();
    let request_topic = datum::dcall::mqtt::request_topic(
        PROJECT,
        APP,
        &v2_authority.diot_placement_id,
        DEST_NODE,
        &v2_authority.channel,
    );
    let reply_topic = datum::dcall::mqtt::reply_topic(PROJECT, APP, SOURCE_NODE, &issued.grant.grant_id);

    let mut subscriber = RumqttcPubSub::connect(&broker, &format!("datum-test-sub-{}", uuid::Uuid::new_v4())).unwrap();
    subscriber.subscribe(&reply_topic).unwrap();
    assert!(
        subscriber.poll(Duration::from_millis(1500)).unwrap().is_none(),
        "no message should exist yet -- this poll only established the subscription"
    );

    let mut publisher = RumqttcPubSub::connect(&broker, &format!("datum-test-pub-{}", uuid::Uuid::new_v4())).unwrap();
    publisher.publish(&request_topic, &envelope_bytes, MqttQos::AtLeastOnce, false).unwrap();
    let _ = publisher.poll(Duration::from_millis(1000)); // flush the publish onto the wire

    let reply = subscriber.poll(Duration::from_secs(5)).unwrap();
    assert!(
        reply.is_none(),
        "a request bound to a STALE runtime binding must produce NO reply at all (redemption must fail closed BEFORE execution, and a redemption failure is silently dropped by design) -- got: {reply:?}"
    );
}

/// 113B4A.1 section 4: an unreachable (but structurally valid) `mqtt://`
/// broker must produce a deterministic, BOUNDED transport failure -- never
/// an infinite wait, never a local/HTTP fallback, never a false success.
/// Uses the full real production `dispatch_validated_emit` path (the
/// source-side connection-failure branch IS naturally reachable this way,
/// unlike the stale-binding race above).
#[test]
#[ignore = "spawns a real DServer subprocess and requires network access to attempt (and fail) a real MQTT connection; run explicitly via cargo test --locked --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1"]
fn real_broker_unavailable_broker_fails_closed_without_execution() {
    // This scenario needs no externally-provided broker at all -- it proves
    // failure against a broker that structurally cannot exist -- but it
    // still gates on the same env var as the other real-broker scenarios
    // for consistent, explicit opt-in (never runs in an ordinary `cargo test`).
    if broker_uri_from_env().is_none() {
        eprintln!("PREREQUISITE MISSING: DATUM_TEST_MQTT_BROKER_URI is not set -- skipping before any process was spawned.");
        return;
    }
    let unreachable_port = free_port();
    let unreachable_broker_uri = format!("mqtt://127.0.0.1:{unreachable_port}");
    let topology = setup_governed_topology(&unreachable_broker_uri);
    // Deliberately NOT starting the listener -- it could never connect
    // either, and its absence is not what this scenario is proving.

    let (source_result, emit_index) = run_real_source_invocation(&topology.dserver_url, &topology.node_a_root, SOURCE_NODE);
    let context = SourceExecutionContext {
        dserver_url: topology.dserver_url.clone(),
        dnode_root: topology.node_a_root.to_string_lossy().to_string(),
    };

    let start = Instant::now();
    let results = dispatch_validated_emit(&context, &source_result, emit_index);
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 1);
    let evidence = &results[0];
    assert!(
        !matches!(evidence.outcome, DCallDeliveryOutcome::Success),
        "delivery to an unreachable broker must never report Success: {evidence:?}"
    );
    assert!(evidence.error_code.is_some(), "an unreachable broker must produce a deterministic error code");
    assert!(evidence.destination_dcode_evidence_sha256.is_none(), "destination WASM must never have executed");
    assert!(evidence.redeemed_at.is_none(), "the grant must never have been redeemed");
    assert!(
        elapsed < Duration::from_secs(30),
        "failure against an unreachable broker must be BOUNDED, took {elapsed:?}"
    );
}

/// 113B4A.1 section 5: broker reachable, topology fully valid, but NO
/// destination listener running. Distinguishes "broker unavailable" from
/// "broker available but destination runtime absent" -- the source must
/// publish successfully (broker accepts/progresses real MQTT traffic) and
/// then time out waiting for a reply that will never come, with no false
/// success and no automatic republish.
#[test]
#[ignore = "spawns a real DServer subprocess and requires a real reachable MQTT broker; run explicitly via DATUM_TEST_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 cargo test --locked --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1"]
fn real_broker_listener_absent_source_times_out_without_execution() {
    let Some(broker_uri) = broker_uri_from_env() else {
        eprintln!("PREREQUISITE MISSING: DATUM_TEST_MQTT_BROKER_URI is not set -- skipping before any process was spawned.");
        return;
    };
    let topology = setup_governed_topology(&broker_uri);
    // Deliberately NOT calling start_listener(&topology).

    let (source_result, emit_index) = run_real_source_invocation(&topology.dserver_url, &topology.node_a_root, SOURCE_NODE);
    let context = SourceExecutionContext {
        dserver_url: topology.dserver_url.clone(),
        dnode_root: topology.node_a_root.to_string_lossy().to_string(),
    };

    let start = Instant::now();
    let results = dispatch_validated_emit(&context, &source_result, emit_index);
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 1);
    let evidence = &results[0];
    assert!(
        !matches!(evidence.outcome, DCallDeliveryOutcome::Success),
        "delivery with no destination listener must never report Success: {evidence:?}"
    );
    assert!(evidence.error_code.is_some());
    assert!(evidence.destination_dcode_evidence_sha256.is_none(), "destination WASM must never have executed -- no listener was running");
    assert!(evidence.redeemed_at.is_none(), "the grant must never have been redeemed -- no listener was running to redeem it");
    // Production caps `max_wait` at exactly 30s (`deliver_via_diot_pubsub`'s
    // caller: `bound.min(Duration::from_secs(30))`), so the observed elapsed
    // time is that ~30s wait PLUS the real overhead of every real HTTP/MQTT
    // call this scenario also performs (governance setup, grant issuance,
    // broker connect/subscribe/publish) -- the bound below is deliberately
    // generous relative to the real 30s cap, not an independently-invented
    // timeout.
    assert!(
        elapsed < Duration::from_secs(45),
        "the timeout must be BOUNDED (production caps the wait at 30s) even though the broker itself is healthy, took {elapsed:?}"
    );
}

/// 113B4A.1 section 6: a physical duplicate MQTT delivery of the exact same
/// governed request/grant, against a real broker and a real destination
/// listener subprocess, must execute the destination exactly once.
///
/// HONEST DISCLOSURE: like the stale-binding scenario above, this cannot go
/// through `dispatch_validated_emit` alone (it issues one grant and
/// delivers exactly once by construction) -- it uses the same real
/// `request_grant` HTTP call to obtain one real grant+token, then publishes
/// the IDENTICAL production envelope TWICE to the real broker. The REAL
/// listener subprocess and REAL DServer redemption path handle both
/// deliveries for real; the single-use grant is the only dedupe authority
/// exercised (no parallel test/production dedupe state is added -- section
/// 6/7). Because a redemption failure (here: "already redeemed") causes the
/// listener to publish NO response, the externally observable proof is:
/// exactly one reply arrives (the first, carrying real destination evidence)
/// and a second bounded poll for a further reply times out.
#[test]
#[ignore = "spawns real DServer + smartsentinel-dcall-mqtt-listen subprocesses and requires a real MQTT broker; run explicitly via DATUM_TEST_MQTT_BROKER_URI=mqtt://127.0.0.1:1883 cargo test --locked --test dcall_mqtt_governed_e2e -- --ignored --test-threads=1"]
fn real_broker_duplicate_mqtt_delivery_executes_destination_exactly_once() {
    let Some(broker_uri) = broker_uri_from_env() else {
        eprintln!("PREREQUISITE MISSING: DATUM_TEST_MQTT_BROKER_URI is not set -- skipping before any process was spawned.");
        return;
    };
    let topology = setup_governed_topology(&broker_uri);
    let _listener_guard = start_listener(&topology);

    let (source_result, emit_index) = run_real_source_invocation(&topology.dserver_url, &topology.node_a_root, SOURCE_NODE);
    let issued = manually_request_v2_grant(&topology.dserver_url, &source_result, emit_index, "m-duplicate-delivery");
    assert!(issued.grant.v2_dcall_transport_authority.is_some());
    let v2_authority = issued.grant.v2_dcall_transport_authority.as_ref().unwrap();

    let emitted_payload = source_result.validated_emits()[emit_index].payload().to_vec();
    let envelope = build_request_envelope(&issued.grant, &issued.token, &emitted_payload);
    let envelope_bytes = serde_json::to_vec(&envelope).unwrap();

    let broker = datum::dcall::mqtt::parse_broker_uri_for_delivery(&broker_uri).unwrap();
    let request_topic = datum::dcall::mqtt::request_topic(
        PROJECT,
        APP,
        &v2_authority.diot_placement_id,
        DEST_NODE,
        &v2_authority.channel,
    );
    let reply_topic = datum::dcall::mqtt::reply_topic(PROJECT, APP, SOURCE_NODE, &issued.grant.grant_id);

    let mut subscriber = RumqttcPubSub::connect(&broker, &format!("datum-test-sub-{}", uuid::Uuid::new_v4())).unwrap();
    subscriber.subscribe(&reply_topic).unwrap();
    assert!(
        subscriber.poll(Duration::from_millis(1500)).unwrap().is_none(),
        "no message should exist yet -- this poll only established the subscription"
    );

    let mut publisher = RumqttcPubSub::connect(&broker, &format!("datum-test-pub-{}", uuid::Uuid::new_v4())).unwrap();

    // First delivery: must succeed and execute the destination exactly once.
    publisher.publish(&request_topic, &envelope_bytes, MqttQos::AtLeastOnce, false).unwrap();
    let _ = publisher.poll(Duration::from_millis(1000));
    let (first_reply_topic, first_reply_bytes) = subscriber
        .poll(Duration::from_secs(10))
        .unwrap()
        .expect("the FIRST delivery of a fresh grant must produce a reply");
    assert_eq!(first_reply_topic, reply_topic);
    let first_response: datum::dcall::mqtt::MqttDCallResponseEnvelopeV1 =
        serde_json::from_slice(&first_reply_bytes).unwrap();
    assert_eq!(first_response.grant_id, issued.grant.grant_id);
    assert_eq!(first_response.outcome, "success", "the first delivery must succeed: {first_response:?}");
    assert!(
        first_response.dcode_evidence.is_some(),
        "the first delivery must carry real destination D-Code evidence proving it actually executed"
    );

    // Duplicate delivery: the EXACT SAME envelope, published again. The
    // grant is already Redeemed -- DServer's redemption must reject it
    // ("already redeemed"), which (per `handle_mqtt_dcall_request`'s own
    // documented behavior) means the listener publishes NO second reply at
    // all, and -- because redemption gates execution -- the destination
    // D-Code is structurally never invoked a second time.
    publisher.publish(&request_topic, &envelope_bytes, MqttQos::AtLeastOnce, false).unwrap();
    let _ = publisher.poll(Duration::from_millis(1000));
    let duplicate_reply = subscriber.poll(Duration::from_secs(5)).unwrap();
    assert!(
        duplicate_reply.is_none(),
        "a duplicate delivery of an already-redeemed grant must produce NO second reply (no second execution) -- got: {duplicate_reply:?}"
    );
}
