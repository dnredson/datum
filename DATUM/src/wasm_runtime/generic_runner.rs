use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use wasmtime::{Config, Engine, Instance, Module, Store, Val, ValType};

use crate::wasm_runtime::{
    artifact::LoadedWasmArtifact, WasmRuntimeLimitOutcome, WasmRuntimeLimits,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum WasmScalarValue {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
}

impl WasmScalarValue {
    fn type_name(&self) -> &'static str {
        match self {
            Self::I32(_) => "i32",
            Self::I64(_) => "i64",
            Self::F32(_) => "f32",
            Self::F64(_) => "f64",
        }
    }

    fn matches_type(&self, value_type: &ValType) -> bool {
        matches!(
            (self, value_type),
            (Self::I32(_), ValType::I32)
                | (Self::I64(_), ValType::I64)
                | (Self::F32(_), ValType::F32)
                | (Self::F64(_), ValType::F64)
        )
    }

    fn to_wasmtime_val(&self) -> Val {
        match self {
            Self::I32(value) => Val::I32(*value),
            Self::I64(value) => Val::I64(*value),
            Self::F32(value) => Val::F32(value.to_bits()),
            Self::F64(value) => Val::F64(value.to_bits()),
        }
    }

    fn from_wasmtime_val(value: &Val) -> Option<Self> {
        match value {
            Val::I32(value) => Some(Self::I32(*value)),
            Val::I64(value) => Some(Self::I64(*value)),
            Val::F32(value) => Some(Self::F32(f32::from_bits(*value))),
            Val::F64(value) => Some(Self::F64(f64::from_bits(*value))),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GenericWasmRuntimeInput {
    pub module_name: String,
    pub function_name: String,
    pub args: Vec<WasmScalarValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GenericWasmRuntimeResult {
    pub module_name: String,
    pub function_name: String,
    pub status: String,
    pub results: Vec<WasmScalarValue>,
    pub message: Option<String>,
    pub integrity_verified: bool,
    pub artifact_content_id: Option<String>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
    pub filesystem_access: bool,
    pub network_access: bool,
    pub host_command_access: bool,
    pub docker_access: bool,
    pub limits: WasmRuntimeLimits,
    pub limit_outcome: WasmRuntimeLimitOutcome,
}

pub(crate) fn run_generic_local_wasm_artifact(
    artifact: &LoadedWasmArtifact,
    input: &GenericWasmRuntimeInput,
    limits: &WasmRuntimeLimits,
) -> Result<GenericWasmRuntimeResult> {
    if limits.fuel == Some(0) {
        return Ok(failed_result_with_outcome(
            input,
            "generic external WASM artifact execution blocked because configured fuel was 0"
                .to_string(),
            limits.clone(),
            build_limit_outcome(limits, Some(0), Some(0), true),
        ));
    }

    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build generic WASM engine with fuel accounting")?;
    let module = match Module::new(&engine, artifact.bytes()) {
        Ok(module) => module,
        Err(error) => {
            return Ok(failed_result(
                input,
                format!("compile generic external WASM artifact failed: {error}"),
                limits.clone(),
            ));
        }
    };

    if let Some(import) = module.imports().next() {
        return Ok(blocked_result(
            input,
            format!(
                "generic external WASM artifact imports host item '{}::{}'; host imports are not allowed in the controlled sandbox",
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
            .context("configure generic external WASM artifact fuel budget")?;
    }

    let fuel_before = current_fuel(&mut store);
    let instance = match Instance::new(&mut store, &module, &[]) {
        Ok(instance) => instance,
        Err(error) => {
            let fuel_after = current_fuel(&mut store);
            return Ok(failed_result_with_outcome(
                input,
                format!("instantiate generic external WASM artifact failed: {error}"),
                limits.clone(),
                build_limit_outcome(
                    limits,
                    fuel_before,
                    fuel_after,
                    is_fuel_exhaustion(&error),
                ),
            ));
        }
    };

    let function = match instance.get_func(&mut store, &input.function_name) {
        Some(function) => function,
        None => {
            return Ok(failed_result(
                input,
                format!(
                    "generic external WASM artifact does not export function '{}'",
                    input.function_name
                ),
                limits.clone(),
            ));
        }
    };

    let function_type = function.ty(&store);
    let parameter_types: Vec<ValType> = function_type.params().collect();
    let result_types: Vec<ValType> = function_type.results().collect();

    if input.args.len() != parameter_types.len() {
        return Ok(blocked_result(
            input,
            format!(
                "generic external WASM function '{}' expects {} arguments but received {}",
                input.function_name,
                parameter_types.len(),
                input.args.len()
            ),
            limits.clone(),
        ));
    }

    for (index, (argument, expected_type)) in input
        .args
        .iter()
        .zip(parameter_types.iter())
        .enumerate()
    {
        if !is_supported_scalar_type(expected_type) {
            return Ok(blocked_result(
                input,
                format!(
                    "generic external WASM function '{}' parameter {} uses unsupported type {}",
                    input.function_name,
                    index,
                    wasm_type_name(expected_type)
                ),
                limits.clone(),
            ));
        }

        if !argument.matches_type(expected_type) {
            return Ok(blocked_result(
                input,
                format!(
                    "generic external WASM function '{}' parameter {} expects {} but received {}",
                    input.function_name,
                    index,
                    wasm_type_name(expected_type),
                    argument.type_name()
                ),
                limits.clone(),
            ));
        }
    }

    for (index, result_type) in result_types.iter().enumerate() {
        if !is_supported_scalar_type(result_type) {
            return Ok(blocked_result(
                input,
                format!(
                    "generic external WASM function '{}' result {} uses unsupported type {}",
                    input.function_name,
                    index,
                    wasm_type_name(result_type)
                ),
                limits.clone(),
            ));
        }
    }

    let params: Vec<Val> = input
        .args
        .iter()
        .map(WasmScalarValue::to_wasmtime_val)
        .collect();
    let mut results: Vec<Val> = result_types
        .iter()
        .map(default_value_for_type)
        .collect::<Option<Vec<_>>>()
        .expect("supported result types always have scalar placeholders");

    if let Err(error) = function.call(&mut store, &params, &mut results) {
        let fuel_after = current_fuel(&mut store);
        return Ok(failed_result_with_outcome(
            input,
            format!("generic external WASM function execution failed: {error}"),
            limits.clone(),
            build_limit_outcome(
                limits,
                fuel_before,
                fuel_after,
                is_fuel_exhaustion(&error),
            ),
        ));
    }

    let fuel_after = current_fuel(&mut store);
    let scalar_results: Vec<WasmScalarValue> = results
        .iter()
        .filter_map(WasmScalarValue::from_wasmtime_val)
        .collect();

    if scalar_results.len() != results.len() {
        return Ok(blocked_result(
            input,
            "generic external WASM function produced an unsupported runtime value".to_string(),
            limits.clone(),
        ));
    }

    Ok(GenericWasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "executed".to_string(),
        results: scalar_results,
        message: Some(format!(
            "executed generic isolated external WASM function '{}' with scalar-only ABI and without host imports",
            input.function_name
        )),
        integrity_verified: false,
        artifact_content_id: None,
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

fn is_supported_scalar_type(value_type: &ValType) -> bool {
    matches!(
        value_type,
        ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
    )
}

fn wasm_type_name(value_type: &ValType) -> &'static str {
    match value_type {
        ValType::I32 => "i32",
        ValType::I64 => "i64",
        ValType::F32 => "f32",
        ValType::F64 => "f64",
        ValType::V128 => "v128",
        ValType::Ref(_) => "reference",
    }
}

fn default_value_for_type(value_type: &ValType) -> Option<Val> {
    match value_type {
        ValType::I32 => Some(Val::I32(0)),
        ValType::I64 => Some(Val::I64(0)),
        ValType::F32 => Some(Val::F32(0)),
        ValType::F64 => Some(Val::F64(0)),
        _ => None,
    }
}

pub(crate) fn blocked_generic_integrity_result(
    input: &GenericWasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
) -> GenericWasmRuntimeResult {
    GenericWasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "blocked".to_string(),
        results: Vec::new(),
        message: Some(message),
        integrity_verified: false,
        artifact_content_id: None,
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

fn blocked_result(
    input: &GenericWasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
) -> GenericWasmRuntimeResult {
    GenericWasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "blocked".to_string(),
        results: Vec::new(),
        message: Some(message),
        integrity_verified: false,
        artifact_content_id: None,
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
    input: &GenericWasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
) -> GenericWasmRuntimeResult {
    failed_result_with_outcome(
        input,
        message,
        limits.clone(),
        build_limit_outcome(&limits, None, None, false),
    )
}

fn failed_result_with_outcome(
    input: &GenericWasmRuntimeInput,
    message: String,
    limits: WasmRuntimeLimits,
    limit_outcome: WasmRuntimeLimitOutcome,
) -> GenericWasmRuntimeResult {
    GenericWasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "failed".to_string(),
        results: Vec::new(),
        message: Some(message),
        integrity_verified: false,
        artifact_content_id: None,
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

    use super::{
        run_generic_local_wasm_artifact, GenericWasmRuntimeInput, WasmScalarValue,
    };
    use crate::wasm_runtime::{artifact::load_local_wasm_artifact, WasmRuntimeLimits};

    fn write_test_artifact(source: &str) -> PathBuf {
        let bytes = wat::parse_str(source).expect("test WAT should compile");
        let path = std::env::temp_dir().join(format!("datum-c62-{}.wasm", Uuid::new_v4()));
        fs::write(&path, bytes).expect("test artifact should be written");
        path
    }

    fn input(function_name: &str, args: Vec<WasmScalarValue>) -> GenericWasmRuntimeInput {
        GenericWasmRuntimeInput {
            module_name: "external:generic".to_string(),
            function_name: function_name.to_string(),
            args,
        }
    }

    #[test]
    fn generic_i32_add_executes() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input(
                "add",
                vec![WasmScalarValue::I32(2), WasmScalarValue::I32(3)],
            ),
            &WasmRuntimeLimits::default(),
        )
        .expect("generic add should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(result.results, vec![WasmScalarValue::I32(5)]);
        assert!(!result.integrity_verified);
        assert!(!result.safe_to_auto_execute);
        assert!(!result.side_effects_performed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_i64_function_executes() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "mul64") (param i64 i64) (result i64)
    local.get 0
    local.get 1
    i64.mul))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input(
                "mul64",
                vec![WasmScalarValue::I64(6), WasmScalarValue::I64(7)],
            ),
            &WasmRuntimeLimits::default(),
        )
        .expect("generic i64 call should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(result.results, vec![WasmScalarValue::I64(42)]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_f32_function_executes() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "add32") (param f32 f32) (result f32)
    local.get 0
    local.get 1
    f32.add))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input(
                "add32",
                vec![WasmScalarValue::F32(1.25), WasmScalarValue::F32(2.5)],
            ),
            &WasmRuntimeLimits::default(),
        )
        .expect("generic f32 call should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(result.results, vec![WasmScalarValue::F32(3.75)]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_f64_function_executes() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "sub64") (param f64 f64) (result f64)
    local.get 0
    local.get 1
    f64.sub))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input(
                "sub64",
                vec![WasmScalarValue::F64(10.5), WasmScalarValue::F64(0.25)],
            ),
            &WasmRuntimeLimits::default(),
        )
        .expect("generic f64 call should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(result.results, vec![WasmScalarValue::F64(10.25)]);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_multi_value_result_executes() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "pair") (param i32 i64) (result i32 i64)
    local.get 0
    local.get 1))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input(
                "pair",
                vec![WasmScalarValue::I32(9), WasmScalarValue::I64(11)],
            ),
            &WasmRuntimeLimits::default(),
        )
        .expect("generic multi-value call should return a result");

        assert_eq!(result.status, "executed");
        assert_eq!(
            result.results,
            vec![WasmScalarValue::I32(9), WasmScalarValue::I64(11)]
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn generic_zero_arg_zero_result_function_executes() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "ping")))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input("ping", Vec::new()),
            &WasmRuntimeLimits::default(),
        )
        .expect("generic zero-arg call should return a result");

        assert_eq!(result.status, "executed");
        assert!(result.results.is_empty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn mismatched_scalar_argument_type_is_blocked() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "identity") (param i64) (result i64)
    local.get 0))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input("identity", vec![WasmScalarValue::I32(7)]),
            &WasmRuntimeLimits::default(),
        )
        .expect("type mismatch should return a safe result");

        assert_eq!(result.status, "blocked");
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("expects i64 but received i32"));
        assert!(!result.side_effects_performed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn unsupported_v128_signature_is_blocked() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "vector") (result v128)
    v128.const i32x4 0 0 0 0))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input("vector", Vec::new()),
            &WasmRuntimeLimits::default(),
        )
        .expect("unsupported result should return a safe result");

        assert_eq!(result.status, "blocked");
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("unsupported type v128"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn host_import_remains_blocked_in_generic_runner() {
        let path = write_test_artifact(
            r#"
(module
  (import "host" "log" (func $log))
  (func (export "ping")))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input("ping", Vec::new()),
            &WasmRuntimeLimits::default(),
        )
        .expect("host import rejection should return a safe result");

        assert_eq!(result.status, "blocked");
        assert!(result
            .message
            .as_deref()
            .unwrap_or("")
            .contains("host imports are not allowed"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn zero_fuel_fails_before_generic_execution() {
        let path = write_test_artifact(
            r#"
(module
  (func (export "ping")))
"#,
        );
        let artifact = load_local_wasm_artifact(&path).expect("artifact should load");
        let result = run_generic_local_wasm_artifact(
            &artifact,
            &input("ping", Vec::new()),
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
    fn source_guard_keeps_generic_runner_isolated() {
        let source = include_str!("generic_runner.rs");
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
                "generic WASM runner must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
