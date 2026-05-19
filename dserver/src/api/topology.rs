use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
};

use crate::AppState;

fn extract_project_id(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    headers
        .get("x-datum-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .ok_or((StatusCode::BAD_REQUEST, "missing project".into()))
}

pub async fn set_topology(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<(), (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    state
        .firebase
        .store_project_topology(&project_id, &payload)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(())
}

pub async fn get_topology(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let topo = state
        .firebase
        .get_project_topology(&project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(topo.unwrap_or_else(|| serde_json::json!({}))))
}
