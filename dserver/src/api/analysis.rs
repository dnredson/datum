use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{core::analyzer::analyze_graph, AppState};

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

pub async fn analyze_latest_graph(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let raw = state
        .firebase
        .get_graph_latest(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("get_graph_latest failed: {}", e),
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
                "graph payload missing".to_string(),
            )
        })?;

    let graph: serde_json::Value = serde_json::from_str(payload).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("invalid graph payload: {}", e),
        )
    })?;

    let analysis = analyze_graph(&graph);

    state
        .firebase
        .store_analysis_latest(&project_id, &analysis)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store_analysis_latest failed: {}", e),
            )
        })?;

    Ok(Json(analysis))
}

pub async fn get_materialized_analysis(
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

    Ok(Json(raw))
}
