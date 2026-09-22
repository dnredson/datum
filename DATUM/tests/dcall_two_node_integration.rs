//! Mandatory real two-logical-D-Node-on-one-host integration test (ADR-0022,
//! section 30). This is deliberately NOT mocked: it builds and launches the
//! REAL `DServer` binary (a separate crate/workspace member) against
//! isolated temp storage/config, declares two real logical D-Node local
//! identities ("fog-01" and "cloud-01") in two separate runtime roots,
//! launches the REAL `smartsentinel-dcall-listen` binary as node cloud-01's
//! listener, and drives the REAL `dcall::delivery::dispatch_validated_emit`
//! library seam end-to-end: grant issuance -> HTTP -> node-B listener ->
//! grant redemption -> a REAL isolated-worker D-Code invocation on node B ->
//! evidence.
//!
//! Independent review fix v2 (I1E/I8): the SOURCE side's own D-Code
//! invocation IS now actually executed here, through the same real
//! `IsolatedWorkerExecutor` used for the destination, via
//! `dcode::invoke::invoke_dcode_capturing_emits` -- the dispatched payload is
//! the REAL decoded emit bytes from that REAL successful invocation's own
//! recorded evidence, not a host-injected literal. This closes the v1 gap
//! where the "source module" was declared but never actually run.
//!
//! Seven tests use default production APIs, a provisioned real sibling
//! worker, and real isolated DServer/storage. Ordinary broad runs discover
//! these tests but skip them. Run explicitly with:
//! `cargo test --locked --test dcall_two_node_integration -- --ignored --test-threads=1`.

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "support/dcall_proof.rs"]
mod proof;
use proof::RecordingProxy;

use datum::agent::canonical_dcontinuum::{
    CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
    CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
};
use datum::agent::canonical_dgraph::{
    CanonicalDCallV1, CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
    CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
};
use datum::agent::canonical_dmap::CanonicalDServPlacementV1;
use datum::dcall::delivery::{dispatch_validated_emit, SourceExecutionContext};
use datum::dcall::evidence::DCallDeliveryOutcome;
use datum::dcall::grant::SourceExecutionBindingV1;
use datum::dcode::abi::DNODE_ABI_V0_REQUIRED_EXPORTS;
use datum::dcode::authorization::DCodeInvocationAuthorizationV1;
use datum::dcode::dserv_artifact::{
    CanonicalDServArtifactV1, DServArtifactAbiV1, DServArtifactConstraintsV1, DServArtifactDCodeV1,
    DServArtifactLimitsV1, DServArtifactPortV1, DServArtifactPortsV1, DServArtifactSafetyV1,
    CANONICAL_DSERV_ARTIFACT_SCHEMA,
};
use datum::dcode::identity::sha256_hex;
use datum::dcode::invoke::invoke_dcode_governed_capturing_emits;
use datum::dcode::local_identity::init_local_identity;
use datum::dcode::module_store::DCodeModuleStore;

const TRIGGER_PORT: &str = "trigger";
const EMITTED_PAYLOAD: &[u8] = b"integration-test-emitted-payload";

const PROJECT: &str = "proj:dcall-two-node-it";
const APP: &str = "app:pipeline";
const SOURCE_SVC: &str = "sensor";
const DEST_SVC: &str = "decoder";
const CALL_ID: &str = "sensor-to-decoder";
const OUTPUT_PORT: &str = "raw";
const INPUT_PORT: &str = "in";
const PAYLOAD_SCHEMA: &str = "raw-bytes/1";
const SOURCE_NODE: &str = "fog-01";
const DEST_NODE: &str = "cloud-01";

fn install_real_sibling_worker() {
    let installed = datum::dcode::worker::trusted_sibling_worker_binary_path();
    let built = PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"));
    std::fs::copy(&built, &installed)
        .expect("provision real Cargo-built sibling worker for test installation");
    assert_eq!(
        std::fs::read(built).unwrap(),
        std::fs::read(installed).unwrap()
    );
}

