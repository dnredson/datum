use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const ARTIFACT_INTEGRITY_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphArtifactIntegrityResponse {
    pub project_id: String,
    pub contract_version: String,
    pub verification_status: String,
    pub artifact_inventory_loaded: bool,
    pub artifact_count: u64,
    pub digest_algorithm: Option<String>,
    pub artifact_digests_verified: bool,
    pub signatures_verified: bool,
    pub provenance_verified: bool,
    pub trusted_provenance_source: Option<String>,
    pub integrity_verified: bool,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_fail_closed_artifact_integrity_contract(
    project_id: &str,
) -> ProjectDGraphArtifactIntegrityResponse {
    ProjectDGraphArtifactIntegrityResponse {
        project_id: project_id.to_string(),
        contract_version: ARTIFACT_INTEGRITY_CONTRACT_VERSION.to_string(),
        verification_status: "not_evaluated".to_string(),
        artifact_inventory_loaded: false,
        artifact_count: 0,
        digest_algorithm: None,
        artifact_digests_verified: false,
        signatures_verified: false,
        provenance_verified: false,
        trusted_provenance_source: None,
        integrity_verified: false,
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "artifact_integrity_and_provenance_boundary",
            "artifact_references_are_verification": false,
            "artifact_bytes_read": false,
            "artifact_transfer_performed": false,
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
    fn artifact_integrity_contract_is_fail_closed() {
        let integrity = build_fail_closed_artifact_integrity_contract("project-a");

        assert_eq!(integrity.project_id, "project-a");
        assert_eq!(integrity.verification_status, "not_evaluated");
        assert!(!integrity.artifact_inventory_loaded);
        assert_eq!(integrity.artifact_count, 0);
        assert!(integrity.digest_algorithm.is_none());
        assert!(!integrity.artifact_digests_verified);
        assert!(!integrity.signatures_verified);
        assert!(!integrity.provenance_verified);
        assert!(integrity.trusted_provenance_source.is_none());
        assert!(!integrity.integrity_verified);
        assert!(!integrity.authorization_granted);
        assert!(!integrity.side_effects_performed);
    }

    #[test]
    fn serialized_contract_keeps_artifact_references_non_authoritative() {
        let value = serde_json::to_value(build_fail_closed_artifact_integrity_contract(
            "project-a",
        ))
        .expect("artifact integrity contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["verification_status"], "not_evaluated");
        assert_eq!(value["artifact_inventory_loaded"], false);
        assert_eq!(value["artifact_count"], 0);
        assert!(value["digest_algorithm"].is_null());
        assert_eq!(value["artifact_digests_verified"], false);
        assert_eq!(value["signatures_verified"], false);
        assert_eq!(value["provenance_verified"], false);
        assert!(value["trusted_provenance_source"].is_null());
        assert_eq!(value["integrity_verified"], false);
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(
            value["metadata"]["artifact_references_are_verification"],
            false
        );
        assert_eq!(value["metadata"]["artifact_bytes_read"], false);
        assert_eq!(value["metadata"]["artifact_transfer_performed"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(
            value["metadata"]["execution_safety_gate_satisfied"],
            false
        );
    }

    #[test]
    fn source_guard_contains_no_artifact_read_transfer_or_execution_path() {
        let source = include_str!("dgraph_artifact_integrity.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        for forbidden in [
            "std::process::Command",
            "Command::new",
            "fs::read",
            "File::open",
            "reqwest",
            "docker pull",
            "docker run",
            "run_embedded_wasm",
            "dispatch --execute",
            "store_",
            "artifact_inventory_loaded: true",
            "artifact_digests_verified: true",
            "signatures_verified: true",
            "provenance_verified: true",
            "integrity_verified: true",
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "artifact integrity contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
