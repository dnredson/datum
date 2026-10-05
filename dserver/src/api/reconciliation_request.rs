//! Phase 138C: the control-plane trigger API. `dserver` persists request
//! and claim state only -- it never executes anything. The node-resident
//! DATUM agent remains the sole thing that performs host mutation,
//! discovering a claimable request through the EXISTING control-slice poll
//! (`pending_request`, wired in `core::operational_dgraph`) and claiming it
//! explicitly through this module's claim endpoint.

use axum::{
    extract::{Json, Path, Query},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    api::operational_dgraph::operational_dgraph_slice_snapshot,
    core::{
        reconciliation_attempt::ReconciliationAttemptRecordV1,
        reconciliation_request::{
            default_claim_expires_at, default_expires_at, derive_batch_status, validate_mode,
            ReconciliationRequestV1, ReconciliationRequestStatus, MAX_NODES_PER_BATCH,
            MAX_PENDING_REQUESTS_PER_NODE, RECONCILIATION_REQUEST_SCHEMA_VERSION,
        },
    },
    storage::{
        control_state::{configured_control_state_db_path, load_domain_json},
        reconciliation_request::{
            claim_request, create_request, get_request, list_for_batch, list_recent_for_project, mark_stale,
            ClaimOutcome, CreateRequestOutcome,
        },
    },
};

type ApiError = (StatusCode, JsonResponse<Value>);

#[derive(Debug, Deserialize)]
pub struct CreateReconciliationRequestPayload {
    pub node_id: String,
    pub target_dgraph_id: String,
    pub target_dgraph_revision: u64,
    pub mode: String,
    pub requested_by: String,
    pub client_request_id: String,
    #[serde(default)]
    pub reconciliation_batch_id: Option<String>,
}

/// `POST /api/v1/reconciliation-requests` (Part C). Validates the target
/// against LIVE operational authority before persisting (Part C1) --
/// `dserver` already has this data locally (the same source
/// `core::operational_dgraph` serves to the agent), so no node round trip
/// is needed to reject a request pointed at an already-stale target.
pub async fn post_reconciliation_request(
    headers: HeaderMap,
    Json(payload): Json<CreateReconciliationRequestPayload>,
) -> Result<JsonResponse<ReconciliationRequestV1>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let request = build_and_validate_request(&project_id, &payload, None).map_err(bad_request)?;

    let db_path = control_state_db_path()?;
    let (outcome, stored) = create_request(&db_path, &request).map_err(internal_error)?;
    if outcome == CreateRequestOutcome::NodeQueueFull {
        // Independent review (Audit 43): fail explicitly rather than
        // silently dropping or ever accepting unbounded pending work for
        // one node.
        return Err(conflict(format!(
            "node '{}' already has the maximum of {MAX_PENDING_REQUESTS_PER_NODE} pending/claimed reconciliation requests; wait for one to resolve before creating another",
            payload.node_id
        )));
    }
    if outcome == CreateRequestOutcome::AlreadyExists
        && (stored.node_id != request.node_id
            || stored.target_dgraph_id != request.target_dgraph_id
            || stored.target_dgraph_revision != request.target_dgraph_revision
            || stored.mode != request.mode)
    {
        return Err(conflict(format!(
            "client_request_id '{}' was already used for a different node/target/mode -- reuse requires an identical request",
            payload.client_request_id
        )));
    }
    Ok(JsonResponse(stored))
}

