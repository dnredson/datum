use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::core::{
    controlled_dgraph_deployment::ControlledObservedDGraphState,
    local_dgraph_snapshot::LocalDGraphSnapshot,
};

pub const RESOURCE_OBSERVATION_FRESHNESS_SECONDS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeResourceObservation {
    pub node_id: String,
    pub snapshot_id: String,
    pub observed_at_utc: String,
    pub reported_at_utc: String,
    pub age_seconds: i64,
    pub freshness_threshold_seconds: i64,
    pub freshness: String,
    pub cpu: CpuObservation,
    pub memory: MemoryObservation,
    pub process_count: u64,
    pub container_count: u64,
    pub supported_capabilities: Vec<String>,
    pub runtime_count: usize,
    pub module_count: usize,
    pub health_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CpuObservation {
    pub logical_cores: u64,
    pub usage_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryObservation {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResourceJoinSummary {
    pub observation_count: usize,
    pub fresh_count: usize,
    pub stale_count: usize,
    pub missing_node_ids: Vec<String>,
    pub unmatched_node_ids: Vec<String>,
    pub freshness_threshold_seconds: i64,
    pub automatic_placement_performed: bool,
    pub deployment_side_effects_performed: bool,
    pub governed_execution_performed: bool,
}

pub fn parse_resource_observation(
    snapshot: &LocalDGraphSnapshot,
    now: DateTime<Utc>,
) -> Result<Option<NodeResourceObservation>, String> {
    let metadata = match snapshot.metadata.as_ref().and_then(Value::as_object) {
        Some(metadata) => metadata,
        None => return Ok(None),
    };
    let resource = match metadata.get("resource_observation") {
        Some(Value::Object(resource)) => resource,
        Some(_) => {
            return Err(format!(
                "snapshot '{}' metadata.resource_observation must be an object",
                snapshot.snapshot_id
            ))
        }
        None => return Ok(None),
    };

    let observed_at_utc = required_string(resource, "observed_at_utc")?;
    let observed_at = DateTime::parse_from_rfc3339(&observed_at_utc)
        .map_err(|error| format!("resource observed_at_utc must be RFC3339: {error}"))?
        .with_timezone(&Utc);
    let age_seconds = (now - observed_at).num_seconds().max(0);
    let freshness = if age_seconds <= RESOURCE_OBSERVATION_FRESHNESS_SECONDS {
        "fresh"
    } else {
        "stale"
    };

    let cpu = required_object(resource, "cpu")?;
    let logical_cores = required_u64(cpu, "logical_cores")?;
    if logical_cores == 0 {
        return Err("resource cpu.logical_cores must be greater than zero".to_string());
    }
    let usage_percent = required_f64(cpu, "usage_percent")?;
    if !(0.0..=100.0).contains(&usage_percent) {
        return Err("resource cpu.usage_percent must be between 0 and 100".to_string());
    }

    let memory = required_object(resource, "memory")?;
    let total_bytes = required_u64(memory, "total_bytes")?;
    let available_bytes = required_u64(memory, "available_bytes")?;
    if total_bytes == 0 {
        return Err("resource memory.total_bytes must be greater than zero".to_string());
    }
    if available_bytes > total_bytes {
        return Err("resource memory.available_bytes must not exceed total_bytes".to_string());
    }

    let process_count = required_u64(resource, "process_count")?;
    let container_count = required_u64(resource, "container_count")?;
    let supported_capabilities = snapshot
        .capabilities
        .iter()
        .filter(|capability| capability.supported)
        .map(|capability| capability.capability_id.trim().to_string())
        .filter(|capability_id| !capability_id.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    Ok(Some(NodeResourceObservation {
        node_id: snapshot.node_id.clone(),
        snapshot_id: snapshot.snapshot_id.clone(),
        observed_at_utc,
        reported_at_utc: snapshot.reported_at_utc.clone(),
        age_seconds,
        freshness_threshold_seconds: RESOURCE_OBSERVATION_FRESHNESS_SECONDS,
        freshness: freshness.to_string(),
        cpu: CpuObservation {
            logical_cores,
            usage_percent,
        },
        memory: MemoryObservation {
            total_bytes,
            available_bytes,
        },
        process_count,
        container_count,
        supported_capabilities,
        runtime_count: snapshot.runtimes.len(),
        module_count: snapshot.modules.len(),
        health_count: snapshot.health.len(),
    }))
}

pub fn materialize_resource_observations(
    observed: &mut ControlledObservedDGraphState,
    snapshots: &[LocalDGraphSnapshot],
    now: DateTime<Utc>,
) -> Result<ResourceJoinSummary, String> {
    let graph_node_ids = observed
        .graph
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<BTreeSet<_>>();
    let mut joined_node_ids = BTreeSet::new();
    let mut unmatched_node_ids = BTreeSet::new();
    let mut observations = Vec::new();

    for snapshot in snapshots {
        if snapshot.project_id != observed.project_id {
            continue;
        }
        let Some(observation) = parse_resource_observation(snapshot, now)? else {
            continue;
        };
        if !graph_node_ids.contains(&observation.node_id) {
            unmatched_node_ids.insert(observation.node_id.clone());
            continue;
        }
        if joined_node_ids.contains(&observation.node_id) {
            return Err(format!(
                "multiple resource observations resolved for node '{}'",
                observation.node_id
            ));
        }

        let node = observed
            .graph
            .nodes
            .iter_mut()
            .find(|node| node.node_id == observation.node_id)
            .expect("validated node id exists");
        node.metadata = Some(merge_metadata(
            node.metadata.take(),
            json!({"resource_observation": observation}),
        ));
        joined_node_ids.insert(observation.node_id.clone());
        observations.push(observation);
    }

    observations.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    let fresh_count = observations
        .iter()
        .filter(|observation| observation.freshness == "fresh")
        .count();
    let stale_count = observations.len() - fresh_count;
    let missing_node_ids = graph_node_ids
        .difference(&joined_node_ids)
        .cloned()
        .collect::<Vec<_>>();
    let summary = ResourceJoinSummary {
        observation_count: observations.len(),
        fresh_count,
        stale_count,
        missing_node_ids,
        unmatched_node_ids: unmatched_node_ids.into_iter().collect(),
        freshness_threshold_seconds: RESOURCE_OBSERVATION_FRESHNESS_SECONDS,
        automatic_placement_performed: false,
        deployment_side_effects_performed: false,
        governed_execution_performed: false,
    };

    let observations_value = serde_json::to_value(&observations)
        .map_err(|error| format!("serialize resource observations: {error}"))?;
    let summary_value = serde_json::to_value(&summary)
        .map_err(|error| format!("serialize resource join summary: {error}"))?;
    observed.graph.metadata = Some(merge_metadata(
        observed.graph.metadata.take(),
        json!({
            "resource_observations": observations_value.clone(),
            "resource_join": summary_value.clone(),
        }),
    ));
    observed.metadata = Some(merge_metadata(
        observed.metadata.take(),
        json!({
            "resource_observations": observations_value,
            "resource_join": summary_value,
        }),
    ));

    Ok(summary)
}

fn required_object<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a Map<String, Value>, String> {
    object
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("resource field '{key}' must be an object"))
}

fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| format!("resource field '{key}' must be a non-empty string"))
}

fn required_u64(object: &Map<String, Value>, key: &str) -> Result<u64, String> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("resource field '{key}' must be an unsigned integer"))
}

