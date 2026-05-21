use anyhow::{anyhow, Result};
use serde_json::json;

use crate::{
    config::{self, Config},
    wasm_runtime::{
        client::{
            fetch_latest_wasm_runtime_report, fetch_latest_wasm_runtime_reports_for_project,
            fetch_wasm_module, fetch_wasm_modules, publish_wasm_runtime_report,
        },
        registry::{
            WasmFunctionDescriptor, WasmModuleDescriptor, WasmModuleDescriptorList,
            WasmModuleSafetyMetadata,
        },
        report::{
            validate_local_wasm_runtime_report_publish_invariants, WasmRuntimeReport,
            WasmRuntimeReportList,
        },
        runner::run_embedded_wasm_add_report_with_limits,
        WasmRuntimeLimits,
    },
};

enum WasmTestMode {
    LocalAdd,
    PublishAdd,
    LatestReport,
    LatestReports,
    Modules,
    Module(String),
}

pub fn run_wasm_test_command(
    config_path: &str,
    node_id_override: Option<&str>,
    project_id_override: Option<&str>,
    url_override: Option<&str>,
    add_args: &[i32],
    fuel: Option<u64>,
    timeout_ms: Option<u64>,
    publish_report: bool,
    latest_report: bool,
    latest_reports: bool,
    modules: bool,
    module_id: Option<&str>,
    json_output: bool,
) -> Result<()> {
    let mode = validate_wasm_test_mode(
        add_args,
        publish_report,
        latest_report,
        latest_reports,
        modules,
        module_id,
    )?;
    let limits = WasmRuntimeLimits {
        fuel: fuel.or(WasmRuntimeLimits::default().fuel),
        timeout_ms: timeout_ms.or(WasmRuntimeLimits::default().timeout_ms),
    };

    match mode {
        WasmTestMode::LocalAdd => {
            let report = build_local_add_report(add_args, &limits)?;
            print_local_add_output(&report, json_output);
            Ok(())
        }
        WasmTestMode::PublishAdd => {
            let report = build_local_add_report(add_args, &limits)?;
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
        WasmTestMode::Modules => {
            let (base_url, project_id) =
                resolve_project_server_context(config_path, project_id_override, url_override)?;
            let module_list = fetch_wasm_modules(&base_url, &project_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&module_list)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_module_list(&module_list));
            Ok(())
        }
        WasmTestMode::Module(module_id) => {
            let (base_url, project_id) =
                resolve_project_server_context(config_path, project_id_override, url_override)?;
            let module = fetch_wasm_module(&base_url, &project_id, &module_id)?;

            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&module)
                        .unwrap_or_else(|_| "{\"error\":\"failed to render JSON\"}".to_string())
                );
                return Ok(());
            }

            print!("{}", format_module_descriptor(&module));
            Ok(())
        }
    }
}

fn build_local_add_report(
    add_args: &[i32],
    limits: &WasmRuntimeLimits,
) -> Result<WasmRuntimeReport> {
    if add_args.len() != 2 {
        return Err(anyhow!("--add requires exactly two i32 arguments"));
    }

    run_embedded_wasm_add_report_with_limits(add_args[0], add_args[1], limits)
}

