use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const EXECUTION_MODE_EMBEDDED_TEST: &str = "embedded_test";
const EXECUTION_MODE_NO_OP: &str = "no_op";
const EXECUTION_MODE_DRY_RUN: &str = "dry_run";

const STATUS_EXECUTED: &str = "executed";
const STATUS_FAILED: &str = "failed";
const STATUS_BLOCKED: &str = "blocked";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmRuntimeReport {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub report_id: String,
    #[serde(default)]
    pub reported_at_utc: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub module_name: String,
    #[serde(default)]
    pub function_name: String,
    #[serde(default)]
    pub execution_mode: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub result_i32: Option<i32>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub started_at_utc: Option<String>,
    #[serde(default)]
    pub finished_at_utc: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub side_effects_performed: bool,
    #[serde(default)]
    pub filesystem_access: bool,
    #[serde(default)]
    pub network_access: bool,
    #[serde(default)]
    pub host_command_access: bool,
    #[serde(default)]
    pub docker_access: bool,
    #[serde(default)]
    pub artifact_access: bool,
    #[serde(default)]
    pub installation_task_access: bool,
    #[serde(default)]
    pub assisted_execution_access: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WasmRuntimeReportList {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub reports: Vec<WasmRuntimeReport>,
}

fn report_id_for(project_id: &str, node_id: &str) -> String {
    format!(
        "project:{}:node:{}:report:{}",
        project_id,
        node_id,
        Uuid::new_v4()
    )
}

fn is_valid_execution_mode(mode: &str) -> bool {
    matches!(
        mode,
        EXECUTION_MODE_EMBEDDED_TEST | EXECUTION_MODE_NO_OP | EXECUTION_MODE_DRY_RUN
    )
}

fn is_valid_status(status: &str) -> bool {
    matches!(status, STATUS_EXECUTED | STATUS_FAILED | STATUS_BLOCKED)
}

pub fn normalize_wasm_runtime_report(
    project_id: &str,
    node_id: &str,
    mut input: WasmRuntimeReport,
) -> WasmRuntimeReport {
    input.project_id = project_id.to_string();
    input.node_id = node_id.to_string();
    input.report_id = report_id_for(project_id, node_id);
    input.reported_at_utc = Utc::now().to_rfc3339();

    if input
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        input.source = Some("client".to_string());
    }

    input
}

