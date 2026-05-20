use anyhow::{anyhow, Result};
use serde_json::json;

use crate::{
    config::{self, Config},
    wasm_runtime::{
        client::{
            fetch_latest_wasm_runtime_report, fetch_latest_wasm_runtime_reports_for_project,
            publish_wasm_runtime_report,
        },
        report::{
            validate_local_wasm_runtime_report_publish_invariants, WasmRuntimeReport,
            WasmRuntimeReportList,
        },
        runner::run_embedded_wasm_add_report,
    },
};

enum WasmTestMode {
    LocalAdd,
    PublishAdd,
    LatestReport,
    LatestReports,
}

pub fn run_wasm_test_command(
    config_path: &str,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    add_args: &[i32],
    publish_report: bool,
    latest_report: bool,
    latest_reports: bool,
    json_output: bool,
) -> Result<()> {
    let mode = validate_wasm_test_mode(add_args, publish_report, latest_report, latest_reports)?;

    match mode {
        WasmTestMode::LocalAdd => {
            let report = build_local_add_report(add_args)?;
            print_local_add_output(&report, json_output);
            Ok(())
        }
        WasmTestMode::PublishAdd => {
            let report = build_local_add_report(add_args)?;
            validate_local_wasm_runtime_report_publish_invariants(&report)?;

            let (base_url, project_id, node_id) = resolve_server_runtime_context(
                config_path,
                node_id_override,
                project_id_override,
                url_override,
            )?;

            let server_report =
                publish_wasm_runtime_report(&base_url, &project_id, &node_id, &report)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "local_report": report,
                        "server_report": server_report,
                        "published": true,
                        "side_effects_performed": false,
                    }))
                    .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_wasm_test_report(&report));
            print!("{}", format_published_report(&server_report));
            println!();
            println!("Note: this command published an isolated WASM runtime test report only. It did not execute installation and is not connected to assisted execution.");
            Ok(())
        }
        WasmTestMode::LatestReport => {
            let (base_url, project_id, node_id) = resolve_server_runtime_context(
                config_path,
                node_id_override,
                project_id_override,
                url_override,
            )?;
            let report = fetch_latest_wasm_runtime_report(&base_url, &project_id, &node_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_latest_report(&report));
            Ok(())
        }
        WasmTestMode::LatestReports => {
            let (base_url, project_id) =
                resolve_project_server_context(config_path, project_id_override, url_override)?;
            let report_list =
                fetch_latest_wasm_runtime_reports_for_project(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report_list)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_latest_report_list(&report_list));
            Ok(())
        }
    }
}

fn build_local_add_report(add_args: &[i32]) -> Result<WasmRuntimeReport> {
    if add_args.len() != 2 {
        return Err(anyhow!("--add requires exactly two i32 arguments"));
    }

    run_embedded_wasm_add_report(add_args[0], add_args[1])
}

fn validate_wasm_test_mode(
    add_args: &[i32],
    publish_report: bool,
    latest_report: bool,
    latest_reports: bool,
) -> Result<WasmTestMode> {
    let latest_count = latest_report as u8 + latest_reports as u8;
    if latest_count > 1 {
        return Err(anyhow!(
            "--latest-report and --latest-reports cannot be combined"
        ));
    }

    if publish_report && latest_count > 0 {
        return Err(anyhow!(
            "--publish-report cannot be combined with latest-report modes"
        ));
    }

    if publish_report {
        if add_args.len() != 2 {
            return Err(anyhow!(
                "--publish-report requires --add with exactly two i32 arguments"
            ));
        }
        return Ok(WasmTestMode::PublishAdd);
    }

    if latest_report {
        if !add_args.is_empty() {
            return Err(anyhow!("--latest-report cannot be combined with --add"));
        }
        return Ok(WasmTestMode::LatestReport);
    }

    if latest_reports {
        if !add_args.is_empty() {
            return Err(anyhow!("--latest-reports cannot be combined with --add"));
        }
        return Ok(WasmTestMode::LatestReports);
    }

    if add_args.len() == 2 {
        return Ok(WasmTestMode::LocalAdd);
    }

    Err(anyhow!(
        "one mode is required: --add A B, --add A B --publish-report, --latest-report, or --latest-reports"
    ))
}

