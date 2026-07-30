use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    materialized::ProjectDGraphMaterializationSummary,
    planner::ProjectDGraphPlannerDryRunSummaryResponse,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphReadinessChecks {
    #[serde(default)]
    pub has_snapshots: Option<bool>,
    #[serde(default)]
    pub has_multiple_nodes: Option<bool>,
    #[serde(default)]
    pub has_node_scoped_runtimes: Option<bool>,
    #[serde(default)]
    pub has_modules: Option<bool>,
    #[serde(default)]
    pub has_flow_metadata: Option<bool>,
    #[serde(default)]
    pub has_planner_output: Option<bool>,
    #[serde(default)]
    pub has_materialization_errors: Option<bool>,
    #[serde(default)]
    pub has_warnings: Option<bool>,
    #[serde(default)]
    pub has_blockers: Option<bool>,
    #[serde(default)]
    pub has_degraded_flows: Option<bool>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectDGraphReadinessSummaryResponse {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub readiness_status: Option<String>,
    #[serde(default)]
    pub source_snapshot_count: Option<u64>,
    #[serde(default)]
    pub checks: Option<DGraphReadinessChecks>,
    #[serde(default)]
    pub materialized_summary: Option<ProjectDGraphMaterializationSummary>,
    #[serde(default)]
    pub planner_summary: Option<ProjectDGraphPlannerDryRunSummaryResponse>,
    #[serde(default)]
    pub materialization_validation_errors: Vec<String>,
    #[serde(default)]
    pub warning_count: Option<u64>,
    #[serde(default)]
    pub blocked_count: Option<u64>,
    #[serde(default)]
    pub degraded_flow_count: Option<u64>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::ProjectDGraphReadinessSummaryResponse;

    #[test]
    fn parses_readiness_summary() {
        let response: ProjectDGraphReadinessSummaryResponse = serde_json::from_str(r#"{
            "project_id":"smart-city-campinas", "readiness_status":"ready",
            "source_snapshot_count":2,
            "checks":{"has_snapshots":true,"has_multiple_nodes":true,"side_effects_performed":false},
            "materialized_summary":{"summary":{"node_count":2,"runtime_count":2,"module_count":2,"flow_count":1}},
            "planner_summary":{"status":"planned","warning_count":0,"blocked_count":0},
            "warning_count":0,"blocked_count":0,"degraded_flow_count":0,
            "side_effects_performed":false,
            "metadata":{"planner_mode":"dry_run","execution_performed":false}
        }"#).expect("readiness summary should parse");
        assert_eq!(response.readiness_status.as_deref(), Some("ready"));
        assert_eq!(response.source_snapshot_count, Some(2));
        assert_eq!(response.side_effects_performed, Some(false));
    }
}