pub fn validate_wasm_runtime_report(report: &WasmRuntimeReport) -> Result<(), String> {
    if report.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }

    if report.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if report.report_id.trim().is_empty() {
        return Err("report_id must not be empty".to_string());
    }

    if report.reported_at_utc.trim().is_empty() {
        return Err("reported_at_utc must not be empty".to_string());
    }

    if report.module_name.trim().is_empty() {
        return Err("module_name must not be empty".to_string());
    }

    if report.function_name.trim().is_empty() {
        return Err("function_name must not be empty".to_string());
    }

    if !is_valid_execution_mode(report.execution_mode.trim()) {
        return Err(format!(
            "execution_mode is invalid: {}",
            report.execution_mode
        ));
    }

    if !is_valid_status(report.status.trim()) {
        return Err(format!("status is invalid: {}", report.status));
    }

    if report.safe_to_auto_execute {
        return Err("safe_to_auto_execute must be false".to_string());
    }

    if report.side_effects_performed {
        return Err("side_effects_performed must be false".to_string());
    }

    if report.filesystem_access {
        return Err("filesystem_access must be false".to_string());
    }

    if report.network_access {
        return Err("network_access must be false".to_string());
    }

    if report.host_command_access {
        return Err("host_command_access must be false".to_string());
    }

    if report.docker_access {
        return Err("docker_access must be false".to_string());
    }

    if report.artifact_access {
        return Err("artifact_access must be false".to_string());
    }

    if report.installation_task_access {
        return Err("installation_task_access must be false".to_string());
    }

    if report.assisted_execution_access {
        return Err("assisted_execution_access must be false".to_string());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_report() -> WasmRuntimeReport {
        WasmRuntimeReport {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            report_id: "report-1".to_string(),
            reported_at_utc: "2026-05-20T12:00:00Z".to_string(),
            source: Some("client".to_string()),
            module_name: "embedded:add".to_string(),
            function_name: "add".to_string(),
            execution_mode: EXECUTION_MODE_EMBEDDED_TEST.to_string(),
            status: STATUS_EXECUTED.to_string(),
            result_i32: Some(5),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            filesystem_access: false,
            network_access: false,
            host_command_access: false,
            docker_access: false,
            artifact_access: false,
            installation_task_access: false,
            assisted_execution_access: false,
            ..WasmRuntimeReport::default()
        }
    }

    #[test]
    fn normalization_overrides_project_id_and_node_id() {
        let normalized = normalize_wasm_runtime_report(
            "project-a",
            "node-b",
            WasmRuntimeReport {
                project_id: "wrong".to_string(),
                node_id: "wrong".to_string(),
                module_name: "embedded:add".to_string(),
                function_name: "add".to_string(),
                execution_mode: EXECUTION_MODE_EMBEDDED_TEST.to_string(),
                status: STATUS_EXECUTED.to_string(),
                ..WasmRuntimeReport::default()
            },
        );

        assert_eq!(normalized.project_id, "project-a");
        assert_eq!(normalized.node_id, "node-b");
        assert!(!normalized.report_id.is_empty());
        assert!(!normalized.reported_at_utc.is_empty());
    }

    #[test]
    fn normalization_sets_source_to_client_when_missing() {
        let normalized = normalize_wasm_runtime_report(
            "project-a",
            "node-b",
            WasmRuntimeReport {
                module_name: "embedded:add".to_string(),
                function_name: "add".to_string(),
                execution_mode: EXECUTION_MODE_EMBEDDED_TEST.to_string(),
                status: STATUS_EXECUTED.to_string(),
                ..WasmRuntimeReport::default()
            },
        );

        assert_eq!(normalized.source.as_deref(), Some("client"));
    }

    #[test]
    fn validation_accepts_valid_embedded_test_executed_report() {
        assert!(validate_wasm_runtime_report(&base_report()).is_ok());
    }

    #[test]
    fn validation_accepts_failed_report_with_error_message() {
        let mut report = base_report();
        report.status = STATUS_FAILED.to_string();
        report.result_i32 = None;
        report.error_message = Some("runtime trap".to_string());

        assert!(validate_wasm_runtime_report(&report).is_ok());
    }

    #[test]
    fn validation_rejects_invalid_execution_mode() {
        let mut report = base_report();
        report.execution_mode = "explode".to_string();
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_invalid_status() {
        let mut report = base_report();
        report.status = "explode".to_string();
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_empty_module_name() {
        let mut report = base_report();
        report.module_name = "".to_string();
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_empty_function_name() {
        let mut report = base_report();
        report.function_name = "".to_string();
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_safe_to_auto_execute_true() {
        let mut report = base_report();
        report.safe_to_auto_execute = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_side_effects_performed_true() {
        let mut report = base_report();
        report.side_effects_performed = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_filesystem_access_true() {
        let mut report = base_report();
        report.filesystem_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_network_access_true() {
        let mut report = base_report();
        report.network_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_host_command_access_true() {
        let mut report = base_report();
        report.host_command_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_docker_access_true() {
        let mut report = base_report();
        report.docker_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_artifact_access_true() {
        let mut report = base_report();
        report.artifact_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_installation_task_access_true() {
        let mut report = base_report();
        report.installation_task_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }

    #[test]
    fn validation_rejects_assisted_execution_access_true() {
        let mut report = base_report();
        report.assisted_execution_access = true;
        assert!(validate_wasm_runtime_report(&report).is_err());
    }
}