fn resolve_server_runtime_context(
    config_path: &str,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
) -> Result<(String, String, String)> {
    if let (Some(url), Some(project_id), Some(node_id)) =
        (url_override, project_id_override, node_id_override)
    {
        return Ok((url.to_string(), project_id.to_string(), node_id.to_string()));
    }

    let cfg = config::load_config(config_path)?;
    let base_url = resolve_base_url(&cfg, url_override)?;
    let project_id = resolve_project_id(&cfg, project_id_override)?;
    let node_id = resolve_node_id(&cfg, node_id_override)?;
    Ok((base_url, project_id, node_id))
}

fn resolve_project_server_context(
    config_path: &str,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
) -> Result<(String, String)> {
    if let (Some(url), Some(project_id)) = (url_override, project_id_override) {
        return Ok((url.to_string(), project_id.to_string()));
    }

    let cfg = config::load_config(config_path)?;
    let base_url = resolve_base_url(&cfg, url_override)?;
    let project_id = resolve_project_id(&cfg, project_id_override)?;
    Ok((base_url, project_id))
}

fn resolve_node_id(cfg: &Config, node_id_override: Option<&str>) -> Result<String> {
    node_id_override
        .map(str::to_string)
        .or_else(|| {
            let trimmed = cfg.agent.node_id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .ok_or_else(|| anyhow!("node_id is required in --node-id or agent.node_id"))
}

fn resolve_project_id(cfg: &Config, project_id_override: Option<&str>) -> Result<String> {
    project_id_override
        .map(str::to_string)
        .or_else(|| cfg.dispatch.as_ref().and_then(|dispatch| dispatch.project_id.clone()))
        .or_else(|| cfg.outputs.http.as_ref().and_then(|http| http.project_id.clone()))
        .ok_or_else(|| {
            anyhow!(
                "project_id is required in --project-id, dispatch.project_id, or outputs.http.project_id"
            )
        })
}

fn resolve_base_url(cfg: &Config, url_override: Option<&str>) -> Result<String> {
    url_override
        .map(str::to_string)
        .or_else(|| {
            cfg.outputs
                .http
                .as_ref()
                .filter(|http| http.enabled)
                .map(|http| http.url.clone())
        })
        .ok_or_else(|| {
            anyhow!("outputs.http.enabled=true or --url is required for WASM report API access")
        })
}

fn print_local_add_output(report: &WasmRuntimeReport, json_output: bool) {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(report)
                .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
        );
        return;
    }

    print!("{}", format_wasm_test_report(report));
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

    append_safety_lines(&mut lines, report);
    lines.push(String::new());
    lines.push(
        "Note: this WASM report is local-only and was not published to the server.".to_string(),
    );

    lines.join("\n") + "\n"
}

fn format_published_report(report: &WasmRuntimeReport) -> String {
    let mut lines = vec!["Published WASM runtime report".to_string()];
    if let Some(report_id) = report.report_id.as_deref() {
        lines.push(format!("  report_id: {}", report_id));
    }
    if let Some(node_id) = report.node_id.as_deref() {
        lines.push(format!("  node_id: {}", node_id));
    }
    lines.push(format!("  status: {}", report.status));
    if let Some(value) = report.result_i32 {
        lines.push(format!("  result: {}", value));
    }
    lines.push(format!(
        "  safe to auto execute: {}",
        report.safe_to_auto_execute
    ));
    lines.push(format!(
        "  side effects performed: {}",
        report.side_effects_performed
    ));

    lines.join("\n") + "\n"
}

fn format_latest_report(report: &WasmRuntimeReport) -> String {
    let mut lines = vec!["Latest WASM runtime report".to_string()];
    if let Some(project_id) = report.project_id.as_deref() {
        lines.push(format!("  project: {}", project_id));
    }
    if let Some(node_id) = report.node_id.as_deref() {
        lines.push(format!("  node: {}", node_id));
    }
    if let Some(report_id) = report.report_id.as_deref() {
        lines.push(format!("  report_id: {}", report_id));
    }
    lines.push(format!("  module: {}", report.module_name));
    lines.push(format!("  function: {}", report.function_name));
    lines.push(format!("  mode: {}", report.execution_mode));
    lines.push(format!("  status: {}", report.status));
    if let Some(value) = report.result_i32 {
        lines.push(format!("  result: {}", value));
    }
    append_safety_lines(&mut lines, report);
    lines.join("\n") + "\n"
}

