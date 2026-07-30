use axum::{
    extract::{Json, Path, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::{
    core::local_dgraph_snapshot::{
        normalize_local_dgraph_snapshot, validate_local_dgraph_snapshot, IncomingDGraphSnapshot,
        LocalDGraphSnapshot, LocalDGraphSnapshotList,
    },
    AppState,
};

type ApiError = (StatusCode, JsonResponse<Value>);

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string());

    match project_id {
        Some(project_id) if !project_id.is_empty() => Ok(project_id),
        _ => Err((
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "missing or invalid X-Datum-Project header" })),
        )),
    }
}

fn local_dgraph_snapshot_list(
    project_id: String,
    mut snapshots: Vec<LocalDGraphSnapshot>,
) -> LocalDGraphSnapshotList {
    snapshots.sort_by(|a, b| {
        a.node_id
            .cmp(&b.node_id)
            .then_with(|| a.reported_at_utc.cmp(&b.reported_at_utc))
    });

    LocalDGraphSnapshotList {
        project_id,
        count: snapshots.len() as u64,
        snapshots,
    }
}

pub async fn post_local_dgraph_snapshot(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(payload): Json<IncomingDGraphSnapshot>,
) -> Result<JsonResponse<LocalDGraphSnapshot>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let normalized = normalize_local_dgraph_snapshot(&project_id, &node_id, payload);

    validate_local_dgraph_snapshot(&normalized).map_err(|errors| {
        (
            StatusCode::BAD_REQUEST,
            JsonResponse(json!({ "error": "invalid local dgraph snapshot", "details": errors })),
        )
    })?;

    state
        .firebase
        .put_local_dgraph_snapshot(&project_id, &node_id, &normalized)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": error.to_string() })),
            )
        })?;

    Ok(JsonResponse(normalized))
}

pub async fn get_latest_local_dgraph_snapshot(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
) -> Result<JsonResponse<LocalDGraphSnapshot>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let snapshot = state
        .firebase
        .get_latest_local_dgraph_snapshot(&project_id, &node_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": error.to_string() })),
            )
        })?;

    match snapshot {
        Some(snapshot) => Ok(JsonResponse(snapshot)),
        None => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({
                "error": format!(
                    "local dgraph snapshot not found for project '{}' node '{}'",
                    project_id, node_id
                )
            })),
        )),
    }
}

pub async fn list_latest_local_dgraph_snapshots(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<JsonResponse<LocalDGraphSnapshotList>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let snapshots = state
        .firebase
        .list_latest_local_dgraph_snapshots(&project_id)
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                JsonResponse(json!({ "error": error.to_string() })),
            )
        })?;

    Ok(JsonResponse(local_dgraph_snapshot_list(
        project_id, snapshots,
    )))
}
