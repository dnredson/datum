use std::{fs, path::{Path, PathBuf}};

use anyhow::{Context, Result};
use wasmtime::{Config, Engine, Instance, Module, Store, TypedFunc};

use crate::wasm_runtime::{
    report::{build_wasm_runtime_report_from_result, WasmRuntimeReport},
    WasmRuntimeInput, WasmRuntimeLimitOutcome, WasmRuntimeLimits, WasmRuntimeResult,
};

const WASM_MAGIC: &[u8; 4] = b"\0asm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedWasmArtifact {
    pub path: PathBuf,
    pub size_bytes: usize,
    bytes: Vec<u8>,
}

impl LoadedWasmArtifact {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

pub fn load_local_wasm_artifact(path: impl AsRef<Path>) -> Result<LoadedWasmArtifact> {
    let path = path.as_ref();
    let extension_is_wasm = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("wasm"))
        .unwrap_or(false);

    if !extension_is_wasm {
        anyhow::bail!("external WASM artifact must use the .wasm extension");
    }

    let metadata = fs::metadata(path)
        .with_context(|| format!("read WASM artifact metadata from {}", path.display()))?;
    if !metadata.is_file() {
        anyhow::bail!("external WASM artifact path is not a regular file: {}", path.display());
    }

    let bytes = fs::read(path)
        .with_context(|| format!("read external WASM artifact from {}", path.display()))?;
    if !bytes.starts_with(WASM_MAGIC) {
        anyhow::bail!("external WASM artifact does not contain the WebAssembly binary magic header");
    }

    Ok(LoadedWasmArtifact {
        path: path.to_path_buf(),
        size_bytes: bytes.len(),
        bytes,
    })
}

pub fn run_local_wasm_artifact(
    artifact: &LoadedWasmArtifact,
    input: &WasmRuntimeInput,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeResult> {
    if input.args.len() != 2 {
        return Ok(blocked_result(
            input,
            "external WASM artifact execution currently requires exactly two i32 arguments",
            limits.clone(),
        ));
    }

    if limits.fuel == Some(0) {
        return Ok(failed_result_with_outcome(
            input,
            "external WASM artifact execution blocked because configured fuel was 0".to_string(),
            limits.clone(),
            build_limit_outcome(limits, Some(0), Some(0), true),
        ));
    }

    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build WASM engine with fuel accounting")?;
    let module = match Module::new(&engine, artifact.bytes()) {
        Ok(module) => module,
        Err(error) => {
            return Ok(failed_result(
                input,
                format!("compile external WASM artifact failed: {error}"),
                limits.clone(),
            ));
        }
    };

    if let Some(import) = module.imports().next() {
        return Ok(blocked_result_owned(
            input,
            format!(
                "external WASM artifact imports host item '{}::{}'; host imports are not allowed in the controlled sandbox",
                import.module(),
                import.name()
            ),
            limits.clone(),
        ));
    }

    let mut store = Store::new(&engine, ());
    if let Some(fuel) = limits.fuel {
        store
            .set_fuel(fuel)
            .context("configure external WASM artifact fuel budget")?;
    }

    let fuel_before = current_fuel(&mut store);
    let instance = match Instance::new(&mut store, &module, &[]) {
        Ok(instance) => instance,
        Err(error) => {
            let fuel_after = current_fuel(&mut store);
            let limit_outcome =
                build_limit_outcome(limits, fuel_before, fuel_after, is_fuel_exhaustion(&error));
            return Ok(failed_result_with_outcome(
                input,
                format!("instantiate external WASM artifact failed: {error}"),
                limits.clone(),
                limit_outcome,
            ));
        }
    };

    let function: TypedFunc<(i32, i32), i32> =
        match instance.get_typed_func(&mut store, &input.function_name) {
            Ok(function) => function,
            Err(error) => {
                return Ok(failed_result(
                    input,
                    format!(
                        "resolve exported external WASM function '{}' failed: {error}",
                        input.function_name
                    ),
                    limits.clone(),
                ));
            }
        };

    let value = match function.call(&mut store, (input.args[0], input.args[1])) {
        Ok(value) => value,
        Err(error) => {
            let fuel_after = current_fuel(&mut store);
            let limit_outcome =
                build_limit_outcome(limits, fuel_before, fuel_after, is_fuel_exhaustion(&error));
            return Ok(failed_result_with_outcome(
                input,
                format!("external WASM artifact execution failed: {error}"),
                limits.clone(),
                limit_outcome,
            ));
        }
    };
    let fuel_after = current_fuel(&mut store);

    Ok(WasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "executed".to_string(),
        result_i32: Some(value),
        message: Some(format!(
            "executed isolated external WASM artifact '{}' without host imports",
            artifact.path.display()
        )),
        safe_to_auto_execute: false,
        side_effects_performed: false,
        filesystem_access: false,
        network_access: false,
        host_command_access: false,
        docker_access: false,
        limits: limits.clone(),
        limit_outcome: build_limit_outcome(limits, fuel_before, fuel_after, false),
    })
}