/// Independently inspect SQLite using a read-only connection, including
/// history so an inserted-then-hidden grant cannot escape this assertion.
fn persisted_grants(tmp: &std::path::Path) -> serde_json::Value {
    let python = std::env::var("DATUM_PROOF_PYTHON").unwrap_or_else(|_| "python".to_string());
    let output = Command::new(&python).args(["-B", "-c", r#"
import sqlite3,json,sys,pathlib
c=sqlite3.connect(pathlib.Path(sys.argv[1]).resolve().as_uri()+'?mode=ro',uri=True)
row=c.execute("SELECT payload_json FROM control_state_current WHERE domain='dcall_grant_state'").fetchone()
history=c.execute("SELECT COUNT(*) FROM control_state_history WHERE domain='dcall_grant_state'").fetchone()[0]
print(json.dumps({'state':json.loads(row[0]) if row else {'grants':{}},'history':history}))
"#]).arg(tmp.join("control-plane.db")).output().unwrap_or_else(|error| panic!("read-only SQLite proof process spawn failed: kind={:?} raw_os_error={:?} display={} debug={:?} executable={:?}", error.kind(), error.raw_os_error(), error, error, python));
    assert!(
        output.status.success(),
        "SQLite inspection failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn assert_no_grants(tmp: &std::path::Path) {
    let stored = persisted_grants(tmp);
    assert!(
        stored["state"]["grants"].as_object().unwrap().is_empty(),
        "unexpected grant: {stored}"
    );
    assert_eq!(
        stored["history"], 0,
        "rejection must precede every grant-state write"
    );
}

fn binding_json(e: &datum::dcode::evidence::DCodeInvocationEvidenceV1) -> serde_json::Value {
    serde_json::json!({
        "service_id": e.service_id, "node_id": e.node_id,
        "dgraph_id": e.dgraph_id, "dgraph_revision": e.dgraph_revision, "dgraph_digest": e.dgraph_digest,
        "dmap_id": e.dmap_id, "dmap_revision": e.dmap_revision, "dmap_digest": e.dmap_digest,
        "dserv_artifact_id": e.dserv_artifact_id, "dserv_artifact_digest": e.dserv_artifact_digest,
        "dserv_artifact_descriptor_digest": e.dserv_artifact_descriptor_digest,
    })
}

fn grant_request_json(
    e: &datum::dcode::evidence::DCodeInvocationEvidenceV1,
    binding: serde_json::Value,
    size: u64,
) -> serde_json::Value {
    serde_json::json!({
        "project_id": e.project_id, "application_id": e.application_id, "call_id": CALL_ID,
        "source_node_id": e.node_id, "source_output_port": OUTPUT_PORT,
        "message_id": format!("proof:{}", uuid::Uuid::new_v4()), "correlation_id": e.correlation_id,
        "payload_sha256": sha256_hex(EMITTED_PAYLOAD), "payload_size": size,
        "source_execution": binding,
        "source_dcode_evidence_sha256": datum::dcode::evidence::dcode_evidence_sha256(e),
    })
}

fn assert_real_local_evidence_chain(
    url: &str,
    root: &std::path::Path,
    source: &datum::dcode::invoke::GovernedDCodeInvocationResult,
) {
    // A second, fresh local grant makes the complete transient destination
    // record observable without persisting payloads or changing dispatch's API.
    let e = source.evidence();
    let response = http()
        .post(format!("{url}/api/v1/dcall/grants"))
        .json(&grant_request_json(
            e,
            binding_json(e),
            EMITTED_PAYLOAD.len() as u64,
        ))
        .send()
        .unwrap();
    assert!(response.status().is_success());
    let issued: datum::dcall::grant::RequestGrantResponse = response.json().unwrap();
    let grant = issued.grant;
    let destination = datum::dcall::delivery::receive_governed_delivery(
        url,
        root.to_str().unwrap(),
        &grant.grant_id,
        &issued.token,
        EMITTED_PAYLOAD,
    )
    .unwrap();
    assert_eq!(
        destination.outcome,
        datum::dcode::evidence::DCodeInvocationOutcome::Success
    );
    let bytes = serde_json::to_vec(&destination).unwrap();
    let delivery = datum::dcall::evidence::build_success_evidence(
        "local-chain-proof".to_string(),
        &grant,
        chrono::Utc::now().to_rfc3339(),
        &bytes,
        0,
    );
    assert_eq!(
        datum::dcode::evidence::dcode_evidence_sha256(e),
        grant.source_dcode_evidence_sha256
    );
    assert_eq!(
        delivery.source_dcode_evidence_sha256.as_deref(),
        Some(grant.source_dcode_evidence_sha256.as_str())
    );
    assert_eq!(
        delivery.destination_dcode_evidence_sha256.as_deref(),
        Some(datum::dcode::evidence::dcode_evidence_sha256(&destination).as_str())
    );
    assert_eq!(grant.message_id, delivery.message_id);
    assert_eq!(grant.message_id, destination.message_id);
    assert_eq!(e.correlation_id, grant.correlation_id);
    assert_eq!(e.correlation_id, delivery.correlation_id);
    assert_eq!(e.correlation_id, destination.correlation_id);
    assert!(!serde_json::to_string(&delivery)
        .unwrap()
        .contains(std::str::from_utf8(EMITTED_PAYLOAD).unwrap()));
}

struct ServerGuard {
    proxy: RecordingProxy,
    _process: KillOnDrop,
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
        .join(if cfg!(windows) {
            "DServer.exe"
        } else {
            "DServer"
        })
}

/// Builds (via a real `cargo build` subprocess, so this is honestly whatever
/// is currently checked out on this branch, not a stale artifact) and
/// launches the REAL `DServer` binary against fully isolated temp storage,
/// returning its base URL and a process guard.
fn spawn_real_dserver(tmp: &std::path::Path) -> (String, ServerGuard) {
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
            "[server]\nbind = \"127.0.0.1:{port}\"\ndebug = true\n\n[firebase]\nproject_id = \"dcall-it-unused\"\ncredentials_path = \"unused.json\"\n\n[cors]\nenabled = false\n"
        ),
    )
    .unwrap();

    // Each test owns its executable as well as storage. An abnormal test
    // exit cannot leave a process holding Cargo's build output open.
    let isolated_binary = tmp.join(if cfg!(windows) {
        "DServer.exe"
    } else {
        "DServer"
    });
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
    let proxy = RecordingProxy::new(format!("http://127.0.0.1:{port}"));
    (
        proxy.url.clone(),
        ServerGuard {
            proxy,
            _process: KillOnDrop(child),
        },
    )
}

fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

fn declare_node(base_url: &str, node: &CanonicalDNodeV1) {
    let url = format!("{base_url}/api/v1/dnodes/{}", node.node_id);
    let response = http().put(&url).json(node).send().unwrap();
    assert!(
        response.status().is_success(),
        "declare_node({}) failed: {} {}",
        node.node_id,
        response.status(),
        response.text().unwrap_or_default()
    );
}

fn declare_artifact(base_url: &str, artifact: &CanonicalDServArtifactV1) {
    let url = format!("{base_url}/api/v1/dcode/artifacts");
    let response = http().post(&url).json(artifact).send().unwrap();
    assert!(
        response.status().is_success(),
        "declare_artifact({}/{}) failed: {} {}",
        artifact.application_id,
        artifact.dserv_id,
        response.status(),
        response.text().unwrap_or_default()
    );
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

fn dgraph() -> CanonicalDGraphV1 {
    CanonicalDGraphV1 {
        schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
        application_id: APP.to_string(),
        dgraph_id: "dgraph:pipeline".to_string(),
        revision: 1,
        services: vec![
            CanonicalDServV1 {
                service_id: SOURCE_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 1,
                },
            },
            CanonicalDServV1 {
                service_id: DEST_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 1,
                },
            },
        ],
        calls: vec![CanonicalDCallV1 {
            call_id: CALL_ID.to_string(),
            source_service_id: SOURCE_SVC.to_string(),
            target_service_id: DEST_SVC.to_string(),
        }],
    }
}

fn placeholder_dcontinuum(node_ids: &[&str]) -> CanonicalDContinuumV1 {
    // Overwritten server-side by the authoritative D-Continuum derived from
    // the real D-Node registry (foundation closure, ADR-0019) -- only needs
    // to be schema-valid and consistent with the nodes actually declared.
    let mut nodes: Vec<CanonicalDNodeV1> = node_ids.iter().map(|id| node(id)).collect();
    nodes.dedup_by(|a, b| a.node_id == b.node_id);
    CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: PROJECT.to_string(),
        dcontinuum_id: "dcontinuum:it".to_string(),
        revision: 1,
        nodes,
        links: Vec::new(),
    }
}

