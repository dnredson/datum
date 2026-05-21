use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    core::{
        dgraph_materializer::build_project_dgraph_materialization,
        dgraph_planner::{
            build_project_dgraph_planner_dry_run_response, plan_dgraph_dry_run,
            ProjectDGraphPlannerDryRunResponse, ProjectDGraphPlannerDryRunSummaryResponse,
        },
    },
    AppState,
};

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string());

    match project_id {
        Some(project_id) if !project_id.is_empty() => Ok(project_id),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "missing or invalid X-Datum-Project header".to_string(),
        )),
    }
}

fn build_planner_response(
    project_id: &str,
    snapshots: &[crate::core::local_dgraph_snapshot::LocalDGraphSnapshot],
) -> ProjectDGraphPlannerDryRunResponse {
    let materialization = build_project_dgraph_materialization(project_id, snapshots);
    let planner = plan_dgraph_dry_run(&materialization.graph);

    build_project_dgraph_planner_dry_run_response(
        project_id,
        materialization.source_snapshot_count,
        materialization.validation_errors,
        planner,
    )
}

pub async fn get_dgraph_planner_dry_run(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProjectDGraphPlannerDryRunResponse>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let snapshots = state
        .firebase
        .list_latest_local_dgraph_snapshots(&project_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list_latest_local_dgraph_snapshots failed: {}", error),
            )
        })?;

    Ok(Json(build_planner_response(&project_id, &snapshots)))
}

pub async fn get_dgraph_planner_dry_run_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProjectDGraphPlannerDryRunSummaryResponse>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let snapshots = state
        .firebase
        .list_latest_local_dgraph_snapshots(&project_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list_latest_local_dgraph_snapshots failed: {}", error),
            )
        })?;

    let response = build_planner_response(&project_id, &snapshots);
    Ok(Json(response.as_summary()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::local_dgraph_snapshot::{
        LocalCapabilitySnapshot, LocalDGraphSnapshot, LocalFlowSnapshot, LocalHealthSnapshot,
        LocalModuleSnapshot, LocalRuntimeSnapshot,
    };

    fn snapshot(node_id: &str) -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            project_id: "smart-city-campinas".to_string(),
            node_id: node_id.to_string(),
            snapshot_id: format!("snapshot-{}", node_id),
            reported_at_utc: "2026-05-21T12:00:00Z".to_string(),
            source: Some("client".to_string()),
            generated_at_utc: Some("2026-05-21T11:59:00Z".to_string()),
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: format!("runtime-{}", node_id),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "local_config".to_string(),
                metadata: None,
            }],
            modules: vec![LocalModuleSnapshot {
                module_id: format!("module-{}", node_id),
                module_kind: Some("service".to_string()),
                desired_state: Some("running".to_string()),
                observed_state: Some("missing".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: None,
                local_mapping_key: None,
                local_container_name: Some(format!("container-{}", node_id)),
                source: "service_registry".to_string(),
                metadata: None,
            }],
            flows: vec![LocalFlowSnapshot {
                flow_id: format!("flow-{}", node_id),
                flow_kind: Some("service_dependency".to_string()),
                modules: vec![format!("module-{}", node_id)],
                edges: vec![],
                status: Some("degraded".to_string()),
                source: "local_config".to_string(),
                metadata: None,
            }],
            health: vec![LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: format!("module-{}", node_id),
                status: "unknown".to_string(),
                reason: Some("reporting only".to_string()),
                source: "local_config".to_string(),
                metadata: None,
            }],
            capabilities: vec![LocalCapabilitySnapshot {
                capability_id: format!("capability-{}", node_id),
                capability_kind: "container_runtime_configured".to_string(),
                supported: true,
                source: "local_config".to_string(),
                metadata: None,
            }],
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn planner_summary_for_empty_snapshot_set_is_clean_and_read_only() {
        let response = build_planner_response("smart-city-campinas", &[]);
        let summary = response.as_summary();

        assert_eq!(summary.project_id, "smart-city-campinas");
        assert_eq!(summary.source_snapshot_count, 0);
        assert_eq!(summary.status, "empty");
        assert!(!summary.side_effects_performed);
    }

    #[test]
    fn planner_response_preserves_materialization_validation_errors() {
        let mut broken = snapshot("fog-01");
        broken.flows[0].modules.push("missing-module".to_string());

        let response = build_planner_response("smart-city-campinas", &[broken]);
        assert!(!response.materialization_validation_errors.is_empty());
        assert!(!response.side_effects_performed);
    }

    #[test]
    fn planner_summary_counts_match_response_contents() {
        let response = build_planner_response("smart-city-campinas", &[snapshot("fog-01")]);
        let summary = response.as_summary();

        assert_eq!(
            summary.placement_count,
            response.planner.placements.len() as u64
        );
        assert_eq!(
            summary.reconfiguration_count,
            response.planner.reconfigurations.len() as u64
        );
        assert_eq!(summary.blocked_count, response.planner.blocked.len() as u64);
        assert_eq!(
            summary.warning_count,
            response.planner.warnings.len() as u64
        );
    }

    #[test]
    fn project_header_extraction_requires_x_datum_project() {
        let headers = HeaderMap::new();
        let error = extract_project_id(&headers).expect_err("header should be required");

        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert!(error.1.contains("X-Datum-Project"));
    }
}
