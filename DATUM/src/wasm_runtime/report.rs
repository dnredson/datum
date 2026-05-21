use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

use crate::wasm_runtime::WasmRuntimeResult;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WasmRuntimeReport {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub reported_at_utc: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct WasmRuntimeReportList {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub reports: Vec<WasmRuntimeReport>,
}

pub fn build_wasm_runtime_report_from_result(result: &WasmRuntimeResult) -> WasmRuntimeReport {
    let now = Utc::now().to_rfc3339();

    WasmRuntimeReport {
        project_id: None,
        report_id: Some(format!("wasm-report-{}", Uuid::new_v4())),
        node_id: None,
        reported_at_utc: Some(now.clone()),
        source: Some("client".to_string()),
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
            "limits": {
                "fuel_configured": result.limit_outcome.fuel_configured,
                "fuel_consumed": result.limit_outcome.fuel_consumed,
                "timeout_ms": result.limit_outcome.timeout_ms,
                "limit_exceeded": result.limit_outcome.limit_exceeded,
            }
        })),
    }
}

pub fn validate_local_wasm_runtime_report_publish_invariants(
    report: &WasmRuntimeReport,
) -> anyhow::Result<()> {
    if report.safe_to_auto_execute {
        return Err(anyhow::anyhow!("safe_to_auto_execute must be false"));
    }
    if report.side_effects_performed {
        return Err(anyhow::anyhow!("side_effects_performed must be false"));
    }
    if report.filesystem_access {
        return Err(anyhow::anyhow!("filesystem_access must be false"));
    }
    if report.network_access {
        return Err(anyhow::anyhow!("network_access must be false"));
    }
    if report.host_command_access {
        return Err(anyhow::anyhow!("host_command_access must be false"));
    }
    if report.docker_access {
        return Err(anyhow::anyhow!("docker_access must be false"));
    }
    if report.artifact_access {
        return Err(anyhow::anyhow!("artifact_access must be false"));
    }
    if report.installation_task_access {
        return Err(anyhow::anyhow!("installation_task_access must be false"));
    }
    if report.assisted_execution_access {
        return Err(anyhow::anyhow!("assisted_execution_access must be false"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        build_wasm_runtime_report_from_result,
        validate_local_wasm_runtime_report_publish_invariants,
    };
    use crate::wasm_runtime::{WasmRuntimeLimits, WasmRuntimeResult};

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
            limits: WasmRuntimeLimits::default(),
            limit_outcome: crate::wasm_runtime::WasmRuntimeLimitOutcome {
                fuel_configured: Some(1_000_000),
                fuel_consumed: Some(3),
                timeout_ms: Some(1_000),
                limit_exceeded: Some(false),
            },
        }
    }

    #[test]
    fn report_preserves_status_and_result() {
        let report = build_wasm_runtime_report_from_result(&sample_result());

        assert_eq!(report.status, "executed");
        assert_eq!(report.result_i32, Some(5));
        assert_eq!(report.execution_mode, "embedded_test");
        assert_eq!(report.source.as_deref(), Some("client"));
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
        assert!(rendered.contains("\"fuel_configured\": 1000000"));
        assert!(rendered.contains("\"timeout_ms\": 1000"));
    }

    #[test]
    fn report_includes_limit_metadata() {
        let mut result = sample_result();
        result.limits = WasmRuntimeLimits {
            fuel: Some(42),
            timeout_ms: Some(7),
        };
        result.limit_outcome.fuel_configured = Some(42);
        result.limit_outcome.fuel_consumed = Some(3);
        result.limit_outcome.timeout_ms = Some(7);
        result.limit_outcome.limit_exceeded = Some(false);

        let report = build_wasm_runtime_report_from_result(&result);
        let limits = report
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("limits"))
            .expect("limit metadata should be present");

        assert_eq!(
            limits
                .get("fuel_configured")
                .and_then(|value| value.as_u64()),
            Some(42)
        );
        assert_eq!(
            limits.get("fuel_consumed").and_then(|value| value.as_u64()),
            Some(3)
        );
        assert_eq!(
            limits.get("timeout_ms").and_then(|value| value.as_u64()),
            Some(7)
        );
        assert_eq!(
            limits
                .get("limit_exceeded")
                .and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn local_publish_invariants_accept_safe_report() {
        let report = build_wasm_runtime_report_from_result(&sample_result());
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_ok());
    }

    #[test]
    fn local_publish_invariants_reject_host_or_side_effect_flags() {
        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.safe_to_auto_execute = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.side_effects_performed = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.filesystem_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.network_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.host_command_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.docker_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.artifact_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.installation_task_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());

        let mut report = build_wasm_runtime_report_from_result(&sample_result());
        report.assisted_execution_access = true;
        assert!(validate_local_wasm_runtime_report_publish_invariants(&report).is_err());
    }
}
