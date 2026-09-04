use std::{
    collections::BTreeMap,
    sync::{OnceLock, RwLock},
};

use axum::{
    extract::{Json, Query},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::operational_evidence::{
    validate_operational_evidence_event, OperationalEvidenceEvent,
};

type ApiError = (StatusCode, JsonResponse<Value>);

static OPERATIONAL_EVIDENCE: OnceLock<RwLock<BTreeMap<String, Vec<OperationalEvidenceEvent>>>> =
    OnceLock::new();

fn store() -> &'static RwLock<BTreeMap<String, Vec<OperationalEvidenceEvent>>> {
    OPERATIONAL_EVIDENCE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

#[derive(Debug, Deserialize)]
pub struct OperationalEvidenceQuery {
    #[serde(default)]
    pub resource_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub dgraph_id: Option<String>,
    #[serde(default)]
    pub event_kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn post_operational_evidence(
    headers: HeaderMap,
    Json(mut payload): Json<OperationalEvidenceEvent>,
) -> Result<JsonResponse<OperationalEvidenceEvent>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    payload.project_id = project_id.clone();
    validate_operational_evidence_event(&payload).map_err(bad_request)?;

    let mut evidence = store().write().map_err(lock_error)?;
    let events = evidence.entry(project_id).or_default();
    if events.iter().any(|event| event.event_id == payload.event_id) {
        return Err(conflict(format!(
            "operational evidence event_id '{}' already exists",
            payload.event_id
        )));
    }
    events.push(payload.clone());

    Ok(JsonResponse(payload))
}

pub async fn list_operational_evidence(
    headers: HeaderMap,
    Query(query): Query<OperationalEvidenceQuery>,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let evidence = store().read().map_err(lock_error)?;
    let mut events = evidence
        .get(&project_id)
        .into_iter()
        .flat_map(|events| events.iter())
        .filter(|event| {
            matches_optional(query.resource_id.as_deref(), Some(event.resource_id.as_str()))
                && matches_optional(query.node_id.as_deref(), event.node_id.as_deref())
                && matches_optional(query.dgraph_id.as_deref(), event.dgraph_id.as_deref())
                && matches_optional(query.event_kind.as_deref(), Some(event.event_kind.as_str()))
        })
        .cloned()
        .collect::<Vec<_>>();

    if events.len() > limit {
        events = events.split_off(events.len() - limit);
    }

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": events.len(),
        "events": events,
    })))
}

fn matches_optional(filter: Option<&str>, value: Option<&str>) -> bool {
    match filter.map(str::trim).filter(|value| !value.is_empty()) {
        Some(filter) => value.is_some_and(|value| value == filter),
        None => true,
    }
}

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| bad_request("missing or invalid X-Datum-Project header".to_string()))
}

fn bad_request(message: String) -> ApiError {
    (StatusCode::BAD_REQUEST, JsonResponse(json!({"error": message})))
}

fn conflict(message: String) -> ApiError {
    (StatusCode::CONFLICT, JsonResponse(json!({"error": message})))
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error":"operational evidence store lock poisoned"})),
    )
}
