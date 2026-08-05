use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{
    dgraph_execution_policy::EXECUTION_OPERATION_CLASSES,
    dgraph_readiness::build_project_dgraph_readiness_summary,
    local_dgraph_snapshot::LocalDGraphSnapshot,
};

const EXECUTION_SAFETY_CONTRACT_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphExecutionSafetyEvidence {
    pub readiness_status: String,
    pub source_snapshot_count: u64,
    pub warning_count: u64,
    pub blocked_count: u64,
    pub degraded_flow_count: u64,
    pub placement_proposal_count: u64,
    pub reconfiguration_proposal_count: u64,
    pub planner_output_is_advisory: bool,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphExecutionSafetyGate {
    pub gate: String,
    pub satisfied: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphOperationAuthorization {
    pub operation: String,
    pub authorized: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphExecutionSafetyContractResponse {
    pub project_id: String,
    pub contract_version: String,
    pub decision: String,
    pub authorization_granted: bool,
    pub evidence: DGraphExecutionSafetyEvidence,
    pub gates: Vec<DGraphExecutionSafetyGate>,
    pub operations: Vec<DGraphOperationAuthorization>,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_project_dgraph_execution_safety_contract(
    project_id: &str,
    snapshots: &[LocalDGraphSnapshot],
) -> ProjectDGraphExecutionSafetyContractResponse {
    let readiness = build_project_dgraph_readiness_summary(project_id, snapshots);
    let evidence = DGraphExecutionSafetyEvidence {
        readiness_status: readiness.readiness_status.clone(),
        source_snapshot_count: readiness.source_snapshot_count,
        warning_count: readiness.warning_count,
        blocked_count: readiness.blocked_count,
        degraded_flow_count: readiness.degraded_flow_count,
        placement_proposal_count: readiness.planner_summary.summary.placement_count,
        reconfiguration_proposal_count: readiness.planner_summary.summary.reconfiguration_count,
        planner_output_is_advisory: true,
        side_effects_performed: readiness.side_effects_performed,
    };

    ProjectDGraphExecutionSafetyContractResponse {
        project_id: project_id.to_string(),
        contract_version: EXECUTION_SAFETY_CONTRACT_VERSION.to_string(),
        decision: "denied".to_string(),
        authorization_granted: false,
        evidence,
        gates: required_execution_gates(),
        operations: EXECUTION_OPERATION_CLASSES
            .into_iter()
            .map(|operation| DGraphOperationAuthorization {
                operation: operation.to_string(),
                authorized: false,
                reason: "execution authority is not implemented; readiness and planner output are advisory evidence only"
                    .to_string(),
            })
            .collect(),
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "execution_authorization_boundary",
            "readiness_is_authorization": false,
            "planner_proposals_are_authorization": false,
            "execution_performed": false,
        })),
    }
}

fn required_execution_gates() -> Vec<DGraphExecutionSafetyGate> {
    [
        (
            "explicit_execution_policy",
            "only the built-in deny-all fallback is active; no explicit project-scoped execution policy is configured",
        ),
        (
            "authenticated_operator_approval",
            "the fail-closed operator approval contract reports no authenticated project-scoped execution approval",
        ),
        (
            "artifact_integrity_and_provenance",
            "artifact signatures, hashes, and provenance are not verified for execution",
        ),
        (
            "target_and_function_allowlists",
            "target, module, runtime, and function allowlists are not enforced",
        ),
        (
            "capability_negotiation",
            "execution capability negotiation is not implemented",
        ),
        (
            "resource_limits",
            "execution fuel, timeout, and host-capability limits are not enforced",
        ),
        (
            "rollback_and_audit",
            "rollback guarantees and an execution audit trail are not implemented",
        ),
    ]
    .into_iter()
    .map(|(gate, reason)| DGraphExecutionSafetyGate {
        gate: gate.to_string(),
        satisfied: false,
        reason: reason.to_string(),
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::local_dgraph_snapshot::{
        LocalDGraphSnapshot, LocalFlowSnapshot, LocalModuleSnapshot, LocalRuntimeSnapshot,
    };

    fn snapshot(node_id: &str) -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            project_id: "smart-city-campinas".to_string(),
            node_id: node_id.to_string(),
            snapshot_id: format!("snapshot-{node_id}"),
            reported_at_utc: "2026-08-04T12:00:00Z".to_string(),
            source: Some("fixture".to_string()),
            generated_at_utc: None,
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: "runtime-container".to_string(),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "fixture".to_string(),
                metadata: None,
            }],
            modules: vec![LocalModuleSnapshot {
                module_id: format!("module-{node_id}"),
                module_kind: Some("service".to_string()),
                desired_state: Some("running".to_string()),
                observed_state: Some("configured".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: None,
                local_mapping_key: None,
                local_container_name: None,
                source: "fixture".to_string(),
                metadata: None,
            }],
            flows: vec![LocalFlowSnapshot {
                flow_id: "flow-fixture".to_string(),
                flow_kind: Some("service_dependency".to_string()),
                modules: vec![format!("module-{node_id}")],
                edges: vec![],
                status: Some("configured".to_string()),
                source: "fixture".to_string(),
                metadata: None,
            }],
            health: vec![],
            capabilities: vec![],
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn empty_project_is_denied_fail_closed() {
        let contract = build_project_dgraph_execution_safety_contract("isolated-project", &[]);

        assert_eq!(contract.decision, "denied");
        assert!(!contract.authorization_granted);
        assert_eq!(contract.evidence.readiness_status, "incomplete");
        assert!(contract.gates.iter().all(|gate| !gate.satisfied));
        assert!(contract.operations.iter().all(|item| !item.authorized));
        assert!(!contract.side_effects_performed);
    }

    #[test]
    fn ready_metadata_graph_does_not_grant_execution_authority() {
        let contract = build_project_dgraph_execution_safety_contract(
            "smart-city-campinas",
            &[snapshot("fog-01"), snapshot("fog-02")],
        );

        assert_eq!(contract.evidence.readiness_status, "ready");
        assert_eq!(contract.decision, "denied");
        assert!(!contract.authorization_granted);
        assert!(contract.evidence.planner_output_is_advisory);
        assert!(contract.operations.iter().all(|item| !item.authorized));
    }

    #[test]
    fn contract_denies_every_operation_class_explicitly() {
        let contract = build_project_dgraph_execution_safety_contract("project-a", &[]);
        let operations = contract
            .operations
            .iter()
            .map(|item| item.operation.as_str())
            .collect::<Vec<_>>();

        assert_eq!(operations, EXECUTION_OPERATION_CLASSES);
        assert!(contract.operations.iter().all(|item| !item.authorized));
    }

    #[test]
    fn serialized_contract_keeps_readiness_and_planner_non_authoritative() {
        let value = serde_json::to_value(build_project_dgraph_execution_safety_contract(
            "project-a",
            &[],
        ))
        .expect("contract should serialize");

        assert_eq!(value["contract_version"], "0.1.0");
        assert_eq!(value["decision"], "denied");
        assert_eq!(value["authorization_granted"], false);
        assert_eq!(value["metadata"]["readiness_is_authorization"], false);
        assert_eq!(
            value["metadata"]["planner_proposals_are_authorization"],
            false
        );
        assert_eq!(value["metadata"]["execution_performed"], false);
        assert_eq!(value["side_effects_performed"], false);
    }

    #[test]
    fn source_guard_contains_no_execution_or_mutation_path() {
        let source = include_str!("dgraph_execution_safety.rs");
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
        ] {
            assert!(
                !implementation.contains(forbidden),
                "execution safety contract must not contain forbidden pattern: {forbidden}"
            );
        }
    }
}