fn build_and_validate_request(
    project_id: &str,
    payload: &CreateReconciliationRequestPayload,
    reconciliation_batch_id: Option<String>,
) -> Result<ReconciliationRequestV1, String> {
    validate_mode(&payload.mode)?;
    required_bounded("node_id", &payload.node_id)?;
    required_bounded("requested_by", &payload.requested_by)?;
    required_bounded("client_request_id", &payload.client_request_id)?;
    required_bounded("target_dgraph_id", &payload.target_dgraph_id)?;

    let slice = operational_dgraph_slice_snapshot(project_id, &payload.node_id)
        .map_err(|error| format!("read live operational authority: {error}"))?
        .ok_or_else(|| format!("node '{}' is not a known resource for this project", payload.node_id))?;
    if slice.dgraph_id != payload.target_dgraph_id || slice.dgraph_revision != payload.target_dgraph_revision {
        return Err(format!(
            "target {}@{} is already stale -- live authority is {}@{}; refusing to create a request pinned to a superseded target",
            payload.target_dgraph_id, payload.target_dgraph_revision, slice.dgraph_id, slice.dgraph_revision
        ));
    }

    let now = Utc::now();
    let now_utc = now.to_rfc3339();
    Ok(ReconciliationRequestV1 {
        schema_version: RECONCILIATION_REQUEST_SCHEMA_VERSION.to_string(),
        reconciliation_request_id: format!("reconciliation-request:{}", uuid::Uuid::new_v4()),
        client_request_id: payload.client_request_id.clone(),
        reconciliation_batch_id: reconciliation_batch_id.or_else(|| payload.reconciliation_batch_id.clone()),
        project_id: project_id.to_string(),
        node_id: payload.node_id.clone(),
        target_dgraph_id: payload.target_dgraph_id.clone(),
        target_dgraph_revision: payload.target_dgraph_revision,
        mode: payload.mode.clone(),
        requested_by: payload.requested_by.clone(),
        created_at_utc: now_utc.clone(),
        expires_at_utc: default_expires_at(now),
        status: ReconciliationRequestStatus::Pending,
        claim_token: None,
        claimed_at_utc: None,
        claim_expires_at_utc: None,
        linked_attempt_id: None,
        updated_at_utc: now_utc,
    })
}

#[derive(Debug, Deserialize)]
pub struct CreateReconciliationBatchPayload {
    pub application_id: String,
    pub mode: String,
    pub requested_by: String,
    pub client_request_id: String,
}

