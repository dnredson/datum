use anyhow::{Context, Result};
use wasmtime::{Config, Engine, Instance, Module, Store, TypedFunc};

use crate::wasm_runtime::{
    report::{build_wasm_runtime_report_from_result, WasmRuntimeReport},
    WasmRuntimeInput, WasmRuntimeLimitOutcome, WasmRuntimeLimits, WasmRuntimeResult,
};

const EMBEDDED_ADD_WAT: &str = r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#;

pub fn run_embedded_wasm_add(a: i32, b: i32) -> Result<WasmRuntimeResult> {
    run_embedded_wasm_add_with_limits(a, b, &WasmRuntimeLimits::default())
}

pub fn run_embedded_wasm_add_with_limits(
    a: i32,
    b: i32,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeResult> {
    let input = WasmRuntimeInput {
        module_name: "embedded:add".to_string(),
        function_name: "add".to_string(),
        args: vec![a, b],
    };
    run_embedded_wasm_operation(&input, limits)
}

pub fn run_embedded_wasm_add_report(a: i32, b: i32) -> Result<WasmRuntimeReport> {
    run_embedded_wasm_add_report_with_limits(a, b, &WasmRuntimeLimits::default())
}

pub fn run_embedded_wasm_add_report_with_limits(
    a: i32,
    b: i32,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeReport> {
    let result = run_embedded_wasm_add_with_limits(a, b, limits)?;
    Ok(build_wasm_runtime_report_from_result(&result))
}

pub fn run_embedded_wasm_operation(
    input: &WasmRuntimeInput,
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeResult> {
    if input.args.len() != 2 {
        return Ok(blocked_result(
            input,
            "embedded WASM add test requires exactly two i32 arguments",
            limits.clone(),
        ));
    }

    if input.function_name != "add" {
        return Ok(failed_result(
            input,
            format!(
                "embedded WASM module does not export function '{}'",
                input.function_name
            ),
            limits.clone(),
        ));
    }

    if limits.fuel == Some(0) {
        return Ok(failed_result_with_outcome(
            input,
            "embedded WASM add execution blocked because configured fuel was 0".to_string(),
            limits.clone(),
            build_limit_outcome(limits, Some(0), Some(0), true),
        ));
    }

    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build WASM engine with fuel accounting")?;
    let module_bytes = wat::parse_str(EMBEDDED_ADD_WAT).context("compile embedded WAT")?;
    let module = Module::new(&engine, module_bytes).context("build embedded WASM module")?;
    let mut store = Store::new(&engine, ());
    if let Some(fuel) = limits.fuel {
        store
            .set_fuel(fuel)
            .context("configure embedded WASM fuel budget")?;
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
                format!("instantiate embedded WASM module failed: {error}"),
                limits.clone(),
                limit_outcome,
            ));
        }
    };
    let add: TypedFunc<(i32, i32), i32> = instance
        .get_typed_func(&mut store, &input.function_name)
        .context("resolve exported WASM function")?;

    let value = match add.call(&mut store, (input.args[0], input.args[1])) {
        Ok(value) => value,
        Err(error) => {
            let fuel_after = current_fuel(&mut store);
            let limit_outcome =
                build_limit_outcome(limits, fuel_before, fuel_after, is_fuel_exhaustion(&error));
            return Ok(failed_result_with_outcome(
                input,
                format!("embedded WASM add execution failed: {error}"),
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
        message: Some("executed isolated embedded WASM test without host access".to_string()),
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

fn blocked_result(
    input: &WasmRuntimeInput,
    message: &str,
    limits: WasmRuntimeLimits,
) -> WasmRuntimeResult {
    WasmRuntimeResult {
        module_name: input.module_name.clone(),
        function_name: input.function_name.clone(),
        status: "blocked".to_string(),
        result_i32: None,
        message: Some(message.to_string()),
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
    let fuel_configured = limits.fuel;
    let fuel_consumed = match (fuel_before, fuel_after) {
        (Some(before), Some(after)) if before >= after => Some(before - after),
        _ => None,
    };

    WasmRuntimeLimitOutcome {
        fuel_configured,
        fuel_consumed,
        timeout_ms: limits.timeout_ms,
        limit_exceeded: Some(limit_exceeded),
    }
}

fn is_fuel_exhaustion(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("fuel")
}

#[cfg(test)]
mod tests {
    use super::{
        run_embedded_wasm_add, run_embedded_wasm_add_report, run_embedded_wasm_add_with_limits,
        run_embedded_wasm_operation,
    };
    use crate::wasm_runtime::{WasmRuntimeInput, WasmRuntimeLimits};

    #[test]
    fn embedded_add_returns_expected_result() {
        let result = run_embedded_wasm_add(2, 3).expect("embedded add should succeed");

        assert_eq!(result.status, "executed");
        assert_eq!(result.result_i32, Some(5));
    }

    #[test]
    fn result_safety_flags_remain_false() {
        let result = run_embedded_wasm_add(2, 3).expect("embedded add should succeed");

        assert!(!result.side_effects_performed);
        assert!(!result.safe_to_auto_execute);
        assert!(!result.filesystem_access);
        assert!(!result.network_access);
        assert!(!result.host_command_access);
        assert!(!result.docker_access);
        assert_eq!(result.limit_outcome.limit_exceeded, Some(false));
    }

    #[test]
    fn add_report_has_expected_status_result_and_mode() {
        let report =
            run_embedded_wasm_add_report(2, 3).expect("embedded add report should succeed");

        assert_eq!(report.status, "executed");
        assert_eq!(report.result_i32, Some(5));
        assert_eq!(report.execution_mode, "embedded_test");
    }

    #[test]
    fn default_limits_are_present_in_result() {
        let result = run_embedded_wasm_add(2, 3).expect("embedded add should succeed");

        assert_eq!(result.limits.fuel, Some(1_000_000));
        assert_eq!(result.limits.timeout_ms, Some(1_000));
        assert_eq!(result.limit_outcome.fuel_configured, Some(1_000_000));
        assert_eq!(result.limit_outcome.timeout_ms, Some(1_000));
        assert_eq!(result.limit_outcome.limit_exceeded, Some(false));
    }

    #[test]
    fn custom_fuel_is_reflected_in_result() {
        let result = run_embedded_wasm_add_with_limits(
            2,
            3,
            &WasmRuntimeLimits {
                fuel: Some(100_000),
                timeout_ms: Some(1_000),
            },
        )
        .expect("embedded add should succeed");

        assert_eq!(result.status, "executed");
        assert_eq!(result.result_i32, Some(5));
        assert_eq!(result.limits.fuel, Some(100_000));
        assert_eq!(result.limit_outcome.fuel_configured, Some(100_000));
        assert_eq!(result.limit_outcome.timeout_ms, Some(1_000));
        assert_eq!(result.limit_outcome.limit_exceeded, Some(false));
    }

    #[test]
    fn custom_timeout_is_reflected_in_result() {
        let result = run_embedded_wasm_add_with_limits(
            2,
            3,
            &WasmRuntimeLimits {
                fuel: Some(100_000),
                timeout_ms: Some(250),
            },
        )
        .expect("embedded add should succeed");

        assert_eq!(result.status, "executed");
        assert_eq!(result.limits.timeout_ms, Some(250));
        assert_eq!(result.limit_outcome.timeout_ms, Some(250));
        assert_eq!(result.limit_outcome.limit_exceeded, Some(false));
    }

    #[test]
    fn zero_fuel_fails_safely() {
        let result = run_embedded_wasm_add_with_limits(
            2,
            3,
            &WasmRuntimeLimits {
                fuel: Some(0),
                timeout_ms: Some(1_000),
            },
        )
        .expect("zero fuel should return a safe result");

        assert_eq!(result.status, "failed");
        assert_eq!(result.result_i32, None);
        assert!(!result.safe_to_auto_execute);
        assert!(!result.side_effects_performed);
        assert!(!result.filesystem_access);
        assert!(!result.network_access);
        assert!(!result.host_command_access);
        assert!(!result.docker_access);
        assert_eq!(result.limit_outcome.limit_exceeded, Some(true));
        assert_eq!(result.limit_outcome.fuel_configured, Some(0));
    }

    #[test]
    fn invalid_function_name_returns_failed_result() {
        let result = run_embedded_wasm_operation(
            &WasmRuntimeInput {
                module_name: "embedded:add".to_string(),
                function_name: "missing".to_string(),
                args: vec![2, 3],
            },
            &WasmRuntimeLimits::default(),
        )
        .expect("invalid function name should return a safe result");

        assert_eq!(result.status, "failed");
        assert_eq!(result.result_i32, None);
        assert!(!result.side_effects_performed);
    }

    #[test]
    fn missing_args_returns_blocked_result() {
        let result = run_embedded_wasm_operation(
            &WasmRuntimeInput {
                module_name: "embedded:add".to_string(),
                function_name: "add".to_string(),
                args: vec![2],
            },
            &WasmRuntimeLimits::default(),
        )
        .expect("missing args should return a safe result");

        assert_eq!(result.status, "blocked");
        assert_eq!(result.result_i32, None);
        assert!(!result.safe_to_auto_execute);
    }

    #[test]
    fn source_guard_confirms_no_host_access_or_installation_integration() {
        let source = include_str!("runner.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section should exist");

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
            "TcpStream",
            "UdpSocket",
            "reqwest",
            "installation_tasks",
            "assisted_execution",
            "Module::from_file",
            ".wasm",
            ".wat",
            "fs::read",
            "read_to_end",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "wasm runtime must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
