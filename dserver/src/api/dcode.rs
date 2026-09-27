//! Canonical D-Serv artifact / D-Code authorization HTTP surface (ADR-0021).
//! Declares canonical `datum.dserv-artifact/1` descriptors and issues
//! `DCodeInvocationAuthorizationV1` capabilities proving the full active-
//! DMap -> D-Graph -> D-Node -> artifact chain (`core::dcode_authorization`).
//! DServer never executes D-Code and never stores module bytes -- see
//! `documentation/adr/ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md`.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use axum::{extract::Json, http::StatusCode, Json as JsonResponse};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::dcode_authorization::{
    authorize_dcode_invocation, DCodeInvocationAuthorizationV1,
};
use crate::core::dcode_registry::{
    declare_dserv_artifact_revision, load_state_v2 as load_dcode_state,
    persist_state_v2 as persist_dcode_state, DeclareDServArtifactRevisionError,
};
use crate::core::dserv_artifact::CanonicalDServArtifactV1;
use crate::storage::control_state::default_control_state_db_path;

type ApiError = (StatusCode, JsonResponse<Value>);

static DCODE_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

/// `pub(crate)`: acquired ONLY by handlers in this file, and only as the
/// INNERMOST lock when nested (see `post_authorize_dcode_invocation`'s
/// `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DCODE_TRANSACTION` order) --
/// never acquired before either of those, so there is no reverse-order
/// acquisition path and therefore no deadlock.
pub(crate) fn dcode_transaction() -> &'static Mutex<()> {
    DCODE_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn db_path() -> PathBuf {
    default_control_state_db_path()
}

fn bad_request(message: impl Into<String>) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        JsonResponse(json!({"error": message.into()})),
    )
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("dcode registry transaction lock poisoned")
}

fn findings_json(findings: &[CanonicalValidationFinding]) -> Vec<Value> {
    findings
        .iter()
        .map(|finding| json!({"code": finding.code, "message": finding.message}))
        .collect()
}

#[derive(Debug, Serialize)]
pub struct DeclareDServArtifactResponse {
    pub artifact_id: String,
    pub descriptor_digest: String,
    pub inserted: bool,
}

/// Declares (or idempotently re-declares) one immutable canonical D-Serv
/// artifact REVISION (Phase 117G). The registry key is now `(application_id,
/// dserv_id, descriptor_digest)` -- NOT `artifact_id`/`version` and NOT the
/// old flat `(application_id, dserv_id)` alone -- so declaring a NEW content
/// revision for an already-known logical service never replaces or removes
/// any prior revision; it simply adds another immutable entry. Two
/// declarations with the exact same digest are idempotent (`inserted:
/// false`); a different digest for the same `(application_id, dserv_id)` is
/// always accepted as a new revision, never a 409 conflict. Declaring a
/// revision here changes NOTHING about which revision any already-active
/// D-Deploy deployment actually executes -- see
/// `core::dcode_registry::declare_dserv_artifact_revision` and
/// `documentation/reference/phase117g-versioned-dcode-governed-binding.md`.
pub async fn post_declare_dserv_artifact(
    Json(artifact): Json<CanonicalDServArtifactV1>,
) -> Result<JsonResponse<DeclareDServArtifactResponse>, ApiError> {
    let _transaction = dcode_transaction().lock().map_err(lock_error)?;
    let db_path = db_path();
    let mut state = load_dcode_state(&db_path).map_err(internal_error)?;
    let artifact_id = artifact.artifact_id.clone();
    let application_id = artifact.application_id.clone();
    let dserv_id = artifact.dserv_id.clone();

    let already_present_digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(&artifact)
        .ok()
        .filter(|digest| state.get_exact(&application_id, &dserv_id, digest).is_some());

    let descriptor_digest =
        declare_dserv_artifact_revision(&mut state, artifact).map_err(|error| match error {
            DeclareDServArtifactRevisionError::Invalid(findings) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                JsonResponse(
                    json!({"error": "dserv_artifact_invalid", "findings": findings_json(&findings)}),
                ),
            ),
            DeclareDServArtifactRevisionError::Conflict(message) => (
                StatusCode::CONFLICT,
                JsonResponse(json!({"error": "dserv_artifact_conflict", "message": message})),
            ),
            DeclareDServArtifactRevisionError::DigestComputationFailed(message) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                JsonResponse(json!({"error": "dserv_artifact_digest_computation_failed", "message": message})),
            ),
        })?;

    let already_present = already_present_digest.as_deref() == Some(descriptor_digest.as_str());

    if !already_present {
        persist_dcode_state(&db_path, &state).map_err(internal_error)?;
    }

    Ok(JsonResponse(DeclareDServArtifactResponse {
        artifact_id,
        descriptor_digest,
        inserted: !already_present,
    }))
}

