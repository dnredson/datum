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

use crate::{
    core::{
        operational_evidence::{validate_operational_evidence_event, OperationalEvidenceEvent},
        reconciliation_attempt::{
            attempt_domain, attempt_index_domain, attempt_summary, fold_evidence_into_attempt,
            EvidenceForFold, ReconciliationAttemptRecordV1,
        },
    },
    storage::{
        control_state::{configured_control_state_db_path, load_domain_json, persist_domain_json},
        operational_evidence::{
            append_operational_evidence, load_operational_evidence,
            AppendOperationalEvidenceOutcome, OperationalEvidenceFilter,
        },
    },
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
    pub dgraph_revision: Option<u64>,
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

    if let Some(db_path) = configured_control_state_db_path() {
        match append_operational_evidence(&db_path, &payload).map_err(internal_error)? {
            AppendOperationalEvidenceOutcome::Inserted => {
                // Phase 138A (Part C6): fold into the DMap-native
                // reconciliation-attempt read model ONLY when this event
                // actually carries one -- every pre-138A/non-reconciliation
                // event kind is completely unaffected. Folding failure
                // (e.g. a stale/out-of-order sequence) is logged, never
                // surfaced as a failure of the evidence POST itself: the
                // evidence event is already durably accepted at this point,
                // and the attempt projection is a best-effort derived view
                // over it, not the source of truth.
                if let Some(reconciliation_attempt_id) = payload.reconciliation_attempt_id.as_deref() {
                    if let Err(error) = fold_reconciliation_attempt(&db_path, reconciliation_attempt_id, &payload) {
                        eprintln!(
                            "operational-evidence: reconciliation_attempt fold failed for '{reconciliation_attempt_id}' (event_id={}): {error}",
                            payload.event_id
                        );
                    }
                }
                return Ok(JsonResponse(payload));
            }
            AppendOperationalEvidenceOutcome::Duplicate => {
                return Err(conflict(format!(
                    "operational evidence event_id '{}' already exists",
                    payload.event_id
                )))
            }
        }
    }

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

    let events = if let Some(db_path) = configured_control_state_db_path() {
        load_operational_evidence(
            &db_path,
            OperationalEvidenceFilter {
                project_id: &project_id,
                resource_id: query.resource_id.as_deref(),
                node_id: query.node_id.as_deref(),
                dgraph_id: query.dgraph_id.as_deref(),
                dgraph_revision: query.dgraph_revision,
                event_kind: query.event_kind.as_deref(),
                limit: limit as u32,
            },
        )
        .map_err(internal_error)?
    } else {
        let evidence = store().read().map_err(lock_error)?;
        let mut events = evidence
            .get(&project_id)
            .into_iter()
            .flat_map(|events| events.iter())
            .filter(|event| {
                matches_optional(query.resource_id.as_deref(), Some(event.resource_id.as_str()))
                    && matches_optional(query.node_id.as_deref(), event.node_id.as_deref())
                    && matches_optional(query.dgraph_id.as_deref(), event.dgraph_id.as_deref())
                    && query
                        .dgraph_revision
                        .map(|revision| event.dgraph_revision == Some(revision))
                        .unwrap_or(true)
                    && matches_optional(query.event_kind.as_deref(), Some(event.event_kind.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();

        if events.len() > limit {
            events = events.split_off(events.len() - limit);
        }
        events
    };

    Ok(JsonResponse(json!({
        "project_id": project_id,
        "count": events.len(),
        "events": events,
    })))
}

/// Phase 138A (Part C6): fold one just-accepted evidence event into its
/// `reconciliation_attempt_id`'s durable record, then refresh the project's
/// bounded recent-attempts index. Both reads/writes go through the SAME
/// `control_state` SQLite domain store `core::ddeploy`/`operational_dgraph`
/// already use (`persist_domain_json`/`load_domain_json`) -- no new
/// persistence mechanism, no Firebase/Firestore, no legacy
/// `deployment_manifest`/`deployment_assignment` involvement.
fn fold_reconciliation_attempt(
    db_path: &std::path::Path,
    reconciliation_attempt_id: &str,
    event: &OperationalEvidenceEvent,
) -> Result<(), String> {
    let node_id = event
        .node_id
        .as_deref()
        .ok_or_else(|| "reconciliation evidence event missing node_id".to_string())?;
    let dgraph_id = event
        .dgraph_id
        .as_deref()
        .ok_or_else(|| "reconciliation evidence event missing dgraph_id".to_string())?;

    // Best-effort correlation field: read directly from the already-parsed
    // payload (the one HTTP request this handler has in hand) rather than
    // round-tripping through SQLite storage, since `operational_evidence`'s
    // own table does not persist `reconciliation_attempt_id` as a separate
    // queryable column today (Phase 138A scope: this fold reads the posted
    // payload directly, never relies on re-loading it). `None` when the
    // payload does not carry a recognizable value -- never fabricated.
    //
    // Independent review (Audit 16): `requested_by`/`application_id` were
    // removed from this extraction entirely -- neither
    // `OperationalReconciliationReport` nor `OperationalServiceReconciliationReport`
    // (the only shapes the agent actually serializes into this payload)
    // carries either field today, so both extractions always resolved to
    // `None` in practice; `application_id` specifically would additionally
    // have been semantically wrong had it ever resolved to a value, since
    // one node-scoped cycle can span multiple applications and taking the
    // first service's id would misrepresent that. `authorization_id` is
    // safe to take from any one service because exactly one authorization
    // covers the WHOLE cycle (see `ReconciliationAttemptRecordV1::authorization_id`'s
    // own doc comment) -- it is not an arbitrary per-service choice.
    let authorization_id = event
        .payload
        .get("authorization_id")
        .and_then(|value| value.as_str())
        .or_else(|| {
            event
                .payload
                .get("services")
                .and_then(|value| value.as_array())
                .and_then(|services| services.first())
                .and_then(|service| service.get("authorization_id"))
                .and_then(|value| value.as_str())
        });

    let now_utc = chrono::Utc::now().to_rfc3339();
    let existing: Option<ReconciliationAttemptRecordV1> =
        load_domain_json(db_path, &attempt_domain(reconciliation_attempt_id))?;

    let record = fold_evidence_into_attempt(
        existing,
        reconciliation_attempt_id,
        authorization_id,
        &now_utc,
        &EvidenceForFold {
            project_id: &event.project_id,
            node_id,
            dgraph_id,
            dgraph_revision: event.dgraph_revision,
            sequence: event.sequence,
            occurred_at_utc: &event.occurred_at_utc,
            side_effects_performed: event.side_effects_performed,
            payload: &event.payload,
        },
    )?;

    persist_domain_json(db_path, &attempt_domain(reconciliation_attempt_id), &record)?;

    let mut index = load_domain_json(db_path, &attempt_index_domain(&event.project_id))?
        .unwrap_or_else(|| crate::core::reconciliation_attempt::ReconciliationAttemptIndexV1::empty(&event.project_id));
    index.upsert(attempt_summary(&record));
    persist_domain_json(db_path, &attempt_index_domain(&event.project_id), &index)?;

    Ok(())
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

fn internal_error(message: String) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message})),
    )
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error":"operational evidence store lock poisoned"})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::reconciliation_attempt::{
        ReconciliationActionResult, ReconciliationAttemptStatus,
    };

    fn temp_db_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-reconciliation-attempt-fold-test-{}.db", uuid::Uuid::new_v4()))
    }

    fn reconciliation_event(reconciliation_attempt_id: &str, sequence: u64, payload: Value) -> OperationalEvidenceEvent {
        OperationalEvidenceEvent {
            schema_version: crate::core::operational_evidence::OPERATIONAL_EVIDENCE_SCHEMA_VERSION.to_string(),
            event_id: format!("opev:{}", uuid::Uuid::new_v4()),
            occurred_at_utc: "2026-10-05T12:00:00Z".to_string(),
            project_id: "project-a".to_string(),
            resource_id: "resource:os:abc".to_string(),
            node_id: Some("node-a".to_string()),
            dgraph_id: Some("dgraph:app-1".to_string()),
            dgraph_revision: Some(4),
            sequence,
            event_kind: "reconciliation".to_string(),
            side_effects_performed: true,
            payload,
            reconciliation_attempt_id: Some(reconciliation_attempt_id.to_string()),
        }
    }

    // Phase 138A (Part C6/K): exercises the REAL SQLite round-trip
    // (`persist_domain_json`/`load_domain_json`) this handler uses, not
    // just the pure `fold_evidence_into_attempt` function -- proves the
    // handler-level wiring (attempt id extraction, authorization_id/
    // application_id best-effort extraction, index upsert) actually works,
    // without depending on the process-global `CONTROL_STATE_DB_PATH_ENV`
    // env var (unsafe under parallel test execution) -- `fold_reconciliation_attempt`
    // already takes an explicit `db_path`, so this test supplies its own
    // temp file directly, exactly like `storage::operational_evidence`'s
    // own test convention.
    #[test]
    fn fold_reconciliation_attempt_persists_a_record_readable_via_the_read_api_helpers() {
        let db_path = temp_db_path();
        let payload = json!({
            "authorization_id": "opauth:1",
            "services": [
                {"service_id": "service-a", "host_mutation_performed": true, "execution_status": "started"}
            ]
        });
        let event = reconciliation_event("reconciliation-attempt:fold-1", 1, payload);

        fold_reconciliation_attempt(&db_path, "reconciliation-attempt:fold-1", &event).unwrap();

        let record: ReconciliationAttemptRecordV1 =
            load_domain_json(&db_path, &attempt_domain("reconciliation-attempt:fold-1")).unwrap().unwrap();
        assert_eq!(record.project_id, "project-a");
        assert_eq!(record.node_id, "node-a");
        assert_eq!(record.dgraph_id, "dgraph:app-1");
        assert_eq!(record.dgraph_revision, Some(4));
        assert_eq!(record.authorization_id.as_deref(), Some("opauth:1"));
        assert_eq!(record.status, ReconciliationAttemptStatus::Succeeded);
        assert_eq!(record.services["service-a"].result, ReconciliationActionResult::Mutated);

        let index: crate::core::reconciliation_attempt::ReconciliationAttemptIndexV1 =
            load_domain_json(&db_path, &attempt_index_domain("project-a")).unwrap().unwrap();
        assert_eq!(index.attempts.len(), 1);
        assert_eq!(index.attempts[0].reconciliation_attempt_id, "reconciliation-attempt:fold-1");
    }

    #[test]
    fn fold_reconciliation_attempt_rejects_a_duplicate_sequence_and_leaves_the_record_unchanged() {
        let db_path = temp_db_path();
        let payload = json!({"services": [{"service_id": "service-a", "host_mutation_performed": true, "execution_status": "started"}]});
        let event = reconciliation_event("reconciliation-attempt:fold-2", 1, payload.clone());
        fold_reconciliation_attempt(&db_path, "reconciliation-attempt:fold-2", &event).unwrap();

        let replay = reconciliation_event("reconciliation-attempt:fold-2", 1, payload);
        let error = fold_reconciliation_attempt(&db_path, "reconciliation-attempt:fold-2", &replay).unwrap_err();
        assert!(error.contains("stale or duplicate"), "{error}");

        let record: ReconciliationAttemptRecordV1 =
            load_domain_json(&db_path, &attempt_domain("reconciliation-attempt:fold-2")).unwrap().unwrap();
        assert_eq!(record.last_applied_sequence, 1);
    }

    #[test]
    fn fold_reconciliation_attempt_requires_node_id_and_dgraph_id() {
        let db_path = temp_db_path();
        let mut event = reconciliation_event("reconciliation-attempt:fold-3", 1, json!({}));
        event.node_id = None;
        let error = fold_reconciliation_attempt(&db_path, "reconciliation-attempt:fold-3", &event).unwrap_err();
        assert!(error.contains("node_id"), "{error}");
    }
}