fn required_f64(object: &Map<String, Value>, key: &str) -> Result<f64, String> {
    object
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| format!("resource field '{key}' must be numeric"))
}

fn merge_metadata(existing: Option<Value>, extra: Value) -> Value {
    let mut object = match existing {
        Some(Value::Object(object)) => object,
        Some(value) => {
            let mut object = Map::new();
            object.insert("source_metadata".to_string(), value);
            object
        }
        None => Map::new(),
    };
    if let Value::Object(extra) = extra {
        object.extend(extra);
    }
    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use crate::core::{
        controlled_dgraph_deployment::ControlledObservedDGraphState,
        dgraph::{DGraph, DGraphNode, DGraphSummary},
        local_dgraph_snapshot::{LocalCapabilitySnapshot, LocalDGraphSnapshot},
    };

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 31, 17, 0, 0).unwrap()
    }

    fn snapshot(node_id: &str, observed_at: &str) -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            project_id: "project-i42".to_string(),
            node_id: node_id.to_string(),
            snapshot_id: format!("snapshot-{node_id}"),
            reported_at_utc: "2026-08-31T17:00:00Z".to_string(),
            capabilities: vec![LocalCapabilitySnapshot {
                capability_id: "wasm".to_string(),
                capability_kind: "runtime".to_string(),
                supported: true,
                source: "test".to_string(),
                metadata: None,
            }],
            metadata: Some(json!({
                "resource_observation": {
                    "observed_at_utc": observed_at,
                    "cpu": {"logical_cores": 8, "usage_percent": 25.0},
                    "memory": {"total_bytes": 16000, "available_bytes": 8000},
                    "process_count": 100,
                    "container_count": 2
                }
            })),
            ..Default::default()
        }
    }

    fn observed() -> ControlledObservedDGraphState {
        ControlledObservedDGraphState {
            source_dgraph_id: "dgraph-i42".to_string(),
            manifest_id: "manifest-i42".to_string(),
            project_id: "project-i42".to_string(),
            materialized_at_utc: "2026-08-31T17:00:00Z".to_string(),
            report_count: 0,
            graph: DGraph {
                project_id: "project-i42".to_string(),
                nodes: vec![DGraphNode {
                    node_id: "fog-01".to_string(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            summary: DGraphSummary::default(),
            validation_errors: Vec::new(),
            side_effects_performed: false,
            metadata: None,
        }
    }

    #[test]
    fn fresh_resource_observation_is_parsed() {
        let value = parse_resource_observation(&snapshot("fog-01", "2026-08-31T16:59:00Z"), now())
            .unwrap()
            .unwrap();
        assert_eq!(value.freshness, "fresh");
        assert_eq!(value.age_seconds, 60);
        assert_eq!(value.supported_capabilities, vec!["wasm"]);
    }

    #[test]
    fn stale_resource_observation_is_distinguishable() {
        let value = parse_resource_observation(&snapshot("fog-01", "2026-08-31T16:50:00Z"), now())
            .unwrap()
            .unwrap();
        assert_eq!(value.freshness, "stale");
        assert_eq!(value.age_seconds, 600);
    }

    #[test]
    fn invalid_cpu_percentage_is_rejected() {
        let mut value = snapshot("fog-01", "2026-08-31T16:59:00Z");
        value.metadata.as_mut().unwrap()["resource_observation"]["cpu"]["usage_percent"] = json!(101.0);
        assert!(parse_resource_observation(&value, now()).is_err());
    }

    #[test]
    fn invalid_memory_range_is_rejected() {
        let mut value = snapshot("fog-01", "2026-08-31T16:59:00Z");
        value.metadata.as_mut().unwrap()["resource_observation"]["memory"]["available_bytes"] = json!(32000);
        assert!(parse_resource_observation(&value, now()).is_err());
    }

    #[test]
    fn resource_join_materializes_node_metadata_and_summary() {
        let mut graph = observed();
        let summary = materialize_resource_observations(
            &mut graph,
            &[snapshot("fog-01", "2026-08-31T16:59:00Z")],
            now(),
        )
        .unwrap();
        assert_eq!(summary.observation_count, 1);
        assert_eq!(summary.fresh_count, 1);
        assert!(summary.missing_node_ids.is_empty());
        assert_eq!(
            graph.graph.nodes[0].metadata.as_ref().unwrap()["resource_observation"]["freshness"],
            "fresh"
        );
    }

    #[test]
    fn unmatched_snapshot_is_reported_without_mutating_graph_nodes() {
        let mut graph = observed();
        let summary = materialize_resource_observations(
            &mut graph,
            &[snapshot("probe-01", "2026-08-31T16:59:00Z")],
            now(),
        )
        .unwrap();
        assert_eq!(summary.observation_count, 0);
        assert_eq!(summary.unmatched_node_ids, vec!["probe-01"]);
        assert_eq!(summary.missing_node_ids, vec!["fog-01"]);
        assert!(graph.graph.nodes[0].metadata.is_none());
    }
}