/// `POST /api/v1/reconciliation-requests/batch` (Part I1). Resolves every
/// node the given application currently has accepted placements on from
/// LIVE operational authority (server-owned projection -- the browser
/// never supplies or derives the node list, Part I1's own explicit
/// requirement) and creates one node-scoped request per node, all sharing
/// one `reconciliation_batch_id`. Each member's OWN idempotency key is
/// derived deterministically from the caller's `client_request_id` plus
/// the node id, so retrying the exact same batch POST (e.g. after a lost
/// response) re-resolves to the SAME member requests rather than creating
/// duplicates (Part O's double-click protection, extended to the whole
/// batch).
///
/// Independent review (Audit 35/36) -- IMPORTANT SCOPE NOTE: `application_id`
/// here is used ONLY to resolve which NODES get a request; the resulting
/// `ReconciliationRequestV1` members are each scoped to their node's WHOLE
/// operational D-Graph revision (`target_dgraph_id`/`target_dgraph_revision`
/// is the project's operational graph identity, not a per-application
/// slice), exactly like every other reconciliation request. This means a
/// node hosting placements from MULTIPLE applications will have its agent
/// reconcile (and potentially mutate) ALL of those applications' services
/// when this batch's resulting request is claimed and run -- not only the
/// one named by `application_id`. This is not a regression introduced by
/// this endpoint: the underlying agent reconciliation cycle has always been
/// node-scoped, never application-scoped (see
/// `reconciliation_attempt.rs`'s own "Attempt scope" doc comment). This
/// endpoint deliberately does NOT add a per-application filter to agent
/// reconciliation -- doing so is explicit future scope, not something to
/// silently imply here. Any future Console surface built on this endpoint
/// MUST present "Reconcile application X" as "reconcile node(s) hosting
/// application X" (i.e. node-wide), never as an application-isolated action.
pub async fn post_reconciliation_batch(
    headers: HeaderMap,
    Json(payload): Json<CreateReconciliationBatchPayload>,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    validate_mode(&payload.mode).map_err(bad_request)?;
    required_bounded("application_id", &payload.application_id).map_err(bad_request)?;
    required_bounded("requested_by", &payload.requested_by).map_err(bad_request)?;
    required_bounded("client_request_id", &payload.client_request_id).map_err(bad_request)?;

    let graph = crate::api::operational_dgraph::operational_dgraph_snapshot(&project_id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found("no operational authority exists yet for this project".to_string()))?;

    let mut node_ids: Vec<String> = graph
        .nodes
        .iter()
        .filter(|node| node.desired_services.iter().any(|service| service.application_id == payload.application_id))
        .map(|node| node.node_id.clone())
        .collect();
    node_ids.sort();

    if node_ids.is_empty() {
        return Err(not_found(format!(
            "application '{}' has no accepted placements on any node in this project's current operational authority",
            payload.application_id
        )));
    }
    if node_ids.len() > MAX_NODES_PER_BATCH {
        return Err(bad_request(format!(
            "application '{}' spans {} nodes, exceeding the maximum of {MAX_NODES_PER_BATCH} nodes per batch",
            payload.application_id,
            node_ids.len()
        )));
    }

    let reconciliation_batch_id = format!("reconciliation-batch:{}", uuid::Uuid::new_v4());
    let db_path = control_state_db_path()?;
    let mut created = Vec::with_capacity(node_ids.len());
    for node_id in &node_ids {
        let member_payload = CreateReconciliationRequestPayload {
            node_id: node_id.clone(),
            target_dgraph_id: graph.dgraph_id.clone(),
            target_dgraph_revision: graph.revision,
            mode: payload.mode.clone(),
            requested_by: payload.requested_by.clone(),
            client_request_id: format!("{}:{}", payload.client_request_id, node_id),
            reconciliation_batch_id: None,
        };
        let request = build_and_validate_request(&project_id, &member_payload, Some(reconciliation_batch_id.clone())).map_err(bad_request)?;
        let (member_outcome, stored) = create_request(&db_path, &request).map_err(internal_error)?;
        if member_outcome == CreateRequestOutcome::NodeQueueFull {
            // Independent review (Audit 37/43): never silently include a
            // phantom (never-persisted) member in a reported-successful
            // batch -- fail the whole batch creation explicitly. Members
            // already created for earlier nodes in this loop remain
            // persisted (Audit 37's own documented, accepted non-atomicity:
            // each member's idempotent `client_request_id` makes a retry of
            // the identical batch POST safe and resumes cleanly).
            return Err(conflict(format!(
                "node '{node_id}' already has the maximum of {MAX_PENDING_REQUESTS_PER_NODE} pending/claimed reconciliation requests; batch creation stopped"
            )));
        }
        created.push(stored);
    }

    Ok(JsonResponse(json!({
        "reconciliation_batch_id": reconciliation_batch_id,
        "application_id": payload.application_id,
        "requests": created,
    })))
}

/// `POST /api/v1/nodes/:node_id/reconciliation-requests/:request_id/claim`
/// (Part D). The ONE real atomic compare-and-swap in this foundation.
/// Re-validates the target is still live-current BEFORE attempting the
/// claim (Part D4) -- a stale request is marked `stale` and refused,
/// never silently claimed against a superseded target. Claiming does NOT
/// check or grant `OperationalReconciliationAuthorization` (Part J) --
/// that remains an entirely separate, unchanged gate the agent's own
/// Phase 138A mutation path still enforces at attempt time.
pub async fn post_claim_reconciliation_request(
    headers: HeaderMap,
    Path((node_id, reconciliation_request_id)): Path<(String, String)>,
) -> Result<JsonResponse<ReconciliationRequestV1>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = control_state_db_path()?;

    let existing = get_request(&db_path, &project_id, &reconciliation_request_id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found(format!("reconciliation_request_id '{reconciliation_request_id}' not found")))?;
    if existing.node_id != node_id {
        // Part Q: a request for node-a must never be claimable via node-b's
        // own route -- masked identically to "not found" rather than
        // leaking that the request exists for a different node.
        return Err(not_found(format!("reconciliation_request_id '{reconciliation_request_id}' not found")));
    }

    let now = Utc::now();
    let now_utc = now.to_rfc3339();
    let slice = operational_dgraph_slice_snapshot(&project_id, &node_id).map_err(internal_error)?;
    let still_current = slice.as_ref().is_some_and(|slice| {
        slice.dgraph_id == existing.target_dgraph_id && slice.dgraph_revision == existing.target_dgraph_revision
    });
    if !still_current {
        mark_stale(&db_path, &project_id, &reconciliation_request_id, &now_utc).map_err(internal_error)?;
        return Err(conflict(format!(
            "reconciliation_request_id '{reconciliation_request_id}' target is no longer current authority; refusing claim"
        )));
    }

    let claim_token = format!("claim:{}", uuid::Uuid::new_v4());
    let claim_expires_at_utc = default_claim_expires_at(now);
    let outcome = claim_request(&db_path, &project_id, &node_id, &reconciliation_request_id, &claim_token, &now_utc, &claim_expires_at_utc, &now_utc)
        .map_err(internal_error)?;
    match outcome {
        ClaimOutcome::Claimed => {
            let claimed = get_request(&db_path, &project_id, &reconciliation_request_id)
                .map_err(internal_error)?
                .ok_or_else(|| internal_error("claimed request vanished immediately after claim".to_string()))?;
            Ok(JsonResponse(claimed))
        }
        ClaimOutcome::Conflict => Err(conflict(format!(
            "reconciliation_request_id '{reconciliation_request_id}' could not be claimed (already claimed, terminal, or expired)"
        ))),
    }
}

