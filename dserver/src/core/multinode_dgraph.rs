use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::operational_dgraph::{validate_operational_dgraph, OperationalDGraph};

pub const MULTINODE_METADATA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultinodeDGraphMetadata {
    pub schema_version: String,
    pub node_descriptors: Vec<MultinodeNodeDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultinodeNodeDescriptor {
    pub node_id: String,
    pub continuum_role: String,
    pub resources: Vec<MultinodeResourceDescriptor>,
    #[serde(default)]
    pub attributes: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MultinodeResourceDescriptor {
    pub resource_id: String,
    pub resource_kind: String,
    #[serde(default)]
    pub attributes: serde_json::Map<String, Value>,
}

pub fn validate_multinode_metadata(graph: &OperationalDGraph) -> Result<(), String> {
    validate_operational_dgraph(graph)?;
    let Some(metadata) = graph.metadata.as_ref() else {
        return Ok(());
    };
    let Some(descriptors) = metadata.get("node_descriptors") else {
        return Ok(());
    };
    let descriptors: Vec<MultinodeNodeDescriptor> =
        serde_json::from_value(descriptors.clone())
            .map_err(|error| format!("invalid operational D-Graph node_descriptors: {error}"))?;
    if descriptors.len() != graph.nodes.len() {
        return Err(format!(
            "multinode node_descriptors must contain one descriptor per graph node: expected {}, received {}",
            graph.nodes.len(),
            descriptors.len()
        ));
    }

    let expected = graph.nodes.iter().map(|node| node.node_id.as_str()).collect::<HashSet<_>>();
    let mut seen_nodes = HashSet::new();
    let mut seen_resources = HashSet::new();
    for descriptor in descriptors {
        let node_id = descriptor.node_id.trim();
        if node_id.is_empty() || !expected.contains(node_id) || !seen_nodes.insert(node_id) {
            return Err(format!("invalid or duplicate multinode descriptor node_id '{}'", descriptor.node_id));
        }
        if descriptor.continuum_role.trim().is_empty() {
            return Err(format!("multinode descriptor '{}' has empty continuum_role", node_id));
        }
        for resource in descriptor.resources {
            if resource.resource_id.trim().is_empty() || resource.resource_kind.trim().is_empty() {
                return Err(format!("multinode descriptor '{}' contains an incomplete resource", node_id));
            }
            if !seen_resources.insert(resource.resource_id) {
                return Err(format!("duplicate multinode resource '{}'", resource.resource_id));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::core::operational_dgraph::{
        OperationalDGraph, OperationalNodeDesiredState,
    };

    fn graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: "0.1.0".into(),
            dgraph_id: "dgraph:multinode".into(),
            revision: 5,
            project_id: "project-a".into(),
            nodes: vec![
                OperationalNodeDesiredState { node_id: "fog-01".into(), desired_services: vec![] },
                OperationalNodeDesiredState { node_id: "fog-02".into(), desired_services: vec![] },
                OperationalNodeDesiredState { node_id: "cloud-01".into(), desired_services: vec![] },
            ],
            metadata: Some(json!({
                "node_descriptors": [
                    {"node_id":"fog-01","continuum_role":"fog","resources":[{"resource_id":"host:fog-01","resource_kind":"host","attributes":{"ip":"192.168.10.103"}}]},
                    {"node_id":"fog-02","continuum_role":"fog","resources":[{"resource_id":"host:fog-02","resource_kind":"host","attributes":{"ip":"192.168.10.107"}}]},
                    {"node_id":"cloud-01","continuum_role":"cloud","resources":[{"resource_id":"host:cloud-01","resource_kind":"host","attributes":{"ip":"192.168.10.108"}}]}
                ],
                "relations":[
                    {"from":"fog-01","to":"cloud-01","rel":"connected_to"},
                    {"from":"fog-02","to":"cloud-01","rel":"connected_to"}
                ]
            })),
        }
    }

    #[test]
    fn accepts_complete_three_node_descriptors() {
        validate_multinode_metadata(&graph()).unwrap();
    }

    #[test]
    fn rejects_missing_node_descriptor() {
        let mut graph = graph();
        graph.metadata.as_mut().unwrap()["node_descriptors"].as_array_mut().unwrap().pop();
        assert!(validate_multinode_metadata(&graph).is_err());
    }

    #[test]
    fn rejects_duplicate_resource_identity() {
        let mut graph = graph();
        graph.metadata.as_mut().unwrap()["node_descriptors"][1]["resources"][0]["resource_id"] =
            json!("host:fog-01");
        assert!(validate_multinode_metadata(&graph).is_err());
    }
}