fn artifact(
    service_id: &str,
    ports: DServArtifactPortsV1,
    module_bytes: &[u8],
) -> CanonicalDServArtifactV1 {
    let digest = sha256_hex(module_bytes);
    CanonicalDServArtifactV1 {
        schema: CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
        artifact_id: format!("dserv:pipeline:{service_id}"),
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
            required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS
                .iter()
                .map(|s| s.to_string())
                .collect(),
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

/// A module that ignores its (enveloped) input and always returns a fixed,
/// valid ADR-0003 output envelope with no emits -- reusing the exact
/// pattern `dcode::invoke`'s own unit tests already establish for
/// "respond with a canned valid envelope."
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

/// Independent review fix v2 (I1E): the SOURCE module -- ignores its
/// (enveloped) trigger input and always emits `EMITTED_PAYLOAD` on
/// `OUTPUT_PORT`, via the SAME real ADR-0003 output-envelope mechanism the
/// destination modules use. This is what makes the dispatched payload in
/// `real_cross_node_dcall_delivery_end_to_end`/`real_same_node_dcall_delivery_end_to_end`
/// genuinely come from a real, successful source D-Code invocation rather
/// than a host-injected literal.
fn source_module_with_emit() -> Vec<u8> {
    use base64::Engine;
    let payload_b64 = base64::engine::general_purpose::STANDARD.encode(EMITTED_PAYLOAD);
    respond_ok_wat(&format!(
        r#"{{"status":"ok","emits":[{{"port":"{OUTPUT_PORT}","payload_b64":"{payload_b64}"}}]}}"#
    ))
}

/// Independent review fix v2 (I1E/I8): actually invokes the SOURCE D-Serv's
/// real D-Code, through the real isolated worker, fetching a FRESH
/// authorization/artifact from the real DServer exactly as the governed CLI
/// path does -- returns the real `validated_emits` a trusted D-Call dispatch
/// caller would use. Panics (via the various `.expect(...)`s) if the source
/// invocation itself did not succeed, since every caller of this helper
/// requires a genuinely successful source invocation to proceed.
fn run_real_source_invocation(
    dserver_url: &str,
    node_root: &std::path::Path,
    _worker_binary: &std::path::Path,
    node_id: &str,
) -> (datum::dcode::invoke::GovernedDCodeInvocationResult, usize) {
    let authorization: DCodeInvocationAuthorizationV1 =
        datum::dcode::client::fetch_dcode_authorization(
            dserver_url,
            PROJECT,
            APP,
            SOURCE_SVC,
            node_id,
        )
        .expect("fetch source D-Code authorization");
    let artifact: CanonicalDServArtifactV1 =
        datum::dcode::client::fetch_dserv_artifact(dserver_url, APP, SOURCE_SVC)
            .expect("fetch source D-Serv artifact");
    let module_store = DCodeModuleStore::new(node_root);
    // Independent review fix v5 (K1): this is now a `GovernedDCodeInvocationResult`
    // -- still produced through the exact same real isolated-worker call as
    // before, but the TYPE itself now proves that (production D-Call dispatch
    // accepts nothing less), not merely the fact that this test happened to
    // use a real executor.
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

/// A minimal, structurally valid `ServiceArtifact` (the transitional
/// operational bridge type, distinct from canonical `CanonicalDServArtifactV1`
/// -- see `dcall-existing-contract-audit.txt`). `create_proposal`'s own
/// candidate-DMap validation requires exactly one such artifact per
/// canonical service (`resolve_artifact_for_service`, ADR-0002 ELIGIBILITY),
/// independent of the D-Code artifact registry entirely.
fn minimal_service_artifact(service_id: &str, target_node: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "0.2.0",
        "artifact_id": format!("artifact:{service_id}"),
        "service_id": service_id,
        "runtime": {
            "kind": "container",
            "container": {
                "container_name": service_id,
                "image": format!("example.invalid/{service_id}:test"),
                "image_source_kind": "local_build",
            }
        },
        "probes": {
            "execution": {"kind": "process_running"}
        },
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
    assert!(
        status.is_success(),
        "declare_service_artifact failed: {status} {text}"
    );
}

fn accept_a_new_proposal(base_url: &str, source_node: &str, dest_node: &str) {
    accept_a_new_proposal_with_dgraph(base_url, source_node, dest_node, dgraph());
}

/// Section 2 (v6): generalized so a test can accept a SECOND deployment
/// whose D-Graph revision differs from the first -- the exact "load-bearing
/// authority changed after source execution" mutation the stale-source race
/// test needs, using the real accept-proposal path rather than fabricating
/// state directly.
fn accept_a_new_proposal_with_dgraph(
    base_url: &str,
    source_node: &str,
    dest_node: &str,
    dgraph: CanonicalDGraphV1,
) {
    let payload = serde_json::json!({
        "application_id": APP,
        "source": "operator",
        "dgraph": dgraph,
        "dcontinuum": placeholder_dcontinuum(&[source_node, dest_node]),
        "placements": [
            CanonicalDServPlacementV1 { service_id: SOURCE_SVC.to_string(), node_id: source_node.to_string() },
            CanonicalDServPlacementV1 { service_id: DEST_SVC.to_string(), node_id: dest_node.to_string() },
        ],
    });
    let response = http()
        .post(format!("{base_url}/api/v1/ddeploy/proposals"))
        .header("X-Datum-Project", PROJECT)
        .json(&payload)
        .send()
        .unwrap();
    let status = response.status();
    let text = response.text().unwrap_or_default();
    assert!(
        status.is_success(),
        "create_proposal failed: {status} {text}"
    );
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    let proposal_id = body["proposal"]["proposal_id"]
        .as_str()
        .expect("proposal_id in response")
        .to_string();

    let accept = http()
        .post(format!(
            "{base_url}/api/v1/ddeploy/proposals/{proposal_id}/accept"
        ))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({"accepted_by": "dcall-two-node-integration-test"}))
        .send()
        .unwrap();
    let accept_status = accept.status();
    let accept_body = accept.text().unwrap_or_default();
    assert!(
        accept_status.is_success(),
        "accept_proposal failed: {accept_status} {accept_body}"
    );
}

#[test]
#[ignore = "spawns real DServer + smartsentinel-dcall-listen subprocesses; run explicitly via `cargo test --locked --tests -- --test-threads=1 --ignored dcall_two_node`"]
fn real_cross_node_dcall_delivery_end_to_end() {
    let tmp =
        std::env::temp_dir().join(format!("datum-dcall-two-node-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();

    let (dserver_url, _dserver_guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(SOURCE_NODE));
    declare_node(&dserver_url, &node(DEST_NODE));

    let source_module = source_module_with_emit();
    let dest_module = respond_ok_wat(r#"{"status":"ok","reason":null,"emits":[]}"#);

    declare_artifact(
        &dserver_url,
        &artifact(
            SOURCE_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: TRIGGER_PORT.to_string(),
                    payload_schema: "trigger/1".to_string(),
                    external: true,
                }],
                outputs: vec![DServArtifactPortV1 {
                    port: OUTPUT_PORT.to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                }],
            },
            &source_module,
        ),
    );
    declare_artifact(
        &dserver_url,
        &artifact(
            DEST_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: INPUT_PORT.to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                }],
                outputs: Vec::new(),
            },
            &dest_module,
        ),
    );

    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(SOURCE_SVC, SOURCE_NODE),
    );
    declare_service_artifact(&dserver_url, &minimal_service_artifact(DEST_SVC, DEST_NODE));

    accept_a_new_proposal(&dserver_url, SOURCE_NODE, DEST_NODE);

    // Node B (cloud-01, the DESTINATION): its own runtime root, its own
    // local D-Node identity, and its own installed D-Code module.
    let node_b_root = tmp.join("node-cloud-01");
    std::fs::create_dir_all(&node_b_root).unwrap();
    init_local_identity(&node_b_root, DEST_NODE).unwrap();
    let dest_digest = sha256_hex(&dest_module);
    DCodeModuleStore::new(&node_b_root)
        .install(&dest_module, &dest_digest)
        .unwrap();

    let listen_port = free_port();
    let listen_bind = format!("127.0.0.1:{listen_port}");
    let advertise_url = format!("http://127.0.0.1:{listen_port}");
    let listener_child = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcall-listen"))
        .args([
            "--dserver-url",
            &dserver_url,
            "--dnode-root",
            node_b_root.to_str().unwrap(),
            "--bind",
            &listen_bind,
            "--advertise-url",
            &advertise_url,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real smartsentinel-dcall-listen");
    let _listener_guard = KillOnDrop(listener_child);
    wait_for_port(listen_port, Duration::from_secs(10));
    // The listener registers its own endpoint with DServer on startup;
    // give that PUT a moment to land before the source side tries to
    // resolve a remote route to it.
    let endpoint_deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = http()
            .get(format!(
                "{dserver_url}/api/v1/dcall/endpoints/{DEST_NODE}/http"
            ))
            .send();
        if let Ok(response) = response {
            if response.status().is_success() {
                break;
            }
        }
        if Instant::now() > endpoint_deadline {
            panic!("cloud-01's D-Call transport endpoint was never registered with DServer");
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let delivery_proxy = RecordingProxy::new(advertise_url.clone());
    datum::dcall::client::declare_endpoint(&dserver_url, DEST_NODE, "http", &delivery_proxy.url)
        .unwrap();

    // Node A (fog-01, the SOURCE): its own runtime root, local identity, and
    // installed source module -- the source's own D-Code is REALLY invoked
    // below (I1E), so this store must actually have it.
    let node_a_root = tmp.join("node-fog-01");
    std::fs::create_dir_all(&node_a_root).unwrap();
    init_local_identity(&node_a_root, SOURCE_NODE).unwrap();
    let source_digest = sha256_hex(&source_module);
    DCodeModuleStore::new(&node_a_root)
        .install(&source_module, &source_digest)
        .unwrap();

    let worker_binary = PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"));
    let (source_result, emit_index) =
        run_real_source_invocation(&dserver_url, &node_a_root, &worker_binary, SOURCE_NODE);
    let emitted_payload = source_result.validated_emits()[emit_index]
        .payload()
        .to_vec();
    assert_eq!(
        emitted_payload, EMITTED_PAYLOAD,
        "the dispatched bytes must be the REAL emit the source module actually produced"
    );
    let source_correlation_id = source_result.evidence().correlation_id.clone();

    let context = SourceExecutionContext {
        dserver_url: dserver_url.clone(),
        dnode_root: node_a_root.to_string_lossy().to_string(),
    };
    // Independent review fix v3 (J1B): the production/test seam now takes
    // the opaque `source_result` plus an index into its OWN
    // `.validated_emits()` -- there is no longer a separate raw payload
    // argument that could diverge from what the source invocation actually
    // produced.
    let results = dispatch_validated_emit(&context, &source_result, emit_index);

    assert_eq!(results.len(), 1, "expected exactly one D-Call edge to resolve for {SOURCE_SVC}.{OUTPUT_PORT}, got: {results:?}");
    let evidence = &results[0];
    assert!(
        matches!(evidence.outcome, DCallDeliveryOutcome::Success),
        "delivery did not succeed: {evidence:?}"
    );
    assert_eq!(evidence.route_kind, "remote");
    assert_eq!(evidence.destination.service_id, DEST_SVC);
    assert_eq!(evidence.destination.node_id, DEST_NODE);
    assert_eq!(evidence.source.node_id, SOURCE_NODE);
    assert_eq!(evidence.call_id, CALL_ID);
    assert_eq!(evidence.payload_sha256, sha256_hex(&emitted_payload));
    // I2: the D-Call layer's own correlation_id must equal the SOURCE
    // invocation's own host-generated correlation_id (propagated across the
    // whole logical call chain, never independently re-minted) --
    // `dispatch_one_call`'s internal `validate_destination_dcode_evidence_against_grant`
    // additionally proves (unit-tested directly) that the DESTINATION's own
    // DCodeInvocationEvidenceV1.message_id/.correlation_id exactly equal the
    // grant's -- this integration test proves the chain starts from a real
    // source correlation_id, not a literal.
    assert_eq!(evidence.correlation_id, source_correlation_id, "D-Call evidence correlation_id must equal the source invocation's own host-generated correlation_id");
    assert!(!evidence.message_id.is_empty());
    assert!(
        evidence.destination_dcode_evidence_sha256.is_some(),
        "expected a bound destination D-Code evidence digest"
    );
    // Section 9/12: the DCall evidence's own source-evidence digest
    // reference must equal a LOCALLY recomputed digest of the exact real
    // source evidence this test itself has in hand -- not merely "some
    // digest was present" -- proving the forensic chain source-evidence ->
    // grant -> DCall-evidence genuinely round-trips the real value end to
    // end, through a real HTTP grant request/response, not just in-process.
    let recomputed_source_digest =
        datum::dcode::evidence::dcode_evidence_sha256(source_result.evidence());
    assert_eq!(
        evidence.source_dcode_evidence_sha256.as_deref(),
        Some(recomputed_source_digest.as_str()),
        "DCall evidence's source_dcode_evidence_sha256 must equal a locally recomputed digest of the real source invocation evidence"
    );
    assert!(evidence.error_code.is_none());
    let stored = persisted_grants(&tmp);
    let grant = &stored["state"]["grants"][&evidence.grant_id]["grant"];
    assert_eq!(
        grant["source_dcode_evidence_sha256"],
        recomputed_source_digest
    );
    assert_eq!(grant["message_id"], evidence.message_id);
    assert_eq!(grant["correlation_id"], source_correlation_id);
    let exchanges = delivery_proxy.records();
    let wire = exchanges
        .iter()
        .find(|x| x.path == datum::dcall::listener::DELIVER_PATH)
        .unwrap();
    assert_eq!(wire.status, 200);
    let destination: datum::dcode::evidence::DCodeInvocationEvidenceV1 =
        serde_json::from_value(wire.response["dcode_evidence"].clone()).unwrap();
    assert_eq!(
        evidence.destination_dcode_evidence_sha256.as_deref(),
        Some(datum::dcode::evidence::dcode_evidence_sha256(&destination).as_str())
    );
    assert_eq!(destination.message_id, evidence.message_id);
    assert_eq!(destination.correlation_id, source_correlation_id);
    assert!(!serde_json::to_string(&stored)
        .unwrap()
        .contains(std::str::from_utf8(EMITTED_PAYLOAD).unwrap()));
}

/// Section 21 ("LOCAL VS REMOTE SEMANTIC EQUIVALENCE"): the same real
/// DServer + real D-Code worker path, but with source and destination
/// placed on the SAME logical D-Node -- proving `route_kind=local` still
/// goes through real grant issuance/redemption (never bypassed) and still
/// produces the same evidence shape as the cross-node test above, just
/// without an HTTP hop or a second listener process.
#[test]
#[ignore = "spawns a real DServer subprocess; run explicitly via `cargo test --locked --tests -- --test-threads=1 --ignored dcall_two_node`"]
fn real_same_node_dcall_delivery_end_to_end() {
    let tmp =
        std::env::temp_dir().join(format!("datum-dcall-same-node-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&tmp).unwrap();

    let (dserver_url, _dserver_guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(SOURCE_NODE));

    let source_module = source_module_with_emit();
    let dest_module = respond_ok_wat(r#"{"status":"ok","reason":null,"emits":[]}"#);
    declare_artifact(
        &dserver_url,
        &artifact(
            SOURCE_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: TRIGGER_PORT.to_string(),
                    payload_schema: "trigger/1".to_string(),
                    external: true,
                }],
                outputs: vec![DServArtifactPortV1 {
                    port: OUTPUT_PORT.to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                }],
            },
            &source_module,
        ),
    );
    declare_artifact(
        &dserver_url,
        &artifact(
            DEST_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: INPUT_PORT.to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                }],
                outputs: Vec::new(),
            },
            &dest_module,
        ),
    );
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(SOURCE_SVC, SOURCE_NODE),
    );
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(DEST_SVC, SOURCE_NODE),
    );

    accept_a_new_proposal(&dserver_url, SOURCE_NODE, SOURCE_NODE);

    let node_root = tmp.join("node-fog-01");
    std::fs::create_dir_all(&node_root).unwrap();
    init_local_identity(&node_root, SOURCE_NODE).unwrap();
    let dest_digest = sha256_hex(&dest_module);
    DCodeModuleStore::new(&node_root)
        .install(&dest_module, &dest_digest)
        .unwrap();
    let source_digest = sha256_hex(&source_module);
    DCodeModuleStore::new(&node_root)
        .install(&source_module, &source_digest)
        .unwrap();

    let worker_binary = PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"));
    let (source_result, emit_index) =
        run_real_source_invocation(&dserver_url, &node_root, &worker_binary, SOURCE_NODE);
    let emitted_payload = source_result.validated_emits()[emit_index]
        .payload()
        .to_vec();
    assert_eq!(emitted_payload, EMITTED_PAYLOAD);
    let source_correlation_id = source_result.evidence().correlation_id.clone();

    let context = SourceExecutionContext {
        dserver_url: dserver_url.clone(),
        dnode_root: node_root.to_string_lossy().to_string(),
    };
    let results = dispatch_validated_emit(&context, &source_result, emit_index);

    assert_eq!(results.len(), 1);
    let evidence = &results[0];
    assert!(
        matches!(evidence.outcome, DCallDeliveryOutcome::Success),
        "delivery did not succeed: {evidence:?}"
    );
    assert_eq!(evidence.route_kind, "local");
    assert_eq!(evidence.destination.node_id, SOURCE_NODE);
    assert_eq!(evidence.source.node_id, SOURCE_NODE);
    assert_eq!(evidence.correlation_id, source_correlation_id);
    assert!(evidence.destination_dcode_evidence_sha256.is_some());
    let recomputed_source_digest =
        datum::dcode::evidence::dcode_evidence_sha256(source_result.evidence());
    assert_eq!(
        evidence.source_dcode_evidence_sha256.as_deref(),
        Some(recomputed_source_digest.as_str())
    );
    assert_real_local_evidence_chain(&dserver_url, &node_root, &source_result);
}

