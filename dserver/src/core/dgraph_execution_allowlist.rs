use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const EXECUTION_ALLOWLIST_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphExecutionAllowlistResponse {
    pub project_id: String,
    pub contract_version: String,
    pub evaluation_status: String,
    pub allowlist_configured: bool,
    pub target_allowlist_loaded: bool,
    pub target_allowlist: Vec<String>,
    pub module_allowlist_loaded: bool,
    pub module_allowlist: Vec<String>,
    pub runtime_allowlist_loaded: bool,
    pub runtime_allowlist: Vec<String>,
    pub function_allowlist_loaded: bool,
    pub function_allowlist: Vec<String>,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_fail_closed_execution_allowlist_contract(
    project_id: &str,
) -> ProjectDGraphExecutionAllowlistResponse {
    ProjectDGraphExecutionAllowlistResponse {
        project_id: project_id.to_string(),
        contract_version: EXECUTION_ALLOWLIST_CONTRACT_VERSION.to_string(),
        evaluation_status: "not_evaluated".to_string(),
        allowlist_configured: false,
        target_allowlist_loaded: false,
        target_allowlist: vec![],
        module_allowlist_loaded: false,
        module_allowlist: vec![],
        runtime_allowlist_loaded: false,
        runtime_allowlist: vec![],
        function_allowlist_loaded: false,
        function_allowlist: vec![],
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "target_and_function_allowlist_boundary",
            "dgraph_entities_are_allowlist_entries": false,
            "wildcard_entries_allowed": false,
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
    fn execution_allowlist_contract_is_fail_closed() {
        let allowlists = build_fail_closed_execution_allowlist_contract("project-a");

        assert_eq!(allowlists.project_id, "project-a");
        assert_eq!(allowlists.evaluation_status, "not_evaluated");
        assert!(!allowlists.allowlist_configured);
        assert!(!allowlists.target_allowlist_loaded);
        assert!(allowlists.target_allowlist.is_empty());
        assert!(!allowlists.module_allowlist_loaded);
        assert!(allowlists.module_allowlist.is_empty());
        assert!(!allowlists.runtime_allowlist_loaded);
        assert!(allowlists.runtime_allowlist.is_empty());
        assert!(!allowlists.function_allowlist_loaded);
        assert!(allowlists.function_allowlist.is_empty());
        assert!(!allowlists.authorization_granted);
        assert!(!allowlists.side_effects_performed);
    }

    #[test]
    fn serialized_contract_keeps_dgraph_entities_non_authoritative() {
        let value =
            serde_json::to_value(build_fail_closed_execution_allowlist_contract("project-a"))
                .expect("execution allowlist contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["evaluation_status"], "not_evaluated");
        assert_eq!(value["allowlist_configured"], false);
        assert_eq!(value["target_allowlist_loaded"], false);
        assert_eq!(value["target_allowlist"], json!([]));
        assert_eq!(value["module_allowlist_loaded"], false);
        assert_eq!(value["module_allowlist"], json!([]));
        assert_eq!(value["runtime_allowlist_loaded"], false);
        assert_eq!(value["runtime_allowlist"], json!([]));
        assert_eq!(value["function_allowlist_loaded"], false);
        assert_eq!(value["function_allowlist"], json!([]));
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(
            value["metadata"]["dgraph_entities_are_allowlist_entries"],
            false
        );
        assert_eq!(value["metadata"]["wildcard_entries_allowed"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["metadata"]["execution_safety_gate_satisfied"], false);
    }

    #[test]
    fn source_guard_contains_no_allowlist_load_or_execution_path() {
        let source = include_str!("dgraph_execution_allowlist.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "fs::read",
            "File::open",
            "reqwest",
            "docker run",
            "docker pull",
            "run_embedded_wasm",
            "dispatch --execute",
            "store_",
            "allowlist_configured: true",
            "target_allowlist_loaded: true",
            "module_allowlist_loaded: true",
            "runtime_allowlist_loaded: true",
            "function_allowlist_loaded: true",
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "execution allowlist contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
