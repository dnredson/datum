use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{
    dgraph_materializer::{
        build_project_dgraph_materialization, ProjectDGraphMaterializationSummary,
    },
    dgraph_planner::{
        build_project_dgraph_planner_dry_run_response, plan_dgraph_dry_run,
        ProjectDGraphPlannerDryRunSummaryResponse,
    },
    local_dgraph_snapshot::LocalDGraphSnapshot,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphReadinessChecks {
    pub has_snapshots: bool,
    pub has_multiple_nodes: bool,
    pub has_node_scoped_runtimes: bool,
    pub has_modules: bool,
    pub has_flow_metadata: bool,
    pub has_planner_output: bool,
    pub has_materialization_errors: bool,
    pub has_warnings: bool,
    pub has_blockers: bool,
    pub has_degraded_flows: bool,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphReadinessSummaryResponse {
    pub project_id: String,
    pub readiness_status: String,
    pub source_snapshot_count: u64,
    pub checks: DGraphReadinessChecks,
    pub materialized_summary: ProjectDGraphMaterializationSummary,
    pub planner_summary: ProjectDGraphPlannerDryRunSummaryResponse,
    pub materialization_validation_errors: Vec<String>,
    pub warning_count: u64,
    pub blocked_count: u64,
    pub degraded_flow_count: u64,
    pub side_effects_performed: bool,
    pub metadata: Option<Value>,
}

pub fn build_project_dgraph_readiness_summary(
    project_id: &str,
    snapshots: &[LocalDGraphSnapshot],
) -> ProjectDGraphReadinessSummaryResponse {
    let materialization = build_project_dgraph_materialization(project_id, snapshots);
    let planner_response = build_project_dgraph_planner_dry_run_response(
        project_id,
        materialization.source_snapshot_count,
        materialization.validation_errors.clone(),
        plan_dgraph_dry_run(&materialization.graph),
    );
    let materialized_summary = materialization.as_summary();
    let planner_summary = planner_response.as_summary();
    let summary = &materialized_summary.summary;
    let planner_counts = &planner_summary.summary;
    let side_effects_performed = materialization.side_effects_performed
        || planner_response.side_effects_performed
        || planner_response.planner.side_effects_performed;
    let checks = DGraphReadinessChecks {
        has_snapshots: materialization.source_snapshot_count > 0,
        has_multiple_nodes: summary.node_count >= 2,
        has_node_scoped_runtimes: summary.node_count > 0
            && summary.runtime_count >= summary.node_count,
        has_modules: summary.module_count > 0,
        has_flow_metadata: summary.flow_count > 0,
        has_planner_output: true,
        has_materialization_errors: !materialization.validation_errors.is_empty(),
        has_warnings: planner_counts.warning_count > 0,
        has_blockers: planner_counts.blocked_count > 0,
        has_degraded_flows: planner_counts.degraded_flow_count > 0,
        side_effects_performed,
    };
    let readiness_status = if checks.side_effects_performed
        || checks.has_materialization_errors
        || checks.has_blockers
    {
        "blocked"
    } else if !checks.has_snapshots
        || !checks.has_modules
        || !checks.has_flow_metadata
        || !checks.has_node_scoped_runtimes
    {
        "incomplete"
    } else if checks.has_warnings || checks.has_degraded_flows {
        "ready_with_warnings"
    } else {
        "ready"
    };
    let warning_count = planner_counts.warning_count;
    let blocked_count = planner_counts.blocked_count;
    let degraded_flow_count = planner_counts.degraded_flow_count;

    ProjectDGraphReadinessSummaryResponse {
        project_id: project_id.to_string(),
        readiness_status: readiness_status.to_string(),
        source_snapshot_count: materialization.source_snapshot_count,
        checks,
        materialized_summary,
        planner_summary,
        materialization_validation_errors: materialization.validation_errors,
        warning_count,
        blocked_count,
        degraded_flow_count,
        side_effects_performed,
        metadata: Some(json!({
            "mode": "metadata_only_readiness_summary",
            "planner_mode": "dry_run",
            "execution_performed": false,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::local_dgraph_snapshot::{
        LocalDGraphSnapshot, LocalFlowSnapshot, LocalModuleSnapshot, LocalRuntimeSnapshot,
    };

    fn snapshot(node_id: &str, flow_status: &str) -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            project_id: "smart-city-campinas".to_string(),
            node_id: node_id.to_string(),
            snapshot_id: format!("snapshot-{node_id}"),
            reported_at_utc: "2026-07-30T12:00:00Z".to_string(),
            source: Some("fixture".to_string()),
            generated_at_utc: None,
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: "runtime-container".to_string(),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "fixture".to_string(),
                metadata: None,
            }],
            modules: vec![LocalModuleSnapshot {
                module_id: format!("module-{node_id}"),
                module_kind: Some("service".to_string()),
                desired_state: Some("running".to_string()),
                observed_state: Some("missing".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: None,
                local_mapping_key: None,
                local_container_name: None,
                source: "fixture".to_string(),
                metadata: None,
            }],
            flows: vec![LocalFlowSnapshot {
                flow_id: "flow-chirpstack-to-mqtt-adapter".to_string(),
                flow_kind: Some("mqtt".to_string()),
                modules: vec![format!("module-{node_id}")],
                edges: vec![],
                status: Some(flow_status.to_string()),
                source: "fixture".to_string(),
                metadata: None,
            }],
            health: vec![],
            capabilities: vec![],
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn empty_snapshots_are_incomplete_and_read_only() {
        let response = build_project_dgraph_readiness_summary("isolated-project", &[]);
        assert_eq!(response.readiness_status, "incomplete");
        assert_eq!(response.source_snapshot_count, 0);
        assert!(!response.checks.has_snapshots);
        assert!(!response.side_effects_performed);
        assert_eq!(response.metadata.unwrap()["execution_performed"], false);
    }

    #[test]
    fn healthy_two_node_metadata_graph_is_ready() {
        let response = build_project_dgraph_readiness_summary(
            "smart-city-campinas",
            &[
                snapshot("fog-01", "configured"),
                snapshot("fog-02", "configured"),
            ],
        );
        assert_eq!(response.readiness_status, "ready");
        assert_eq!(response.source_snapshot_count, 2);
        assert!(response.checks.has_multiple_nodes);
        assert!(response.checks.has_node_scoped_runtimes);
        assert!(response.checks.has_modules);
        assert!(response.checks.has_flow_metadata);
        assert_eq!(response.warning_count, 0);
        assert_eq!(response.degraded_flow_count, 0);
        assert_eq!(response.blocked_count, 0);
        assert!(!response.side_effects_performed);
    }

    #[test]
    fn healthy_three_node_continuum_is_ready_and_read_only() {
        let mut fog_01 = snapshot("fog-01", "configured");
        let mut fog_02 = snapshot("fog-02", "configured");
        let mut cloud_01 = snapshot("cloud-01", "configured");
        fog_01.modules[0].module_id = "chirpstack-primary".to_string();
        fog_01.flows[0].modules = vec!["chirpstack-primary".to_string()];
        fog_02.modules[0].module_id = "mqtt-adapter".to_string();
        fog_02.flows[0].flow_id = "flow-mqtt-adapter-to-magistrala-core".to_string();
        fog_02.flows[0].modules = vec!["mqtt-adapter".to_string()];
        cloud_01.modules[0].module_id = "magistrala-core".to_string();
        cloud_01.flows.clear();

        let response = build_project_dgraph_readiness_summary(
            "smart-city-campinas",
            &[fog_01, fog_02, cloud_01],
        );

        assert_eq!(response.readiness_status, "ready");
        assert_eq!(response.source_snapshot_count, 3);
        assert_eq!(response.materialized_summary.summary.node_count, 3);
        assert_eq!(response.materialized_summary.summary.runtime_count, 3);
        assert_eq!(response.materialized_summary.summary.module_count, 3);
        assert_eq!(response.materialized_summary.summary.flow_count, 2);
        assert!(response.checks.has_multiple_nodes);
        assert!(response.checks.has_node_scoped_runtimes);
        assert!(response.checks.has_flow_metadata);
        assert_eq!(response.warning_count, 0);
        assert_eq!(response.degraded_flow_count, 0);
        assert_eq!(response.blocked_count, 0);
        assert!(!response.side_effects_performed);
        assert_eq!(response.metadata.unwrap()["execution_performed"], false);
    }

    #[test]
    fn degraded_flow_is_ready_with_warnings_and_read_only() {
        let response = build_project_dgraph_readiness_summary(
            "smart-city-campinas",
            &[
                snapshot("fog-01", "degraded"),
                snapshot("fog-02", "configured"),
            ],
        );
        assert_eq!(response.readiness_status, "ready_with_warnings");
        assert!(response.warning_count >= 1);
        assert!(response.degraded_flow_count >= 1);
        assert!(response.checks.has_warnings);
        assert!(response.checks.has_degraded_flows);
        assert!(!response.side_effects_performed);
        assert_eq!(response.metadata.unwrap()["planner_mode"], "dry_run");
    }
}
