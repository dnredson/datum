//! DMonitor observation ingestion + read HTTP surface (Phase 114B1,
//! ADR-0024).
//!
//! Lock order: this domain acquires ONLY its own, newly introduced
//! `DMONITOR_TRANSACTION` -- ingestion in Phase 114B1 never reads DDeploy,
//! DNode, D-Code, D-Call, or DIoT runtime-binding state (there is no
//! drift/convergence/authority-currency check yet; see
//! `core::dmonitor_state`'s module doc), so there is no cross-domain lock
//! ordering to establish here. A future phase that correlates a DMonitor
//! observation against ACTIVE DMap/DNode authority (114C) must place
//! `DMONITOR_TRANSACTION` innermost, after `DDEPLOY_TRANSACTION`/
//! `DNODE_TRANSACTION`, consistent with the established chain
//! (`api::dcall`/`api::diot_runtime_binding`'s own module docs are the
//! authoritative statement of that chain) -- not addressed by this module.
//!
//! `received_at_utc` is generated here, from `Utc::now()`, on every accepted
//! or idempotently-replayed write. No request body field can supply or
//! override it (`IngestObservationRequest` IS
//! `canonical_dmonitor::CanonicalDMonitorObservationV1` deserialized
//! directly -- that type has no such field at all).

use std::sync::{Mutex, OnceLock};

use axum::{extract::Json, http::StatusCode, Json as JsonResponse};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};

use crate::core::canonical_dmonitor::CanonicalDMonitorObservationV1;
use crate::core::dmonitor_state::{
    ingest_observation, load_state, persist_state, DMonitorObservationRecordV1,
    IngestDMonitorObservationError, IngestOutcome,
};
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

static DMONITOR_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

pub(crate) fn dmonitor_transaction() -> &'static Mutex<()> {
    DMONITOR_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn db_path() -> std::path::PathBuf {
    default_control_state_db_path()
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("dmonitor transaction lock poisoned")
}

fn ingest_error(error: IngestDMonitorObservationError) -> ApiError {
    match error {
        IngestDMonitorObservationError::Invalid(findings) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({
                "error": "dmonitor_observation_invalid",
                "findings": findings.iter().map(|f| json!({"code": f.code, "message": f.message})).collect::<Vec<_>>(),
            })),
        ),
        IngestDMonitorObservationError::ProjectScopeMismatch { expected, actual } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({
                "error": "dmonitor_project_scope_mismatch",
                "message": format!("observation.project_id '{actual}' does not match the request project scope '{expected}'"),
            })),
        ),
        IngestDMonitorObservationError::ObservationIdConflict {
            observation_id,
            existing_stream_key,
            existing_sequence,
            attempted_stream_key,
        } => (
            StatusCode::CONFLICT,
            JsonResponse(json!({
                "error": "dmonitor_observation_id_conflict",
                "message": format!(
                    "observation_id '{observation_id}' was already accepted under stream '{existing_stream_key}' at sequence {existing_sequence}; it cannot be reused (with a different digest, stream, or sequence -- including under a different project scope) under stream '{attempted_stream_key}'"
                ),
            })),
        ),
        IngestDMonitorObservationError::SequenceConflict { stream_key, sequence } => (
            StatusCode::CONFLICT,
            JsonResponse(json!({
                "error": "dmonitor_sequence_conflict",
                "message": format!("stream '{stream_key}' sequence {sequence} was already accepted with a different canonical observation"),
            })),
        ),
        IngestDMonitorObservationError::SequenceStale { stream_key, sequence, current_head } => (
            StatusCode::CONFLICT,
            JsonResponse(json!({
                "error": "dmonitor_sequence_stale",
                "message": format!("stream '{stream_key}' sequence {sequence} is stale/out-of-order; current head is {current_head}"),
            })),
        ),
    }
}

fn not_found(message: impl Into<String>) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        JsonResponse(json!({"error": message.into()})),
    )
}

#[derive(Debug, Serialize)]
pub struct IngestObservationResponse {
    pub outcome: &'static str,
    pub record: DMonitorObservationRecordV1,
}

/// `POST /api/v1/dmonitor/observations/:project_id`. The request body IS a
/// full `datum.dmonitor-observation/1` document -- strict canonical JSON
/// (`deny_unknown_fields` on every nested type), never a looser envelope.
pub async fn post_dmonitor_observation(
    axum::extract::Path(project_id): axum::extract::Path<String>,
    Json(observation): Json<CanonicalDMonitorObservationV1>,
) -> Result<JsonResponse<IngestObservationResponse>, ApiError> {
    let _dmonitor_transaction = dmonitor_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let mut state = load_state(&db_path).map_err(internal_error)?;

    let result = ingest_observation(&mut state, &project_id, observation, &Utc::now().to_rfc3339())
        .map_err(ingest_error)?;

    if result.outcome == IngestOutcome::Accepted {
        persist_state(&db_path, &state).map_err(internal_error)?;
    }

    let outcome = match result.outcome {
        IngestOutcome::Accepted => "accepted",
        IngestOutcome::IdempotentReplay => "idempotent_replay",
    };
    Ok(JsonResponse(IngestObservationResponse {
        outcome,
        record: result.record,
    }))
}

