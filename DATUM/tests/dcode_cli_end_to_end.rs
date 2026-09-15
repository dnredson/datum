//! Real end-to-end proof (ADR-0021, 111.11) of the two D-Code CLI binaries:
//! compile a real WASM module, install it into a fresh local module store
//! via `smartsentinel-dcode-install`, then invoke it via
//! `smartsentinel-dcode-invoke offline-test-only` (the INTERNAL/TEST-ONLY
//! mode -- independent review v2, R1: this is explicitly NOT the governed
//! production path, which instead fetches authorization/artifact from a
//! live DServer via `smartsentinel-dcode-invoke invoke`), and verify the
//! real subprocess output including its exit code (R6). This is the
//! "minimal CLI for actual human/lab proof" the ADR-0021 vertical slice
//! requires -- not merely a unit test of the library functions the binaries
//! call.

use std::fs;
use std::process::Command;

use serde_json::json;
use uuid::Uuid;

/// Always returns a fixed, valid ADR-0003 output envelope
/// `{"status":"ok","emits":[]}` regardless of input -- a genuinely valid
/// envelope response, needed now that input/output flow through the R4
/// envelope rather than being echoed raw.
const RESPOND_OK_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (data (i32.const 0) "{\22status\22:\22ok\22,\22emits\22:[]}")
  (func (export "datum_abi_version") (result i32) (i32.const 0))
  (func (export "datum_alloc") (param $len i32) (result i32) (i32.const 4096))
  (func (export "datum_dealloc") (param $ptr i32) (param $len i32))
  (func (export "datum_handle") (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.const 0) (i64.const 32)) (i64.const 26))))
"#;

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn worker_binary_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_smartsentinel-dcode-worker"))
}

// -- Independent review v4 (G1/G2): governed-path node-identity and
// worker-selection hardening -------------------------------------------

/// G1C-1/G2B-1: the GOVERNED `invoke` subcommand must not accept a raw
/// `--node-id` or `--worker-binary` override at all -- clap itself must
/// reject them as unrecognized arguments, proving a caller cannot even
/// SYNTACTICALLY attempt to spoof either one through the governed path.
#[test]
fn governed_invoke_subcommand_rejects_raw_node_id_and_worker_binary_arguments() {
    for rejected_flag in ["--node-id", "--worker-binary"] {
        let output = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-invoke"))
            .arg("invoke")
            .arg("--dserver-url").arg("http://127.0.0.1:0")
            .arg("--project-id").arg("proj:lab")
            .arg("--application-id").arg("app:a")
            .arg("--service-id").arg("decode")
            .arg("--input-port").arg("uplink")
            .arg(rejected_flag).arg("node-1")
            .output()
            .expect("smartsentinel-dcode-invoke should run");
        assert!(
            !output.status.success(),
            "governed `invoke` must reject an explicit {rejected_flag} argument"
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("unexpected argument") || stderr.contains("unrecognized"),
            "expected a clap unrecognized-argument error for {rejected_flag}, got: {stderr}"
        );
    }
}

/// G1C-4 (real subprocess proof, not merely the library-level test in
/// `local_identity.rs`): re-initializing an existing local D-Node identity
/// with a DIFFERENT node_id through the real `smartsentinel-dcode-init`
/// binary fails closed, and re-initializing with the SAME node_id remains
/// idempotent.
#[test]
fn smartsentinel_dcode_init_is_idempotent_and_fails_closed_on_conflict() {
    let dnode_root = std::env::temp_dir().join(format!("datum-dcode-init-e2e-{}", Uuid::new_v4()));

    let first = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-init"))
        .arg("--node-id").arg("fog-01")
        .arg("--dnode-root").arg(&dnode_root)
        .output()
        .expect("smartsentinel-dcode-init should run");
    assert!(first.status.success(), "first init should succeed: {}", String::from_utf8_lossy(&first.stderr));
    assert!(String::from_utf8_lossy(&first.stdout).contains("node_id=fog-01"));

    let identical_reinit = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-init"))
        .arg("--node-id").arg("fog-01")
        .arg("--dnode-root").arg(&dnode_root)
        .output()
        .unwrap();
    assert!(identical_reinit.status.success(), "identical re-init must be idempotent");

    let conflicting_reinit = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-init"))
        .arg("--node-id").arg("cloud-01")
        .arg("--dnode-root").arg(&dnode_root)
        .output()
        .unwrap();
    assert!(!conflicting_reinit.status.success(), "conflicting re-init must fail closed");
    let stderr = String::from_utf8_lossy(&conflicting_reinit.stderr);
    assert!(stderr.to_ascii_lowercase().contains("conflicting"), "expected a conflicting-reinit error, got: {stderr}");

    let _ = fs::remove_dir_all(&dnode_root);
}

