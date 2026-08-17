use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::deployment_manifest::{validate_deployment_manifest, DeploymentManifest};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlledWasmLiveReportList {
    pub manifest_id: String,
    pub project_id: String,
    pub count: usize,
    pub reports: Vec<Value>,
}

pub fn validate_controlled_live_report_against_manifest(
    manifest: &DeploymentManifest,
    report: &Value,
) -> Result<(), String> {
    validate_deployment_manifest(manifest)?;

    let execution_id = required_string(report, "execution_id")?;
    if execution_id.trim().is_empty() {
        return Err("execution_id must not be empty".to_string());
    }

    let manifest_id = required_string(report, "manifest_id")?;
    if manifest_id != manifest.manifest_id {
        return Err(format!(
            "live report manifest_id '{}' does not match active manifest '{}'",
            manifest_id, manifest.manifest_id
        ));
    }

    let project_id = required_string(report, "project_id")?;
    if project_id != manifest.project_id {
        return Err(format!(
            "live report project_id '{}' does not match active project '{}'",
            project_id, manifest.project_id
        ));
    }

    let node_id = required_string(report, "node_id")?;
    let assignment_id = required_string(report, "assignment_id")?;
    let module_id = required_string(report, "module_id")?;
    let function_name = required_string(report, "function_name")?;
    let artifact_content_id = required_string(report, "artifact_content_id")?;

    let assignment = manifest
        .assignments
        .iter()
        .find(|assignment| assignment.assignment_id == assignment_id)
        .ok_or_else(|| {
            format!(
                "live report assignment '{}' is not present in active manifest",
                assignment_id
            )
        })?;

    if assignment.target_node != node_id {
        return Err(format!(
            "live report assignment '{}' targets '{}' but report node is '{}'",
            assignment_id, assignment.target_node, node_id
        ));
    }
    if assignment.module_id != module_id {
        return Err(format!(
            "live report module '{}' does not match assignment module '{}'",
            module_id, assignment.module_id
        ));
    }
    if assignment.function_name != function_name {
        return Err(format!(
            "live report function '{}' does not match assignment function '{}'",
            function_name, assignment.function_name
        ));
    }
    if assignment.artifact.content_id != artifact_content_id {
        return Err(format!(
            "live report artifact content identity '{}' does not match assignment '{}'",
            artifact_content_id, assignment.artifact.content_id
        ));
    }

    let status = required_string(report, "status")?;
    if status != "executed" {
        return Err(format!(
            "C67 success evidence requires status='executed', received '{status}'"
        ));
    }

    require_bool(report, "explicit_execute", true)?;
    require_bool(report, "integrity_verified", true)?;
    require_bool(report, "wasmtime_compiled", true)?;
    require_bool(report, "wasmtime_instantiated", true)?;
    require_bool(report, "function_invocation_attempted", true)?;
    require_bool(report, "guest_execution_performed", true)?;
    require_bool(report, "wasm_guest_side_effects", false)?;
    require_bool(report, "governed_execution_performed", false)?;
    require_bool(report, "host_mutation_performed", false)?;
    require_bool(report, "safe_to_auto_execute", false)?;
    require_bool(report, "timeout_enforced", false)?;

    let acquisition = report
        .get("acquisition")
        .ok_or_else(|| "live report acquisition evidence is required".to_string())?;
    require_bool(acquisition, "artifact_delivery_performed", true)?;
    require_bool(acquisition, "host_network_fetch_performed", true)?;
    require_bool(acquisition, "persistent_write_performed", false)?;
    require_bool(acquisition, "host_mutation_performed", false)?;

    Ok(())
}

pub fn build_controlled_live_report_list(
    manifest: &DeploymentManifest,
    reports: &[Value],
) -> ControlledWasmLiveReportList {
    ControlledWasmLiveReportList {
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        count: reports.len(),
        reports: reports.to_vec(),
    }
}

pub fn report_execution_id(report: &Value) -> Option<&str> {
    report.get("execution_id").and_then(Value::as_str)
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("live report field '{field}' must be a non-empty string"))
}

