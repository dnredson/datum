use std::collections::{BTreeSet, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    controlled_dgraph_deployment::ControlledObservedDGraphState,
    deployment_manifest::{validate_deployment_manifest, DeploymentAssignment, DeploymentManifest},
    sentinel_registration::{SentinelLeaseStatus, SentinelRegistrationView},
};

pub const DEPLOYMENT_READINESS_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentReadinessStatus {
    ReadyForExplicitExecution,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentReadinessStatus {
    Ready,
    WaitingOnDependencies,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadinessBlockReason {
    pub code: String,
    pub scope: String,
    pub node_id: Option<String>,
    pub assignment_id: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssignmentReadinessEvidence {
    pub assignment_id: String,
    pub node_id: String,
    pub deployment_stage: Option<u64>,
    pub depends_on_assignments: Vec<String>,
    pub waiting_on_assignments: Vec<String>,
    pub status: AssignmentReadinessStatus,
    pub sentinel_active: bool,
    pub resource_fresh: bool,
    pub wasm_capable: bool,
    pub artifact_integrity_valid: bool,
    pub execution_boundary_valid: bool,
    pub successful_runtime_report_present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentReadinessSummary {
    pub readiness_version: String,
    pub manifest_id: String,
    pub project_id: String,
    pub evaluated_at_utc: String,
    pub status: DeploymentReadinessStatus,
    pub assignment_count: usize,
    pub target_node_count: usize,
    pub ready_assignment_count: usize,
    pub waiting_assignment_count: usize,
    pub completed_assignment_count: usize,
    pub blocked_assignment_count: usize,
    pub blocking_reasons: Vec<ReadinessBlockReason>,
    pub assignments: Vec<AssignmentReadinessEvidence>,
    pub automatic_execution_allowed: bool,
    pub governed_execution_performed: bool,
    pub readiness_side_effects_performed: bool,
}

pub fn build_deployment_readiness(
    manifest: &DeploymentManifest,
    observed: &ControlledObservedDGraphState,
    registrations: &[SentinelRegistrationView],
    reports: &[Value],
    now: DateTime<Utc>,
) -> DeploymentReadinessSummary {
    let mut blocking_reasons = Vec::new();
    let manifest_validation = validate_deployment_manifest(manifest);
    if let Err(error) = &manifest_validation {
        blocking_reasons.push(reason(
            "manifest_invalid",
            "deployment",
            None,
            None,
            error.clone(),
        ));
    }

    if !observed.validation_errors.is_empty() {
        blocking_reasons.push(reason(
            "observed_dgraph_invalid",
            "deployment",
            None,
            None,
            observed.validation_errors.join("; "),
        ));
    }

    if observed.project_id != manifest.project_id || observed.manifest_id != manifest.manifest_id {
        blocking_reasons.push(reason(
            "observed_identity_mismatch",
            "deployment",
            None,
            None,
            "observed D-Graph identity does not match active manifest".to_string(),
        ));
    }

    let execution_boundary_valid = manifest.execution_boundary.explicit_invocation_required
        && !manifest.execution_boundary.automatic_execution_allowed
        && !manifest.execution_boundary.governed_execution_authorized;
    if !execution_boundary_valid {
        blocking_reasons.push(reason(
            "execution_boundary_invalid",
            "deployment",
            None,
            None,
            "controlled execution boundary must require explicit invocation and keep automatic/governed execution disabled".to_string(),
        ));
    }

    let target_nodes = manifest
        .assignments
        .iter()
        .map(|assignment| assignment.target_node.clone())
        .collect::<BTreeSet<_>>();

    for node_id in &target_nodes {
        match registrations.iter().find(|registration| registration.node_id == *node_id) {
            None => blocking_reasons.push(reason(
                "sentinel_missing",
                "node",
                Some(node_id.clone()),
                None,
                format!("target node '{node_id}' has no Sentinel registration"),
            )),
            Some(registration) if registration.status != SentinelLeaseStatus::Active => {
                blocking_reasons.push(reason(
                    "sentinel_stale",
                    "node",
                    Some(node_id.clone()),
                    None,
                    format!("target node '{node_id}' Sentinel lease is stale"),
                ))
            }
            Some(_) => {}
        }

        let node = observed.graph.nodes.iter().find(|node| node.node_id == *node_id);
        let resource = node.and_then(resource_observation);
        match resource {
            None => blocking_reasons.push(reason(
                "resource_observation_missing",
                "node",
                Some(node_id.clone()),
                None,
                format!("target node '{node_id}' has no joined resource observation"),
            )),
            Some(resource) => {
                let freshness = resource
                    .get("freshness")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if freshness != "fresh" {
                    blocking_reasons.push(reason(
                        "resource_observation_stale",
                        "node",
                        Some(node_id.clone()),
                        None,
                        format!("target node '{node_id}' resource observation is '{freshness}'"),
                    ));
                }
                if !supported_capability(resource, "wasm") {
                    blocking_reasons.push(reason(
                        "wasm_capability_missing",
                        "node",
                        Some(node_id.clone()),
                        None,
                        format!("target node '{node_id}' does not report supported capability 'wasm'"),
                    ));
                }
            }
        }
    }

    blocking_reasons.sort_by(|a, b| {
        (&a.code, &a.node_id, &a.assignment_id, &a.detail)
            .cmp(&(&b.code, &b.node_id, &b.assignment_id, &b.detail))
    });
    blocking_reasons.dedup();

    let successful_assignments = reports
        .iter()
        .filter(|report| report.get("status").and_then(Value::as_str) == Some("executed"))
        .filter_map(|report| report.get("assignment_id").and_then(Value::as_str))
        .map(ToString::to_string)
        .collect::<HashSet<_>>();

    let global_blocked = !blocking_reasons.is_empty();
    let mut assignments = manifest
        .assignments
        .iter()
        .map(|assignment| {
            build_assignment_readiness(
                assignment,
                observed,
                registrations,
                &successful_assignments,
                execution_boundary_valid,
                global_blocked,
                manifest_validation.is_ok(),
            )
        })
        .collect::<Vec<_>>();
    assignments.sort_by(|a, b| a.assignment_id.cmp(&b.assignment_id));

    let ready_assignment_count = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentReadinessStatus::Ready)
        .count();
    let waiting_assignment_count = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentReadinessStatus::WaitingOnDependencies)
        .count();
    let completed_assignment_count = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentReadinessStatus::Completed)
        .count();
    let blocked_assignment_count = assignments
        .iter()
        .filter(|assignment| assignment.status == AssignmentReadinessStatus::Blocked)
        .count();

    DeploymentReadinessSummary {
        readiness_version: DEPLOYMENT_READINESS_VERSION.to_string(),
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        evaluated_at_utc: now.to_rfc3339(),
        status: if global_blocked {
            DeploymentReadinessStatus::Blocked
        } else {
            DeploymentReadinessStatus::ReadyForExplicitExecution
        },
        assignment_count: assignments.len(),
        target_node_count: target_nodes.len(),
        ready_assignment_count,
        waiting_assignment_count,
        completed_assignment_count,
        blocked_assignment_count,
        blocking_reasons,
        assignments,
        automatic_execution_allowed: false,
        governed_execution_performed: false,
        readiness_side_effects_performed: false,
    }
}

fn build_assignment_readiness(
    assignment: &DeploymentAssignment,
    observed: &ControlledObservedDGraphState,
    registrations: &[SentinelRegistrationView],
    successful_assignments: &HashSet<String>,
    execution_boundary_valid: bool,
    global_blocked: bool,
    manifest_valid: bool,
) -> AssignmentReadinessEvidence {
    let sentinel_active = registrations
        .iter()
        .find(|registration| registration.node_id == assignment.target_node)
        .map(|registration| registration.status == SentinelLeaseStatus::Active)
        .unwrap_or(false);
    let resource = observed
        .graph
        .nodes
        .iter()
        .find(|node| node.node_id == assignment.target_node)
        .and_then(resource_observation);
    let resource_fresh = resource
        .and_then(|resource| resource.get("freshness"))
        .and_then(Value::as_str)
        == Some("fresh");
    let wasm_capable = resource
        .map(|resource| supported_capability(resource, "wasm"))
        .unwrap_or(false);
    let artifact_integrity_valid = manifest_valid
        && assignment.artifact.content_id
            == format!("sha256:{}", assignment.artifact.sha256.to_ascii_lowercase());
    let depends_on_assignments = assignment_dependencies(assignment);
    let waiting_on_assignments = depends_on_assignments
        .iter()
        .filter(|dependency| !successful_assignments.contains(*dependency))
        .cloned()
        .collect::<Vec<_>>();
    let successful_runtime_report_present = successful_assignments.contains(&assignment.assignment_id);

    let status = if successful_runtime_report_present {
        AssignmentReadinessStatus::Completed
    } else if global_blocked
        || !sentinel_active
        || !resource_fresh
        || !wasm_capable
        || !artifact_integrity_valid
        || !execution_boundary_valid
    {
        AssignmentReadinessStatus::Blocked
    } else if !waiting_on_assignments.is_empty() {
        AssignmentReadinessStatus::WaitingOnDependencies
    } else {
        AssignmentReadinessStatus::Ready
    };

    AssignmentReadinessEvidence {
        assignment_id: assignment.assignment_id.clone(),
        node_id: assignment.target_node.clone(),
        deployment_stage: assignment_stage(assignment),
        depends_on_assignments,
        waiting_on_assignments,
        status,
        sentinel_active,
        resource_fresh,
        wasm_capable,
        artifact_integrity_valid,
        execution_boundary_valid,
        successful_runtime_report_present,
    }
}

fn resource_observation(node: &crate::core::dgraph::DGraphNode) -> Option<&serde_json::Map<String, Value>> {
    node.metadata
        .as_ref()?
        .as_object()?
        .get("resource_observation")?
        .as_object()
}

fn supported_capability(resource: &serde_json::Map<String, Value>, capability: &str) -> bool {
    resource
        .get("supported_capabilities")
        .and_then(Value::as_array)
        .map(|items| items.iter().any(|item| item.as_str() == Some(capability)))
        .unwrap_or(false)
}

fn assignment_dependencies(assignment: &DeploymentAssignment) -> Vec<String> {
    assignment
        .metadata
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("depends_on_assignments"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn assignment_stage(assignment: &DeploymentAssignment) -> Option<u64> {
    assignment
        .metadata
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("deployment_stage"))
        .and_then(Value::as_u64)
}

fn reason(
    code: &str,
    scope: &str,
    node_id: Option<String>,
    assignment_id: Option<String>,
    detail: String,
) -> ReadinessBlockReason {
    ReadinessBlockReason {
        code: code.to_string(),
        scope: scope.to_string(),
        node_id,
        assignment_id,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use crate::core::{
        controlled_dgraph_deployment::ControlledObservedDGraphState,
        deployment_manifest::{
            build_deployment_artifact_identity, DeploymentAssignment, DeploymentExecutionBoundary,
            DeploymentManifest, DeploymentRuntimeLimits, DeploymentStrategy, GraphDistribution,
            DEPLOYMENT_MANIFEST_VERSION,
        },
        dgraph::{DGraph, DGraphNode, DGraphSummary},
        sentinel_registration::{SentinelLeaseStatus, SentinelRegistrationView},
    };

    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 31, 18, 0, 0).unwrap()
    }

    fn assignment(id: &str, node: &str, stage: u64, deps: &[&str]) -> DeploymentAssignment {
        DeploymentAssignment {
            assignment_id: id.to_string(),
            module_id: format!("module-{id}"),
            function_name: "run".to_string(),
            artifact: build_deployment_artifact_identity(
                format!("artifact-{id}"),
                format!("https://example.invalid/{id}.wasm"),
                SHA,
            )
            .unwrap(),
            target_node: node.to_string(),
            limits: DeploymentRuntimeLimits {
                fuel: Some(1000),
                timeout_ms: Some(1000),
            },
            metadata: Some(json!({
                "deployment_stage": stage,
                "depends_on_assignments": deps,
            })),
        }
    }

    fn manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest-ready".to_string(),
            project_id: "project-ready".to_string(),
            strategy: DeploymentStrategy::Distributed,
            graph_distribution: GraphDistribution::NodeSlice,
            source_dgraph_id: Some("dgraph-ready".to_string()),
            assignments: vec![
                assignment("a", "fog-01", 0, &[]),
                assignment("b", "local-01", 1, &["a"]),
            ],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    fn resource_node(node_id: &str, freshness: &str, capabilities: &[&str]) -> DGraphNode {
        DGraphNode {
            node_id: node_id.to_string(),
            metadata: Some(json!({
                "resource_observation": {
                    "freshness": freshness,
                    "supported_capabilities": capabilities,
                }
            })),
            ..Default::default()
        }
    }

    fn observed() -> ControlledObservedDGraphState {
        ControlledObservedDGraphState {
            source_dgraph_id: "dgraph-ready".to_string(),
            manifest_id: "manifest-ready".to_string(),
            project_id: "project-ready".to_string(),
            materialized_at_utc: now().to_rfc3339(),
            report_count: 0,
            graph: DGraph {
                project_id: "project-ready".to_string(),
                nodes: vec![
                    resource_node("fog-01", "fresh", &["wasm"]),
                    resource_node("local-01", "fresh", &["wasm"]),
                ],
                ..Default::default()
            },
            summary: DGraphSummary::default(),
            validation_errors: Vec::new(),
            side_effects_performed: false,
            metadata: None,
        }
    }

    fn registration(node_id: &str, status: SentinelLeaseStatus) -> SentinelRegistrationView {
        SentinelRegistrationView {
            node_id: node_id.to_string(),
            instance_id: format!("instance-{node_id}"),
            stage: None,
            capabilities: vec!["wasm".to_string()],
            registered_at_utc: now().to_rfc3339(),
            last_seen_at_utc: now().to_rfc3339(),
            lease_seconds: 300,
            status,
            deployment_side_effects_performed: false,
            authenticated_identity_attested: false,
        }
    }

    fn registrations() -> Vec<SentinelRegistrationView> {
        vec![
            registration("fog-01", SentinelLeaseStatus::Active),
            registration("local-01", SentinelLeaseStatus::Active),
        ]
    }

    #[test]
    fn healthy_envelope_is_ready_with_dependency_waiting() {
        let summary = build_deployment_readiness(&manifest(), &observed(), &registrations(), &[], now());
        assert_eq!(summary.status, DeploymentReadinessStatus::ReadyForExplicitExecution);
        assert_eq!(summary.ready_assignment_count, 1);
        assert_eq!(summary.waiting_assignment_count, 1);
        assert_eq!(summary.blocked_assignment_count, 0);
        assert!(!summary.automatic_execution_allowed);
    }

    #[test]
    fn stale_sentinel_blocks_with_deterministic_reason() {
        let mut regs = registrations();
        regs[0].status = SentinelLeaseStatus::Stale;
        let summary = build_deployment_readiness(&manifest(), &observed(), &regs, &[], now());
        assert_eq!(summary.status, DeploymentReadinessStatus::Blocked);
        assert!(summary.blocking_reasons.iter().any(|reason| reason.code == "sentinel_stale" && reason.node_id.as_deref() == Some("fog-01")));
    }

    #[test]
    fn stale_resource_blocks() {
        let mut graph = observed();
        graph.graph.nodes[0].metadata.as_mut().unwrap()["resource_observation"]["freshness"] = json!("stale");
        let summary = build_deployment_readiness(&manifest(), &graph, &registrations(), &[], now());
        assert_eq!(summary.status, DeploymentReadinessStatus::Blocked);
        assert!(summary.blocking_reasons.iter().any(|reason| reason.code == "resource_observation_stale"));
    }

    #[test]
    fn missing_wasm_capability_blocks() {
        let mut graph = observed();
        graph.graph.nodes[1].metadata.as_mut().unwrap()["resource_observation"]["supported_capabilities"] = json!(["docker"]);
        let summary = build_deployment_readiness(&manifest(), &graph, &registrations(), &[], now());
        assert_eq!(summary.status, DeploymentReadinessStatus::Blocked);
        assert!(summary.blocking_reasons.iter().any(|reason| reason.code == "wasm_capability_missing" && reason.node_id.as_deref() == Some("local-01")));
    }

    #[test]
    fn successful_upstream_report_releases_dependency() {
        let reports = vec![json!({"assignment_id":"a","status":"executed"})];
        let summary = build_deployment_readiness(&manifest(), &observed(), &registrations(), &reports, now());
        assert_eq!(summary.completed_assignment_count, 1);
        assert_eq!(summary.ready_assignment_count, 1);
        assert_eq!(summary.waiting_assignment_count, 0);
        let b = summary.assignments.iter().find(|item| item.assignment_id == "b").unwrap();
        assert_eq!(b.status, AssignmentReadinessStatus::Ready);
    }

    #[test]
    fn observed_validation_error_blocks() {
        let mut graph = observed();
        graph.validation_errors = vec!["bad relationship".to_string()];
        let summary = build_deployment_readiness(&manifest(), &graph, &registrations(), &[], now());
        assert_eq!(summary.status, DeploymentReadinessStatus::Blocked);
        assert!(summary.blocking_reasons.iter().any(|reason| reason.code == "observed_dgraph_invalid"));
    }
}
