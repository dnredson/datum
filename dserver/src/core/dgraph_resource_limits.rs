use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const RESOURCE_LIMITS_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphResourceLimitsResponse {
    pub project_id: String,
    pub contract_version: String,
    pub limits_status: String,
    pub limits_configured: bool,
    pub limits_loaded: bool,
    pub fuel_limit_configured: bool,
    pub fuel_limit: Option<u64>,
    pub timeout_limit_configured: bool,
    pub timeout_ms: Option<u64>,
    pub memory_limit_configured: bool,
    pub memory_limit_bytes: Option<u64>,
    pub host_capability_limits_loaded: bool,
    pub host_capability_limits: Vec<String>,
    pub limits_enforced: bool,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_fail_closed_resource_limits_contract(
    project_id: &str,
) -> ProjectDGraphResourceLimitsResponse {
    ProjectDGraphResourceLimitsResponse {
        project_id: project_id.to_string(),
        contract_version: RESOURCE_LIMITS_CONTRACT_VERSION.to_string(),
        limits_status: "not_enforced".to_string(),
        limits_configured: false,
        limits_loaded: false,
        fuel_limit_configured: false,
        fuel_limit: None,
        timeout_limit_configured: false,
        timeout_ms: None,
        memory_limit_configured: false,
        memory_limit_bytes: None,
        host_capability_limits_loaded: false,
        host_capability_limits: vec![],
        limits_enforced: false,
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "resource_limit_boundary",
            "planner_metadata_is_resource_limit_policy": false,
            "runtime_capability_reports_are_enforced_limits": false,
            "host_resources_reserved": false,
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
    fn resource_limits_contract_is_fail_closed() {
        let limits = build_fail_closed_resource_limits_contract("project-a");

        assert_eq!(limits.project_id, "project-a");
        assert_eq!(limits.limits_status, "not_enforced");
        assert!(!limits.limits_configured);
        assert!(!limits.limits_loaded);
        assert!(!limits.fuel_limit_configured);
        assert_eq!(limits.fuel_limit, None);
        assert!(!limits.timeout_limit_configured);
        assert_eq!(limits.timeout_ms, None);
        assert!(!limits.memory_limit_configured);
        assert_eq!(limits.memory_limit_bytes, None);
        assert!(!limits.host_capability_limits_loaded);
        assert!(limits.host_capability_limits.is_empty());
        assert!(!limits.limits_enforced);
        assert!(!limits.authorization_granted);
        assert!(!limits.side_effects_performed);
    }

    #[test]
    fn serialized_contract_keeps_limits_absent_and_non_authoritative() {
        let value =
            serde_json::to_value(build_fail_closed_resource_limits_contract("project-a"))
                .expect("resource limits contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["limits_status"], "not_enforced");
        assert_eq!(value["limits_configured"], false);
        assert_eq!(value["limits_loaded"], false);
        assert_eq!(value["fuel_limit_configured"], false);
        assert_eq!(value["fuel_limit"], Value::Null);
        assert_eq!(value["timeout_limit_configured"], false);
        assert_eq!(value["timeout_ms"], Value::Null);
        assert_eq!(value["memory_limit_configured"], false);
        assert_eq!(value["memory_limit_bytes"], Value::Null);
        assert_eq!(value["host_capability_limits_loaded"], false);
        assert_eq!(value["host_capability_limits"], json!([]));
        assert_eq!(value["limits_enforced"], false);
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(
            value["metadata"]["planner_metadata_is_resource_limit_policy"],
            false
        );
        assert_eq!(
            value["metadata"]["runtime_capability_reports_are_enforced_limits"],
            false
        );
        assert_eq!(value["metadata"]["host_resources_reserved"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["metadata"]["execution_safety_gate_satisfied"], false);
    }

    #[test]
    fn source_guard_contains_no_limit_enforcement_or_execution_path() {
        let source = include_str!("dgraph_resource_limits.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "fs::read",
            "File::open",
            "reqwest",
            "wasmtime",
            "StoreLimitsBuilder",
            "consume_fuel",
            "set_fuel",
            "tokio::time::timeout(",
            "docker run",
            "docker pull",
            "run_embedded_wasm",
            "dispatch --execute",
            "store_",
            "limits_configured: true",
            "limits_loaded: true",
            "limits_enforced: true",
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "resource limits contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
