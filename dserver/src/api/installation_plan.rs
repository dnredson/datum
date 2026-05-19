use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::{json, Value};

use crate::{
    core::installation_plan::{
        derive_installation_plan_for_node, derive_installation_plan_from_deployment_planning,
        empty_installation_plan,
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

pub async fn get_latest_installation_plan(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let plan = state
        .firebase
        .get_action_plan_latest_optional(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_action_plan_latest_optional failed: {}", e),
            )
        })?;

    let deployment_planning = plan
        .as_ref()
        .and_then(parse_action_plan_payload)
        .and_then(|plan| plan.get("deployment_planning").cloned());
    let installation_plan = deployment_planning
        .as_ref()
        .map(derive_installation_plan_from_deployment_planning)
        .unwrap_or_else(empty_installation_plan);

    Ok(Json(json!({
        "project_id": project_id,
        "installation_plan": installation_plan
    })))
}

pub async fn get_installation_plan_for_node(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;
    let plan = state
        .firebase
        .get_action_plan_latest_optional(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_action_plan_latest_optional failed: {}", e),
            )
        })?;

    let deployment_planning = plan
        .as_ref()
        .and_then(parse_action_plan_payload)
        .and_then(|plan| plan.get("deployment_planning").cloned());
    let installation_plan = deployment_planning
        .as_ref()
        .map(|deployment_planning| derive_installation_plan_for_node(deployment_planning, &node_id))
        .unwrap_or_else(empty_installation_plan);

    Ok(Json(json!({
        "project_id": project_id,
        "node_id": node_id,
        "installation_plan": installation_plan
    })))
}

fn parse_action_plan_payload(raw: &Value) -> Option<Value> {
    let payload = raw
        .get("fields")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())?;

    serde_json::from_str(payload).ok()
}
