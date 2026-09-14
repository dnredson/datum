//! D-Node/D-Continuum authority foundation closure (ADR-0019) HTTP surface:
//! declare/list/remove a structural D-Node, and fetch the current
//! authoritative `datum.dcontinuum/1` for a project. See
//! `core::dnode_registry` for the full rationale and the D-Deploy authority
//! transition this enables (`api::ddeploy::authoritative_dcontinuum_for_project`).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use axum::{
    extract::{Json, Path},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use serde_json::{json, Value};

use crate::core::canonical_dcontinuum::{CanonicalDContinuumV1, CanonicalDNodeV1};
use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::dnode_registry::{
    build_canonical_dcontinuum, declare_dnode, load_state, persist_state, remove_dnode, DeclareOutcome,
};
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

static DNODE_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

/// `pub(crate)` (foundation closure v2, ADR-0019 R1): `api::ddeploy`'s
/// acceptance handler also acquires this same lock, nested INSIDE its own
/// `DDEPLOY_TRANSACTION`, for the entire duration of a pending proposal's
/// acceptance -- this is what makes the acceptance-time D-Node authority
/// revalidation TOCTOU-safe (a concurrent `put_dnode`/`delete_dnode` cannot
/// commit while an acceptance is deciding whether the proposal's structural
/// authority is still current). Fixed lock order, never reversed: every
/// caller in this crate acquires `DDEPLOY_TRANSACTION` (if at all) BEFORE
/// this lock, never after -- `put_dnode`/`delete_dnode` below acquire ONLY
/// this lock and never touch `DDEPLOY_TRANSACTION`, so there is no
/// reverse-order acquisition path and therefore no deadlock.
pub(crate) fn dnode_transaction() -> &'static Mutex<()> {
    DNODE_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn dnode_db_path() -> PathBuf {
    default_control_state_db_path()
}

fn extract_project_id(headers: &HeaderMap) -> Result<String, ApiError> {
    let project_id = headers
        .get("x-datum-project")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string());
    match project_id {
        Some(project_id) if !project_id.is_empty() => Ok(project_id),
        _ => Err(bad_request("missing or invalid X-Datum-Project header")),
    }
}

fn bad_request(message: impl Into<String>) -> ApiError {
    (StatusCode::BAD_REQUEST, JsonResponse(json!({"error": message.into()})))
}

fn unprocessable(findings: &[CanonicalValidationFinding]) -> ApiError {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        JsonResponse(json!({"error": "dnode_declaration_invalid", "findings": findings_json(findings)})),
    )
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message.into()})))
}

fn persistence_failure(message: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": "persistence_conflict", "message": message.into()})))
}

fn findings_json(findings: &[CanonicalValidationFinding]) -> Vec<Value> {
    findings.iter().map(|finding| json!({"code": finding.code, "message": finding.message})).collect()
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("dnode registry transaction lock poisoned")
}

#[derive(Debug, serde::Serialize)]
pub struct DNodeDeclareResponse {
    pub node: CanonicalDNodeV1,
    pub inserted: bool,
    pub registry_revision: u64,
}

/// Declares (or idempotently re-declares) one structural D-Node. Path
/// `node_id` must match the payload's own `node_id` -- mirrors
/// `put_sentinel_registration`'s identical path/body consistency check.
pub async fn put_dnode(
    Path(node_id): Path<String>,
    Json(payload): Json<CanonicalDNodeV1>,
) -> Result<JsonResponse<DNodeDeclareResponse>, ApiError> {
    let path_node = node_id.trim();
    if path_node != payload.node_id.trim() {
        return Err(bad_request(format!(
            "declaration path node_id '{}' does not match payload node_id '{}'",
            node_id, payload.node_id
        )));
    }

    let _transaction = dnode_transaction().lock().map_err(lock_error)?;
    let db_path = dnode_db_path();
    let mut state = load_state(&db_path).map_err(internal_error)?;
    let outcome = declare_dnode(&mut state, payload).map_err(|findings| unprocessable(&findings))?;
    if matches!(outcome, DeclareOutcome::Inserted) {
        persist_state(&db_path, &state).map_err(persistence_failure)?;
    }

    let node = state.nodes.get(path_node).cloned().expect("just declared or already present");
    Ok(JsonResponse(DNodeDeclareResponse {
        node,
        inserted: matches!(outcome, DeclareOutcome::Inserted),
        registry_revision: state.revision,
    }))
}

pub async fn get_dnode(Path(node_id): Path<String>) -> Result<JsonResponse<CanonicalDNodeV1>, ApiError> {
    let db_path = dnode_db_path();
    let state = load_state(&db_path).map_err(internal_error)?;
    state
        .nodes
        .get(node_id.trim())
        .cloned()
        .map(JsonResponse)
        .ok_or_else(|| (StatusCode::NOT_FOUND, JsonResponse(json!({"error": format!("D-Node '{}' is not declared", node_id)}))))
}

#[derive(Debug, serde::Serialize)]
pub struct DNodeListResponse {
    pub count: usize,
    pub registry_revision: u64,
    pub nodes: Vec<CanonicalDNodeV1>,
}

pub async fn list_dnodes() -> Result<JsonResponse<DNodeListResponse>, ApiError> {
    let db_path = dnode_db_path();
    let state = load_state(&db_path).map_err(internal_error)?;
    let nodes: Vec<CanonicalDNodeV1> = state.nodes.values().cloned().collect();
    Ok(JsonResponse(DNodeListResponse { count: nodes.len(), registry_revision: state.revision, nodes }))
}

#[derive(Debug, serde::Serialize)]
pub struct DNodeRemoveResponse {
    pub removed: bool,
    pub registry_revision: u64,
}

pub async fn delete_dnode(Path(node_id): Path<String>) -> Result<JsonResponse<DNodeRemoveResponse>, ApiError> {
    let _transaction = dnode_transaction().lock().map_err(lock_error)?;
    let db_path = dnode_db_path();
    let mut state = load_state(&db_path).map_err(internal_error)?;
    let removed = remove_dnode(&mut state, node_id.trim());
    if removed {
        persist_state(&db_path, &state).map_err(persistence_failure)?;
    }
    Ok(JsonResponse(DNodeRemoveResponse { removed, registry_revision: state.revision }))
}

/// The authoritative `datum.dcontinuum/1` for `X-Datum-Project`, derived
/// entirely from the server-owned structural registry. This is the exact
/// object canonical D-Deploy now verifies proposal input against
/// (`api::ddeploy::authoritative_dcontinuum_for_project`) -- fetching it
/// here first is the intended client workflow for constructing a proposal
/// that will pass the authority check.
pub async fn get_authoritative_dcontinuum(headers: HeaderMap) -> Result<JsonResponse<CanonicalDContinuumV1>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = dnode_db_path();
    let state = load_state(&db_path).map_err(internal_error)?;
    let dcontinuum = build_canonical_dcontinuum(&state, &project_id).map_err(|findings| unprocessable(&findings))?;
    Ok(JsonResponse(dcontinuum))
}