/// G1C-1/2/3 (partial, real subprocess proof): the governed path resolves
/// `local_node_id` from the local identity file BEFORE ever attempting to
/// contact DServer -- an uninitialized `--dnode-root` must fail closed with
/// a local-identity error, never a network error, proving the local check
/// runs first and a caller cannot bypass it by pointing at an unreachable
/// DServer URL.
#[test]
fn governed_invoke_without_prior_init_fails_closed_before_any_network_attempt() {
    let dnode_root = std::env::temp_dir().join(format!("datum-dcode-invoke-noinit-e2e-{}", Uuid::new_v4()));

    let output = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-invoke"))
        .arg("invoke")
        .arg("--dserver-url").arg("http://127.0.0.1:1") // deliberately unreachable
        .arg("--project-id").arg("proj:lab")
        .arg("--application-id").arg("app:a")
        .arg("--service-id").arg("decode")
        .arg("--input-port").arg("uplink")
        .arg("--dnode-root").arg(&dnode_root)
        .output()
        .expect("smartsentinel-dcode-invoke should run");

    assert!(!output.status.success(), "invoke against an uninitialized dnode-root must fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("resolve local D-Node identity"),
        "expected the local-identity resolution failure to occur BEFORE any network attempt, got: {stderr}"
    );

    let _ = fs::remove_dir_all(&dnode_root);
}