/// Legacy, logical `(application_id, dserv_id)` lookup, kept ONLY for
/// backward compatibility and debugging (Phase 117G Section 32) -- it is NOT
/// suitable for governed invocation once a service has more than one
/// registered revision. While exactly zero or one revision is registered it
/// behaves exactly as before (404 / the single revision); the moment a
/// SECOND revision is declared for the same logical service this endpoint
/// fails closed with 409 rather than guessing "latest"/"first"/"newest". A
/// governed caller MUST instead fetch the exact revision it was authorized
/// for via `GET /api/v1/dcode/artifacts/:application_id/:dserv_id/:descriptor_digest`.
pub async fn get_dserv_artifact(
    axum::extract::Path((application_id, dserv_id)): axum::extract::Path<(String, String)>,
) -> Result<JsonResponse<CanonicalDServArtifactV1>, ApiError> {
    let state = load_dcode_state(&db_path()).map_err(internal_error)?;
    match state.get_logical_if_unambiguous(&application_id, &dserv_id) {
        Ok(Some(artifact)) => Ok(JsonResponse(artifact.clone())),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            JsonResponse(json!({"error": format!(
                "no canonical D-Serv artifact is registered for application '{application_id}' service '{dserv_id}'"
            )})),
        )),
        Err(message) => Err((
            StatusCode::CONFLICT,
            JsonResponse(json!({"error": "dserv_artifact_lookup_ambiguous", "message": message})),
        )),
    }
}

/// Exact-revision governed fetch (Phase 117G Section 31): the ONLY lookup a
/// governed invocation client should ever use, keyed by the precise
/// `descriptor_digest` a fresh `DCodeInvocationAuthorizationV1` names in its
/// own `dserv_artifact_descriptor_digest` field. Never ambiguous, never
/// "latest" -- either this exact immutable revision exists or it does not.
pub async fn get_dserv_artifact_revision(
    axum::extract::Path((application_id, dserv_id, descriptor_digest)): axum::extract::Path<(
        String,
        String,
        String,
    )>,
) -> Result<JsonResponse<CanonicalDServArtifactV1>, ApiError> {
    let state = load_dcode_state(&db_path()).map_err(internal_error)?;
    state
        .get_exact(&application_id, &dserv_id, &descriptor_digest)
        .cloned()
        .map(JsonResponse)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({"error": format!(
                    "no D-Serv artifact revision '{descriptor_digest}' is registered for application '{application_id}' service '{dserv_id}'"
                )})),
            )
        })
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeDCodeInvocationRequest {
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
    pub node_id: String,
}

/// Issues a fresh `DCodeInvocationAuthorizationV1` under the established
/// `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION` stable-view lock order
/// (extended with `DCODE_TRANSACTION` as the innermost lock), so the
/// active-D-Map/D-Graph/D-Node/artifact chain it proves cannot be
/// invalidated by a concurrent acceptance, D-Node declaration/removal, or
/// artifact declaration between the check and the response being built.
/// This authorization is always freshly derived, never cached server-side
/// -- `core::dcode_authorization::verify_dcode_authorization_still_current`
/// is the mechanism for re-checking a previously-issued authorization later.
///
/// Independent review v3 (F5) correction: an earlier version of this
/// comment incorrectly claimed the v0.1 governed SmartSentinel invocation
/// CLI does not itself call back to DServer before executing "since that
/// would require D-Call/distribution machinery." That became FALSE as of
/// independent review v2 (R1) -- the GOVERNED CLI path now explicitly calls
/// this endpoint (and `GET /api/v1/dcode/artifacts/...`) immediately before
/// execution; only a clearly-labeled `offline-test-only` CLI mode skips it.
/// Control-plane authorization verification is NOT D-Call (D-Call is
/// D-Serv-to-D-Serv message transport/routing, phase 112); calling this
/// endpoint from the CLI required no D-Call machinery at all. The genuinely
/// HONEST remaining limitation is distributed TOCTOU: a control-plane state
/// change can still occur between this endpoint's response and the actual
/// guest execution that follows it -- see ADR-0021 and
/// `stale-authority-audit.txt` / `control-plane-preflight-audit.txt` for the
/// precise scope of that residual gap.
pub async fn post_authorize_dcode_invocation(
    Json(request): Json<AuthorizeDCodeInvocationRequest>,
) -> Result<JsonResponse<DCodeInvocationAuthorizationV1>, ApiError> {
    for (field, value) in [
        ("project_id", &request.project_id),
        ("application_id", &request.application_id),
        ("service_id", &request.service_id),
        ("node_id", &request.node_id),
    ] {
        if value.trim().is_empty() {
            return Err(bad_request(format!("{field} must be a non-empty string")));
        }
    }

    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    let _dcode_transaction = dcode_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state =
        crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let dcode_registry_state = load_dcode_state(&db_path).map_err(internal_error)?;

    let authorization = authorize_dcode_invocation(
        &ddeploy_state,
        &dnode_registry_state,
        &dcode_registry_state,
        &request.project_id,
        &request.application_id,
        &request.service_id,
        &request.node_id,
    )
    .map_err(|finding| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": finding.code, "message": finding.message})),
        )
    })?;

    Ok(JsonResponse(authorization))
}
