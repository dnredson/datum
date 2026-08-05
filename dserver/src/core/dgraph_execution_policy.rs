use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const EXECUTION_POLICY_CONTRACT_VERSION: &str = "0.1.0";

pub const EXECUTION_OPERATION_CLASSES: [&str; 8] = [
    "placement",
    "recovery",
    "migration",
    "reconfiguration",
    "deployment",
    "installation",
    "dispatch",
    "wasm_execution",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphExecutionPolicyRule {
    pub operation: String,
    pub effect: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphExecutionPolicyResponse {
    pub project_id: String,
    pub contract_version: String,
    pub policy_source: String,
    pub policy_configured: bool,
    pub policy_mode: String,
    pub default_decision: String,
    pub execution_enabled: bool,
    pub rules: Vec<DGraphExecutionPolicyRule>,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_default_deny_execution_policy_contract(
    project_id: &str,
) -> ProjectDGraphExecutionPolicyResponse {
    ProjectDGraphExecutionPolicyResponse {
        project_id: project_id.to_string(),
        contract_version: EXECUTION_POLICY_CONTRACT_VERSION.to_string(),
        policy_source: "builtin_fail_closed_fallback".to_string(),
        policy_configured: false,
        policy_mode: "deny_all".to_string(),
        default_decision: "deny".to_string(),
        execution_enabled: false,
        rules: EXECUTION_OPERATION_CLASSES
            .into_iter()
            .map(|operation| DGraphExecutionPolicyRule {
                operation: operation.to_string(),
                effect: "deny".to_string(),
                reason: "no explicit project-scoped execution policy is configured".to_string(),
            })
            .collect(),
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "execution_policy_fallback",
            "project_scoped_policy_loaded": false,
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
    fn fallback_policy_is_fail_closed_for_every_operation() {
        let policy = build_default_deny_execution_policy_contract("project-a");

        assert_eq!(policy.project_id, "project-a");
        assert!(!policy.policy_configured);
        assert_eq!(policy.policy_mode, "deny_all");
        assert_eq!(policy.default_decision, "deny");
        assert!(!policy.execution_enabled);
        assert!(!policy.authorization_granted);
        assert_eq!(policy.rules.len(), EXECUTION_OPERATION_CLASSES.len());
        assert!(policy.rules.iter().all(|rule| rule.effect == "deny"));
        assert!(!policy.side_effects_performed);
    }

    #[test]
    fn serialized_policy_cannot_be_mistaken_for_authorization() {
        let value = serde_json::to_value(build_default_deny_execution_policy_contract("project-a"))
            .expect("policy contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["policy_source"], "builtin_fail_closed_fallback");
        assert_eq!(value["policy_configured"], false);
        assert_eq!(value["execution_enabled"], false);
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(value["metadata"]["project_scoped_policy_loaded"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["metadata"]["execution_safety_gate_satisfied"], false);
    }

    #[test]
    fn source_guard_contains_no_policy_write_or_execution_path() {
        let source = include_str!("dgraph_execution_policy.rs");
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
            "effect: \"allow\"",
            "authorization_granted: true",
            "execution_enabled: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "execution policy contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
