use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const ROLLBACK_AUDIT_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphRollbackAuditResponse {
    pub project_id: String,
    pub contract_version: String,
    pub rollback_status: String,
    pub audit_status: String,
    pub rollback_plan_loaded: bool,
    pub rollback_plan_id: Option<String>,
    pub rollback_checkpoints_loaded: bool,
    pub rollback_checkpoints: Vec<String>,
    pub rollback_preconditions_evaluated: bool,
    pub rollback_guaranteed: bool,
    pub audit_policy_loaded: bool,
    pub audit_sink_configured: bool,
    pub audit_records_loaded: bool,
    pub audit_records: Vec<String>,
    pub audit_trail_complete: bool,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_fail_closed_rollback_audit_contract(
    project_id: &str,
) -> ProjectDGraphRollbackAuditResponse {
    ProjectDGraphRollbackAuditResponse {
        project_id: project_id.to_string(),
        contract_version: ROLLBACK_AUDIT_CONTRACT_VERSION.to_string(),
        rollback_status: "not_guaranteed".to_string(),
        audit_status: "not_available".to_string(),
        rollback_plan_loaded: false,
        rollback_plan_id: None,
        rollback_checkpoints_loaded: false,
        rollback_checkpoints: vec![],
        rollback_preconditions_evaluated: false,
        rollback_guaranteed: false,
        audit_policy_loaded: false,
        audit_sink_configured: false,
        audit_records_loaded: false,
        audit_records: vec![],
        audit_trail_complete: false,
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "rollback_and_audit_boundary",
            "planner_proposals_are_rollback_plans": false,
            "legacy_action_results_are_execution_audit_trail": false,
            "rollback_performed": false,
            "audit_record_written": false,
            "persisted": false,
            "execution_performed": false,
            "execution_safety_gate_satisfied": false,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_audit_contract_is_fail_closed() {
        let contract = build_fail_closed_rollback_audit_contract("project-a");

        assert_eq!(contract.project_id, "project-a");
        assert_eq!(contract.rollback_status, "not_guaranteed");
        assert_eq!(contract.audit_status, "not_available");
        assert!(!contract.rollback_plan_loaded);
        assert_eq!(contract.rollback_plan_id, None);
        assert!(!contract.rollback_checkpoints_loaded);
        assert!(contract.rollback_checkpoints.is_empty());
        assert!(!contract.rollback_preconditions_evaluated);
        assert!(!contract.rollback_guaranteed);
        assert!(!contract.audit_policy_loaded);
        assert!(!contract.audit_sink_configured);
        assert!(!contract.audit_records_loaded);
        assert!(contract.audit_records.is_empty());
        assert!(!contract.audit_trail_complete);
        assert!(!contract.authorization_granted);
        assert!(!contract.side_effects_performed);
    }

    #[test]
    fn serialized_contract_keeps_rollback_and_audit_absent_and_non_authoritative() {
        let value =
            serde_json::to_value(build_fail_closed_rollback_audit_contract("project-a"))
                .expect("rollback and audit contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["rollback_status"], "not_guaranteed");
        assert_eq!(value["audit_status"], "not_available");
        assert_eq!(value["rollback_plan_loaded"], false);
        assert_eq!(value["rollback_plan_id"], Value::Null);
        assert_eq!(value["rollback_checkpoints_loaded"], false);
        assert_eq!(value["rollback_checkpoints"], json!([]));
        assert_eq!(value["rollback_preconditions_evaluated"], false);
        assert_eq!(value["rollback_guaranteed"], false);
        assert_eq!(value["audit_policy_loaded"], false);
        assert_eq!(value["audit_sink_configured"], false);
        assert_eq!(value["audit_records_loaded"], false);
        assert_eq!(value["audit_records"], json!([]));
        assert_eq!(value["audit_trail_complete"], false);
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(
            value["metadata"]["planner_proposals_are_rollback_plans"],
            false
        );
        assert_eq!(
            value["metadata"]["legacy_action_results_are_execution_audit_trail"],
            false
        );
        assert_eq!(value["metadata"]["rollback_performed"], false);
        assert_eq!(value["metadata"]["audit_record_written"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["metadata"]["execution_safety_gate_satisfied"], false);
    }

    #[test]
    fn source_guard_contains_no_rollback_audit_write_or_execution_path() {
        let source = include_str!("dgraph_rollback_audit.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "fs::read",
            "fs::write",
            "File::open",
            "OpenOptions",
            "reqwest",
            "wasmtime",
            "docker run",
            "docker pull",
            "docker rm",
            "run_embedded_wasm",
            "dispatch --execute",
            "store_",
            "rollback_plan_loaded: true",
            "rollback_checkpoints_loaded: true",
            "rollback_guaranteed: true",
            "audit_policy_loaded: true",
            "audit_sink_configured: true",
            "audit_records_loaded: true",
            "audit_trail_complete: true",
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "rollback and audit contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
