use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::json;

use crate::{core::model::SnapshotEnvelope, AppState};

pub async fn ingest_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SnapshotEnvelope>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string());

    let project_id = match project_id {
        Some(p) if !p.is_empty() => p,
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                "missing or invalid X-Datum-Project header".to_string(),
            ))
        }
    };

    if payload.snapshot.meta.node_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing node_id".to_string()));
    }

    if payload.snapshot.meta.cycle_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "missing cycle_id".to_string()));
    }

    if payload.snapshot.meta.observed_at_utc.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "missing observed_at_utc".to_string(),
        ));
    }

    state
        .firebase
        .store_snapshot_raw(&project_id, &payload)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("raw store failed: {}", e),
            )
        })?;

    state
        .firebase
        .store_latest_snapshot(&project_id, &payload)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("latest store failed: {}", e),
            )
        })?;

    state
        .firebase
        .upsert_node(&project_id, &payload)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("node upsert failed: {}", e),
            )
        })?;

    Ok(Json(json!({
        "accepted": true,
        "project_id": project_id,
        "node_id": payload.snapshot.meta.node_id,
        "cycle_id": payload.snapshot.meta.cycle_id,
        "observed_at_utc": payload.snapshot.meta.observed_at_utc
    })))
}
