use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::AppState;

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

pub async fn get_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let graph = state
        .firebase
        .get_graph_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_graph_latest failed: {}", e),
            )
        })?;

    let analysis = state
        .firebase
        .get_analysis_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_analysis_latest failed: {}", e),
            )
        })?;

    let events = state
        .firebase
        .get_global_events_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_global_events_latest failed: {}", e),
            )
        })?;

    Ok(Json(serde_json::json!({
        "project_id": project_id,
        "graph": graph,
        "analysis": analysis,
        "events": events
    })))
}
