use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    deployment_manifest::DeploymentAssignment,
    dgraph::{DGraph, DGraphFlowEdge},
};

pub const DEPLOYMENT_DEPENDENCY_PLAN_VERSION: &str = "0.1.0";
const DEPLOYMENT_DEPENDENCY_FLOW_KIND: &str = "deployment_dependency";
const DEPLOYMENT_DEPENDENCY_METADATA_KEY: &str = "deployment_dependency";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentDependencyAssignmentStage {
    pub assignment_id: String,
    pub module_id: String,
    pub target_node: String,
    pub stage: u32,
    pub depends_on_assignments: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentDependencyStage {
    pub stage: u32,
    pub assignment_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentDependencyPlan {
    pub plan_version: String,
    pub assignment_count: usize,
    pub dependency_edge_count: usize,
    pub stage_count: usize,
    pub stages: Vec<DeploymentDependencyStage>,
    pub assignments: Vec<DeploymentDependencyAssignmentStage>,
    pub automatic_execution_allowed: bool,
    pub governed_execution_performed: bool,
}

pub fn derive_deployment_dependency_plan(
    graph: &DGraph,
    assignments: &[DeploymentAssignment],
) -> Result<DeploymentDependencyPlan, String> {
    if assignments.is_empty() {
        return Err("dependency planning requires at least one deployment assignment".to_string());
    }

    let mut assignment_by_id = BTreeMap::new();
    let mut assignment_by_module = BTreeMap::new();
    for assignment in assignments {
        let assignment_id = assignment.assignment_id.trim();
        let module_id = assignment.module_id.trim();
        if assignment_id.is_empty() || module_id.is_empty() {
            return Err(
                "dependency planning requires non-empty assignment_id and module_id".to_string(),
            );
        }
        if assignment_by_id
            .insert(assignment_id.to_string(), assignment)
            .is_some()
        {
            return Err(format!(
                "dependency planning received duplicate assignment_id '{assignment_id}'"
            ));
        }
        if let Some(previous) = assignment_by_module.insert(module_id.to_string(), assignment_id.to_string()) {
            return Err(format!(
                "deployment dependency semantics are ambiguous because module '{module_id}' maps to both assignment '{previous}' and '{assignment_id}'"
            ));
        }
    }

    let mut dependencies: BTreeMap<String, BTreeSet<String>> = assignment_by_id
        .keys()
        .map(|assignment_id| (assignment_id.clone(), BTreeSet::new()))
        .collect();
    let mut outgoing: BTreeMap<String, BTreeSet<String>> = assignment_by_id
        .keys()
        .map(|assignment_id| (assignment_id.clone(), BTreeSet::new()))
        .collect();
    let mut dependency_edges = BTreeSet::new();

    for flow in &graph.flows {
        let flow_marks_dependencies = flow
            .flow_kind
            .as_deref()
            .map(str::trim)
            .map(|kind| kind.eq_ignore_ascii_case(DEPLOYMENT_DEPENDENCY_FLOW_KIND))
            .unwrap_or(false);

        for edge in &flow.edges {
            if !flow_marks_dependencies && !edge_marks_deployment_dependency(edge) {
                continue;
            }

            let from_module = edge.from_module_id.trim();
            let to_module = edge.to_module_id.trim();
            let from_assignment = assignment_by_module.get(from_module).ok_or_else(|| {
                format!(
                    "deployment dependency edge '{}' in flow '{}' references upstream module '{}' without a deployable assignment",
                    edge.edge_id, flow.flow_id, from_module
                )
            })?;
            let to_assignment = assignment_by_module.get(to_module).ok_or_else(|| {
                format!(
                    "deployment dependency edge '{}' in flow '{}' references downstream module '{}' without a deployable assignment",
                    edge.edge_id, flow.flow_id, to_module
                )
            })?;

            if from_assignment == to_assignment {
                return Err(format!(
                    "deployment dependency edge '{}' creates a self-cycle on assignment '{}'",
                    edge.edge_id, from_assignment
                ));
            }

            if dependency_edges.insert((from_assignment.clone(), to_assignment.clone())) {
                dependencies
                    .get_mut(to_assignment)
                    .expect("known downstream assignment")
                    .insert(from_assignment.clone());
                outgoing
                    .get_mut(from_assignment)
                    .expect("known upstream assignment")
                    .insert(to_assignment.clone());
            }
        }
    }

    let mut indegree: BTreeMap<String, usize> = dependencies
        .iter()
        .map(|(assignment_id, deps)| (assignment_id.clone(), deps.len()))
        .collect();
    let mut remaining: BTreeSet<String> = assignment_by_id.keys().cloned().collect();
    let mut stage_by_assignment = BTreeMap::new();
    let mut stages = Vec::new();
    let mut stage_number = 0_u32;

    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|assignment_id| indegree.get(*assignment_id).copied().unwrap_or(0) == 0)
            .cloned()
            .collect::<Vec<_>>();

        if ready.is_empty() {
            return Err(format!(
                "deployment dependency graph contains a cycle involving assignments: {}",
                remaining.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }

        for assignment_id in &ready {
            stage_by_assignment.insert(assignment_id.clone(), stage_number);
        }
        stages.push(DeploymentDependencyStage {
            stage: stage_number,
            assignment_ids: ready.clone(),
        });

        for assignment_id in &ready {
            remaining.remove(assignment_id);
            if let Some(children) = outgoing.get(assignment_id) {
                for child in children {
                    let degree = indegree
                        .get_mut(child)
                        .expect("known dependency child has indegree");
                    *degree = degree
                        .checked_sub(1)
                        .expect("dependency indegree cannot underflow");
                }
            }
        }
        stage_number += 1;
    }

    let assignments = assignment_by_id
        .iter()
        .map(|(assignment_id, assignment)| DeploymentDependencyAssignmentStage {
            assignment_id: assignment_id.clone(),
            module_id: assignment.module_id.clone(),
            target_node: assignment.target_node.clone(),
            stage: *stage_by_assignment
                .get(assignment_id)
                .expect("every assignment receives a stage"),
            depends_on_assignments: dependencies
                .get(assignment_id)
                .expect("every assignment has dependency set")
                .iter()
                .cloned()
                .collect(),
        })
        .collect::<Vec<_>>();

    Ok(DeploymentDependencyPlan {
        plan_version: DEPLOYMENT_DEPENDENCY_PLAN_VERSION.to_string(),
        assignment_count: assignments.len(),
        dependency_edge_count: dependency_edges.len(),
        stage_count: stages.len(),
        stages,
        assignments,
        automatic_execution_allowed: false,
        governed_execution_performed: false,
    })
}

fn edge_marks_deployment_dependency(edge: &DGraphFlowEdge) -> bool {
    metadata_bool(&edge.metadata, DEPLOYMENT_DEPENDENCY_METADATA_KEY).unwrap_or(false)
}

fn metadata_bool(metadata: &Option<Value>, key: &str) -> Option<bool> {
    metadata.as_ref()?.as_object()?.get(key)?.as_bool()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::core::{
        deployment_manifest::{
            build_deployment_artifact_identity, DeploymentRuntimeLimits,
        },
        dgraph::{DGraphFlow, DGraphFlowEdge, DGraphModule},
    };

    const SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn assignment(id: &str, module: &str, node: &str) -> DeploymentAssignment {
        DeploymentAssignment {
            assignment_id: id.to_string(),
            module_id: module.to_string(),
            function_name: "run".to_string(),
            artifact: build_deployment_artifact_identity(
                format!("artifact-{id}"),
                format!("https://example.invalid/{id}.wasm"),
                SHA,
            )
            .unwrap(),
            target_node: node.to_string(),
            limits: DeploymentRuntimeLimits::default(),
            metadata: None,
        }
    }

    fn graph_with_modules(modules: &[&str], edges: Vec<DGraphFlowEdge>) -> DGraph {
        DGraph {
            project_id: "i40-test".to_string(),
            modules: modules
                .iter()
                .map(|module_id| DGraphModule {
                    module_id: (*module_id).to_string(),
                    ..Default::default()
                })
                .collect(),
            flows: vec![DGraphFlow {
                flow_id: "deployment-order".to_string(),
                flow_kind: Some("deployment_dependency".to_string()),
                modules: modules.iter().map(|value| (*value).to_string()).collect(),
                edges,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn edge(id: &str, from: &str, to: &str) -> DGraphFlowEdge {
        DGraphFlowEdge {
            edge_id: id.to_string(),
            from_module_id: from.to_string(),
            to_module_id: to.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn acyclic_three_node_dependencies_produce_deterministic_stages() {
        let graph = graph_with_modules(
            &["fog", "local", "cloud"],
            vec![edge("e1", "fog", "local"), edge("e2", "local", "cloud")],
        );
        let assignments = vec![
            assignment("z-cloud", "cloud", "cloud-01"),
            assignment("m-local", "local", "local-01"),
            assignment("a-fog", "fog", "fog-01"),
        ];

        let plan = derive_deployment_dependency_plan(&graph, &assignments).unwrap();
        assert_eq!(plan.stage_count, 3);
        assert_eq!(plan.dependency_edge_count, 2);
        assert_eq!(plan.stages[0].assignment_ids, vec!["a-fog"]);
        assert_eq!(plan.stages[1].assignment_ids, vec!["m-local"]);
        assert_eq!(plan.stages[2].assignment_ids, vec!["z-cloud"]);
    }

    #[test]
    fn cycle_is_rejected_fail_closed() {
        let graph = graph_with_modules(
            &["a", "b"],
            vec![edge("e1", "a", "b"), edge("e2", "b", "a")],
        );
        let assignments = vec![assignment("a", "a", "fog-01"), assignment("b", "b", "fog-01")];
        let error = derive_deployment_dependency_plan(&graph, &assignments).unwrap_err();
        assert!(error.contains("contains a cycle"));
    }

    #[test]
    fn missing_deployable_dependency_is_rejected() {
        let graph = graph_with_modules(&["external", "managed"], vec![edge("e1", "external", "managed")]);
        let assignments = vec![assignment("managed", "managed", "local-01")];
        let error = derive_deployment_dependency_plan(&graph, &assignments).unwrap_err();
        assert!(error.contains("without a deployable assignment"));
        assert!(error.contains("external"));
    }

    #[test]
    fn ordinary_data_flow_is_not_silently_treated_as_deployment_dependency() {
        let mut graph = graph_with_modules(&["a", "b"], vec![edge("e1", "a", "b")]);
        graph.flows[0].flow_kind = Some("mqtt_data".to_string());
        let assignments = vec![assignment("a", "a", "fog-01"), assignment("b", "b", "local-01")];
        let plan = derive_deployment_dependency_plan(&graph, &assignments).unwrap();
        assert_eq!(plan.stage_count, 1);
        assert_eq!(plan.dependency_edge_count, 0);
        assert_eq!(plan.stages[0].assignment_ids, vec!["a", "b"]);
    }

    #[test]
    fn edge_metadata_can_explicitly_mark_dependency_inside_non_dependency_flow() {
        let mut marked = edge("e1", "a", "b");
        marked.metadata = Some(json!({"deployment_dependency": true}));
        let mut graph = graph_with_modules(&["a", "b"], vec![marked]);
        graph.flows[0].flow_kind = Some("mixed".to_string());
        let assignments = vec![assignment("b", "b", "local-01"), assignment("a", "a", "fog-01")];
        let plan = derive_deployment_dependency_plan(&graph, &assignments).unwrap();
        assert_eq!(plan.stage_count, 2);
        assert_eq!(plan.stages[0].assignment_ids, vec!["a"]);
        assert_eq!(plan.stages[1].assignment_ids, vec!["b"]);
    }
}
