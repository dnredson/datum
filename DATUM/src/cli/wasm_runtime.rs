use anyhow::{anyhow, Result};

use crate::wasm_runtime::{report::WasmRuntimeReport, runner::run_embedded_wasm_add_report};

pub fn run_wasm_test_command(add_args: &[i32], json_output: bool) -> Result<()> {
    if add_args.len() != 2 {
        return Err(anyhow!("--add requires exactly two i32 arguments"));
    }

    let report = run_embedded_wasm_add_report(add_args[0], add_args[1])?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&report)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return Ok(());
    }

    print!("{}", format_wasm_test_report(&report));
    Ok(())
}

fn format_wasm_test_report(report: &WasmRuntimeReport) -> String {
    let mut lines = vec![
        "WASM runtime v0 report".to_string(),
        format!("  module: {}", report.module_name),
        format!("  function: {}", report.function_name),
        format!("  mode: {}", report.execution_mode),
        format!("  status: {}", report.status),
    ];

    if let Some(value) = report.result_i32 {
        lines.push(format!("  result: {}", value));
    }
    if let Some(message) = report.error_message.as_deref() {
        lines.push(format!("  message: {}", message));
    }

    lines.push(format!(
        "  safe to auto execute: {}",
        report.safe_to_auto_execute
    ));
    lines.push(format!(
        "  side effects performed: {}",
        report.side_effects_performed
    ));
    lines.push(format!("  filesystem access: {}", report.filesystem_access));
    lines.push(format!("  network access: {}", report.network_access));
    lines.push(format!(
        "  host command access: {}",
        report.host_command_access
    ));
    lines.push(format!("  docker access: {}", report.docker_access));
    lines.push(format!("  artifact access: {}", report.artifact_access));
    lines.push(format!(
        "  installation task access: {}",
        report.installation_task_access
    ));
    lines.push(format!(
        "  assisted execution access: {}",
        report.assisted_execution_access
    ));
    lines.push(String::new());
    lines.push(
        "Note: this WASM report is local-only and was not published to the server.".to_string(),
    );

    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::format_wasm_test_report;
    use crate::wasm_runtime::report::WasmRuntimeReport;

    #[test]
    fn cli_formatting_includes_local_only_note_and_access_flags() {
        let rendered = format_wasm_test_report(&WasmRuntimeReport {
            report_id: Some("wasm-report-1".to_string()),
            node_id: None,
            module_name: "embedded:add".to_string(),
            function_name: "add".to_string(),
            execution_mode: "embedded_test".to_string(),
            status: "executed".to_string(),
            result_i32: Some(5),
            error_message: None,
            started_at_utc: None,
            finished_at_utc: None,
            duration_ms: Some(0),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            filesystem_access: false,
            network_access: false,
            host_command_access: false,
            docker_access: false,
            artifact_access: false,
            installation_task_access: false,
            assisted_execution_access: false,
            metadata: None,
        });

        assert!(rendered.contains("WASM runtime v0 report"));
        assert!(rendered.contains("mode: embedded_test"));
        assert!(rendered.contains("result: 5"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("filesystem access: false"));
        assert!(rendered.contains("network access: false"));
        assert!(rendered.contains("host command access: false"));
        assert!(rendered.contains("docker access: false"));
        assert!(rendered.contains("artifact access: false"));
        assert!(rendered.contains("installation task access: false"));
        assert!(rendered.contains("assisted execution access: false"));
        assert!(rendered
            .contains("Note: this WASM report is local-only and was not published to the server."));
    }
}
