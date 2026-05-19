use std::collections::BTreeMap;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    core::{
        action_feedback::annotate_action_plan_with_latest_results,
        action_results::ActionExecutionReport,
        capabilities::NodeCapabilities,
        planner::{annotate_action_plan_with_capabilities, build_global_action_plan},
    },
    AppState,
};

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    match project_id {
        Some(p) if !p.is_empty() => Ok(p),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "missing or invalid X-Datum-Project header".to_string(),
        )),
    }
}

pub async fn build_latest_action_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let graph_raw = state
        .firebase
        .get_graph_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_graph_latest failed: {}", e),
            )
        })?;

    let analysis_raw = state
        .firebase
        .get_analysis_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_analysis_latest failed: {}", e),
            )
        })?;

    let graph_payload = graph_raw
        .get("fields")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "graph payload missing".to_string(),
        ))?;

    let analysis_payload = analysis_raw
        .get("fields")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "analysis payload missing".to_string(),
        ))?;

    let graph: serde_json::Value = serde_json::from_str(graph_payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid graph payload: {}", e),
        )
    })?;

    let analysis: serde_json::Value = serde_json::from_str(analysis_payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid analysis payload: {}", e),
        )
    })?;

    let dmap = state
        .firebase
        .get_project_dmap(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_project_dmap failed: {}", e),
            )
        })?;

    let capabilities = state
        .firebase
        .list_node_capabilities(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list_node_capabilities failed: {}", e),
            )
        })?;

    let capabilities_by_node = capabilities
        .into_iter()
        .map(|capability: NodeCapabilities| (capability.node_id.clone(), capability))
        .collect::<BTreeMap<_, _>>();

    let mut plan = annotate_action_plan_with_capabilities(
        build_global_action_plan(&project_id, &graph, &analysis, dmap.as_ref()),
        &capabilities_by_node,
    );

    match state
        .firebase
        .list_latest_action_execution_reports(&project_id)
        .await
    {
        Ok(reports) => {
            let reports_by_node = reports
                .into_iter()
                .map(|report: ActionExecutionReport| (report.node_id.clone(), report))
                .collect::<std::collections::HashMap<_, _>>();
            plan = annotate_action_plan_with_latest_results(plan, &reports_by_node);
        }
        Err(err) => {
            if let Some(plan_obj) = plan.as_object_mut() {
                plan_obj.insert(
                    "feedback_load_error".to_string(),
                    serde_json::json!(err.to_string()),
                );
            }
        }
    }

    state
        .firebase
        .store_action_plan_latest(&project_id, &plan)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store_action_plan_latest failed: {}", e),
            )
        })?;

    Ok(Json(plan))
}

pub async fn get_materialized_action_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw = state
        .firebase
        .get_action_plan_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_action_plan_latest failed: {}", e),
            )
        })?;

    Ok(Json(raw))
}