pub fn run_local_wasm_artifact_report(
    artifact: &LoadedWasmArtifact,
    input: &WasmRuntimeInput,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeReport> {
    let result = run_local_wasm_artifact(artifact, input, limits)?;
    Ok(build_wasm_runtime_report_from_result(&result))
}

fn blocked_result(
    input: &WasmRuntimeInput,
    message: &str,
    limits: WasmRuntimeLimits,
) -> WasmRuntimeResult {
    blocked_result_owned(input, message.to_string(), limits)
}

fn blocked_result_owned(
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
        limit_outcome: build_limit_outcome(&limits, None, None, false),
    }
}

fn failed_result(
    input: &WasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
) -> WasmRuntimeResult {
    failed_result_with_outcome(
        input,
        message,
        limits.clone(),
        build_limit_outcome(&limits, None, None, false),
    )
}

fn failed_result_with_outcome(
    input: &WasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
    limit_outcome: WasmRuntimeLimitOutcome,
) -> WasmRuntimeResult {
    WasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "failed".to_string(),
        result_i32: None,
        message: Some(message),
        safe_to_auto_execute: false,
        side_effects_performed: false,
        filesystem_access: false,
        network_access: false,
        host_command_access: false,
        docker_access: false,
        limits,
        limit_outcome,
    }
}

fn current_fuel(store: &mut Store<()>) -> Option<u64> {
    store.get_fuel().ok()
}

fn build_limit_outcome(
    limits: &WasmRuntimeLimits,
    fuel_before: Option<u64>,
    fuel_after: Option<u64>,
    limit_exceeded: bool,
) -> WasmRuntimeLimitOutcome {
    let fuel_consumed = match (fuel_before, fuel_after) {
        (Some(before), Some(after)) if before >= after => Some(before - after),
        _ => None,
    };

    WasmRuntimeLimitOutcome {
        fuel_configured: limits.fuel,
        fuel_consumed,
        timeout_ms: limits.timeout_ms,
        limit_exceeded: Some(limit_exceeded),
    }
}