/// `GET /api/v1/dmonitor/observations/:project_id/:node_id/:monitor_instance_id/head`.
/// Returns the CURRENT (newest accepted sequence) record for that exact
/// stream. "Current" is an ordering concept only -- this endpoint makes no
/// freshness claim about the returned record (`core::dmonitor_freshness` is
/// a separate, explicit evaluation a caller must run itself).
pub async fn get_dmonitor_stream_head(
    axum::extract::Path((project_id, node_id, monitor_instance_id)): axum::extract::Path<(
        String,
        String,
        String,
    )>,
) -> Result<JsonResponse<DMonitorObservationRecordV1>, ApiError> {
    let state = load_state(&db_path()).map_err(internal_error)?;
    state
        .stream_head(&project_id, &node_id, &monitor_instance_id)
        .cloned()
        .map(JsonResponse)
        .ok_or_else(|| {
            not_found(format!(
                "no dmonitor stream found for project '{project_id}' node '{node_id}' monitor_instance '{monitor_instance_id}'"
            ))
        })
}

/// `GET /api/v1/dmonitor/observations/:project_id/by-id/:observation_id`.
/// Project-scoped: an observation accepted under a DIFFERENT project scope
/// is reported not-found here, never leaked across the isolation boundary.
pub async fn get_dmonitor_observation_by_id(
    axum::extract::Path((project_id, observation_id)): axum::extract::Path<(String, String)>,
) -> Result<JsonResponse<DMonitorObservationRecordV1>, ApiError> {
    let state = load_state(&db_path()).map_err(internal_error)?;
    let record = state.by_observation_id(&observation_id).ok_or_else(|| {
        not_found(format!(
            "no dmonitor observation found with observation_id '{observation_id}' in project '{project_id}'"
        ))
    })?;
    if record.server_project_scope != project_id {
        return Err(not_found(format!(
            "no dmonitor observation found with observation_id '{observation_id}' in project '{project_id}'"
        )));
    }
    Ok(JsonResponse(record.clone()))
}

/// `GET /api/v1/dmonitor/observations/:project_id/streams`. The minimal read
/// surface for "current observation candidates" (section 13): EVERY
/// stream's current head, scoped to one project. Deliberately NOT filtered
/// by subject server-side -- a caller filters client-side by the typed
/// `subject` field on each returned record. This is intentionally not a
/// general historical/analytics query API: only current heads are exposed,
/// never a range or a full stream history.
pub async fn list_dmonitor_stream_heads(
    axum::extract::Path(project_id): axum::extract::Path<String>,
) -> Result<JsonResponse<Vec<DMonitorObservationRecordV1>>, ApiError> {
    let state = load_state(&db_path()).map_err(internal_error)?;
    let mut heads: Vec<DMonitorObservationRecordV1> = state
        .stream_heads_for_project(&project_id)
        .into_iter()
        .cloned()
        .collect();
    heads.sort_by(|a, b| {
        (a.observation.node_id.as_str(), a.observation.monitor_instance_id.as_str())
            .cmp(&(b.observation.node_id.as_str(), b.observation.monitor_instance_id.as_str()))
    });
    Ok(JsonResponse(heads))
}

#[cfg(test)]
mod tests {
    fn production_source() -> String {
        let source = include_str!("dmonitor.rs");
        let production_source: String = source
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(production_source.len() < source.len());
        production_source
    }

    #[test]
    fn ingestion_never_acquires_ddeploy_or_dnode_locks() {
        let source = production_source();
        assert!(
            !source.contains("ddeploy_transaction()") && !source.contains("dnode_registry::dnode_transaction()"),
            "Phase 114B1 ingestion must not correlate against DDeploy/DNode authority"
        );
    }

    #[test]
    fn post_handler_acquires_its_own_transaction_lock() {
        let source = production_source();
        let start = source.find("pub async fn post_dmonitor_observation(").unwrap();
        let body = &source[start..];
        let end = body[1..].find("\npub async fn ").map(|i| i + 1).unwrap_or(body.len());
        assert!(body[..end].contains("dmonitor_transaction()"));
    }

    /// Phase 114B1.1 closure, section 14: the `by-id` lookup must compare
    /// the resolved record's `server_project_scope` against the request's
    /// project scope and refuse a cross-project match, never merely trust
    /// that `by_observation_id` alone is sufficient isolation (it is not --
    /// `observation_id` identity is global, see `core::dmonitor_state`'s
    /// module doc section 4).
    #[test]
    fn by_id_lookup_enforces_project_scope_isolation() {
        let source = production_source();
        let start = source.find("pub async fn get_dmonitor_observation_by_id(").unwrap();
        let body = &source[start..];
        let end = body[1..].find("\npub async fn ").map(|i| i + 1).unwrap_or(body.len());
        assert!(
            body[..end].contains("record.server_project_scope != project_id"),
            "get_dmonitor_observation_by_id must explicitly compare server_project_scope against the request's project scope"
        );
    }

    /// Phase 114B1.1 closure, section 14: the mixed static
    /// (`streams`/`by-id`) and dynamic (`:node_id`/`:monitor_instance_id`)
    /// route siblings under `/api/v1/dmonitor/observations/:project_id/...`
    /// must not conflict under axum's actual router -- `Router::route`
    /// panics immediately at registration time if two patterns are
    /// ambiguous, so simply building the router here is the proof; no live
    /// server or `AppState` is needed since none of these handlers extract
    /// `State<_>`.
    #[test]
    fn dmonitor_routes_do_not_conflict_under_axum_routing() {
        use axum::routing::{get, post};
        let _router: axum::Router = axum::Router::new()
            .route(
                "/api/v1/dmonitor/observations/:project_id",
                post(super::post_dmonitor_observation),
            )
            .route(
                "/api/v1/dmonitor/observations/:project_id/streams",
                get(super::list_dmonitor_stream_heads),
            )
            .route(
                "/api/v1/dmonitor/observations/:project_id/by-id/:observation_id",
                get(super::get_dmonitor_observation_by_id),
            )
            .route(
                "/api/v1/dmonitor/observations/:project_id/:node_id/:monitor_instance_id/head",
                get(super::get_dmonitor_stream_head),
            );
    }
}
