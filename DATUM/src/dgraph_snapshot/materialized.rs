use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphSummary {
    #[serde(default)]
    pub node_count: Option<u64>,
    #[serde(default)]
    pub runtime_count: Option<u64>,
    #[serde(default)]
    pub module_count: Option<u64>,
    #[serde(default)]
    pub artifact_count: Option<u64>,
    #[serde(default)]
    pub flow_count: Option<u64>,
    #[serde(default)]
    pub relationship_count: Option<u64>,
    #[serde(default)]
    pub healthy_targets: Option<u64>,
    #[serde(default)]
    pub unknown_targets: Option<u64>,
    #[serde(default)]
    pub degraded_targets: Option<u64>,
    #[serde(default)]
    pub unavailable_targets: Option<u64>,
    #[serde(default)]
    pub unhealthy_targets: Option<u64>,
    #[serde(default)]
    pub pending_deployment_intents: Option<u64>,
    #[serde(default)]
    pub proposed_placements: Option<u64>,
    #[serde(default)]
    pub proposed_reconfigurations: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectDGraphMaterialization {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub source_snapshot_count: Option<u64>,
    #[serde(default)]
    pub graph: Option<Value>,
    #[serde(default)]
    pub summary: Option<DGraphSummary>,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectDGraphMaterializationSummary {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub source_snapshot_count: Option<u64>,
    #[serde(default)]
    pub summary: Option<DGraphSummary>,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::{DGraphSummary, ProjectDGraphMaterialization, ProjectDGraphMaterializationSummary};

    #[test]
    fn parse_project_dgraph_materialization() {
        let raw = r#"{
            "project_id":"smart-city-campinas",
            "generated_at_utc":"2026-05-21T12:00:00Z",
            "source_snapshot_count":2,
            "graph":{"nodes":[{"node_id":"fog-01"},{"node_id":"fog-02"}]},
            "summary":{"node_count":2,"runtime_count":2,"module_count":4,"relationship_count":8},
            "validation_errors":[],
            "side_effects_performed":false,
            "metadata":{"materialization_kind":"project_local_dgraph_snapshots"}
        }"#;

        let parsed: ProjectDGraphMaterialization =
            serde_json::from_str(raw).expect("materialization should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.source_snapshot_count, Some(2));
        assert_eq!(
            parsed
                .summary
                .as_ref()
                .and_then(|summary| summary.node_count),
            Some(2)
        );
        assert_eq!(
            parsed
                .summary
                .as_ref()
                .and_then(|summary| summary.unknown_targets),
            None
        );
        assert!(parsed.graph.is_some());
        assert_eq!(parsed.side_effects_performed, Some(false));
    }

    #[test]
    fn parse_project_dgraph_materialization_summary() {
        let raw = r#"{
            "project_id":"smart-city-campinas",
            "source_snapshot_count":2,
            "summary":{
                "node_count":2,
                "runtime_count":2,
                "module_count":4,
                "artifact_count":2,
                "flow_count":1,
                "relationship_count":8,
                "unhealthy_targets":0,
                "pending_deployment_intents":0,
                "proposed_placements":0,
                "proposed_reconfigurations":0
            },
            "validation_errors":[],
            "side_effects_performed":false
        }"#;

        let parsed: ProjectDGraphMaterializationSummary =
            serde_json::from_str(raw).expect("materialization summary should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.source_snapshot_count, Some(2));
        assert_eq!(parsed.side_effects_performed, Some(false));
        assert!(parsed.validation_errors.is_empty());
    }

    #[test]
    fn parse_summary_counts() {
        let raw = r#"{
            "node_count":2,
            "runtime_count":2,
            "module_count":4,
            "artifact_count":2,
            "flow_count":1,
            "relationship_count":8,
            "healthy_targets":1,
            "unknown_targets":2,
            "degraded_targets":1,
            "unavailable_targets":0,
            "unhealthy_targets":1,
            "pending_deployment_intents":0,
            "proposed_placements":0,
            "proposed_reconfigurations":0
        }"#;

        let parsed: DGraphSummary = serde_json::from_str(raw).expect("summary should parse");

        assert_eq!(parsed.node_count, Some(2));
        assert_eq!(parsed.runtime_count, Some(2));
        assert_eq!(parsed.module_count, Some(4));
        assert_eq!(parsed.relationship_count, Some(8));
        assert_eq!(parsed.healthy_targets, Some(1));
        assert_eq!(parsed.unknown_targets, Some(2));
        assert_eq!(parsed.degraded_targets, Some(1));
        assert_eq!(parsed.unavailable_targets, Some(0));
        assert_eq!(parsed.unhealthy_targets, Some(1));
        assert_eq!(parsed.proposed_reconfigurations, Some(0));
    }
}
