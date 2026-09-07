use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::operational_dgraph::{
    validate_operational_dgraph, OperationalDGraph, OperationalDGraphSlice,
};

pub const ACTIVE_OPERATIONAL_DGRAPH_SCHEMA_VERSION: &str = "0.1.0";
pub const ACTIVE_OPERATIONAL_DGRAPH_MODE: &str = "active_complete_operational_dgraph";
pub const ACTIVE_OPERATIONAL_DGRAPH_CONSISTENCY_MODEL: &str =
    "single_accepted_operational_dgraph_revision_with_node_scoped_compatible_slices";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalTopologyNode {
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalTopologyRelation {
    pub from: String,
    pub to: String,
    pub rel: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraphTopology {
    pub representation: String,
    pub relation_source: String,
    pub nodes: Vec<OperationalTopologyNode>,
    pub relations: Vec<OperationalTopologyRelation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalDGraphSliceCompatibility {
    pub compatible: bool,
    pub expected_node_count: usize,
    pub slice_count: usize,
    pub slice_schema_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveOperationalDGraphAcquisition {
    pub schema_version: String,
    pub acquisition_mode: String,
    pub consistency_model: String,
    pub acquired_at_utc: String,
    pub operational_dgraph: OperationalDGraph,
    pub topology: OperationalDGraphTopology,
    pub slices: Vec<OperationalDGraphSlice>,
    pub slice_compatibility: OperationalDGraphSliceCompatibility,
    pub side_effects_performed: bool,
}

pub fn build_active_operational_dgraph_acquisition(
    graph: OperationalDGraph,
    slices: Vec<OperationalDGraphSlice>,
    acquired_at: DateTime<Utc>,
) -> Result<ActiveOperationalDGraphAcquisition, String> {
    validate_operational_dgraph(&graph)?;
    let topology = build_explicit_topology(&graph)?;
    validate_slice_compatibility(&graph, &slices)?;

    Ok(ActiveOperationalDGraphAcquisition {
        schema_version: ACTIVE_OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
        acquisition_mode: ACTIVE_OPERATIONAL_DGRAPH_MODE.to_string(),
        consistency_model: ACTIVE_OPERATIONAL_DGRAPH_CONSISTENCY_MODEL.to_string(),
        acquired_at_utc: acquired_at.to_rfc3339(),
        operational_dgraph: graph.clone(),
        topology,
        slice_compatibility: OperationalDGraphSliceCompatibility {
            compatible: true,
            expected_node_count: graph.nodes.len(),
            slice_count: slices.len(),
            slice_schema_version: graph.schema_version.clone(),
        },
        slices,
        side_effects_performed: false,
    })
}

pub fn build_explicit_topology(graph: &OperationalDGraph) -> Result<OperationalDGraphTopology, String> {
    validate_operational_dgraph(graph)?;

    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.node_id.trim().to_string())
        .collect::<HashSet<_>>();
    let nodes = graph
        .nodes
        .iter()
        .map(|node| OperationalTopologyNode {
            node_id: node.node_id.clone(),
        })
        .collect::<Vec<_>>();

    let Some(relations_value) = graph
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("relations"))
    else {
        return Ok(OperationalDGraphTopology {
            representation: "explicit_node_relation_topology".to_string(),
            relation_source: "operational_dgraph.nodes".to_string(),
            nodes,
            relations: Vec::new(),
        });
    };

    let relation_values = relations_value.as_array().ok_or_else(|| {
        "operational D-Graph metadata.relations must be an array when present".to_string()
    })?;

    let mut relations = Vec::with_capacity(relation_values.len());
    let mut seen = HashSet::new();
    for (index, relation_value) in relation_values.iter().enumerate() {
        let relation = relation_value.as_object().ok_or_else(|| {
            format!("operational D-Graph metadata.relations[{index}] must be an object")
        })?;
        let from = required_relation_string(relation, "from", index)?;
        let to = required_relation_string(relation, "to", index)?;
        let rel = required_relation_string(relation, "rel", index)?;

        if !node_ids.contains(&from) {
            return Err(format!(
                "operational D-Graph metadata.relations[{index}].from references unknown node '{}'",
                from
            ));
        }
        if !node_ids.contains(&to) {
            return Err(format!(
                "operational D-Graph metadata.relations[{index}].to references unknown node '{}'",
                to
            ));
        }

        let identity = (from.clone(), to.clone(), rel.clone());
        if !seen.insert(identity) {
            return Err(format!(
                "duplicate operational D-Graph topology relation '{} -> {} ({})'",
                from, to, rel
            ));
        }

        let attributes = relation
            .iter()
            .filter(|(key, _)| !matches!(key.as_str(), "from" | "to" | "rel"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();

        relations.push(OperationalTopologyRelation {
            from,
            to,
            rel,
            attributes,
        });
    }

    Ok(OperationalDGraphTopology {
        representation: "explicit_node_relation_topology".to_string(),
        relation_source: "operational_dgraph.metadata.relations".to_string(),
        nodes,
        relations,
    })
}

fn validate_slice_compatibility(
    graph: &OperationalDGraph,
    slices: &[OperationalDGraphSlice],
) -> Result<(), String> {
    if slices.len() != graph.nodes.len() {
        return Err(format!(
            "active Operational D-Graph acquisition requires one compatible slice per graph node: expected {}, received {}",
            graph.nodes.len(),
            slices.len()
        ));
    }

    let mut seen_nodes = HashSet::new();
    for slice in slices {
        if !seen_nodes.insert(slice.node_id.trim().to_string()) {
            return Err(format!(
                "duplicate operational D-Graph slice for node '{}'",
                slice.node_id
            ));
        }
        if slice.schema_version != graph.schema_version
            || slice.dgraph_id != graph.dgraph_id
            || slice.dgraph_revision != graph.revision
            || slice.project_id != graph.project_id
        {
            return Err(format!(
                "operational D-Graph slice for node '{}' does not match the acquired graph identity/revision",
                slice.node_id
            ));
        }
        let node = graph
            .nodes
            .iter()
            .find(|node| node.node_id.trim() == slice.node_id.trim())
            .ok_or_else(|| {
                format!(
                    "operational D-Graph slice references unexpected node '{}'",
                    slice.node_id
                )
            })?;
        if slice.desired_services != node.desired_services {
            return Err(format!(
                "operational D-Graph slice for node '{}' does not preserve desired_services",
                slice.node_id
            ));
        }
    }

    Ok(())
}

fn required_relation_string(
    relation: &serde_json::Map<String, Value>,
    field: &str,
    index: usize,
) -> Result<String, String> {
    relation
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| {
            format!(
                "operational D-Graph metadata.relations[{index}].{field} must be a non-empty string"
            )
        })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::core::operational_dgraph::{
        build_operational_slice, OperationalDesiredService, OperationalNodeDesiredState,
        OPERATIONAL_DGRAPH_SCHEMA_VERSION,
    };

    fn multi_node_graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: "dgraph:o2-multinode-test".to_string(),
            revision: 8,
            project_id: "monitor-f49c8".to_string(),
            nodes: vec![
                OperationalNodeDesiredState {
                    node_id: "fog-01".to_string(),
                    desired_services: vec![OperationalDesiredService {
                        service_id: "mqtt".to_string(),
                        runtime: "container".to_string(),
                        desired_state: "running".to_string(),
                    }],
                },
                OperationalNodeDesiredState {
                    node_id: "cloud-01".to_string(),
                    desired_services: Vec::new(),
                },
            ],
            metadata: Some(json!({
                "relations": [{
                    "from": "fog-01",
                    "to": "cloud-01",
                    "rel": "connected_to",
                    "latency_ms": 12.5,
                    "transport": "lan"
                }]
            })),
        }
    }

    fn slices(graph: &OperationalDGraph) -> Vec<OperationalDGraphSlice> {
        graph
            .nodes
            .iter()
            .map(|node| build_operational_slice(graph, &node.node_id, None).unwrap())
            .collect()
    }

    #[test]
    fn materializes_explicit_multi_node_topology_without_losing_relation_attributes() {
        let graph = multi_node_graph();
        let topology = build_explicit_topology(&graph).unwrap();
        assert_eq!(topology.nodes.len(), 2);
        assert_eq!(topology.relations.len(), 1);
        assert_eq!(topology.relations[0].from, "fog-01");
        assert_eq!(topology.relations[0].to, "cloud-01");
        assert_eq!(topology.relations[0].rel, "connected_to");
        assert_eq!(topology.relations[0].attributes["latency_ms"], 12.5);
        assert_eq!(topology.relations[0].attributes["transport"], "lan");
        assert_eq!(topology.relation_source, "operational_dgraph.metadata.relations");
    }

    #[test]
    fn complete_acquisition_preserves_full_graph_and_compatible_slices() {
        let graph = multi_node_graph();
        let expected_slices = slices(&graph);
        let result = build_active_operational_dgraph_acquisition(
            graph.clone(),
            expected_slices.clone(),
            Utc.with_ymd_and_hms(2026, 9, 7, 15, 0, 0).unwrap(),
        )
        .unwrap();

        assert_eq!(result.operational_dgraph, graph);
        assert_eq!(result.slices, expected_slices);
        assert_eq!(result.slice_compatibility.expected_node_count, 2);
        assert_eq!(result.slice_compatibility.slice_count, 2);
        assert!(result.slice_compatibility.compatible);
        assert!(!result.side_effects_performed);

        let encoded = serde_json::to_value(&result).unwrap();
        let decoded: ActiveOperationalDGraphAcquisition = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn legacy_graph_without_relations_remains_acquirable() {
        let mut graph = multi_node_graph();
        graph.metadata = Some(json!({"source": "legacy-live-shape"}));
        let topology = build_explicit_topology(&graph).unwrap();
        assert_eq!(topology.nodes.len(), 2);
        assert!(topology.relations.is_empty());
        assert_eq!(topology.relation_source, "operational_dgraph.nodes");

        let result = build_active_operational_dgraph_acquisition(
            graph.clone(),
            slices(&graph),
            Utc.with_ymd_and_hms(2026, 9, 7, 15, 0, 0).unwrap(),
        )
        .unwrap();
        assert_eq!(result.operational_dgraph.metadata, graph.metadata);
    }

    #[test]
    fn topology_relation_to_unknown_node_is_rejected() {
        let mut graph = multi_node_graph();
        graph.metadata = Some(json!({
            "relations": [{"from":"fog-01","to":"unknown-01","rel":"connected_to"}]
        }));
        let error = build_explicit_topology(&graph).unwrap_err();
        assert!(error.contains("unknown node 'unknown-01'"));
    }

    #[test]
    fn missing_or_mixed_revision_slices_are_rejected() {
        let graph = multi_node_graph();
        let mut graph_slices = slices(&graph);
        graph_slices.pop();
        assert!(build_active_operational_dgraph_acquisition(
            graph.clone(),
            graph_slices,
            Utc.with_ymd_and_hms(2026, 9, 7, 15, 0, 0).unwrap(),
        )
        .is_err());

        let mut graph_slices = slices(&graph);
        graph_slices[0].dgraph_revision -= 1;
        assert!(build_active_operational_dgraph_acquisition(
            graph,
            graph_slices,
            Utc.with_ymd_and_hms(2026, 9, 7, 15, 0, 0).unwrap(),
        )
        .is_err());
    }
}
