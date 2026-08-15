use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};

use crate::wasm_runtime::{
    artifact::{run_local_wasm_artifact, LoadedWasmArtifact},
    report::{build_wasm_runtime_report_from_result, WasmRuntimeReport},
    WasmRuntimeInput, WasmRuntimeLimitOutcome, WasmRuntimeLimits, WasmRuntimeResult,
};

const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmArtifactIdentity {
    pub algorithm: String,
    pub sha256: String,
    pub content_id: String,
    pub size_bytes: usize,
}

pub fn identify_loaded_wasm_artifact(artifact: &LoadedWasmArtifact) -> WasmArtifactIdentity {
    let sha256 = sha256_hex(artifact.bytes());
    WasmArtifactIdentity {
        algorithm: "sha256".to_string(),
        content_id: format!("sha256:{sha256}"),
        sha256,
        size_bytes: artifact.size_bytes,
    }
}

pub fn verify_loaded_wasm_artifact_sha256(
    artifact: &LoadedWasmArtifact,
    expected_sha256: &str,
) -> Result<WasmArtifactIdentity> {
    let expected = normalize_expected_sha256(expected_sha256)?;
    let identity = identify_loaded_wasm_artifact(artifact);

    if identity.sha256 != expected {
        return Err(anyhow!(
            "external WASM artifact SHA-256 mismatch: expected {expected}, calculated {}",
            identity.sha256
        ));
    }

    Ok(identity)
}

pub fn run_verified_local_wasm_artifact(
    artifact: &LoadedWasmArtifact,
    expected_sha256: &str,
    input: &WasmRuntimeInput,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeResult> {
    let identity = match verify_loaded_wasm_artifact_sha256(artifact, expected_sha256) {
        Ok(identity) => identity,
        Err(error) => {
            return Ok(blocked_integrity_result(
                input,
                format!("artifact integrity verification failed before execution: {error}"),
                limits.clone(),
            ));
        }
    };

    let mut result = run_local_wasm_artifact(artifact, input, limits)?;
    if result.status == "executed" {
        let runtime_message = result.message.take().unwrap_or_else(|| {
            "executed isolated external WASM artifact without host imports".to_string()
        });
        result.message = Some(format!(
            "verified {} before execution; {runtime_message}",
            identity.content_id
        ));
    }

    Ok(result)
}

pub fn run_verified_local_wasm_artifact_report(
    artifact: &LoadedWasmArtifact,
    expected_sha256: &str,
    input: &WasmRuntimeInput,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeReport> {
    let result = run_verified_local_wasm_artifact(artifact, expected_sha256, input, limits)?;
    Ok(build_wasm_runtime_report_from_result(&result))
}

fn normalize_expected_sha256(value: &str) -> Result<String> {
    let trimmed = value.trim();
    let digest = trimmed.strip_prefix("sha256:").unwrap_or(trimmed);

    if digest.len() != SHA256_HEX_LEN || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "expected SHA-256 must contain exactly 64 hexadecimal characters, optionally prefixed with 'sha256:'"
        ));
    }

    Ok(digest.to_ascii_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(SHA256_HEX_LEN);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing SHA-256 hex to String cannot fail");
    }
    output
}

