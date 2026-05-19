use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    core::action_results::{
        normalize_action_execution_report, validate_action_execution_report, ActionExecutionReport,
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    match project_id {
        Some(p) if !p.is_empty() => Ok(p),
        _ => Err((
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "missing or invalid X-Datum-Project header" })),
        )),
    }
}

pub async fn post_action_execution_report(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<ActionExecutionReport>,
) -> Result<JsonResponse<ActionExecutionReport>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let normalized = normalize_action_execution_report(&project_id, &node_id, payload);

    validate_action_execution_report(&normalized).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    state
        .firebase
        .put_action_execution_report(&project_id, &node_id, &normalized)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(normalized))
}

pub async fn get_latest_action_execution_report(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<ActionExecutionReport>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let report = state
        .firebase
        .get_latest_action_execution_report(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match report {
        Some(report) => Ok(JsonResponse(report)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "action results not found for project '{}' node '{}'",
                    project_id, node_id
                )
            })),
        )),
    }
}

pub async fn list_latest_action_execution_reports(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let reports = state
        .firebase
        .list_latest_action_execution_reports(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": reports.len(),
        "reports": reports
    })))
}
