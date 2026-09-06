use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::operational_dgraph::{
    validate_operational_dgraph, OperationalDGraph, OperationalNodeDesiredState,
};

pub const OPERATIONAL_ANALYSIS_CONTEXT_SCHEMA_VERSION: &str = "0.1.0";
pub const ANALYSIS_MODE_PASSIVE: &str = "passive";
pub const ANALYSIS_MODE_ACTIVE: &str = "active";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalAnalysisContext {
    pub schema_version: String,
    pub acquisition_mode: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub generated_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_node_id: Option<String>,
    pub node_count: u64,
    pub side_effects_performed: bool,
    pub nodes: Vec<OperationalAnalysisNodeContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalAnalysisNodeContext {
    pub node_id: String,
    pub desired_services: Vec<crate::core::operational_dgraph::OperationalDesiredService>,
    pub telemetry_source: String,
    pub telemetry_available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_observed_at_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_telemetry: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry_error: Option<String>,
}

pub fn build_passive_analysis_context(
    graph: &OperationalDGraph,
    node_id: &str,
    telemetry: Result<Value, String>,
    now: DateTime<Utc>,
) -> Result<OperationalAnalysisContext, String> {
    validate_operational_dgraph(graph)?;
    let node_id = node_id.trim();
    let node = graph
        .nodes
        .iter()
        .find(|node| node.node_id.trim() == node_id)
        .ok_or_else(|| {
            format!(
                "passive analysis trigger node '{}' is not present in active operational D-Graph revision {}",
                node_id, graph.revision
            )
        })?;

    Ok(OperationalAnalysisContext {
        schema_version: OPERATIONAL_ANALYSIS_CONTEXT_SCHEMA_VERSION.to_string(),
        acquisition_mode: ANALYSIS_MODE_PASSIVE.to_string(),
        project_id: graph.project_id.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        generated_at_utc: now.to_rfc3339(),
        trigger_node_id: Some(node.node_id.clone()),
        node_count: 1,
        side_effects_performed: false,
        nodes: vec![build_node_context(node, telemetry)],
    })
}

pub fn build_active_analysis_context(
    graph: &OperationalDGraph,
    mut telemetry_by_node: BTreeMap<String, Result<Value, String>>,
    now: DateTime<Utc>,
) -> Result<OperationalAnalysisContext, String> {
    validate_operational_dgraph(graph)?;

    let nodes = graph
        .nodes
        .iter()
        .map(|node| {
            let telemetry = telemetry_by_node.remove(node.node_id.trim()).unwrap_or_else(|| {
                Err(format!(
                    "active analysis telemetry acquisition result missing for node '{}'",
                    node.node_id
                ))
            });
            build_node_context(node, telemetry)
        })
        .collect::<Vec<_>>();

    Ok(OperationalAnalysisContext {
        schema_version: OPERATIONAL_ANALYSIS_CONTEXT_SCHEMA_VERSION.to_string(),
        acquisition_mode: ANALYSIS_MODE_ACTIVE.to_string(),
        project_id: graph.project_id.clone(),
        dgraph_id: graph.dgraph_id.clone(),
        dgraph_revision: graph.revision,
        generated_at_utc: now.to_rfc3339(),
        trigger_node_id: None,
        node_count: nodes.len() as u64,
        side_effects_performed: false,
        nodes,
    })
}

fn build_node_context(
    node: &OperationalNodeDesiredState,
    telemetry: Result<Value, String>,
) -> OperationalAnalysisNodeContext {
    match telemetry {
        Ok(document) => OperationalAnalysisNodeContext {
            node_id: node.node_id.clone(),
            desired_services: node.desired_services.clone(),
            telemetry_source: "firebase".to_string(),
            telemetry_available: true,
            telemetry_observed_at_utc: firestore_observed_at(&document),
            latest_telemetry: Some(document),
            telemetry_error: None,
        },
        Err(error) => OperationalAnalysisNodeContext {
            node_id: node.node_id.clone(),
            desired_services: node.desired_services.clone(),
            telemetry_source: "firebase".to_string(),
            telemetry_available: false,
            telemetry_observed_at_utc: None,
            latest_telemetry: None,
            telemetry_error: Some(error),
        },
    }
}

fn firestore_observed_at(document: &Value) -> Option<String> {
    document
        .get("fields")
        .and_then(|fields| fields.get("observed_at_utc"))
        .and_then(|value| value.get("stringValue"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::core::operational_dgraph::{OperationalDesiredService, OperationalNodeDesiredState};

    fn graph() -> OperationalDGraph {
        OperationalDGraph {
            schema_version: "0.1.0".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            revision: 4,
            project_id: "project-a".to_string(),
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
            metadata: None,
        }
    }

    #[test]
    fn passive_context_is_scoped_to_trigger_node() {
        let context = build_passive_analysis_context(
            &graph(),
            "fog-01",
            Ok(json!({
                "fields": {
                    "observed_at_utc": {"stringValue":"2026-09-06T12:00:00Z"}
                }
            })),
            Utc.with_ymd_and_hms(2026, 9, 6, 12, 1, 0).unwrap(),
        )
        .unwrap();

        assert_eq!(context.acquisition_mode, ANALYSIS_MODE_PASSIVE);
        assert_eq!(context.trigger_node_id.as_deref(), Some("fog-01"));
        assert_eq!(context.node_count, 1);
        assert!(context.nodes[0].telemetry_available);
        assert_eq!(
            context.nodes[0].telemetry_observed_at_utc.as_deref(),
            Some("2026-09-06T12:00:00Z")
        );
        assert!(!context.side_effects_performed);
    }

    #[test]
    fn active_context_keeps_local_desired_state_when_one_telemetry_read_fails() {
        let mut telemetry = BTreeMap::new();
        telemetry.insert("fog-01".to_string(), Ok(json!({"fields": {}})));
        telemetry.insert(
            "cloud-01".to_string(),
            Err("firebase unavailable".to_string()),
        );

        let context = build_active_analysis_context(
            &graph(),
            telemetry,
            Utc.with_ymd_and_hms(2026, 9, 6, 12, 1, 0).unwrap(),
        )
        .unwrap();

        assert_eq!(context.acquisition_mode, ANALYSIS_MODE_ACTIVE);
        assert_eq!(context.node_count, 2);
        assert!(context.nodes[0].telemetry_available);
        assert!(!context.nodes[1].telemetry_available);
        assert_eq!(context.nodes[1].node_id, "cloud-01");
        assert_eq!(
            context.nodes[1].telemetry_error.as_deref(),
            Some("firebase unavailable")
        );
        assert!(!context.side_effects_performed);
    }
}
