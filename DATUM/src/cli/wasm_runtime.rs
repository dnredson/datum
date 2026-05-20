use anyhow::{anyhow, Result};

use crate::wasm_runtime::{runner::run_embedded_wasm_add, WasmRuntimeResult};

pub fn run_wasm_test_command(add_args: &[i32], json_output: bool) -> Result<()> {
    if add_args.len() != 2 {
        return Err(anyhow!("--add requires exactly two i32 arguments"));
    }

    let result = run_embedded_wasm_add(add_args[0], add_args[1])?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&result)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return Ok(());
    }

    print!("{}", format_wasm_test_result(&result));
    Ok(())
}

fn format_wasm_test_result(result: &WasmRuntimeResult) -> String {
    let mut lines = vec![
        "WASM runtime v0 test".to_string(),
        format!("  module: {}", result.module_name),
        format!("  function: {}", result.function_name),
        format!("  status: {}", result.status),
    ];

    if let Some(value) = result.result_i32 {
        lines.push(format!("  result: {}", value));
    }
    if let Some(message) = result.message.as_deref() {
        lines.push(format!("  message: {}", message));
    }

    lines.push(format!(
        "  safe to auto execute: {}",
        result.safe_to_auto_execute
    ));
    lines.push(format!(
        "  side effects performed: {}",
        result.side_effects_performed
    ));
    lines.push(format!("  filesystem access: {}", result.filesystem_access));
    lines.push(format!("  network access: {}", result.network_access));
    lines.push(format!(
        "  host command access: {}",
        result.host_command_access
    ));
    lines.push(format!("  docker access: {}", result.docker_access));
    lines.push(String::new());
    lines.push(
        "Note: this WASM test is isolated and not connected to installation tasks or assisted execution."
            .to_string(),
    );

    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::format_wasm_test_result;
    use crate::wasm_runtime::WasmRuntimeResult;

    #[test]
    fn cli_formatting_includes_isolated_note_and_safety_flags() {
        let rendered = format_wasm_test_result(&WasmRuntimeResult {
            module_name: "embedded:add".to_string(),
            function_name: "add".to_string(),
            status: "executed".to_string(),
            result_i32: Some(5),
            message: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
            filesystem_access: false,
            network_access: false,
            host_command_access: false,
            docker_access: false,
        });

        assert!(rendered.contains("WASM runtime v0 test"));
        assert!(rendered.contains("result: 5"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("filesystem access: false"));
        assert!(rendered.contains("network access: false"));
        assert!(rendered.contains("host command access: false"));
        assert!(rendered.contains("docker access: false"));
        assert!(rendered.contains(
            "Note: this WASM test is isolated and not connected to installation tasks or assisted execution."
        ));
    }
}
