use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{core::events::build_global_events, AppState};

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

pub async fn build_latest_global_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw = state
        .firebase
        .get_analysis_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_analysis_latest failed: {}", e),
            )
        })?;

    let payload = raw
        .get("fields")
        .and_then(|v| v.get("payload"))
        .and_then(|v| v.get("stringValue"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "analysis payload missing".to_string(),
            )
        })?;

    let analysis: serde_json::Value = serde_json::from_str(payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid analysis payload: {}", e),
        )
    })?;

    let events = build_global_events(&analysis);

    state
        .firebase
        .store_global_events_latest(&project_id, &events)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store_global_events_latest failed: {}", e),
            )
        })?;

    Ok(Json(events))
}

pub async fn get_materialized_global_events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw = state
        .firebase
        .get_global_events_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_global_events_latest failed: {}", e),
            )
        })?;

    Ok(Json(raw))
}
