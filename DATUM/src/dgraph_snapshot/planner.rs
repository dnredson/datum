use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectDGraphPlannerDryRunResponse {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub source_snapshot_count: Option<u64>,
    #[serde(default)]
    pub materialization_validation_errors: Vec<String>,
    #[serde(default)]
    pub planner: Option<DGraphPlannerDryRun>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerDryRun {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub planner_version: Option<String>,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub placements: Vec<DGraphPlacementDecision>,
    #[serde(default)]
    pub reconfigurations: Vec<DGraphReconfigurationDecision>,
    #[serde(default)]
    pub blocked: Vec<DGraphPlannerBlockedItem>,
    #[serde(default)]
    pub warnings: Vec<DGraphPlannerWarning>,
    #[serde(default)]
    pub advisory_proposals: Vec<DGraphPlannerAdvisoryProposal>,
    #[serde(default)]
    pub summary: Option<DGraphPlannerSummary>,
    #[serde(default)]
    pub safe_to_auto_execute: Option<bool>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlacementDecision {
    #[serde(default)]
    pub placement_id: Option<String>,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub selected_node_id: Option<String>,
    #[serde(default)]
    pub selected_runtime_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphReconfigurationDecision {
    #[serde(default)]
    pub reconfiguration_id: Option<String>,
    #[serde(default)]
    pub flow_id: Option<String>,
    #[serde(default)]
    pub affected_module_id: Option<String>,
    #[serde(default)]
    pub from_node_id: Option<String>,
    #[serde(default)]
    pub to_node_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerBlockedItem {
    #[serde(default)]
    pub item_id: Option<String>,
    #[serde(default)]
    pub target_kind: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerWarning {
    #[serde(default)]
    pub warning_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerAdvisoryReason {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerAdvisoryEvidence {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub observed_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerAdvisoryProposal {
    #[serde(default)]
    pub proposal_id: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub reason: Option<DGraphPlannerAdvisoryReason>,
    #[serde(default)]
    pub evidence: Vec<DGraphPlannerAdvisoryEvidence>,
    #[serde(default)]
    pub related_flow_ids: Vec<String>,
    #[serde(default)]
    pub related_node_ids: Vec<String>,
    #[serde(default)]
    pub candidate_node_ids: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub advisory_only: Option<bool>,
    #[serde(default)]
    pub executable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraphPlannerSummary {
    #[serde(default)]
    pub placement_count: Option<u64>,
    #[serde(default)]
    pub reconfiguration_count: Option<u64>,
    #[serde(default)]
    pub blocked_count: Option<u64>,
    #[serde(default)]
    pub warning_count: Option<u64>,
    #[serde(default)]
    pub candidate_node_count: Option<u64>,
    #[serde(default)]
    pub unhealthy_node_count: Option<u64>,
    #[serde(default)]
    pub degraded_flow_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectDGraphPlannerDryRunSummaryResponse {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub source_snapshot_count: Option<u64>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub summary: Option<DGraphPlannerSummary>,
    #[serde(default)]
    pub placement_count: Option<u64>,
    #[serde(default)]
    pub reconfiguration_count: Option<u64>,
    #[serde(default)]
    pub blocked_count: Option<u64>,
    #[serde(default)]
    pub warning_count: Option<u64>,
    #[serde(default)]
    pub side_effects_performed: Option<bool>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::{
        DGraphPlannerDryRun, DGraphPlannerSummary, ProjectDGraphPlannerDryRunResponse,
        ProjectDGraphPlannerDryRunSummaryResponse,
    };

    #[test]
    fn parse_project_dgraph_planner_dry_run_response() {
        let raw = r#"{
            "project_id":"smart-city-campinas",
            "source_snapshot_count":2,
            "materialization_validation_errors":["partial graph"],
            "planner":{
                "project_id":"smart-city-campinas",
                "planner_version":"0.1.0",
                "generated_at_utc":"2026-05-21T12:00:00Z",
                "status":"planned",
                "placements":[{"placement_id":"p1","module_id":"chirpstack-primary","selected_node_id":"fog-01","selected_runtime_id":"runtime-container-fog-01","reason":"available node","status":"proposed"}],
                "reconfigurations":[{"reconfiguration_id":"r1","flow_id":"flow-a","affected_module_id":"broker","from_node_id":"fog-01","to_node_id":"fog-02","reason":"failover","status":"proposed"}],
                "blocked":[{"item_id":"b1","target_kind":"module","target_id":"broker","reason":"no candidate"}],
                "warnings":[{"warning_id":"w1","reason":"runtime fallback"}],
                "advisory_proposals":[{"proposal_id":"advisory:degraded-flow:flow-a","category":"degraded_flow","summary":"Observed degraded flow flow-a.","reason":{"code":"degraded_flow_observed","message":"The flow is marked degraded in the D-Graph."},"evidence":[{"kind":"flow","entity_id":"flow-a","observed_status":"degraded"}],"related_flow_ids":["flow-a"],"related_node_ids":["fog-01"],"candidate_node_ids":["fog-02"],"limitations":["advisory only"],"advisory_only":true,"executable":false}],
                "summary":{"placement_count":1,"reconfiguration_count":1,"blocked_count":1,"warning_count":1,"candidate_node_count":2,"unhealthy_node_count":0,"degraded_flow_count":0},
                "safe_to_auto_execute":false,
                "side_effects_performed":false
            },
            "side_effects_performed":false,
            "metadata":{"planner_mode":"dry_run"}
        }"#;

        let parsed: ProjectDGraphPlannerDryRunResponse =
            serde_json::from_str(raw).expect("planner dry-run should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.source_snapshot_count, Some(2));
        assert_eq!(parsed.side_effects_performed, Some(false));
        assert_eq!(
            parsed
                .planner
                .as_ref()
                .and_then(|planner| planner.status.as_deref()),
            Some("planned")
        );
        let planner = parsed.planner.as_ref().expect("planner should be present");
        assert_eq!(planner.advisory_proposals.len(), 1);
        assert_eq!(
            planner.advisory_proposals[0].proposal_id,
            Some("advisory:degraded-flow:flow-a".to_owned())
        );
        assert_eq!(planner.advisory_proposals[0].advisory_only, Some(true));
        assert_eq!(planner.advisory_proposals[0].executable, Some(false));
        assert_eq!(planner.safe_to_auto_execute, Some(false));
    }

    #[test]
    fn parse_project_dgraph_planner_dry_run_summary_response() {
        let raw = r#"{
            "project_id":"smart-city-campinas",
            "source_snapshot_count":2,
            "status":"planned",
            "summary":{"placement_count":1,"reconfiguration_count":0,"blocked_count":0,"warning_count":0,"candidate_node_count":2,"unhealthy_node_count":0,"degraded_flow_count":0},
            "placement_count":1,
            "reconfiguration_count":0,
            "blocked_count":0,
            "warning_count":0,
            "side_effects_performed":false
        }"#;

        let parsed: ProjectDGraphPlannerDryRunSummaryResponse =
            serde_json::from_str(raw).expect("planner summary should parse");

        assert_eq!(parsed.project_id.as_deref(), Some("smart-city-campinas"));
        assert_eq!(parsed.status.as_deref(), Some("planned"));
        assert_eq!(parsed.side_effects_performed, Some(false));
        assert_eq!(parsed.placement_count, Some(1));
    }

    #[test]
    fn parse_placements_reconfigurations_blocked_and_warnings() {
        let raw = r#"{
            "project_id":"smart-city-campinas",
            "planner_version":"0.1.0",
            "status":"partially_blocked",
            "placements":[{"module_id":"a"}],
            "reconfigurations":[{"flow_id":"f1","affected_module_id":"b"}],
            "blocked":[{"target_kind":"module","target_id":"c","reason":"blocked"}],
            "warnings":[{"reason":"warning"}],
            "summary":{"placement_count":1,"reconfiguration_count":1,"blocked_count":1,"warning_count":1},
            "side_effects_performed":false
        }"#;

        let parsed: DGraphPlannerDryRun =
            serde_json::from_str(raw).expect("planner payload should parse");

        assert_eq!(parsed.placements.len(), 1);
        assert_eq!(parsed.reconfigurations.len(), 1);
        assert_eq!(parsed.blocked.len(), 1);
        assert_eq!(parsed.warnings.len(), 1);
        assert_eq!(parsed.side_effects_performed, Some(false));
    }

    #[test]
    fn parse_summary_counts() {
        let raw = r#"{
            "placement_count":1,
            "reconfiguration_count":2,
            "blocked_count":3,
            "warning_count":4,
            "candidate_node_count":5,
            "unhealthy_node_count":6,
            "degraded_flow_count":7
        }"#;

        let parsed: DGraphPlannerSummary =
            serde_json::from_str(raw).expect("planner summary should parse");

        assert_eq!(parsed.placement_count, Some(1));
        assert_eq!(parsed.reconfiguration_count, Some(2));
        assert_eq!(parsed.blocked_count, Some(3));
        assert_eq!(parsed.warning_count, Some(4));
        assert_eq!(parsed.candidate_node_count, Some(5));
        assert_eq!(parsed.unhealthy_node_count, Some(6));
        assert_eq!(parsed.degraded_flow_count, Some(7));
    }
}