fn blocked_integrity_result(
    input: &WasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
) -> WasmRuntimeResult {
    WasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "blocked".to_string(),
        result_i32: None,
        message: Some(message),
        safe_to_auto_execute: false,
        side_effects_performed: false,
        filesystem_access: false,
        network_access: false,
        host_command_access: false,
        docker_access: false,
        limits: limits.clone(),
        limit_outcome: WasmRuntimeLimitOutcome {
            fuel_configured: limits.fuel,
            fuel_consumed: None,
            timeout_ms: limits.timeout_ms,
            limit_exceeded: Some(false),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use uuid::Uuid;

    use super::{
        identify_loaded_wasm_artifact, run_verified_local_wasm_artifact, sha256_hex,
        verify_loaded_wasm_artifact_sha256,
    };
    use crate::wasm_runtime::{
        artifact::load_local_wasm_artifact, WasmRuntimeInput, WasmRuntimeLimits,
    };

    const ADD_WAT: &str = r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#;

    fn write_test_artifact(source: &str) -> PathBuf {
        let bytes = wat::parse_str(source).expect("test WAT should compile");
        let path = std::env::temp_dir().join(format!("datum-c61-{}.wasm", Uuid::new_v4()));
        fs::write(&path, bytes).expect("test artifact should be written");
        path
    }

    fn runtime_input() -> WasmRuntimeInput {
        WasmRuntimeInput {
            module_name: "external:add".to_string(),
            function_name: "add".to_string(),
            args: vec![2, 3],
        }
    }

    #[test]
    fn sha256_matches_known_abc_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn loaded_artifact_gets_deterministic_content_identity() {
        let path = write_test_artifact(ADD_WAT);
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let first = identify_loaded_wasm_artifact(&artifact);
        let second = identify_loaded_wasm_artifact(&artifact);

        assert_eq!(first, second);
        assert_eq!(first.algorithm, "sha256");
        assert_eq!(first.sha256.len(), 64);
        assert_eq!(first.content_id, format!("sha256:{}", first.sha256));
        assert_eq!(first.size_bytes, artifact.size_bytes);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn matching_sha256_allows_controlled_execution() {
        let path = write_test_artifact(ADD_WAT);
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let identity = identify_loaded_wasm_artifact(&artifact);
        let result = run_verified_local_wasm_artifact(
            &artifact,
            &identity.sha256,
            &runtime_input(),
            &WasmRuntimeLimits::default(),
        )
        .expect("verified execution should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(result.result_i32, Some(5));
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains(&identity.content_id));
        assert!(!result.safe_to_auto_execute);
        assert!(!result.side_effects_performed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn prefixed_uppercase_sha256_is_accepted() {
        let path = write_test_artifact(ADD_WAT);
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let identity = identify_loaded_wasm_artifact(&artifact);
        let expected = format!("sha256:{}", identity.sha256.to_ascii_uppercase());

        let verified = verify_loaded_wasm_artifact_sha256(&artifact, &expected)
            .expect("prefixed uppercase digest should verify");
        assert_eq!(verified.sha256, identity.sha256);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn mismatched_sha256_blocks_before_execution() {
        let path = write_test_artifact(ADD_WAT);
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_verified_local_wasm_artifact(
            &artifact,
            &"0".repeat(64),
            &runtime_input(),
            &WasmRuntimeLimits::default(),
        )
        .expect("mismatch should return a blocked result");

        assert_eq!(result.status, "blocked");
        assert_eq!(result.result_i32, None);
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("SHA-256 mismatch"));
        assert!(!result.side_effects_performed);
        assert_eq!(result.limit_outcome.fuel_consumed, None);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_expected_sha256_blocks_before_execution() {
        let path = write_test_artifact(ADD_WAT);
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_verified_local_wasm_artifact(
            &artifact,
            "not-a-digest",
            &runtime_input(),
            &WasmRuntimeLimits::default(),
        )
        .expect("malformed digest should return a blocked result");

        assert_eq!(result.status, "blocked");
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("64 hexadecimal characters"));
        assert_eq!(result.limit_outcome.fuel_consumed, None);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn integrity_mismatch_precedes_wasmtime_compilation() {
        let path = std::env::temp_dir().join(format!("datum-c61-{}.wasm", Uuid::new_v4()));
        fs::write(&path, b"\0asminvalid-module").expect("invalid artifact should be written");
        let artifact = load_local_wasm_artifact(&path).expect("magic-only artifact should load");
        let result = run_verified_local_wasm_artifact(
            &artifact,
            &"f".repeat(64),
            &runtime_input(),
            &WasmRuntimeLimits::default(),
        )
        .expect("integrity mismatch should be handled before compilation");

        assert_eq!(result.status, "blocked");
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("integrity verification failed before execution"));
        assert!(!result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("compile external WASM artifact failed"));

        let _ = fs::remove_file(path);
    }
}
