use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const CAPABILITY_NEGOTIATION_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphCapabilityNegotiationResponse {
    pub project_id: String,
    pub contract_version: String,
    pub negotiation_status: String,
    pub requirements_loaded: bool,
    pub advertisements_loaded: bool,
    pub required_capabilities: Vec<String>,
    pub advertised_capabilities: Vec<String>,
    pub matched_capabilities: Vec<String>,
    pub compatibility_evaluated: bool,
    pub negotiation_succeeded: bool,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_fail_closed_capability_negotiation_contract(
    project_id: &str,
) -> ProjectDGraphCapabilityNegotiationResponse {
    ProjectDGraphCapabilityNegotiationResponse {
        project_id: project_id.to_string(),
        contract_version: CAPABILITY_NEGOTIATION_CONTRACT_VERSION.to_string(),
        negotiation_status: "not_negotiated".to_string(),
        requirements_loaded: false,
        advertisements_loaded: false,
        required_capabilities: vec![],
        advertised_capabilities: vec![],
        matched_capabilities: vec![],
        compatibility_evaluated: false,
        negotiation_succeeded: false,
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "capability_negotiation_boundary",
            "dgraph_capability_metadata_is_negotiation": false,
            "legacy_node_capability_reports_are_execution_authority": false,
            "network_exchange_performed": false,
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
    fn capability_negotiation_contract_is_fail_closed() {
        let negotiation = build_fail_closed_capability_negotiation_contract("project-a");

        assert_eq!(negotiation.project_id, "project-a");
        assert_eq!(negotiation.negotiation_status, "not_negotiated");
        assert!(!negotiation.requirements_loaded);
        assert!(!negotiation.advertisements_loaded);
        assert!(negotiation.required_capabilities.is_empty());
        assert!(negotiation.advertised_capabilities.is_empty());
        assert!(negotiation.matched_capabilities.is_empty());
        assert!(!negotiation.compatibility_evaluated);
        assert!(!negotiation.negotiation_succeeded);
        assert!(!negotiation.authorization_granted);
        assert!(!negotiation.side_effects_performed);
    }

    #[test]
    fn serialized_contract_keeps_capability_metadata_non_authoritative() {
        let value =
            serde_json::to_value(build_fail_closed_capability_negotiation_contract("project-a"))
                .expect("capability negotiation contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["negotiation_status"], "not_negotiated");
        assert_eq!(value["requirements_loaded"], false);
        assert_eq!(value["advertisements_loaded"], false);
        assert_eq!(value["required_capabilities"], json!([]));
        assert_eq!(value["advertised_capabilities"], json!([]));
        assert_eq!(value["matched_capabilities"], json!([]));
        assert_eq!(value["compatibility_evaluated"], false);
        assert_eq!(value["negotiation_succeeded"], false);
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(
            value["metadata"]["dgraph_capability_metadata_is_negotiation"],
            false
        );
        assert_eq!(
            value["metadata"]["legacy_node_capability_reports_are_execution_authority"],
            false
        );
        assert_eq!(value["metadata"]["network_exchange_performed"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["metadata"]["execution_safety_gate_satisfied"], false);
    }

    #[test]
    fn source_guard_contains_no_negotiation_or_execution_path() {
        let source = include_str!("dgraph_capability_negotiation.rs");
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
            "requirements_loaded: true",
            "advertisements_loaded: true",
            "compatibility_evaluated: true",
            "negotiation_succeeded: true",
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "capability negotiation contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