pub async fn get_reconciliation_request(
    headers: HeaderMap,
    Path(reconciliation_request_id): Path<String>,
) -> Result<JsonResponse<ReconciliationRequestV1>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = control_state_db_path()?;
    get_request(&db_path, &project_id, &reconciliation_request_id)
        .map_err(internal_error)?
        .map(JsonResponse)
        .ok_or_else(|| not_found(format!("reconciliation_request_id '{reconciliation_request_id}' not found")))
}

#[derive(Debug, Deserialize)]
pub struct RecentReconciliationRequestsQuery {
    #[serde(default)]
    pub limit: Option<usize>,
}

pub async fn list_recent_reconciliation_requests(
    headers: HeaderMap,
    Query(query): Query<RecentReconciliationRequestsQuery>,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = control_state_db_path()?;
    let requests = list_recent_for_project(&db_path, &project_id, query.limit.unwrap_or(25)).map_err(internal_error)?;
    Ok(JsonResponse(json!({"project_id": project_id, "count": requests.len(), "requests": requests})))
}

/// `GET /api/v1/reconciliation-requests/batch/:batch_id` (Part I2). The
/// server-computed aggregate -- Console must never re-derive this itself
/// from the member requests (Part H's own "avoid forcing Console to
/// recreate claim/staleness logic" instruction).
pub async fn get_reconciliation_batch(
    headers: HeaderMap,
    Path(reconciliation_batch_id): Path<String>,
) -> Result<JsonResponse<Value>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = control_state_db_path()?;
    let members = list_for_batch(&db_path, &project_id, &reconciliation_batch_id).map_err(internal_error)?;
    if members.is_empty() {
        return Err(not_found(format!("reconciliation_batch_id '{reconciliation_batch_id}' not found")));
    }

    let mut attempt_statuses = std::collections::BTreeMap::new();
    for member in &members {
        if let Some(attempt_id) = &member.linked_attempt_id {
            if let Ok(Some(record)) = load_domain_json::<ReconciliationAttemptRecordV1>(
                &db_path,
                &crate::core::reconciliation_attempt::attempt_domain(attempt_id),
            ) {
                attempt_statuses.insert(attempt_id.clone(), record.status);
            }
        }
    }
    let status = derive_batch_status(&members, &attempt_statuses, Utc::now());

    Ok(JsonResponse(json!({
        "reconciliation_batch_id": reconciliation_batch_id,
        "project_id": project_id,
        "status": status,
        "requests": members,
    })))
}

fn required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{label} must not be empty"))
    } else {
        Ok(())
    }
}

