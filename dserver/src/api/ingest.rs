use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use serde_json::json;

use crate::{
    api::passive_observation_trigger::process_passive_observation_trigger,
    core::model::SnapshotEnvelope,
    AppState,
};

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

    // Snapshot persistence owns the ingest outcome. Passive assessment is fail-soft:
    // a missing D-Graph or temporary analysis problem must not reject telemetry that
    // has already been durably accepted by Firebase.
    let passive_trigger = process_passive_observation_trigger(
        &state,
        &project_id,
        &payload.snapshot.meta.node_id,
        &payload.snapshot.meta.cycle_id,
        &payload.snapshot.meta.observed_at_utc,
        "dserver_snapshot_ingest",
    )
    .await;

    let passive_analysis = json!({
        "trigger_source": passive_trigger.trigger_source,
        "analysis_status": passive_trigger.analysis_status,
        "latest_cycle_id": passive_trigger.latest_cycle_id,
        "latest_observed_at_utc": passive_trigger.latest_observed_at_utc,
        "notification_matches_latest": passive_trigger.notification_matches_latest,
        "telemetry_freshness": passive_trigger
            .assessment
            .as_ref()
            .and_then(|assessment| assessment.nodes.first())
            .map(|node| node.telemetry_freshness.clone()),
        "service_comparisons": passive_trigger
            .assessment
            .as_ref()
            .and_then(|assessment| assessment.nodes.first())
            .map(|node| node.service_comparisons.clone())
            .unwrap_or_default(),
        "analysis_error": passive_trigger.analysis_error,
        "side_effects_performed": false
    });

    Ok(Json(json!({
        "accepted": true,
        "project_id": project_id,
        "node_id": payload.snapshot.meta.node_id,
        "cycle_id": payload.snapshot.meta.cycle_id,
        "observed_at_utc": payload.snapshot.meta.observed_at_utc,
        "passive_analysis": passive_analysis
    })))
}