fn format_latest_report_list(report_list: &WasmRuntimeReportList) -> String {
    let mut lines = vec!["Latest WASM runtime reports".to_string()];
    if let Some(project_id) = report_list.project_id.as_deref() {
        lines.push(format!("  project: {}", project_id));
    }
    lines.push(format!("  count: {}", report_list.count));

    if report_list.reports.is_empty() {
        lines.push("  reports: none".to_string());
        return lines.join("\n") + "\n";
    }

    for report in &report_list.reports {
        lines.push(String::new());
        let node_id = report.node_id.as_deref().unwrap_or("unknown");
        lines.push(format!("- node: {}", node_id));
        if let Some(report_id) = report.report_id.as_deref() {
            lines.push(format!("  report_id: {}", report_id));
        }
        lines.push(format!("  module: {}", report.module_name));
        lines.push(format!("  function: {}", report.function_name));
        lines.push(format!("  status: {}", report.status));
        if let Some(value) = report.result_i32 {
            lines.push(format!("  result: {}", value));
        }
    }

    lines.join("\n") + "\n"
}

fn append_safety_lines(lines: &mut Vec<String>, report: &WasmRuntimeReport) {
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
}

#[cfg(test)]
mod tests {
    use super::{
        format_wasm_test_report, resolve_base_url, resolve_project_id, validate_wasm_test_mode,
        WasmTestMode,
    };
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, FileJsonConfig, HttpConfig, OutputsConfig},
        wasm_runtime::report::WasmRuntimeReport,
    };

    fn sample_config() -> Config {
        Config {
            agent: AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: OutputsConfig {
                file_json: Some(FileJsonConfig {
                    enabled: true,
                    path: "snapshot.json".to_string(),
                }),
                http: Some(HttpConfig {
                    enabled: true,
                    url: "http://localhost:8080/api/v1/snapshots".to_string(),
                    project_id: Some("smart-city-campinas".to_string()),
                    timeout_s: Some(10),
                }),
            },
            dispatch: Some(DispatchConfig {
                project_id: Some("dispatch-project".to_string()),
                ..DispatchConfig::default()
            }),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn cli_formatting_includes_local_only_note_and_access_flags() {
        let rendered = format_wasm_test_report(&WasmRuntimeReport {
            project_id: None,
            report_id: Some("wasm-report-1".to_string()),
            node_id: None,
            reported_at_utc: None,
            source: Some("client".to_string()),
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

    #[test]
    fn cli_mode_validation_accepts_local_and_publish_modes() {
        assert!(matches!(
            validate_wasm_test_mode(&[2, 3], false, false, false).expect("mode should validate"),
            WasmTestMode::LocalAdd
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[2, 3], true, false, false).expect("mode should validate"),
            WasmTestMode::PublishAdd
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[], false, true, false).expect("mode should validate"),
            WasmTestMode::LatestReport
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[], false, false, true).expect("mode should validate"),
            WasmTestMode::LatestReports
        ));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        assert!(validate_wasm_test_mode(&[2, 3], true, true, false).is_err());
        assert!(validate_wasm_test_mode(&[2, 3], false, true, false).is_err());
        assert!(validate_wasm_test_mode(&[], false, true, true).is_err());
        assert!(validate_wasm_test_mode(&[2], true, false, false).is_err());
    }

    #[test]
    fn local_wasm_test_without_publish_does_not_require_server_config_helpers() {
        assert!(matches!(
            validate_wasm_test_mode(&[2, 3], false, false, false).expect("mode should validate"),
            WasmTestMode::LocalAdd
        ));
    }

    #[test]
    fn resolve_project_id_prefers_override() {
        let cfg = sample_config();
        let project_id =
            resolve_project_id(&cfg, Some("override-project")).expect("project id should resolve");
        assert_eq!(project_id, "override-project");
    }

    #[test]
    fn resolve_base_url_uses_output_http() {
        let cfg = sample_config();
        let url = resolve_base_url(&cfg, None).expect("base URL should resolve");
        assert_eq!(url, "http://localhost:8080/api/v1/snapshots");
    }

    #[test]
    fn source_guard_confirms_no_installation_or_assisted_execution_references() {
        let source = include_str!("wasm_runtime.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section should exist");

        for forbidden in [
            "installation_tasks::",
            "assisted_execution::",
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "wasm runtime CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
