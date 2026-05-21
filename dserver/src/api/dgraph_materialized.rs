use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};

use crate::{
    core::dgraph_materializer::{
        build_project_dgraph_materialization, ProjectDGraphMaterialization,
        ProjectDGraphMaterializationSummary,
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

pub async fn get_materialized_dgraph(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProjectDGraphMaterialization>, (StatusCode, String)> {
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

    Ok(Json(build_project_dgraph_materialization(
        &project_id,
        &snapshots,
    )))
}

pub async fn get_materialized_dgraph_summary(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ProjectDGraphMaterializationSummary>, (StatusCode, String)> {
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

    let materialization = build_project_dgraph_materialization(&project_id, &snapshots);
    Ok(Json(materialization.as_summary()))
}
