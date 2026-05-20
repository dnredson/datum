use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::wasm_runtime::WasmRuntimeResult;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmRuntimeReport {
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    pub module_name: String,
    pub function_name: String,
    pub execution_mode: String,
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
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
    pub filesystem_access: bool,
    pub network_access: bool,
    pub host_command_access: bool,
    pub docker_access: bool,
    pub artifact_access: bool,
    pub installation_task_access: bool,
    pub assisted_execution_access: bool,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

pub fn build_wasm_runtime_report_from_result(result: &WasmRuntimeResult) -> WasmRuntimeReport {
    let now = Utc::now().to_rfc3339();

    WasmRuntimeReport {
        report_id: Some(format!("wasm-report-{}", Uuid::new_v4())),
        node_id: None,
        module_name: result.module_name.clone(),
        function_name: result.function_name.clone(),
        execution_mode: "embedded_test".to_string(),
        status: result.status.clone(),
        result_i32: result.result_i32,
        error_message: result.message.clone(),
        started_at_utc: Some(now.clone()),
        finished_at_utc: Some(now),
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
        metadata: Some(json!({
            "local_only": true,
            "published_to_server": false,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::build_wasm_runtime_report_from_result;
    use crate::wasm_runtime::WasmRuntimeResult;

    fn sample_result() -> WasmRuntimeResult {
        WasmRuntimeResult {
            module_name: "embedded:add".to_string(),
            function_name: "add".to_string(),
            status: "executed".to_string(),
            result_i32: Some(5),
            message: Some("executed isolated embedded WASM test without host access".to_string()),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            filesystem_access: false,
            network_access: false,
            host_command_access: false,
            docker_access: false,
        }
    }

    #[test]
    fn report_preserves_status_and_result() {
        let report = build_wasm_runtime_report_from_result(&sample_result());

        assert_eq!(report.status, "executed");
        assert_eq!(report.result_i32, Some(5));
        assert_eq!(report.execution_mode, "embedded_test");
    }

    #[test]
    fn report_safety_and_access_flags_remain_false() {
        let report = build_wasm_runtime_report_from_result(&sample_result());

        assert!(!report.safe_to_auto_execute);
        assert!(!report.side_effects_performed);
        assert!(!report.filesystem_access);
        assert!(!report.network_access);
        assert!(!report.host_command_access);
        assert!(!report.docker_access);
        assert!(!report.artifact_access);
        assert!(!report.installation_task_access);
        assert!(!report.assisted_execution_access);
    }

    #[test]
    fn report_json_includes_safety_access_and_local_only_metadata() {
        let report = build_wasm_runtime_report_from_result(&sample_result());
        let rendered =
            serde_json::to_string_pretty(&report).expect("report should serialize to JSON");

        assert!(rendered.contains("\"execution_mode\": \"embedded_test\""));
        assert!(rendered.contains("\"safe_to_auto_execute\": false"));
        assert!(rendered.contains("\"artifact_access\": false"));
        assert!(rendered.contains("\"installation_task_access\": false"));
        assert!(rendered.contains("\"assisted_execution_access\": false"));
        assert!(rendered.contains("\"published_to_server\": false"));
    }
}