fn validate_wasm_test_mode(
    add_args: &[i32],
    publish_report: bool,
    latest_report: bool,
    latest_reports: bool,
    modules: bool,
    module_id: Option<&str>,
) -> Result<WasmTestMode> {
    let query_count =
        latest_report as u8 + latest_reports as u8 + modules as u8 + module_id.is_some() as u8;
    if query_count > 1 {
        return Err(anyhow!(
            "--latest-report, --latest-reports, --modules, and --module cannot be combined"
        ));
    }

    if publish_report && query_count > 0 {
        return Err(anyhow!(
            "--publish-report cannot be combined with registry or latest-report modes"
        ));
    }

    if modules && module_id.is_some() {
        return Err(anyhow!("--modules and --module cannot be combined"));
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

    if modules {
        if !add_args.is_empty() {
            return Err(anyhow!("--modules cannot be combined with --add"));
        }
        return Ok(WasmTestMode::Modules);
    }

    if let Some(module_id) = module_id {
        if !add_args.is_empty() {
            return Err(anyhow!("--module cannot be combined with --add"));
        }
        if module_id.trim().is_empty() {
            return Err(anyhow!("--module requires a non-empty module identifier"));
        }
        return Ok(WasmTestMode::Module(module_id.trim().to_string()));
    }

    if add_args.len() == 2 {
        return Ok(WasmTestMode::LocalAdd);
    }

    Err(anyhow!(
        "one mode is required: --add A B, --add A B --publish-report, --latest-report, --latest-reports, --modules, or --module <id>"
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

    append_limit_lines(&mut lines, report);
    append_safety_lines(&mut lines, report);
    lines.push(String::new());
    lines.push(
        "Note: this WASM test is isolated, local-only, and not connected to installation tasks or assisted execution.".to_string(),
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
    append_limit_lines(&mut lines, report);
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
    append_limit_lines(&mut lines, report);
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
        append_limit_lines(&mut lines, report);
    }

    lines.join("\n") + "\n"
}

fn format_module_list(module_list: &WasmModuleDescriptorList) -> String {
    let mut lines = vec!["WASM module registry".to_string()];
    lines.push(format!(
        "Project: {}",
        module_list.project_id.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "Modules: {}",
        module_list
            .count
            .unwrap_or(module_list.modules.len() as u64)
    ));

    if module_list.modules.is_empty() {
        lines.push(String::new());
        lines.push("No registry modules found.".to_string());
    } else {
        for module in &module_list.modules {
            lines.push(String::new());
            lines.push(format!(
                "[{}] {}",
                uppercase_status(module.status.as_deref()),
                module.module_id.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "  name: {}",
                module.module_name.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "  kind: {}",
                module.module_kind.as_deref().unwrap_or("unknown")
            ));
            if let Some(version) = module.module_version.as_deref() {
                lines.push(format!("  version: {}", version));
            }
            lines.push(format!(
                "  functions: {}",
                format_module_functions_inline(&module.allowed_functions)
            ));
            append_module_safety_lines(&mut lines, module.safety.as_ref());
        }
    }

    lines.push(String::new());
    lines.push("Note: registry entries are metadata only. This command did not download artifacts or execute WASM modules.".to_string());
    lines.join("\n") + "\n"
}

fn format_module_descriptor(module: &WasmModuleDescriptor) -> String {
    let mut lines = vec![format!(
        "WASM module descriptor: {}",
        module.module_id.as_deref().unwrap_or("unknown")
    )];
    lines.push(format!(
        "  name: {}",
        module.module_name.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "  kind: {}",
        module.module_kind.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "  status: {}",
        module.status.as_deref().unwrap_or("unknown")
    ));
    if let Some(version) = module.module_version.as_deref() {
        lines.push(format!("  version: {}", version));
    }
    if let Some(artifact_ref) = module.artifact_ref.as_ref() {
        lines.push(format!(
            "  artifact uri: {}",
            artifact_ref
                .artifact_uri
                .as_deref()
                .map(|_| "metadata only")
                .unwrap_or("none")
        ));
        if let Some(sha256) = artifact_ref.sha256.as_deref() {
            lines.push(format!("  sha256: {}", sha256));
        }
    }
    append_module_safety_lines(&mut lines, module.safety.as_ref());

    lines.push(String::new());
    lines.push("Functions:".to_string());
    if module.allowed_functions.is_empty() {
        lines.push("  none".to_string());
    } else {
        for function in &module.allowed_functions {
            lines.push(format!(
                "  - {}",
                function.function_name.as_deref().unwrap_or("unknown")
            ));
            lines.push(format!(
                "    modes: {}",
                if function.allowed_execution_modes.is_empty() {
                    "none".to_string()
                } else {
                    function.allowed_execution_modes.join(", ")
                }
            ));
            if let Some(max_fuel) = function.max_fuel {
                lines.push(format!("    max fuel: {}", max_fuel));
            }
            if let Some(timeout_ms) = function.timeout_ms {
                lines.push(format!("    timeout ms: {}", timeout_ms));
            }
        }
    }

    lines.push(String::new());
    lines.push("Note: artifact references are metadata only. This command did not fetch artifacts or execute modules.".to_string());
    lines.join("\n") + "\n"
}

fn append_limit_lines(lines: &mut Vec<String>, report: &WasmRuntimeReport) {
    if let Some(value) = metadata_u64(report, &["limits", "fuel_configured"]) {
        lines.push(format!("  fuel configured: {}", value));
    }
    if let Some(value) = metadata_u64(report, &["limits", "fuel_consumed"]) {
        lines.push(format!("  fuel consumed: {}", value));
    }
    if let Some(value) = metadata_u64(report, &["limits", "timeout_ms"]) {
        lines.push(format!("  timeout ms: {}", value));
    }
    if let Some(value) = metadata_bool(report, &["limits", "limit_exceeded"]) {
        lines.push(format!("  limit exceeded: {}", value));
    }
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

fn append_module_safety_lines(lines: &mut Vec<String>, safety: Option<&WasmModuleSafetyMetadata>) {
    lines.push(format!(
        "  safe to auto execute: {}",
        format_optional_bool(safety.and_then(|value| value.safe_to_auto_execute))
    ));
    lines.push(format!(
        "  side effects allowed: {}",
        format_optional_bool(safety.and_then(|value| value.side_effects_allowed))
    ));
    lines.push(format!(
        "  filesystem access: {}",
        format_optional_bool(safety.and_then(|value| value.filesystem_access))
    ));
    lines.push(format!(
        "  network access: {}",
        format_optional_bool(safety.and_then(|value| value.network_access))
    ));
    lines.push(format!(
        "  host command access: {}",
        format_optional_bool(safety.and_then(|value| value.host_command_access))
    ));
    lines.push(format!(
        "  docker access: {}",
        format_optional_bool(safety.and_then(|value| value.docker_access))
    ));
    lines.push(format!(
        "  artifact access: {}",
        format_optional_bool(safety.and_then(|value| value.artifact_access))
    ));
    lines.push(format!(
        "  installation task access: {}",
        format_optional_bool(safety.and_then(|value| value.installation_task_access))
    ));
    lines.push(format!(
        "  assisted execution access: {}",
        format_optional_bool(safety.and_then(|value| value.assisted_execution_access))
    ));
}

fn metadata_u64(report: &WasmRuntimeReport, path: &[&str]) -> Option<u64> {
    metadata_value(report, path).and_then(|value| value.as_u64())
}

fn metadata_bool(report: &WasmRuntimeReport, path: &[&str]) -> Option<bool> {
    metadata_value(report, path).and_then(|value| value.as_bool())
}

fn metadata_value<'a>(
    report: &'a WasmRuntimeReport,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = report.metadata.as_ref()?;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}

fn format_module_functions_inline(functions: &[WasmFunctionDescriptor]) -> String {
    if functions.is_empty() {
        return "none".to_string();
    }

    functions
        .iter()
        .map(|function| function.function_name.as_deref().unwrap_or("unknown"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn uppercase_status(status: Option<&str>) -> String {
    status.unwrap_or("unknown").to_uppercase()
}

fn format_optional_bool(value: Option<bool>) -> String {
    value
        .map(|flag| flag.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        format_module_descriptor, format_module_list, format_wasm_test_report, resolve_base_url,
        resolve_project_id, validate_wasm_test_mode, WasmTestMode,
    };
    use crate::{
        config::{AgentConfig, Config, DispatchConfig, FileJsonConfig, HttpConfig, OutputsConfig},
        wasm_runtime::{
            registry::{
                WasmArtifactRef, WasmFunctionDescriptor, WasmModuleDescriptor,
                WasmModuleDescriptorList, WasmModuleSafetyMetadata,
            },
            report::WasmRuntimeReport,
        },
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
            metadata: Some(serde_json::json!({
                "local_only": true,
                "published_to_server": false,
                "limits": {
                    "fuel_configured": 1000000,
                    "fuel_consumed": 3,
                    "timeout_ms": 1000,
                    "limit_exceeded": false
                }
            })),
        });

        assert!(rendered.contains("WASM runtime v0 report"));
        assert!(rendered.contains("mode: embedded_test"));
        assert!(rendered.contains("result: 5"));
        assert!(rendered.contains("fuel configured: 1000000"));
        assert!(rendered.contains("fuel consumed: 3"));
        assert!(rendered.contains("timeout ms: 1000"));
        assert!(rendered.contains("limit exceeded: false"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects performed: false"));
        assert!(rendered.contains("filesystem access: false"));
        assert!(rendered.contains("network access: false"));
        assert!(rendered.contains("host command access: false"));
        assert!(rendered.contains("docker access: false"));
        assert!(rendered.contains("artifact access: false"));
        assert!(rendered.contains("installation task access: false"));
        assert!(rendered.contains("assisted execution access: false"));
        assert!(rendered.contains(
            "Note: this WASM test is isolated, local-only, and not connected to installation tasks or assisted execution."
        ));
    }

    #[test]
    fn cli_mode_validation_accepts_local_and_publish_modes() {
        assert!(matches!(
            validate_wasm_test_mode(&[2, 3], false, false, false, false, None)
                .expect("mode should validate"),
            WasmTestMode::LocalAdd
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[2, 3], true, false, false, false, None)
                .expect("mode should validate"),
            WasmTestMode::PublishAdd
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[], false, true, false, false, None)
                .expect("mode should validate"),
            WasmTestMode::LatestReport
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[], false, false, true, false, None)
                .expect("mode should validate"),
            WasmTestMode::LatestReports
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[], false, false, false, true, None)
                .expect("mode should validate"),
            WasmTestMode::Modules
        ));
        assert!(matches!(
            validate_wasm_test_mode(&[], false, false, false, false, Some("embedded-add"))
                .expect("mode should validate"),
            WasmTestMode::Module(module_id) if module_id == "embedded-add"
        ));
    }

    #[test]
    fn cli_mode_validation_rejects_ambiguous_combinations() {
        assert!(validate_wasm_test_mode(&[2, 3], true, true, false, false, None).is_err());
        assert!(validate_wasm_test_mode(&[2, 3], false, true, false, false, None).is_err());
        assert!(validate_wasm_test_mode(&[], false, true, true, false, None).is_err());
        assert!(validate_wasm_test_mode(&[2], true, false, false, false, None).is_err());
        assert!(validate_wasm_test_mode(&[2, 3], false, false, false, true, None).is_err());
        assert!(
            validate_wasm_test_mode(&[2, 3], false, false, false, false, Some("embedded-add"))
                .is_err()
        );
        assert!(
            validate_wasm_test_mode(&[], false, false, false, true, Some("embedded-add")).is_err()
        );
    }

    #[test]
    fn local_wasm_test_without_publish_does_not_require_server_config_helpers() {
        assert!(matches!(
            validate_wasm_test_mode(&[2, 3], false, false, false, false, None)
                .expect("mode should validate"),
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
    fn formatting_module_list_includes_registry_fields_and_metadata_only_note() {
        let rendered = format_module_list(&WasmModuleDescriptorList {
            project_id: Some("smart-city-campinas".to_string()),
            count: Some(1),
            modules: vec![sample_module_descriptor()],
        });

        assert!(rendered.contains("WASM module registry"));
        assert!(rendered.contains("Project: smart-city-campinas"));
        assert!(rendered.contains("[REGISTERED] embedded-add"));
        assert!(rendered.contains("name: embedded:add"));
        assert!(rendered.contains("kind: embedded_test"));
        assert!(rendered.contains("functions: add"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects allowed: false"));
        assert!(rendered.contains("filesystem access: false"));
        assert!(rendered.contains("network access: false"));
        assert!(rendered.contains("host command access: false"));
        assert!(rendered.contains("docker access: false"));
        assert!(rendered.contains("artifact access: false"));
        assert!(rendered.contains("installation task access: false"));
        assert!(rendered.contains("assisted execution access: false"));
        assert!(rendered.contains(
            "Note: registry entries are metadata only. This command did not download artifacts or execute WASM modules."
        ));
    }

    #[test]
    fn formatting_module_descriptor_includes_artifact_metadata_and_no_execution_note() {
        let rendered = format_module_descriptor(&sample_module_descriptor());

        assert!(rendered.contains("WASM module descriptor: embedded-add"));
        assert!(rendered.contains("name: embedded:add"));
        assert!(rendered.contains("kind: embedded_test"));
        assert!(rendered.contains("status: registered"));
        assert!(rendered.contains("artifact uri: metadata only"));
        assert!(rendered.contains("sha256: abc123"));
        assert!(rendered.contains("  - add"));
        assert!(rendered.contains("modes: embedded_test"));
        assert!(rendered.contains("max fuel: 1000000"));
        assert!(rendered.contains("timeout ms: 1000"));
        assert!(rendered.contains("safe to auto execute: false"));
        assert!(rendered.contains("side effects allowed: false"));
        assert!(rendered.contains(
            "Note: artifact references are metadata only. This command did not fetch artifacts or execute modules."
        ));
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
            "reqwest::get(",
            "artifact download",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "wasm runtime CLI must not contain forbidden pattern: {forbidden}"
            );
        }
    }

    fn sample_module_descriptor() -> WasmModuleDescriptor {
        WasmModuleDescriptor {
            project_id: Some("smart-city-campinas".to_string()),
            module_id: Some("embedded-add".to_string()),
            module_name: Some("embedded:add".to_string()),
            module_version: Some("v0".to_string()),
            module_kind: Some("embedded_test".to_string()),
            status: Some("registered".to_string()),
            allowed_functions: vec![WasmFunctionDescriptor {
                function_name: Some("add".to_string()),
                input_schema: None,
                output_schema: None,
                allowed_execution_modes: vec!["embedded_test".to_string()],
                max_fuel: Some(1_000_000),
                timeout_ms: Some(1_000),
                metadata: Some(serde_json::json!({ "note": "embedded only" })),
            }],
            artifact_ref: Some(WasmArtifactRef {
                artifact_id: Some("artifact-1".to_string()),
                artifact_uri: Some("opaque://metadata-only".to_string()),
                sha256: Some("abc123".to_string()),
                signature_ref: None,
                provenance: Some("fixture".to_string()),
                metadata: Some(serde_json::json!({ "kind": "opaque" })),
            }),
            safety: Some(WasmModuleSafetyMetadata {
                filesystem_access: Some(false),
                network_access: Some(false),
                host_command_access: Some(false),
                docker_access: Some(false),
                artifact_access: Some(false),
                installation_task_access: Some(false),
                assisted_execution_access: Some(false),
                side_effects_allowed: Some(false),
                safe_to_auto_execute: Some(false),
            }),
            created_at_utc: None,
            updated_at_utc: None,
            metadata: Some(serde_json::json!({ "note": "metadata only" })),
        }
    }
}
