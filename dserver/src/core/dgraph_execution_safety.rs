use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{
    dgraph_execution_policy::EXECUTION_OPERATION_CLASSES,
    dgraph_execution_prerequisites::build_fail_closed_execution_prerequisites_contract,
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
pub struct DGraphExecutionPrerequisiteSummary {
    pub aggregation_status: String,
    pub required_gate_count: u64,
    pub satisfied_gate_count: u64,
    pub all_prerequisites_satisfied: bool,
    pub aggregation_is_authorization: bool,
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
    pub prerequisite_summary: DGraphExecutionPrerequisiteSummary,
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
    let prerequisites = build_fail_closed_execution_prerequisites_contract(project_id);
    let prerequisite_summary = DGraphExecutionPrerequisiteSummary {
        aggregation_status: prerequisites.aggregation_status.clone(),
        required_gate_count: prerequisites.required_gate_count,
        satisfied_gate_count: prerequisites.satisfied_gate_count,
        all_prerequisites_satisfied: prerequisites.all_prerequisites_satisfied,
        aggregation_is_authorization: false,
    };
    let gates = prerequisites
        .prerequisites
        .iter()
        .map(|prerequisite| DGraphExecutionSafetyGate {
            gate: prerequisite.gate.clone(),
            satisfied: prerequisite.satisfied,
            reason: prerequisite.reason.clone(),
        })
        .collect();

    ProjectDGraphExecutionSafetyContractResponse {
        project_id: project_id.to_string(),
        contract_version: EXECUTION_SAFETY_CONTRACT_VERSION.to_string(),
        decision: "denied".to_string(),
        authorization_granted: false,
        evidence,
        prerequisite_summary,
        gates,
        operations: EXECUTION_OPERATION_CLASSES
            .into_iter()
            .map(|operation| DGraphOperationAuthorization {
                operation: operation.to_string(),
                authorized: false,
                reason: "execution authority and the allow transition are not implemented; prerequisite aggregation, readiness, and planner output are advisory evidence only"
                    .to_string(),
            })
            .collect(),
        side_effects_performed: false,
        metadata: Some(json!({
            "mode": "evaluation_only",
            "contract_scope": "execution_authorization_boundary",
            "readiness_is_authorization": false,
            "planner_proposals_are_authorization": false,
            "prerequisite_aggregation_is_authorization": false,
            "execution_allow_transition_implemented": false,
            "execution_performed": false,
        })),
    }
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
    fn c55_projects_all_prerequisites_into_execution_gates() {
        let prerequisites = build_fail_closed_execution_prerequisites_contract("project-a");
        let contract = build_project_dgraph_execution_safety_contract("project-a", &[]);

        assert_eq!(contract.gates.len(), prerequisites.prerequisites.len());
        for (gate, prerequisite) in contract.gates.iter().zip(&prerequisites.prerequisites) {
            assert_eq!(gate.gate, prerequisite.gate);
            assert_eq!(gate.satisfied, prerequisite.satisfied);
            assert_eq!(gate.reason, prerequisite.reason);
        }
    }

    #[test]
    fn c55_exposes_the_incomplete_prerequisite_summary() {
        let contract = build_project_dgraph_execution_safety_contract("project-a", &[]);

        assert_eq!(
            contract.prerequisite_summary.aggregation_status,
            "incomplete"
        );
        assert_eq!(contract.prerequisite_summary.required_gate_count, 7);
        assert_eq!(contract.prerequisite_summary.satisfied_gate_count, 0);
        assert!(!contract.prerequisite_summary.all_prerequisites_satisfied);
        assert!(!contract.prerequisite_summary.aggregation_is_authorization);
    }

    #[test]
    fn c55_ready_graph_and_prerequisite_evidence_still_deny_every_operation() {
        let contract = build_project_dgraph_execution_safety_contract(
            "smart-city-campinas",
            &[snapshot("fog-01"), snapshot("fog-02")],
        );

        assert_eq!(contract.evidence.readiness_status, "ready");
        assert_eq!(
            contract.prerequisite_summary.aggregation_status,
            "incomplete"
        );
        assert_eq!(contract.decision, "denied");
        assert!(!contract.authorization_granted);
        assert!(contract.gates.iter().all(|gate| !gate.satisfied));
        assert!(contract.operations.iter().all(|item| !item.authorized));
    }

    #[test]
    fn c55_serialized_contract_marks_aggregation_and_allow_transition_non_authoritative() {
        let value = serde_json::to_value(build_project_dgraph_execution_safety_contract(
            "project-a",
            &[],
        ))
        .expect("contract should serialize");

        assert_eq!(
            value["prerequisite_summary"]["aggregation_status"],
            "incomplete"
        );
        assert_eq!(value["prerequisite_summary"]["required_gate_count"], 7);
        assert_eq!(value["prerequisite_summary"]["satisfied_gate_count"], 0);
        assert_eq!(
            value["prerequisite_summary"]["all_prerequisites_satisfied"],
            false
        );
        assert_eq!(
            value["metadata"]["prerequisite_aggregation_is_authorization"],
            false
        );
        assert_eq!(
            value["metadata"]["execution_allow_transition_implemented"],
            false
        );
    }

    #[test]
    fn c55_source_has_no_parallel_gate_table_or_authorization_path() {
        let source = include_str!("dgraph_execution_safety.rs");
        let implementation = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(implementation.contains("build_fail_closed_execution_prerequisites_contract"));
        assert!(!implementation.contains("EXECUTION_SAFETY_GATE_NAMES"));
        assert!(!implementation.contains("EXECUTION_PREREQUISITE_GATE_NAMES"));
        assert!(!implementation.contains("fn required_execution_gates"));
        assert!(!implementation.contains("authorization_granted: true"));
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
