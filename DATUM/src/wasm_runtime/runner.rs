use anyhow::{Context, Result};
use wasmtime::{Engine, Instance, Module, Store, TypedFunc};

use crate::wasm_runtime::{
    report::{build_wasm_runtime_report_from_result, WasmRuntimeReport},
    WasmRuntimeInput, WasmRuntimeResult,
};

const EMBEDDED_ADD_WAT: &str = r#"
(module
  (func (export "add") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.add))
"#;

pub fn run_embedded_wasm_add(a: i32, b: i32) -> Result<WasmRuntimeResult> {
    let input = WasmRuntimeInput {
        module_name: "embedded:add".to_string(),
        function_name: "add".to_string(),
        args: vec![a, b],
    };
    run_embedded_wasm_operation(&input)
}

pub fn run_embedded_wasm_add_report(a: i32, b: i32) -> Result<WasmRuntimeReport> {
    let result = run_embedded_wasm_add(a, b)?;
    Ok(build_wasm_runtime_report_from_result(&result))
}

pub fn run_embedded_wasm_operation(input: &WasmRuntimeInput) -> Result<WasmRuntimeResult> {
    if input.args.len() != 2 {
        return Ok(blocked_result(
            input,
            "embedded WASM add test requires exactly two i32 arguments",
        ));
    }

    if input.function_name != "add" {
        return Ok(failed_result(
            input,
            format!(
                "embedded WASM module does not export function '{}'",
                input.function_name
            ),
        ));
    }

    let engine = Engine::default();
    let module_bytes = wat::parse_str(EMBEDDED_ADD_WAT).context("compile embedded WAT")?;
    let module = Module::new(&engine, module_bytes).context("build embedded WASM module")?;
    let mut store = Store::new(&engine, ());
    let instance =
        Instance::new(&mut store, &module, &[]).context("instantiate embedded WASM module")?;
    let add: TypedFunc<(i32, i32), i32> = instance
        .get_typed_func(&mut store, &input.function_name)
        .context("resolve exported WASM function")?;

    let value = add
        .call(&mut store, (input.args[0], input.args[1]))
        .context("call embedded WASM add function")?;

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
    })
}

fn blocked_result(input: &WasmRuntimeInput, message: &str) -> WasmRuntimeResult {
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
    }
}

fn failed_result(input: &WasmRuntimeInput, message: String) -> WasmRuntimeResult {
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
    }
}

#[cfg(test)]
mod tests {
    use super::{run_embedded_wasm_add, run_embedded_wasm_add_report, run_embedded_wasm_operation};
    use crate::wasm_runtime::WasmRuntimeInput;

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
    fn invalid_function_name_returns_failed_result() {
        let result = run_embedded_wasm_operation(&WasmRuntimeInput {
            module_name: "embedded:add".to_string(),
            function_name: "missing".to_string(),
            args: vec![2, 3],
        })
        .expect("invalid function name should return a safe result");

        assert_eq!(result.status, "failed");
        assert_eq!(result.result_i32, None);
        assert!(!result.side_effects_performed);
    }

    #[test]
    fn missing_args_returns_blocked_result() {
        let result = run_embedded_wasm_operation(&WasmRuntimeInput {
            module_name: "embedded:add".to_string(),
            function_name: "add".to_string(),
            args: vec![2],
        })
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
        ] {
            assert!(
                !implementation.contains(forbidden),
                "wasm runtime must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
