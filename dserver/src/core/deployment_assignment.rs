use serde::{Deserialize, Serialize};

use crate::core::deployment_manifest::{
    validate_deployment_manifest, DeploymentAssignment, DeploymentExecutionBoundary,
    DeploymentManifest, DeploymentStrategy, GraphDistribution,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeDeploymentScope {
    Full,
    NodeSlice,
    Empty,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeScopedDeployment {
    pub manifest_version: String,
    pub manifest_id: String,
    pub project_id: String,
    pub node_id: String,
    pub strategy: DeploymentStrategy,
    pub graph_distribution: GraphDistribution,
    pub delivery_scope: NodeDeploymentScope,
    #[serde(default)]
    pub source_dgraph_id: Option<String>,
    pub source_assignment_count: usize,
    pub selected_assignment_count: usize,
    pub assignments: Vec<DeploymentAssignment>,
    pub execution_boundary: DeploymentExecutionBoundary,
}

pub fn derive_node_scoped_deployment(
    manifest: &DeploymentManifest,
    node_id: &str,
) -> Result<NodeScopedDeployment, String> {
    validate_deployment_manifest(manifest)?;

    let node_id = node_id.trim();
    if node_id.is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    let (delivery_scope, assignments) = match manifest.strategy {
        DeploymentStrategy::Centralized => {
            let central_node = manifest.assignments[0].target_node.trim();
            if central_node == node_id {
                (NodeDeploymentScope::Full, manifest.assignments.clone())
            } else {
                (NodeDeploymentScope::Empty, Vec::new())
            }
        }
        DeploymentStrategy::Distributed => {
            let selected: Vec<DeploymentAssignment> = manifest
                .assignments
                .iter()
                .filter(|assignment| assignment.target_node.trim() == node_id)
                .cloned()
                .collect();

            if selected.is_empty() {
                (NodeDeploymentScope::Empty, selected)
            } else {
                (NodeDeploymentScope::NodeSlice, selected)
            }
        }
    };

    let node_view = NodeScopedDeployment {
        manifest_version: manifest.manifest_version.clone(),
        manifest_id: manifest.manifest_id.clone(),
        project_id: manifest.project_id.clone(),
        node_id: node_id.to_string(),
        strategy: manifest.strategy.clone(),
        graph_distribution: manifest.graph_distribution.clone(),
        delivery_scope,
        source_dgraph_id: manifest.source_dgraph_id.clone(),
        source_assignment_count: manifest.assignments.len(),
        selected_assignment_count: assignments.len(),
        assignments,
        execution_boundary: manifest.execution_boundary.clone(),
    };

    validate_node_scoped_deployment(&node_view)?;
    Ok(node_view)
}

pub fn validate_node_scoped_deployment(view: &NodeScopedDeployment) -> Result<(), String> {
    if view.manifest_version.trim().is_empty() {
        return Err("manifest_version must not be empty".to_string());
    }
    if view.manifest_id.trim().is_empty() {
        return Err("manifest_id must not be empty".to_string());
    }
    if view.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }
    if view.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }
    if view.source_assignment_count < view.selected_assignment_count {
        return Err("selected_assignment_count cannot exceed source_assignment_count".to_string());
    }
    if view.selected_assignment_count != view.assignments.len() {
        return Err("selected_assignment_count must match assignments length".to_string());
    }

    if !view.execution_boundary.explicit_invocation_required {
        return Err("explicit_invocation_required must remain true".to_string());
    }
    if view.execution_boundary.automatic_execution_allowed {
        return Err("automatic_execution_allowed must remain false".to_string());
    }
    if view.execution_boundary.governed_execution_authorized {
        return Err("governed_execution_authorized must remain false".to_string());
    }

    match (&view.strategy, &view.graph_distribution, &view.delivery_scope) {
        (
            DeploymentStrategy::Centralized,
            GraphDistribution::Full,
            NodeDeploymentScope::Full,
        ) => {
            if view.assignments.is_empty() {
                return Err("full centralized delivery must contain assignments".to_string());
            }
            if view
                .assignments
                .iter()
                .any(|assignment| assignment.target_node.trim() != view.node_id)
            {
                return Err(
                    "full centralized delivery may contain only assignments for the central node"
                        .to_string(),
                );
            }
        }
        (
            DeploymentStrategy::Distributed,
            GraphDistribution::NodeSlice,
            NodeDeploymentScope::NodeSlice,
        ) => {
            if view.assignments.is_empty() {
                return Err("distributed node_slice delivery must contain assignments".to_string());
            }
            if view
                .assignments
                .iter()
                .any(|assignment| assignment.target_node.trim() != view.node_id)
            {
                return Err(
                    "distributed node_slice delivery may contain only assignments for its node"
                        .to_string(),
                );
            }
        }
        (
            DeploymentStrategy::Centralized,
            GraphDistribution::Full,
            NodeDeploymentScope::Empty,
        )
        | (
            DeploymentStrategy::Distributed,
            GraphDistribution::NodeSlice,
            NodeDeploymentScope::Empty,
        ) => {
            if !view.assignments.is_empty() || view.selected_assignment_count != 0 {
                return Err("empty delivery scope must not contain assignments".to_string());
            }
        }
        _ => {
            return Err(
                "node-scoped deployment has an invalid strategy/distribution/delivery_scope combination"
                    .to_string(),
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::core::deployment_manifest::{
        build_deployment_artifact_identity, DeploymentArtifactIdentity, DeploymentRuntimeLimits,
        DEPLOYMENT_MANIFEST_VERSION,
    };

    use super::*;

    const TEST_SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn artifact(id: &str) -> DeploymentArtifactIdentity {
        build_deployment_artifact_identity(
            format!("artifact-{id}"),
            format!("file:///artifacts/{id}.wasm"),
            TEST_SHA,
        )
        .expect("test artifact identity should be valid")
    }

    fn assignment(id: &str, node: &str) -> DeploymentAssignment {
        DeploymentAssignment {
            assignment_id: id.to_string(),
            module_id: format!("module-{id}"),
            function_name: "run".to_string(),
            artifact: artifact(id),
            target_node: node.to_string(),
            limits: DeploymentRuntimeLimits {
                fuel: Some(1_000_000),
                timeout_ms: Some(1_000),
            },
            metadata: None,
        }
    }

    fn distributed_manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest:distributed:c64".to_string(),
            project_id: "smart-farm".to_string(),
            strategy: DeploymentStrategy::Distributed,
            graph_distribution: GraphDistribution::NodeSlice,
            source_dgraph_id: Some("dgraph:snapshot:c64".to_string()),
            assignments: vec![
                assignment("sensor-monitor", "fog-01"),
                assignment("mqtt-adapter", "fog-02"),
                assignment("weather-filter", "fog-01"),
            ],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    fn centralized_manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest:centralized:c64".to_string(),
            project_id: "smart-farm".to_string(),
            strategy: DeploymentStrategy::Centralized,
            graph_distribution: GraphDistribution::Full,
            source_dgraph_id: Some("dgraph:snapshot:c64".to_string()),
            assignments: vec![
                assignment("sensor-monitor", "fog-01"),
                assignment("mqtt-adapter", "fog-01"),
            ],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    #[test]
    fn distributed_fog_01_receives_only_its_assignments() {
        let view = derive_node_scoped_deployment(&distributed_manifest(), "fog-01")
            .expect("node view should derive");
        assert_eq!(view.delivery_scope, NodeDeploymentScope::NodeSlice);
        assert_eq!(view.source_assignment_count, 3);
        assert_eq!(view.selected_assignment_count, 2);
        assert!(view
            .assignments
            .iter()
            .all(|assignment| assignment.target_node == "fog-01"));
    }

    #[test]
    fn distributed_fog_02_receives_only_its_assignment() {
        let view = derive_node_scoped_deployment(&distributed_manifest(), "fog-02")
            .expect("node view should derive");
        assert_eq!(view.delivery_scope, NodeDeploymentScope::NodeSlice);
        assert_eq!(view.selected_assignment_count, 1);
        assert_eq!(view.assignments[0].assignment_id, "mqtt-adapter");
    }

    #[test]
    fn distributed_non_target_receives_empty_view() {
        let view = derive_node_scoped_deployment(&distributed_manifest(), "cloud-01")
            .expect("empty node view should derive");
        assert_eq!(view.delivery_scope, NodeDeploymentScope::Empty);
        assert_eq!(view.selected_assignment_count, 0);
        assert!(view.assignments.is_empty());
    }

    #[test]
    fn centralized_target_receives_full_assignment_set() {
        let view = derive_node_scoped_deployment(&centralized_manifest(), "fog-01")
            .expect("central view should derive");
        assert_eq!(view.delivery_scope, NodeDeploymentScope::Full);
        assert_eq!(view.source_assignment_count, 2);
        assert_eq!(view.selected_assignment_count, 2);
        assert_eq!(view.assignments.len(), 2);
    }

    #[test]
    fn centralized_non_target_receives_empty_view() {
        let view = derive_node_scoped_deployment(&centralized_manifest(), "fog-02")
            .expect("non-central view should derive");
        assert_eq!(view.delivery_scope, NodeDeploymentScope::Empty);
        assert!(view.assignments.is_empty());
    }

    #[test]
    fn invalid_source_manifest_is_rejected_before_scoping() {
        let mut manifest = distributed_manifest();
        manifest.graph_distribution = GraphDistribution::Full;
        let error = derive_node_scoped_deployment(&manifest, "fog-01")
            .expect_err("invalid source manifest must fail");
        assert!(error.contains("distributed strategy requires"));
    }

    #[test]
    fn empty_node_id_is_rejected() {
        let error = derive_node_scoped_deployment(&distributed_manifest(), "   ")
            .expect_err("empty node id must fail");
        assert!(error.contains("node_id must not be empty"));
    }

    #[test]
    fn artifact_identity_is_preserved_exactly() {
        let manifest = distributed_manifest();
        let expected = manifest.assignments[1].artifact.clone();
        let view = derive_node_scoped_deployment(&manifest, "fog-02")
            .expect("node view should derive");
        assert_eq!(view.assignments[0].artifact, expected);
    }

    #[test]
    fn execution_boundary_remains_fail_closed() {
        let manifest = distributed_manifest();
        let view = derive_node_scoped_deployment(&manifest, "fog-01")
            .expect("node view should derive");
        assert!(view.execution_boundary.explicit_invocation_required);
        assert!(!view.execution_boundary.automatic_execution_allowed);
        assert!(!view.execution_boundary.governed_execution_authorized);
    }

    #[test]
    fn selected_count_must_match_assignments_length() {
        let mut view = derive_node_scoped_deployment(&distributed_manifest(), "fog-01")
            .expect("node view should derive");
        view.selected_assignment_count = 1;
        let error = validate_node_scoped_deployment(&view)
            .expect_err("inconsistent selected count must fail");
        assert!(error.contains("selected_assignment_count must match"));
    }

    #[test]
    fn node_slice_cannot_leak_another_nodes_assignment() {
        let mut view = derive_node_scoped_deployment(&distributed_manifest(), "fog-01")
            .expect("node view should derive");
        view.assignments.push(assignment("leak", "fog-02"));
        view.selected_assignment_count = view.assignments.len();
        let error = validate_node_scoped_deployment(&view)
            .expect_err("cross-node assignment must fail");
        assert!(error.contains("only assignments for its node"));
    }

    #[test]
    fn serde_exposes_reserved_delivery_scope_strings() {
        let view = derive_node_scoped_deployment(&distributed_manifest(), "fog-01")
            .expect("node view should derive");
        let value = serde_json::to_value(&view).expect("view should serialize");
        assert_eq!(value["delivery_scope"].as_str(), Some("node_slice"));

        let round_trip: NodeScopedDeployment =
            serde_json::from_value(value).expect("view should deserialize");
        assert_eq!(round_trip, view);
    }
}
