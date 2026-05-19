use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    core::{graph_builder::build_latest_graph, topology::apply_topology},
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

pub async fn get_latest_graph(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    // 1. snapshots
    let docs = state
        .firebase
        .list_latest_snapshots_by_project(&project_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("list_latest_snapshots_by_project failed: {}", e),
            )
        })?;

    // 2. build graph
    let mut graph = build_latest_graph(&project_id, &docs);

    // 3. load topology from firebase
    let topology = state
        .firebase
        .get_project_topology(&project_id)
        .await
        .ok()
        .flatten();

    // 4. apply topology
    graph = apply_topology(graph, topology.as_ref());

    // 5. persist
    state
        .firebase
        .store_graph_latest(&project_id, &graph)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("store_graph_latest failed: {}", e),
            )
        })?;

    Ok(Json(graph))
}

pub async fn get_materialized_graph(
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

    Ok(Json(raw))
}
