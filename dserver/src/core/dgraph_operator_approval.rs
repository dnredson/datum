use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const OPERATOR_APPROVAL_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphOperatorApprovalResponse {
    pub project_id: String,
    pub contract_version: String,
    pub authentication_status: String,
    pub identity_authenticated: bool,
    pub operator_id: Option<String>,
    pub approval_status: String,
    pub approval_present: bool,
    pub approval_id: Option<String>,
    pub approval_valid: bool,
    pub approval_scope: Vec<String>,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_fail_closed_operator_approval_contract(
    project_id: &str,
) -> ProjectDGraphOperatorApprovalResponse {
    ProjectDGraphOperatorApprovalResponse {
        project_id: project_id.to_string(),
        contract_version: OPERATOR_APPROVAL_CONTRACT_VERSION.to_string(),
        authentication_status: "not_evaluated".to_string(),
        identity_authenticated: false,
        operator_id: None,
        approval_status: "missing".to_string(),
        approval_present: false,
        approval_id: None,
        approval_valid: false,
        approval_scope: vec![],
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "operator_approval_boundary",
            "project_scoped_approval_loaded": false,
            "legacy_installation_task_approvals_evaluated": false,
            "legacy_installation_task_approvals_are_authorization": false,
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
    fn operator_approval_contract_is_fail_closed() {
        let approval = build_fail_closed_operator_approval_contract("project-a");

        assert_eq!(approval.project_id, "project-a");
        assert_eq!(approval.authentication_status, "not_evaluated");
        assert!(!approval.identity_authenticated);
        assert!(approval.operator_id.is_none());
        assert_eq!(approval.approval_status, "missing");
        assert!(!approval.approval_present);
        assert!(approval.approval_id.is_none());
        assert!(!approval.approval_valid);
        assert!(approval.approval_scope.is_empty());
        assert!(!approval.authorization_granted);
        assert!(!approval.side_effects_performed);
    }

    #[test]
    fn serialized_contract_excludes_legacy_approvals_from_authorization() {
        let value = serde_json::to_value(build_fail_closed_operator_approval_contract("project-a"))
            .expect("operator approval contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["identity_authenticated"], false);
        assert_eq!(value["approval_present"], false);
        assert_eq!(value["approval_valid"], false);
        assert_eq!(value["approval_scope"], json!([]));
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(
            value["metadata"]["legacy_installation_task_approvals_evaluated"],
            false
        );
        assert_eq!(
            value["metadata"]["legacy_installation_task_approvals_are_authorization"],
            false
        );
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["metadata"]["execution_safety_gate_satisfied"], false);
    }

    #[test]
    fn source_guard_contains_no_approval_write_or_execution_path() {
        let source = include_str!("dgraph_operator_approval.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "docker run",
            "docker pull",
            "docker rm",
            "docker inspect",
            "fs::write",
            "OpenOptions",
            "run_embedded_wasm",
            "dispatch --execute",
            "store_",
            "InstallationTaskApproval",
            "identity_authenticated: true",
            "approval_present: true",
            "approval_valid: true",
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "operator approval contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