/// Self-review finding (see `dcall-payload-leakage-audit.txt`): the
/// listener recomputes SHA-256 of the actually-decoded `payload_b64` bytes
/// and compares it to the request's own DECLARED `payload_sha256` BEFORE
/// ever spending a grant redemption on it -- without this, a request could
/// declare whatever digest a real grant expects while carrying DIFFERENT
/// actual bytes, passing redemption but executing on tampered content. This
/// check runs before any DServer network call, so it needs no real DServer
/// at all -- only a running listener.
#[test]
#[ignore = "spawns a real smartsentinel-dcall-listen subprocess; run explicitly via `cargo test --locked --tests -- --test-threads=1 --ignored dcall_two_node`"]
fn listener_rejects_a_payload_that_does_not_match_its_declared_digest() {
    let tmp = std::env::temp_dir().join(format!(
        "datum-dcall-payload-mismatch-it-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    init_local_identity(&tmp, DEST_NODE).unwrap();

    let listen_port = free_port();
    let listener_child = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcall-listen"))
        .args([
            "--dserver-url",
            "http://127.0.0.1:1",
            "--dnode-root",
            tmp.to_str().unwrap(),
            "--bind",
            &format!("127.0.0.1:{listen_port}"),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn real smartsentinel-dcall-listen");
    let _listener_guard = KillOnDrop(listener_child);
    wait_for_port(listen_port, Duration::from_secs(10));

    use base64::Engine;
    let actual_payload = b"the real bytes actually being sent";
    let request = serde_json::json!({
        "grant_id": "grant-does-not-matter-here",
        "token": "token-does-not-matter-here",
        "payload_b64": base64::engine::general_purpose::STANDARD.encode(actual_payload),
        // Deliberately a digest for DIFFERENT bytes than `actual_payload`.
        "payload_sha256": sha256_hex(b"a completely different payload"),
        "payload_size": actual_payload.len() as u64,
    });
    let response = http()
        .post(format!(
            "http://127.0.0.1:{listen_port}/datum-dnode/v1/dcall/deliver"
        ))
        .json(&request)
        .send()
        .unwrap();
    assert!(
        response.status().is_success(),
        "listener should respond 200 with a JSON failure body, not an HTTP error status"
    );
    let body: serde_json::Value = response.json().unwrap();
    assert_eq!(body["outcome"], "failed");
    assert_eq!(body["error_code"], "dcall_payload_digest_mismatch");
    assert!(
        body["dcode_evidence"].is_null(),
        "no execution should have occurred, and none did -- no evidence should be returned"
    );
}

/// v6, section 2 -- the K2 stale-source/new-topology race, proven for real
/// against a real DServer subprocess (NOT a direct unit call to
/// `check_source_execution_binding`). Flow: a real source D-Code invocation
/// executes under deployment A (D-Graph revision 1); a SECOND real deployment
/// is then accepted for the same application with a bumped D-Graph revision
/// (a genuine, legitimate authority change -- not a fabricated state edit);
/// the OLD `GovernedDCodeInvocationResult` (still carrying deployment A's
/// D-Graph revision/digest in its own evidence) is then used to request a
/// D-Call grant. The grant request must be rejected with
/// `dcall_source_execution_stale` -- proven via the real HTTP response body,
/// not merely a generic failure -- and the higher-level dispatch seam must
/// report the delivery as failed with no destination execution ever
/// occurring. Read-only SQLite current/history checks prove no grant was
/// inserted. Installed destination modules, zero destination authorization
/// requests, and (for the DMap race) a real observed destination listener
/// prove rejection occurs before destination execution can begin.
#[test]
#[ignore = "spawns a real DServer subprocess; run explicitly via `cargo test --locked --tests -- --test-threads=1 --ignored dcall_two_node`"]
fn stale_source_authority_after_dgraph_revision_change_is_rejected_before_grant_issuance() {
    run_stale_source_proof(true);
}

#[test]
#[ignore = "real DServer and governed isolated-worker source execution"]
fn stale_source_authority_after_dmap_placement_change_is_rejected_before_grant_issuance() {
    run_stale_source_proof(false);
}

fn run_stale_source_proof(change_graph: bool) {
    let tmp = std::env::temp_dir().join(format!(
        "datum-dcall-stale-source-it-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    let (dserver_url, _dserver_guard) = spawn_real_dserver(&tmp);

    // Same-node (local route) deliberately: `check_source_execution_binding`
    // runs BEFORE the remote-endpoint lookup in `post_request_grant` (see
    // dserver's dcall.rs), so this race is fully provable without a second
    // listener process -- the grant request itself is still a real HTTP
    // round trip to a real DServer, which is what "not a direct unit call"
    // requires.
    declare_node(&dserver_url, &node(SOURCE_NODE));
    declare_node(&dserver_url, &node(DEST_NODE));

    let source_module = source_module_with_emit();
    declare_artifact(
        &dserver_url,
        &artifact(
            SOURCE_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: TRIGGER_PORT.to_string(),
                    payload_schema: "trigger/1".to_string(),
                    external: true,
                }],
                outputs: vec![DServArtifactPortV1 {
                    port: OUTPUT_PORT.to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                }],
            },
            &source_module,
        ),
    );
    // Register and install a genuinely executable destination. Execution
    // absence is established independently, not inferred from missing bytes.
    let dest_module = respond_ok_wat(r#"{"status":"ok","reason":null,"emits":[]}"#);
    declare_artifact(
        &dserver_url,
        &artifact(
            DEST_SVC,
            DServArtifactPortsV1 {
                inputs: vec![DServArtifactPortV1 {
                    port: INPUT_PORT.to_string(),
                    payload_schema: PAYLOAD_SCHEMA.to_string(),
                    external: false,
                }],
                outputs: Vec::new(),
            },
            &dest_module,
        ),
    );
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(SOURCE_SVC, SOURCE_NODE),
    );
    let mut eligibility = minimal_service_artifact(DEST_SVC, SOURCE_NODE);
    eligibility["control_plane"]["target_nodes"] = serde_json::json!([SOURCE_NODE, DEST_NODE]);
    declare_service_artifact(&dserver_url, &eligibility);

    // Deployment A: D-Graph revision 1.
    accept_a_new_proposal(&dserver_url, SOURCE_NODE, SOURCE_NODE);

    let node_root = tmp.join("node-fog-01");
    std::fs::create_dir_all(&node_root).unwrap();
    init_local_identity(&node_root, SOURCE_NODE).unwrap();
    DCodeModuleStore::new(&node_root)
        .install(&dest_module, &sha256_hex(&dest_module))
        .unwrap();
    let source_digest = sha256_hex(&source_module);
    DCodeModuleStore::new(&node_root)
        .install(&source_module, &source_digest)
        .unwrap();

    let worker_binary = PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"));

    // Real source invocation executes under deployment A. Its resulting
    // `GovernedDCodeInvocationResult` carries deployment A's D-Graph
    // revision/digest inside its own evidence -- this is the "old" state
    // that becomes stale the moment deployment B is accepted below.
    let (source_result, emit_index) =
        run_real_source_invocation(&dserver_url, &node_root, &worker_binary, SOURCE_NODE);
    let old_dgraph_revision = source_result.evidence().dgraph_revision;
    assert_eq!(
        old_dgraph_revision, 1,
        "sanity check: source must have executed under D-Graph revision 1"
    );

    // For the DMap race, B has a fully provisioned executable destination
    // and a real listener. Rejection cannot be explained by missing modules
    // or transport. Its observer must see zero delivery requests.
    let mut destination_listener = None;
    let mut destination_observer = None;
    if !change_graph {
        let destination_root = tmp.join("node-cloud-01");
        init_local_identity(&destination_root, DEST_NODE).unwrap();
        DCodeModuleStore::new(&destination_root)
            .install(&dest_module, &sha256_hex(&dest_module))
            .unwrap();
        let port = free_port();
        let bind = format!("127.0.0.1:{port}");
        let url = format!("http://{bind}");
        let child = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcall-listen"))
            .args([
                "--dserver-url",
                &dserver_url,
                "--dnode-root",
                destination_root.to_str().unwrap(),
                "--bind",
                &bind,
                "--advertise-url",
                &url,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        destination_listener = Some(KillOnDrop(child));
        wait_for_port(port, Duration::from_secs(10));
        let deadline = Instant::now() + Duration::from_secs(10);
        while datum::dcall::client::fetch_endpoint(&dserver_url, DEST_NODE, "http").is_err() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
        let observer = RecordingProxy::new(url);
        datum::dcall::client::declare_endpoint(&dserver_url, DEST_NODE, "http", &observer.url)
            .unwrap();
        destination_observer = Some(observer);
    }

    // Independent field mismatches exercise the real issuance handler, not
    // merely its comparator. Inspect both SQLite current state and history
    // after every rejection to prove insertion never happened.
    let original = binding_json(source_result.evidence());
    for field in [
        "service_id",
        "node_id",
        "dgraph_id",
        "dgraph_revision",
        "dgraph_digest",
        "dmap_id",
        "dmap_revision",
        "dmap_digest",
        "dserv_artifact_id",
        "dserv_artifact_digest",
        "dserv_artifact_descriptor_digest",
    ] {
        let mut binding = original.clone();
        binding[field] = if field.ends_with("revision") {
            serde_json::json!(99)
        } else if field == "dserv_artifact_digest" {
            serde_json::json!("9".repeat(64))
        } else if field.ends_with("digest") {
            serde_json::json!(format!("sha256:{}", "9".repeat(64)))
        } else {
            serde_json::json!("independent-mismatch")
        };
        let response = http()
            .post(format!("{dserver_url}/api/v1/dcall/grants"))
            .json(&grant_request_json(source_result.evidence(), binding, 4))
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 422, "field {field}");
        let body: serde_json::Value = response.json().unwrap();
        assert_eq!(
            body["error"], "dcall_source_execution_stale",
            "field {field}: {body}"
        );
        assert_no_grants(&tmp);
    }

    // Deployment B: a real, legitimate SECOND acceptance for the SAME
    // application with a bumped D-Graph revision -- a genuine authority
    // change, not a fabricated/corrupted state edit.
    let mut new_dgraph = dgraph();
    if change_graph {
        new_dgraph.revision = 2;
    }
    accept_a_new_proposal_with_dgraph(
        &dserver_url,
        SOURCE_NODE,
        if change_graph { SOURCE_NODE } else { DEST_NODE },
        new_dgraph,
    );
    let current = datum::dcode::client::fetch_dcode_authorization(
        &dserver_url,
        PROJECT,
        APP,
        SOURCE_SVC,
        SOURCE_NODE,
    )
    .unwrap();
    assert_ne!(
        current.dmap_ref.digest,
        source_result.evidence().dmap_digest
    );
    if !change_graph {
        assert_eq!(
            current.dgraph_ref.digest,
            source_result.evidence().dgraph_digest
        );
    }

    // Attempt to dispatch the OLD emit under the now-stale source authority.
    let context = SourceExecutionContext {
        dserver_url: dserver_url.clone(),
        dnode_root: node_root.to_string_lossy().to_string(),
    };
    let results = dispatch_validated_emit(&context, &source_result, emit_index);

    assert_eq!(
        results.len(),
        1,
        "expected exactly one D-Call edge to resolve, got: {results:?}"
    );
    let evidence = &results[0];
    assert!(matches!(evidence.outcome, DCallDeliveryOutcome::Failed), "delivery must be reported as failed, not success, once source authority is stale: {evidence:?}");
    assert!(
        evidence.grant_id.is_empty(),
        "no grant should ever have been created for a stale source execution"
    );
    assert_eq!(
        evidence.error_code.as_deref(),
        Some("dcall_source_execution_stale")
    );
    assert!(
        evidence.destination_dcode_evidence_sha256.is_none(),
        "destination D-Code must never have executed"
    );

    // Independently confirm the same safe deterministic server rejection via
    // the real HTTP client. v7 preserves this whitelisted code in dispatch
    // evidence and typed errors, while arbitrary server text remains hashed.
    let stale_binding = SourceExecutionBindingV1 {
        service_id: source_result.evidence().service_id.clone(),
        node_id: source_result.evidence().node_id.clone(),
        dgraph_id: source_result.evidence().dgraph_id.clone(),
        dgraph_revision: source_result.evidence().dgraph_revision,
        dgraph_digest: source_result.evidence().dgraph_digest.clone(),
        dmap_id: source_result.evidence().dmap_id.clone(),
        dmap_revision: source_result.evidence().dmap_revision,
        dmap_digest: source_result.evidence().dmap_digest.clone(),
        dserv_artifact_id: source_result.evidence().dserv_artifact_id.clone(),
        dserv_artifact_digest: source_result.evidence().dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: source_result
            .evidence()
            .dserv_artifact_descriptor_digest
            .clone(),
    };
    let source_dcode_evidence_sha256 =
        datum::dcode::evidence::dcode_evidence_sha256(source_result.evidence());
    let raw_result = datum::dcall::client::request_grant(
        &dserver_url,
        &source_result.evidence().project_id,
        &source_result.evidence().application_id,
        CALL_ID,
        SOURCE_NODE,
        OUTPUT_PORT,
        "msg-stale-source-test",
        &source_result.evidence().correlation_id,
        &sha256_hex(EMITTED_PAYLOAD),
        EMITTED_PAYLOAD.len() as u64,
        &stale_binding,
        &source_dcode_evidence_sha256,
    );
    let error = raw_result.expect_err("a grant request carrying deployment A's stale D-Graph authority must be rejected now that deployment B is active");
    let message = error.to_string();
    assert!(message.contains("dcall_source_execution_stale"), "expected the exact stable rejection code 'dcall_source_execution_stale' in the real DServer response, got: {message}");
    assert!(
        message.contains("422") || message.to_lowercase().contains("unprocessable"),
        "expected a 422 Unprocessable Entity response, got: {message}"
    );
    assert_no_grants(&tmp);
    assert!(!_dserver_guard.proxy.records().iter().any(|x| x.path == "/api/v1/dcode/authorize" && x.request["service_id"] == DEST_SVC), "no destination authorization request means no production destination execution was attempted");
    if let Some(observer) = &destination_observer {
        assert!(
            observer.records().is_empty(),
            "stale emit must never reach the real destination listener"
        );
    }
    drop(destination_observer);
    drop(destination_listener);
}

/// v6, section 4 -- the source/destination `max_message_bytes` preflight
/// checks in `post_request_grant` (`dcall_payload_exceeds_source_message_limit`
/// / `dcall_payload_exceeds_destination_message_limit`) have existed since
/// an earlier review round but had ZERO automated coverage until now. Proves,
/// against a real DServer, that: (1) a payload exceeding the DESTINATION
/// artifact's limit is rejected with the destination code (checked first);
/// (2) a payload exceeding only the SOURCE artifact's (lower) limit, while
/// still under the destination's, is rejected with the source code; (3) a
/// payload sitting exactly at the boundary is accepted (issuance succeeds,
/// a real grant_id/token comes back).
#[test]
#[ignore = "spawns a real DServer subprocess; run explicitly via `cargo test --locked --tests -- --test-threads=1 --ignored dcall_two_node`"]
fn message_limit_preflight_checks_reject_oversized_payloads_before_grant_issuance() {
    let tmp = std::env::temp_dir().join(format!(
        "datum-dcall-message-limit-it-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    let (dserver_url, _dserver_guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(SOURCE_NODE));

    let source_module = source_module_with_emit();
    let dest_module = respond_ok_wat(r#"{"status":"ok","reason":null,"emits":[]}"#);

    // Destination limit deliberately LOW (10 bytes); source limit deliberately
    // HIGH (4096, the artifact() default) -- so a well-chosen payload size
    // exercises exactly one check at a time.
    let mut dest_artifact = artifact(
        DEST_SVC,
        DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: INPUT_PORT.to_string(),
                payload_schema: PAYLOAD_SCHEMA.to_string(),
                external: false,
            }],
            outputs: Vec::new(),
        },
        &dest_module,
    );
    dest_artifact.limits.max_message_bytes = 10;
    let source_artifact_for_dest_test = artifact(
        SOURCE_SVC,
        DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: TRIGGER_PORT.to_string(),
                payload_schema: "trigger/1".to_string(),
                external: true,
            }],
            outputs: vec![DServArtifactPortV1 {
                port: OUTPUT_PORT.to_string(),
                payload_schema: PAYLOAD_SCHEMA.to_string(),
                external: false,
            }],
        },
        &source_module,
    );
    declare_artifact(&dserver_url, &source_artifact_for_dest_test);
    declare_artifact(&dserver_url, &dest_artifact);
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(SOURCE_SVC, SOURCE_NODE),
    );
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(DEST_SVC, SOURCE_NODE),
    );
    accept_a_new_proposal(&dserver_url, SOURCE_NODE, SOURCE_NODE);

    let node_root = tmp.join("node-fog-01");
    std::fs::create_dir_all(&node_root).unwrap();
    init_local_identity(&node_root, SOURCE_NODE).unwrap();
    DCodeModuleStore::new(&node_root)
        .install(&source_module, &sha256_hex(&source_module))
        .unwrap();
    let worker_binary = PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"));
    let (source_result, _emit_index) =
        run_real_source_invocation(&dserver_url, &node_root, &worker_binary, SOURCE_NODE);

    let binding = SourceExecutionBindingV1 {
        service_id: source_result.evidence().service_id.clone(),
        node_id: source_result.evidence().node_id.clone(),
        dgraph_id: source_result.evidence().dgraph_id.clone(),
        dgraph_revision: source_result.evidence().dgraph_revision,
        dgraph_digest: source_result.evidence().dgraph_digest.clone(),
        dmap_id: source_result.evidence().dmap_id.clone(),
        dmap_revision: source_result.evidence().dmap_revision,
        dmap_digest: source_result.evidence().dmap_digest.clone(),
        dserv_artifact_id: source_result.evidence().dserv_artifact_id.clone(),
        dserv_artifact_digest: source_result.evidence().dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: source_result
            .evidence()
            .dserv_artifact_descriptor_digest
            .clone(),
    };
    let source_dcode_evidence_sha256 =
        datum::dcode::evidence::dcode_evidence_sha256(source_result.evidence());
    let placeholder_payload_sha256 = "a".repeat(64);

    let request = |payload_size: u64, message_id: &str| {
        datum::dcall::client::request_grant(
            &dserver_url,
            &source_result.evidence().project_id,
            &source_result.evidence().application_id,
            CALL_ID,
            SOURCE_NODE,
            OUTPUT_PORT,
            message_id,
            &source_result.evidence().correlation_id,
            &placeholder_payload_sha256,
            payload_size,
            &binding,
            &source_dcode_evidence_sha256,
        )
    };

    // (1) 11 bytes exceeds the destination's 10-byte limit -- rejected with
    // the DESTINATION code, checked before the (much higher) source limit.
    let destination_error = request(11, "msg-dest-limit")
        .expect_err("must be rejected: exceeds destination max_message_bytes");
    let destination_message = destination_error.to_string();
    assert!(
        destination_message.contains("dcall_payload_exceeds_destination_message_limit"),
        "got: {destination_message}"
    );
    assert!(
        !destination_message.contains("dcall_payload_exceeds_source_message_limit"),
        "destination's lower limit must be checked first, got: {destination_message}"
    );
    assert_no_grants(&tmp);

    // (2) Exactly at the destination's boundary (10 bytes) succeeds this
    // preflight -- proves this is a genuine `>` bound, not an off-by-one `>=`.
    let boundary_result = request(10, "msg-boundary")
        .expect("payload_size exactly at the limit must pass this preflight check");
    assert!(
        !boundary_result.grant.grant_id.is_empty(),
        "a real grant must have been issued for the boundary-exact payload size"
    );
    assert!(!boundary_result.token.is_empty());

    // (3) No grant/redeemable-state leak from the rejected oversized request:
    // the boundary-accepted grant above is real and redeemable, but the
    // oversized request's grant_id was never returned to us at all (we hold
    // no such id), which is the direct, structural consequence of rejection
    // happening before `issue_grant` runs -- there is nothing further to
    // query, since the client received no identifier for a non-existent grant.
}