/// Independent review (Audit 49): bounds an opaque caller-supplied
/// identifier (`client_request_id`, `requested_by`, `node_id`,
/// `target_dgraph_id`, `application_id`) the same way this crate already
/// bounds other opaque ids (e.g. `api::dgraph_editor`'s own `id.len() > 256
/// || id.trim() != id || id.chars().any(char::is_control)` convention) --
/// never empty, never multi-megabyte, never carrying control characters
/// that would corrupt logs/JSON round-tripping.
const MAX_OPAQUE_ID_LEN: usize = 256;

fn required_bounded(label: &str, value: &str) -> Result<(), String> {
    required(label, value)?;
    if value.len() > MAX_OPAQUE_ID_LEN {
        return Err(format!("{label} must not exceed {MAX_OPAQUE_ID_LEN} bytes"));
    }
    if value.trim() != value {
        return Err(format!("{label} must not have leading or trailing whitespace"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!("{label} must not contain control characters"));
    }
    Ok(())
}

fn control_state_db_path() -> Result<std::path::PathBuf, ApiError> {
    configured_control_state_db_path().ok_or_else(|| internal_error("control-state persistence is not configured".to_string()))
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
fn conflict(message: String) -> ApiError {
    (StatusCode::CONFLICT, JsonResponse(json!({"error": message})))
}
fn internal_error(message: String) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message})))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Phase 138C independent review (Audit 49): bounds on opaque
    // caller-supplied identifiers.
    #[test]
    fn required_bounded_rejects_empty_oversized_and_control_character_values() {
        assert!(required_bounded("client_request_id", "client:1").is_ok());
        assert!(required_bounded("client_request_id", "").is_err());
        assert!(required_bounded("client_request_id", "   ").is_err());
        assert!(required_bounded("client_request_id", &"x".repeat(MAX_OPAQUE_ID_LEN)).is_ok());
        assert!(required_bounded("client_request_id", &"x".repeat(MAX_OPAQUE_ID_LEN + 1)).is_err());
        assert!(required_bounded("client_request_id", " client:1").is_err(), "leading whitespace must be rejected");
        assert!(required_bounded("client_request_id", "client:1 ").is_err(), "trailing whitespace must be rejected");
        assert!(required_bounded("client_request_id", "client:1\u{0007}").is_err(), "control characters must be rejected");
        assert!(required_bounded("client_request_id", "client:1\n").is_err(), "newlines must be rejected");
    }

    // Phase 138C (Part P): structural regression proof that dserver's
    // reconciliation-request foundation (core + storage + this API module)
    // never pushes anything to a node -- it only persists and serves
    // request/claim state. The node-resident DATUM agent remains the sole
    // thing that ever makes an outbound call to perform host mutation.
    // Mirrors the established source-text-scan pattern used elsewhere in
    // this crate (e.g. `dforward_dependency_readiness.rs`'s
    // `handler_never_calls_dmonitor_readiness_over_http`,
    // `reconciliation_attempt.rs`'s own
    // `source_has_no_code_dependency_on_ddeploy_dmonitor_or_legacy_deployment_lineage`).
    // Scans implementation code only (comments stripped, this file's own
    // `#[cfg(test)]` boundary excluded) across all three Phase 138C files,
    // since the forbidden words below legitimately appear in this very
    // test's own source.
    #[test]
    fn phase138c_reconciliation_request_foundation_never_makes_an_outbound_call_to_a_node() {
        let sources = [
            include_str!("reconciliation_request.rs"),
            include_str!("../core/reconciliation_request.rs"),
            include_str!("../storage/reconciliation_request.rs"),
        ];
        for source in sources {
            let implementation_only = source.split("#[cfg(test)]").next().unwrap_or(source);
            let implementation: String = implementation_only
                .lines()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    !trimmed.starts_with("//") && !trimmed.starts_with("///") && !trimmed.starts_with("//!")
                })
                .collect::<Vec<_>>()
                .join("\n");
            for forbidden in ["reqwest", "Client::new", "Client::builder", "ssh2", "std::process::Command"] {
                assert!(
                    !implementation.contains(forbidden),
                    "Phase 138C reconciliation-request foundation's actual code (comments excluded) must never reference '{forbidden}' -- dserver never pushes to a node"
                );
            }
        }
    }
}