#[test]
fn install_then_invoke_real_wasm_module_via_the_cli_binaries() {
    let workdir = std::env::temp_dir().join(format!("datum-dcode-cli-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&workdir).unwrap();

    let module_bytes = wat::parse_str(RESPOND_OK_WAT).expect("test WAT should compile");
    let digest = sha256_hex(&module_bytes);
    let module_path = workdir.join("decode.wasm");
    fs::write(&module_path, &module_bytes).unwrap();

    let store_root = workdir.join("dcode-store");

    // 1. Install via the real CLI binary.
    let install_output = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-install"))
        .arg("--module-path")
        .arg(&module_path)
        .arg("--declared-sha256")
        .arg(&digest)
        .arg("--store-root")
        .arg(&store_root)
        .output()
        .expect("smartsentinel-dcode-install should run");
    assert!(install_output.status.success(), "install failed: {}", String::from_utf8_lossy(&install_output.stderr));
    let install_stdout = String::from_utf8_lossy(&install_output.stdout);
    assert!(install_stdout.contains("status=ok"));
    assert!(install_stdout.contains(&digest));
    assert!(store_root.join("dcode").join("sha256").join(format!("{digest}.wasm")).is_file());

    // 2. Write an authorization + full artifact file (standing in for what
    // DServer's /api/v1/dcode/authorize and /api/v1/dcode/artifacts/... would
    // return -- offline-test-only mode never contacts DServer).
    let artifact = json!({
        "schema": "datum.dserv-artifact/1",
        "artifact_id": "dserv:sensor-pipeline:decode",
        "version": "0.1.0",
        "application_id": "app:sensor-pipeline",
        "dserv_id": "decode",
        "dcode": {"uri": format!("datum-blob://sha256/{digest}"), "sha256": digest, "content_id": format!("sha256:{digest}"), "size_bytes": module_bytes.len()},
        "abi": {"name": "datum-dnode", "version": "0", "required_exports": ["memory","datum_abi_version","datum_alloc","datum_dealloc","datum_handle"], "allowed_imports": []},
        "ports": {"inputs": [{"port": "uplink", "payload_schema": "s", "external": true}], "outputs": [{"port": "reading", "payload_schema": "s", "external": false}]},
        "limits": {"fuel_per_invocation": 5_000_000, "timeout_ms": 2000, "max_memory_pages": 4, "max_message_bytes": 65536, "max_emits_per_invocation": 8},
        "safety": {"filesystem_access": false, "network_access": false, "host_command_access": false, "docker_access": false, "side_effects_allowed": false},
        "state_model": "stateless",
        "instantiation": "fresh_instance_per_invocation"
    });
    let artifact_path = workdir.join("artifact.json");
    fs::write(&artifact_path, serde_json::to_string_pretty(&artifact).unwrap()).unwrap();

    // Compute the real canonical descriptor digest via the DATUM-side
    // mirror, exactly as a real DServer authorization would bind it.
    let parsed_artifact: datum::dcode::dserv_artifact::CanonicalDServArtifactV1 =
        serde_json::from_value(artifact.clone()).unwrap();
    let descriptor_digest = datum::dcode::dserv_artifact::recompute_dserv_artifact_digest(&parsed_artifact).unwrap();

    let authorization = json!({
        "authorization_id": "dcode-auth:proj:app:decode:node-1:1",
        "project_id": "proj:lab",
        "application_id": "app:sensor-pipeline",
        "service_id": "decode",
        "node_id": "node-1",
        "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
        "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
        "dserv_artifact_id": "dserv:sensor-pipeline:decode",
        "dserv_artifact_digest": digest,
        "dserv_artifact_descriptor_digest": descriptor_digest,
        "dnode_abi": "datum-dnode/0"
    });
    let authorization_path = workdir.join("authorization.json");
    fs::write(&authorization_path, serde_json::to_string_pretty(&authorization).unwrap()).unwrap();

    let input_path = workdir.join("input.bin");
    fs::write(&input_path, b"real end-to-end proof").unwrap();
    let evidence_out = workdir.join("evidence.json");

    // 3. Invoke via the real CLI binary, offline-test-only mode.
    let invoke_output = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-invoke"))
        .arg("offline-test-only")
        .arg("--authorization-path")
        .arg(&authorization_path)
        .arg("--artifact-path")
        .arg(&artifact_path)
        .arg("--node-id")
        .arg("node-1")
        .arg("--input-port")
        .arg("uplink")
        .arg("--store-root")
        .arg(&store_root)
        .arg("--worker-binary")
        .arg(worker_binary_path())
        .arg("--input-path")
        .arg(&input_path)
        .arg("--evidence-out")
        .arg(&evidence_out)
        .output()
        .expect("smartsentinel-dcode-invoke should run");
    assert!(invoke_output.status.success(), "invoke failed: {}", String::from_utf8_lossy(&invoke_output.stderr));
    let invoke_stdout = String::from_utf8_lossy(&invoke_output.stdout);
    assert!(invoke_stdout.contains("status=ok"));
    assert!(invoke_stdout.contains("outcome=Success"));
    assert!(invoke_stdout.contains("host_mutation=false"));

    let evidence: serde_json::Value = serde_json::from_str(&fs::read_to_string(&evidence_out).unwrap()).unwrap();
    assert_eq!(evidence["outcome"], "success");
    assert_eq!(evidence["output_status"], "ok");
    assert_eq!(evidence["input_size"], 21);
    assert_eq!(evidence["fresh_instance_per_invocation"], true);
    assert_eq!(evidence["node_id"], "node-1");
    assert!(evidence["message_id"].as_str().unwrap().starts_with("dcode-msg:"));
    assert!(evidence["correlation_id"].as_str().unwrap().starts_with("dcode-corr:"));

    let _ = fs::remove_dir_all(&workdir);
}

#[test]
fn invoking_on_the_wrong_node_is_refused_by_the_cli_and_exits_nonzero() {
    let workdir = std::env::temp_dir().join(format!("datum-dcode-cli-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&workdir).unwrap();
    let module_bytes = wat::parse_str(RESPOND_OK_WAT).unwrap();
    let digest = sha256_hex(&module_bytes);
    let module_path = workdir.join("decode.wasm");
    fs::write(&module_path, &module_bytes).unwrap();
    let store_root = workdir.join("dcode-store");

    Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-install"))
        .arg("--module-path").arg(&module_path)
        .arg("--declared-sha256").arg(&digest)
        .arg("--store-root").arg(&store_root)
        .output()
        .unwrap();

    let artifact = json!({
        "schema": "datum.dserv-artifact/1", "artifact_id": "dserv:sensor-pipeline:decode", "version": "0.1.0",
        "application_id": "app:sensor-pipeline", "dserv_id": "decode",
        "dcode": {"uri": format!("datum-blob://sha256/{digest}"), "sha256": digest, "content_id": format!("sha256:{digest}"), "size_bytes": module_bytes.len()},
        "abi": {"name": "datum-dnode", "version": "0", "required_exports": ["memory","datum_abi_version","datum_alloc","datum_dealloc","datum_handle"], "allowed_imports": []},
        "ports": {"inputs": [{"port": "uplink", "payload_schema": "s", "external": true}], "outputs": []},
        "limits": {"fuel_per_invocation": 5_000_000, "timeout_ms": 2000, "max_memory_pages": 4, "max_message_bytes": 65536, "max_emits_per_invocation": 8},
        "safety": {"filesystem_access": false, "network_access": false, "host_command_access": false, "docker_access": false, "side_effects_allowed": false},
        "state_model": "stateless", "instantiation": "fresh_instance_per_invocation"
    });
    let artifact_path = workdir.join("artifact.json");
    fs::write(&artifact_path, serde_json::to_string_pretty(&artifact).unwrap()).unwrap();

    let parsed_artifact: datum::dcode::dserv_artifact::CanonicalDServArtifactV1 =
        serde_json::from_value(artifact.clone()).unwrap();
    let descriptor_digest = datum::dcode::dserv_artifact::recompute_dserv_artifact_digest(&parsed_artifact).unwrap();

    let authorization = json!({
        "authorization_id": "dcode-auth:proj:app:decode:node-1:1",
        "project_id": "proj:lab", "application_id": "app:sensor-pipeline", "service_id": "decode",
        "node_id": "node-1",
        "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
        "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
        "dserv_artifact_id": "dserv:sensor-pipeline:decode", "dserv_artifact_digest": digest,
        "dserv_artifact_descriptor_digest": descriptor_digest,
        "dnode_abi": "datum-dnode/0"
    });
    let authorization_path = workdir.join("authorization.json");
    fs::write(&authorization_path, serde_json::to_string_pretty(&authorization).unwrap()).unwrap();
    let input_path = workdir.join("input.bin");
    fs::write(&input_path, b"x").unwrap();

    let invoke_output = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-invoke"))
        .arg("offline-test-only")
        .arg("--authorization-path").arg(&authorization_path)
        .arg("--artifact-path").arg(&artifact_path)
        .arg("--node-id").arg("node-2") // WRONG node
        .arg("--input-port").arg("uplink")
        .arg("--store-root").arg(&store_root)
        .arg("--worker-binary").arg(worker_binary_path())
        .arg("--input-path").arg(&input_path)
        .output()
        .unwrap();

    // Independent review v2, R6: a non-Success outcome must exit non-zero.
    // A prior version of this test asserted `.status.success()` here, which
    // was itself the bug R6 fixes -- the CLI used to always exit 0.
    assert!(!invoke_output.status.success(), "wrong-node invocation must exit non-zero");
    let stdout = String::from_utf8_lossy(&invoke_output.stdout);
    assert!(stdout.contains("status=failed"));
    assert!(stdout.contains("outcome=StaleAuthority"));

    let _ = fs::remove_dir_all(&workdir);
}

#[test]
fn offline_test_only_mode_prints_a_warning_that_it_bypasses_dserver() {
    let workdir = std::env::temp_dir().join(format!("datum-dcode-cli-e2e-{}", Uuid::new_v4()));
    fs::create_dir_all(&workdir).unwrap();
    let module_bytes = wat::parse_str(RESPOND_OK_WAT).unwrap();
    let digest = sha256_hex(&module_bytes);
    let store_root = workdir.join("dcode-store");
    fs::write(workdir.join("decode.wasm"), &module_bytes).unwrap();
    Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-install"))
        .arg("--module-path").arg(workdir.join("decode.wasm"))
        .arg("--declared-sha256").arg(&digest)
        .arg("--store-root").arg(&store_root)
        .output()
        .unwrap();

    let artifact = json!({
        "schema": "datum.dserv-artifact/1", "artifact_id": "dserv:a:decode", "version": "0.1.0",
        "application_id": "app:a", "dserv_id": "decode",
        "dcode": {"uri": format!("datum-blob://sha256/{digest}"), "sha256": digest, "content_id": format!("sha256:{digest}"), "size_bytes": module_bytes.len()},
        "abi": {"name": "datum-dnode", "version": "0", "required_exports": ["memory","datum_abi_version","datum_alloc","datum_dealloc","datum_handle"], "allowed_imports": []},
        "ports": {"inputs": [{"port": "uplink", "payload_schema": "s", "external": true}], "outputs": []},
        "limits": {"fuel_per_invocation": 5_000_000, "timeout_ms": 2000, "max_memory_pages": 4, "max_message_bytes": 65536, "max_emits_per_invocation": 8},
        "safety": {"filesystem_access": false, "network_access": false, "host_command_access": false, "docker_access": false, "side_effects_allowed": false},
        "state_model": "stateless", "instantiation": "fresh_instance_per_invocation"
    });
    let artifact_path = workdir.join("artifact.json");
    fs::write(&artifact_path, serde_json::to_string_pretty(&artifact).unwrap()).unwrap();
    let parsed_artifact: datum::dcode::dserv_artifact::CanonicalDServArtifactV1 =
        serde_json::from_value(artifact.clone()).unwrap();
    let descriptor_digest = datum::dcode::dserv_artifact::recompute_dserv_artifact_digest(&parsed_artifact).unwrap();
    let authorization = json!({
        "authorization_id": "dcode-auth:proj:app:decode:node-1:1", "project_id": "proj:lab",
        "application_id": "app:a", "service_id": "decode", "node_id": "node-1",
        "dmap_ref": {"id": "dmap:1", "revision": 1, "digest": format!("sha256:{}", "1".repeat(64))},
        "dgraph_ref": {"id": "dgraph:1", "revision": 1, "digest": format!("sha256:{}", "2".repeat(64))},
        "dserv_artifact_id": "dserv:a:decode", "dserv_artifact_digest": digest,
        "dserv_artifact_descriptor_digest": descriptor_digest, "dnode_abi": "datum-dnode/0"
    });
    let authorization_path = workdir.join("authorization.json");
    fs::write(&authorization_path, serde_json::to_string_pretty(&authorization).unwrap()).unwrap();
    let input_path = workdir.join("input.bin");
    fs::write(&input_path, b"x").unwrap();

    let invoke_output = Command::new(env!("CARGO_BIN_EXE_smartsentinel-dcode-invoke"))
        .arg("offline-test-only")
        .arg("--authorization-path").arg(&authorization_path)
        .arg("--artifact-path").arg(&artifact_path)
        .arg("--node-id").arg("node-1")
        .arg("--input-port").arg("uplink")
        .arg("--store-root").arg(&store_root)
        .arg("--worker-binary").arg(worker_binary_path())
        .arg("--input-path").arg(&input_path)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&invoke_output.stderr);
    assert!(stderr.contains("NEVER use this for real governed execution"));

    let _ = fs::remove_dir_all(&workdir);
}
