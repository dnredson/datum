//! LEGACY, pre-Stage-111 endpoint: an untyped Firebase JSON blob keyed by
//! project, with no schema and no relationship to the canonical
//! `datum.dmap/1` contract (Stage 110, `core::canonical_dmap`) or to the
//! D-Deploy proposal/acceptance lifecycle (Stage 111, ADR-0018,
//! `core::ddeploy` / `api::ddeploy`). Its `"accepted"` field, if present in
//! caller-supplied JSON, is opaque application data here -- it is NEVER
//! interpreted as a D-Deploy acceptance, and this handler never parses its
//! stored payload as a `CanonicalDMapV1`. Kept unmodified for backward
//! compatibility; do not extend it to carry canonical placement authority.

use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
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

pub async fn set_dmap(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    state
        .firebase
        .store_project_dmap(&project_id, &payload)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "accepted": true,
        "project_id": project_id
    })))
}

pub async fn get_dmap(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let project_id = extract_project_id(&headers)?;

    let dmap = state
        .firebase
        .get_project_dmap(&project_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(dmap.unwrap_or_else(|| serde_json::json!({}))))
}