fn is_fuel_exhaustion(error: &anyhow::Error) -> bool {
    error.to_string().to_ascii_lowercase().contains("fuel")
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use uuid::Uuid;

    use super::{load_local_wasm_artifact, run_local_wasm_artifact};
    use crate::wasm_runtime::{WasmRuntimeInput, WasmRuntimeLimits};

    const ADD_WAT: &str = r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#;

    fn write_test_artifact(source: &str, extension: &str) -> PathBuf {
        let bytes = wat::parse_str(source).expect("test WAT should compile");
        let path = std::env::temp_dir().join(format!(
            "datum-c60-{}.{}",
            Uuid::new_v4(),
            extension
        ));
        fs::write(&path, bytes).expect("test artifact should be written");
        path
    }

    #[test]
    fn loader_accepts_external_wasm_binary() {
        let path = write_test_artifact(ADD_WAT, "wasm");
        let artifact = load_local_wasm_artifact(&path).expect("external artifact should load");

        assert_eq!(artifact.path, path);
        assert!(artifact.size_bytes > 8);
        assert!(artifact.bytes().starts_with(b"\0asm"));

        let _ = fs::remove_file(artifact.path);
    }

    #[test]
    fn loader_rejects_non_wasm_extension() {
        let path = write_test_artifact(ADD_WAT, "wat");
        let error = load_local_wasm_artifact(&path).expect_err(".wat should be rejected");

        assert!(error.to_string().contains(".wasm extension"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn loader_rejects_invalid_binary_magic() {
        let path = std::env::temp_dir().join(format!("datum-c60-{}.wasm", Uuid::new_v4()));
        fs::write(&path, b"not-wasm").expect("invalid test artifact should be written");

        let error = load_local_wasm_artifact(&path).expect_err("invalid magic should be rejected");
        assert!(error.to_string().contains("binary magic header"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn external_add_executes_in_controlled_sandbox() {
        let path = write_test_artifact(ADD_WAT, "wasm");
        let artifact = load_local_wasm_artifact(&path).expect("external artifact should load");
        let result = run_local_wasm_artifact(
            &artifact,
            &WasmRuntimeInput {
                module_name: "external:add".to_string(),
                function_name: "add".to_string(),
                args: vec![2, 3],
            },
            &WasmRuntimeLimits::default(),
        )
        .expect("external artifact execution should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(result.result_i32, Some(5));
        assert!(!result.safe_to_auto_execute);
        assert!(!result.side_effects_performed);
        assert!(!result.filesystem_access);
        assert!(!result.network_access);
        assert!(!result.host_command_access);
        assert!(!result.docker_access);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn external_artifact_with_host_import_is_blocked() {
        let source = r#"
(module
  (import "host" "log" (func $log))
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#;
        let path = write_test_artifact(source, "wasm");
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load before import validation");
        let result = run_local_wasm_artifact(
            &artifact,
            &WasmRuntimeInput {
                module_name: "external:importing".to_string(),
                function_name: "add".to_string(),
                args: vec![2, 3],
            },
            &WasmRuntimeLimits::default(),
        )
        .expect("import rejection should return a safe result");

        assert_eq!(result.status, "blocked");
        assert_eq!(result.result_i32, None);
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("host imports are not allowed"));
        assert!(!result.side_effects_performed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_export_fails_without_side_effects() {
        let path = write_test_artifact(ADD_WAT, "wasm");
        let artifact = load_local_wasm_artifact(&path).expect("external artifact should load");
        let result = run_local_wasm_artifact(
            &artifact,
            &WasmRuntimeInput {
                module_name: "external:add".to_string(),
                function_name: "missing".to_string(),
                args: vec![2, 3],
            },
            &WasmRuntimeLimits::default(),
        )
        .expect("missing export should return a safe result");

        assert_eq!(result.status, "failed");
        assert_eq!(result.result_i32, None);
        assert!(!result.side_effects_performed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn zero_fuel_fails_safely() {
        let path = write_test_artifact(ADD_WAT, "wasm");
        let artifact = load_local_wasm_artifact(&path).expect("external artifact should load");
        let result = run_local_wasm_artifact(
            &artifact,
            &WasmRuntimeInput {
                module_name: "external:add".to_string(),
                function_name: "add".to_string(),
                args: vec![2, 3],
            },
            &WasmRuntimeLimits {
                fuel: Some(0),
                timeout_ms: Some(1_000),
            },
        )
        .expect("zero fuel should return a safe result");

        assert_eq!(result.status, "failed");
        assert_eq!(result.limit_outcome.limit_exceeded, Some(true));
        assert!(!result.side_effects_performed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn source_guard_keeps_external_artifact_corridor_isolated() {
        let source = include_str!("artifact.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section should exist");

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "TcpStream",
            "UdpSocket",
            "reqwest",
            "docker run",
            "docker pull",
            "installation_tasks",
            "assisted_execution",
            "wasmtime_wasi",
            "WasiCtx",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "external WASM artifact corridor must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