fn require_bool(value: &Value, field: &str, expected: bool) -> Result<(), String> {
    match value.get(field).and_then(Value::as_bool) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(format!(
            "live report field '{field}' must be {expected}, received {actual}"
        )),
        None => Err(format!("live report field '{field}' must be a boolean")),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::core::deployment_manifest::{
        build_deployment_artifact_identity, DeploymentAssignment, DeploymentExecutionBoundary,
        DeploymentManifest, DeploymentRuntimeLimits, DeploymentStrategy, GraphDistribution,
        DEPLOYMENT_MANIFEST_VERSION,
    };

    use super::*;

    const SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "c67-three-node".to_string(),
            project_id: "c67-live-project".to_string(),
            strategy: DeploymentStrategy::Distributed,
            graph_distribution: GraphDistribution::NodeSlice,
            source_dgraph_id: Some("c67-dgraph".to_string()),
            assignments: vec![DeploymentAssignment {
                assignment_id: "fog-scale".to_string(),
                module_id: "fog-scale-module".to_string(),
                function_name: "fog_scale".to_string(),
                artifact: build_deployment_artifact_identity(
                    "fog-scale-artifact",
                    "http://127.0.0.1:8090/artifacts/fog-scale.wasm",
                    SHA,
                )
                .unwrap(),
                target_node: "fog-01".to_string(),
                limits: DeploymentRuntimeLimits {
                    fuel: Some(1_000_000),
                    timeout_ms: Some(1_000),
                },
                metadata: None,
            }],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    fn report() -> Value {
        json!({
            "execution_id": "exec-1",
            "manifest_id": "c67-three-node",
            "project_id": "c67-live-project",
            "node_id": "fog-01",
            "assignment_id": "fog-scale",
            "module_id": "fog-scale-module",
            "function_name": "fog_scale",
            "artifact_content_id": format!("sha256:{SHA}"),
            "explicit_execute": true,
            "status": "executed",
            "integrity_verified": true,
            "wasmtime_compiled": true,
            "wasmtime_instantiated": true,
            "function_invocation_attempted": true,
            "guest_execution_performed": true,
            "wasm_guest_side_effects": false,
            "governed_execution_performed": false,
            "host_mutation_performed": false,
            "safe_to_auto_execute": false,
            "timeout_enforced": false,
            "acquisition": {
                "artifact_delivery_performed": true,
                "host_network_fetch_performed": true,
                "persistent_write_performed": false,
                "host_mutation_performed": false
            }
        })
    }

    #[test]
    fn valid_network_delivered_live_report_is_accepted() {
        assert!(validate_controlled_live_report_against_manifest(&manifest(), &report()).is_ok());
    }

    #[test]
    fn wrong_node_is_rejected() {
        let mut value = report();
        value["node_id"] = json!("cloud-01");
        let error = validate_controlled_live_report_against_manifest(&manifest(), &value).unwrap_err();
        assert!(error.contains("targets 'fog-01'"));
    }

    #[test]
    fn wrong_artifact_identity_is_rejected() {
        let mut value = report();
        value["artifact_content_id"] = json!(format!("sha256:{}", "f".repeat(64)));
        let error = validate_controlled_live_report_against_manifest(&manifest(), &value).unwrap_err();
        assert!(error.contains("content identity"));
    }

    #[test]
    fn non_executed_report_is_rejected_for_c67_success_evidence() {
        let mut value = report();
        value["status"] = json!("failed");
        let error = validate_controlled_live_report_against_manifest(&manifest(), &value).unwrap_err();
        assert!(error.contains("status='executed'"));
    }

    #[test]
    fn missing_explicit_execute_is_rejected() {
        let mut value = report();
        value["explicit_execute"] = json!(false);
        assert!(validate_controlled_live_report_against_manifest(&manifest(), &value).is_err());
    }

    #[test]
    fn governed_execution_true_is_rejected() {
        let mut value = report();
        value["governed_execution_performed"] = json!(true);
        assert!(validate_controlled_live_report_against_manifest(&manifest(), &value).is_err());
    }

    #[test]
    fn guest_side_effects_true_is_rejected() {
        let mut value = report();
        value["wasm_guest_side_effects"] = json!(true);
        assert!(validate_controlled_live_report_against_manifest(&manifest(), &value).is_err());
    }

    #[test]
    fn local_file_acquisition_is_rejected_for_c67_network_delivery_evidence() {
        let mut value = report();
        value["acquisition"]["artifact_delivery_performed"] = json!(false);
        value["acquisition"]["host_network_fetch_performed"] = json!(false);
        assert!(validate_controlled_live_report_against_manifest(&manifest(), &value).is_err());
    }

    #[test]
    fn timeout_enforced_true_is_rejected_until_runtime_support_exists() {
        let mut value = report();
        value["timeout_enforced"] = json!(true);
        assert!(validate_controlled_live_report_against_manifest(&manifest(), &value).is_err());
    }

    #[test]
    fn report_list_preserves_active_manifest_identity() {
        let reports = vec![report()];
        let list = build_controlled_live_report_list(&manifest(), &reports);
        assert_eq!(list.manifest_id, "c67-three-node");
        assert_eq!(list.project_id, "c67-live-project");
        assert_eq!(list.count, 1);
    }
}
