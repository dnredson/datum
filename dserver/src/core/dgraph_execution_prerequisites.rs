use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{
    dgraph_artifact_integrity::build_fail_closed_artifact_integrity_contract,
    dgraph_capability_negotiation::build_fail_closed_capability_negotiation_contract,
    dgraph_execution_allowlist::build_fail_closed_execution_allowlist_contract,
    dgraph_execution_policy::build_default_deny_execution_policy_contract,
    dgraph_operator_approval::build_fail_closed_operator_approval_contract,
    dgraph_resource_limits::build_fail_closed_resource_limits_contract,
    dgraph_rollback_audit::build_fail_closed_rollback_audit_contract,
};

const EXECUTION_PREREQUISITES_CONTRACT_VERSION: &str = "0.1.0";
const REQUIRED_GATE_COUNT: u64 = 7;

pub const EXECUTION_PREREQUISITE_GATE_NAMES: [&str; 7] = [
    "explicit_execution_policy",
    "authenticated_operator_approval",
    "artifact_integrity_and_provenance",
    "target_and_function_allowlists",
    "capability_negotiation",
    "resource_limits",
    "rollback_and_audit",
];

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphExecutionPrerequisiteEvidence {
    pub gate: String,
    pub source_contract: String,
    pub source_contract_version: String,
    pub satisfied: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphExecutionPrerequisitesResponse {
    pub project_id: String,
    pub contract_version: String,
    pub aggregation_status: String,
    pub required_gate_count: u64,
    pub satisfied_gate_count: u64,
    pub all_prerequisites_satisfied: bool,
    pub prerequisites: Vec<DGraphExecutionPrerequisiteEvidence>,
    pub authorization_granted: bool,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

fn prerequisite(
    gate: &str,
    source_contract: &str,
    source_contract_version: &str,
    satisfied: bool,
    reason: &str,
) -> DGraphExecutionPrerequisiteEvidence {
    DGraphExecutionPrerequisiteEvidence {
        gate: gate.to_string(),
        source_contract: source_contract.to_string(),
        source_contract_version: source_contract_version.to_string(),
        satisfied,
        reason: reason.to_string(),
    }
}

pub fn build_fail_closed_execution_prerequisites_contract(
    project_id: &str,
) -> ProjectDGraphExecutionPrerequisitesResponse {
    let policy = build_default_deny_execution_policy_contract(project_id);
    let approval = build_fail_closed_operator_approval_contract(project_id);
    let integrity = build_fail_closed_artifact_integrity_contract(project_id);
    let allowlists = build_fail_closed_execution_allowlist_contract(project_id);
    let negotiation = build_fail_closed_capability_negotiation_contract(project_id);
    let limits = build_fail_closed_resource_limits_contract(project_id);
    let rollback_audit = build_fail_closed_rollback_audit_contract(project_id);

    let prerequisites = vec![
        prerequisite(
            "explicit_execution_policy",
            "C47_default_deny_execution_policy",
            &policy.contract_version,
            policy.policy_configured
                && policy.execution_enabled
                && policy.authorization_granted,
            "no explicit project-scoped execution policy is configured or enabled",
        ),
        prerequisite(
            "authenticated_operator_approval",
            "C48_fail_closed_operator_approval",
            &approval.contract_version,
            approval.identity_authenticated
                && approval.approval_present
                && approval.approval_valid
                && approval.authorization_granted,
            "no authenticated project-scoped operator approval is present and valid",
        ),
        prerequisite(
            "artifact_integrity_and_provenance",
            "C49_fail_closed_artifact_integrity_and_provenance",
            &integrity.contract_version,
            integrity.artifact_inventory_loaded
                && integrity.artifact_digests_verified
                && integrity.signatures_verified
                && integrity.provenance_verified
                && integrity.integrity_verified
                && integrity.authorization_granted,
            "artifact inventory, digests, signatures, provenance, and integrity are not verified",
        ),
        prerequisite(
            "target_and_function_allowlists",
            "C50_fail_closed_execution_allowlists",
            &allowlists.contract_version,
            allowlists.allowlist_configured
                && allowlists.target_allowlist_loaded
                && allowlists.module_allowlist_loaded
                && allowlists.runtime_allowlist_loaded
                && allowlists.function_allowlist_loaded
                && !allowlists.target_allowlist.is_empty()
                && !allowlists.module_allowlist.is_empty()
                && !allowlists.runtime_allowlist.is_empty()
                && !allowlists.function_allowlist.is_empty()
                && allowlists.authorization_granted,
            "project-scoped target, module, runtime, and function allowlists are not configured and loaded",
        ),
        prerequisite(
            "capability_negotiation",
            "C51_fail_closed_capability_negotiation",
            &negotiation.contract_version,
            negotiation.requirements_loaded
                && negotiation.advertisements_loaded
                && negotiation.compatibility_evaluated
                && negotiation.negotiation_succeeded
                && negotiation.authorization_granted,
            "capability requirements and advertisements are not loaded and compatibility is not negotiated",
        ),
        prerequisite(
            "resource_limits",
            "C52_fail_closed_resource_limits",
            &limits.contract_version,
            limits.limits_configured
                && limits.limits_loaded
                && limits.fuel_limit_configured
                && limits.timeout_limit_configured
                && limits.memory_limit_configured
                && limits.host_capability_limits_loaded
                && limits.limits_enforced
                && limits.authorization_granted,
            "fuel, timeout, memory, and host-capability limits are not configured, loaded, and enforced",
        ),
        prerequisite(
            "rollback_and_audit",
            "C53_fail_closed_rollback_and_audit",
            &rollback_audit.contract_version,
            rollback_audit.rollback_plan_loaded
                && rollback_audit.rollback_checkpoints_loaded
                && !rollback_audit.rollback_checkpoints.is_empty()
                && rollback_audit.rollback_preconditions_evaluated
                && rollback_audit.rollback_guaranteed
                && rollback_audit.audit_policy_loaded
                && rollback_audit.audit_sink_configured
                && rollback_audit.audit_records_loaded
                && !rollback_audit.audit_records.is_empty()
                && rollback_audit.audit_trail_complete
                && rollback_audit.authorization_granted,
            "rollback plans, verified checkpoints, guarantees, and a complete audit trail are not available",
        ),
    ];

    let satisfied_gate_count = prerequisites
        .iter()
        .filter(|prerequisite| prerequisite.satisfied)
        .count() as u64;
    let all_prerequisites_satisfied = satisfied_gate_count == REQUIRED_GATE_COUNT
        && prerequisites.len() as u64 == REQUIRED_GATE_COUNT;

    ProjectDGraphExecutionPrerequisitesResponse {
        project_id: project_id.to_string(),
        contract_version: EXECUTION_PREREQUISITES_CONTRACT_VERSION.to_string(),
        aggregation_status: if all_prerequisites_satisfied {
            "complete"
        } else {
            "incomplete"
        }
        .to_string(),
        required_gate_count: REQUIRED_GATE_COUNT,
        satisfied_gate_count,
        all_prerequisites_satisfied,
        prerequisites,
        authorization_granted: false,
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "execution_prerequisite_aggregation_boundary",
            "fail_closed_contracts_aggregated": true,
            "source_contract_count": REQUIRED_GATE_COUNT,
            "readiness_is_authorization": false,
            "planner_proposals_are_authorization": false,
            "aggregation_is_authorization": false,
            "persisted": false,
            "execution_performed": false,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregation_contains_exactly_seven_fail_closed_prerequisites() {
        let contract = build_fail_closed_execution_prerequisites_contract("project-a");

        assert_eq!(contract.project_id, "project-a");
        assert_eq!(contract.aggregation_status, "incomplete");
        assert_eq!(contract.required_gate_count, 7);
        assert_eq!(contract.satisfied_gate_count, 0);
        assert!(!contract.all_prerequisites_satisfied);
        assert_eq!(contract.prerequisites.len(), 7);
        assert!(contract
            .prerequisites
            .iter()
            .all(|prerequisite| !prerequisite.satisfied));
        assert!(contract
            .prerequisites
            .iter()
            .all(|prerequisite| prerequisite.source_contract_version == "0.1.0"));
        assert!(!contract.authorization_granted);
        assert!(!contract.side_effects_performed);
    }

    #[test]
    fn serialized_aggregation_preserves_gate_order_and_non_authoritative_semantics() {
        let value = serde_json::to_value(build_fail_closed_execution_prerequisites_contract(
            "project-a",
        ))
        .expect("execution prerequisites contract should serialize");

        let gates = value["prerequisites"]
            .as_array()
            .expect("prerequisites should be an array")
            .iter()
            .map(|item| item["gate"].as_str().expect("gate should be a string"))
            .collect::<Vec<_>>();

        assert_eq!(
            gates.as_slice(),
            EXECUTION_PREREQUISITE_GATE_NAMES.as_slice()
        );
        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["aggregation_status"], "incomplete");
        assert_eq!(value["required_gate_count"], 7);
        assert_eq!(value["satisfied_gate_count"], 0);
        assert_eq!(value["all_prerequisites_satisfied"], false);
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(value["metadata"]["fail_closed_contracts_aggregated"], true);
        assert_eq!(value["metadata"]["source_contract_count"], 7);
        assert_eq!(value["metadata"]["aggregation_is_authorization"], false);
        assert_eq!(value["metadata"]["persisted"], false);
        assert_eq!(value["metadata"]["execution_performed"], false);
    }

    #[test]
    fn source_guard_contains_no_load_write_authorization_or_execution_path() {
        let source = include_str!("dgraph_execution_prerequisites.rs");
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
            "authorization_granted: true",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "execution prerequisites contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
