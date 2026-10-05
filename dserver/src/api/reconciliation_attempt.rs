//! Phase 138A (Part D): the smallest possible read-only API over the
//! DMap-native reconciliation-attempt record (`core::reconciliation_attempt`).
//! Deliberately NO execute endpoint here -- the live production execution
//! trigger remains the `smartsentinel --execute` agent CLI invocation
//! (Phase 138A.0 trace); Phase 138 proper decides how/whether Console
//! orchestration ever triggers the agent. This surface exists only so
//! Phase 138's eventual Console workspace has something real to read.

use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    core::reconciliation_attempt::{
        attempt_domain, attempt_index_domain, ReconciliationAttemptIndexV1, ReconciliationAttemptRecordV1,
    },
    storage::control_state::{configured_control_state_db_path, load_domain_json},
};

type ApiError = (StatusCode, JsonResponse<Value>);

#[derive(Debug, Deserialize)]
pub struct RecentReconciliationAttemptsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `GET /api/v1/reconciliation-attempts/:attempt_id` -- the exact attempt
/// record, project-scoped: a request for an attempt belonging to a
/// different project is masked as 404, mirroring the established
/// cross-project masking convention used by `ddeploy`'s own proposal
/// lookup (never leaks that a differently-scoped attempt id exists).
pub async fn get_reconciliation_attempt(
    headers: HeaderMap,
    Path(reconciliation_attempt_id): Path<String>,
) -> Result<JsonResponse<ReconciliationAttemptRecordV1>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = configured_control_state_db_path().ok_or_else(|| {
        internal_error("control-state persistence is not configured".to_string())
    })?;
    let record: Option<ReconciliationAttemptRecordV1> =
        load_domain_json(&db_path, &attempt_domain(&reconciliation_attempt_id)).map_err(internal_error)?;
    match record {
        Some(record) if record.project_id == project_id => Ok(JsonResponse(record)),
        _ => Err(not_found(format!(
            "reconciliation_attempt_id '{reconciliation_attempt_id}' not found"
        ))),
    }
}

/// `GET /api/v1/reconciliation-attempts` -- the calling project's bounded
/// recent-attempts index (most-recent-first, capped at
/// `MAX_RECENT_ATTEMPTS_PER_PROJECT`; `limit` narrows further, never widens
/// past that bound). Project-scoped via `X-Datum-Project`, mirroring
/// `GET /api/v1/operational-evidence`'s own convention exactly. No full
/// historical explorer -- summaries only; fetch the exact attempt via
/// `get_reconciliation_attempt` for detail.
pub async fn list_recent_reconciliation_attempts(
    headers: HeaderMap,
    Query(query): Query<RecentReconciliationAttemptsQuery>,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = configured_control_state_db_path().ok_or_else(|| {
        internal_error("control-state persistence is not configured".to_string())
    })?;
    let index: ReconciliationAttemptIndexV1 = load_domain_json(&db_path, &attempt_index_domain(&project_id))
        .map_err(internal_error)?
        .unwrap_or_else(|| ReconciliationAttemptIndexV1::empty(&project_id));

    let mut attempts = index.attempts;
    if let Some(limit) = query.limit {
        attempts.truncate(limit.max(1));
    }

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": attempts.len(),
        "attempts": attempts,
    })))
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

fn not_found(message: String) -> ApiError {
    (StatusCode::NOT_FOUND, JsonResponse(json!({"error": message})))
}

fn internal_error(message: String) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message})))
}