/// v6, section 4 (source-limit variant): the DESTINATION check in
/// `post_request_grant` runs first, so proving the SOURCE-specific code
/// fires requires the source artifact's limit to be the one actually
/// exceeded while the destination's remains high enough to pass -- the
/// inverse configuration from the test above. A separate application/call
/// identity is used (rather than re-declaring the shared fixtures' artifacts
/// with different limits) because the D-Code artifact registry is
/// immutable-by-digest per `(application_id, dserv_id)`.
#[test]
#[ignore = "spawns a real DServer subprocess; run explicitly via `cargo test --locked --tests -- --test-threads=1 --ignored dcall_two_node`"]
fn source_message_limit_preflight_check_rejects_oversized_payload_before_grant_issuance() {
    const LIMIT_APP: &str = "app:pipeline-source-limit";
    const LIMIT_CALL: &str = "sensor-to-decoder-source-limit";

    let tmp = std::env::temp_dir().join(format!(
        "datum-dcall-source-limit-it-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let (dserver_url, _dserver_guard) = spawn_real_dserver(&tmp);

    declare_node(&dserver_url, &node(SOURCE_NODE));

    let source_module = source_module_with_emit();
    let dest_module = respond_ok_wat(r#"{"status":"ok","reason":null,"emits":[]}"#);

    let mut source_artifact = artifact(
        SOURCE_SVC,
        DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: TRIGGER_PORT.to_string(),
                payload_schema: "trigger/1".to_string(),
                external: true,
            }],
            outputs: vec![DServArtifactPortV1 {
                port: OUTPUT_PORT.to_string(),
                payload_schema: PAYLOAD_SCHEMA.to_string(),
                external: false,
            }],
        },
        &source_module,
    );
    source_artifact.application_id = LIMIT_APP.to_string();
    source_artifact.artifact_id = format!("dserv:pipeline-source-limit:{SOURCE_SVC}");
    source_artifact.limits.max_message_bytes = 1000; // deliberately LOW relative to destination's 4096, but comfortably above the real emitted-envelope size
    let mut dest_artifact = artifact(
        DEST_SVC,
        DServArtifactPortsV1 {
            inputs: vec![DServArtifactPortV1 {
                port: INPUT_PORT.to_string(),
                payload_schema: PAYLOAD_SCHEMA.to_string(),
                external: false,
            }],
            outputs: Vec::new(),
        },
        &dest_module,
    );
    dest_artifact.application_id = LIMIT_APP.to_string();
    dest_artifact.artifact_id = format!("dserv:pipeline-source-limit:{DEST_SVC}");
    // destination limit left at the artifact()-default 4096, deliberately HIGH.
    declare_artifact(&dserver_url, &source_artifact);
    declare_artifact(&dserver_url, &dest_artifact);

    let dgraph = CanonicalDGraphV1 {
        schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
        application_id: LIMIT_APP.to_string(),
        dgraph_id: "dgraph:pipeline-source-limit".to_string(),
        revision: 1,
        services: vec![
            CanonicalDServV1 {
                service_id: SOURCE_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 1,
                },
            },
            CanonicalDServV1 {
                service_id: DEST_SVC.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores: 1,
                    memory_bytes: 1,
                },
            },
        ],
        calls: vec![CanonicalDCallV1 {
            call_id: LIMIT_CALL.to_string(),
            source_service_id: SOURCE_SVC.to_string(),
            target_service_id: DEST_SVC.to_string(),
        }],
    };
    let payload = serde_json::json!({
        "application_id": LIMIT_APP,
        "source": "operator",
        "dgraph": dgraph,
        "dcontinuum": placeholder_dcontinuum(&[SOURCE_NODE]),
        "placements": [
            CanonicalDServPlacementV1 { service_id: SOURCE_SVC.to_string(), node_id: SOURCE_NODE.to_string() },
            CanonicalDServPlacementV1 { service_id: DEST_SVC.to_string(), node_id: SOURCE_NODE.to_string() },
        ],
    });
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(SOURCE_SVC, SOURCE_NODE),
    );
    declare_service_artifact(
        &dserver_url,
        &minimal_service_artifact(DEST_SVC, SOURCE_NODE),
    );
    let response = http()
        .post(format!("{dserver_url}/api/v1/ddeploy/proposals"))
        .header("X-Datum-Project", PROJECT)
        .json(&payload)
        .send()
        .unwrap();
    assert!(
        response.status().is_success(),
        "create_proposal failed: {} {}",
        response.status(),
        response.text().unwrap_or_default()
    );
    let body: serde_json::Value = serde_json::from_str(&response.text().unwrap()).unwrap();
    let proposal_id = body["proposal"]["proposal_id"]
        .as_str()
        .unwrap()
        .to_string();
    let accept = http()
        .post(format!(
            "{dserver_url}/api/v1/ddeploy/proposals/{proposal_id}/accept"
        ))
        .header("X-Datum-Project", PROJECT)
        .json(&serde_json::json!({"accepted_by": "test"}))
        .send()
        .unwrap();
    assert!(
        accept.status().is_success(),
        "accept_proposal failed: {} {}",
        accept.status(),
        accept.text().unwrap_or_default()
    );

    let node_root = tmp.join("node-fog-01");
    std::fs::create_dir_all(&node_root).unwrap();
    init_local_identity(&node_root, SOURCE_NODE).unwrap();
    DCodeModuleStore::new(&node_root)
        .install(&source_module, &sha256_hex(&source_module))
        .unwrap();
    let authorization: DCodeInvocationAuthorizationV1 =
        datum::dcode::client::fetch_dcode_authorization(
            &dserver_url,
            PROJECT,
            LIMIT_APP,
            SOURCE_SVC,
            SOURCE_NODE,
        )
        .expect("fetch source authorization");
    let real_artifact: CanonicalDServArtifactV1 =
        datum::dcode::client::fetch_dserv_artifact(&dserver_url, LIMIT_APP, SOURCE_SVC)
            .expect("fetch source artifact");
    let module_store = DCodeModuleStore::new(&node_root);
    let source_result = invoke_dcode_governed_capturing_emits(
        &module_store,
        &authorization,
        &real_artifact,
        &node_root,
        TRIGGER_PORT,
        b"trigger-input-ignored",
    )
    .unwrap();
    assert_eq!(
        source_result.evidence().outcome,
        datum::dcode::evidence::DCodeInvocationOutcome::Success
    );

    let binding = SourceExecutionBindingV1 {
        service_id: source_result.evidence().service_id.clone(),
        node_id: source_result.evidence().node_id.clone(),
        dgraph_id: source_result.evidence().dgraph_id.clone(),
        dgraph_revision: source_result.evidence().dgraph_revision,
        dgraph_digest: source_result.evidence().dgraph_digest.clone(),
        dmap_id: source_result.evidence().dmap_id.clone(),
        dmap_revision: source_result.evidence().dmap_revision,
        dmap_digest: source_result.evidence().dmap_digest.clone(),
        dserv_artifact_id: source_result.evidence().dserv_artifact_id.clone(),
        dserv_artifact_digest: source_result.evidence().dserv_artifact_digest.clone(),
        dserv_artifact_descriptor_digest: source_result
            .evidence()
            .dserv_artifact_descriptor_digest
            .clone(),
    };
    let source_dcode_evidence_sha256 =
        datum::dcode::evidence::dcode_evidence_sha256(source_result.evidence());

    // 1001 bytes is under the destination's 4096 limit but over the
    // source's 1000-byte limit -- must be rejected with the SOURCE code
    // specifically. This is a synthetic `payload_size` claim in the grant
    // request itself, independent of the real (much smaller) emitted
    // payload the source invocation above actually produced -- the DCall
    // preflight check validates the caller-declared size, not a recomputed
    // one (integrity of the declared value against actual bytes is a
    // REDEMPTION-time concern, proven separately by the payload-digest-
    // mismatch test).
    let error = datum::dcall::client::request_grant(
        &dserver_url,
        PROJECT,
        LIMIT_APP,
        LIMIT_CALL,
        SOURCE_NODE,
        OUTPUT_PORT,
        "msg-source-limit",
        &source_result.evidence().correlation_id,
        &"a".repeat(64),
        1001,
        &binding,
        &source_dcode_evidence_sha256,
    )
    .expect_err("must be rejected: exceeds source max_message_bytes even though destination's limit is not exceeded");
    let message = error.to_string();
    assert!(
        message.contains("dcall_payload_exceeds_source_message_limit"),
        "got: {message}"
    );
    assert_no_grants(&tmp);
    let boundary = datum::dcall::client::request_grant(
        &dserver_url,
        PROJECT,
        LIMIT_APP,
        LIMIT_CALL,
        SOURCE_NODE,
        OUTPUT_PORT,
        "source-boundary",
        &source_result.evidence().correlation_id,
        &"a".repeat(64),
        1000,
        &binding,
        &source_dcode_evidence_sha256,
    )
    .unwrap();
    assert!(!boundary.grant.grant_id.is_empty());
    assert_eq!(
        persisted_grants(&tmp)["state"]["grants"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
}
