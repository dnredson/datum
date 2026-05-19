use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::Utc;
use serde_json::{json, Value};

use crate::{
    core::capabilities::{
        normalize_node_capabilities, validate_node_capabilities, NodeCapabilities,
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

pub async fn put_node_capabilities(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<NodeCapabilities>,
) -> Result<JsonResponse<NodeCapabilities>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let mut normalized = payload;
    normalized.project_id = project_id.clone();
    normalized.node_id = node_id.clone();
    normalized.updated_at_utc = Utc::now().to_rfc3339();
    normalized = normalize_node_capabilities(normalized);

    validate_node_capabilities(&normalized).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": err })),
        )
    })?;

    state
        .firebase
        .put_node_capabilities(&project_id, &node_id, &normalized)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(normalized))
}

pub async fn get_node_capabilities(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<NodeCapabilities>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let capabilities = state
        .firebase
        .get_node_capabilities(&project_id, &node_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    match capabilities {
        Some(caps) => Ok(JsonResponse(caps)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!("capabilities not found for project '{}' node '{}'", project_id, node_id)
            })),
        )),
    }
}

pub async fn list_capabilities(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let capabilities = state
        .firebase
        .list_node_capabilities(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": e.to_string() })),
            )
        })?;

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": capabilities.len(),
        "capabilities": capabilities
    })))
}
