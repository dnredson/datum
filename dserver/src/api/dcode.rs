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
use crate::core::dcode_authorization::{authorize_dcode_invocation, DCodeInvocationAuthorizationV1};
use crate::core::dcode_registry::{declare_dserv_artifact, load_state as load_dcode_state, persist_state as persist_dcode_state, DCodeArtifactRegistryStateV1, DeclareDServArtifactError};
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
    (StatusCode::BAD_REQUEST, JsonResponse(json!({"error": message.into()})))
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message.into()})))
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("dcode registry transaction lock poisoned")
}

fn findings_json(findings: &[CanonicalValidationFinding]) -> Vec<Value> {
    findings.iter().map(|finding| json!({"code": finding.code, "message": finding.message})).collect()
}

#[derive(Debug, Serialize)]
pub struct DeclareDServArtifactResponse {
    pub artifact_id: String,
    pub inserted: bool,
}

/// Declares (or idempotently re-declares) one canonical D-Serv artifact.
/// The registry key is `(application_id, dserv_id)` -- NOT `artifact_id`/
/// `version` -- so a conflicting redeclaration (same `(application_id,
/// dserv_id)`, different content) fails closed with 409 regardless of what
/// its `artifact_id`/`version` say (independent review v2, R8: an earlier
/// version of this comment incorrectly implied `artifact_id`/`version` was
/// itself an escape hatch). v0.1 has no artifact replacement/version-
/// transition mechanism at all -- one immutable registered canonical
/// artifact per `(application_id, dserv_id)`, full stop; see
/// `core::dcode_registry::declare_dserv_artifact` for the authoritative
/// wording.
pub async fn post_declare_dserv_artifact(
    Json(artifact): Json<CanonicalDServArtifactV1>,
) -> Result<JsonResponse<DeclareDServArtifactResponse>, ApiError> {
    let _transaction = dcode_transaction().lock().map_err(lock_error)?;
    let db_path = db_path();
    let mut state = load_dcode_state(&db_path).map_err(internal_error)?;
    let key = crate::core::dcode_registry::dserv_artifact_key(&artifact.application_id, &artifact.dserv_id);
    let already_present = state.artifacts.contains_key(&key);
    let artifact_id = artifact.artifact_id.clone();

    declare_dserv_artifact(&mut state, artifact).map_err(|error| match error {
        DeclareDServArtifactError::Invalid(findings) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            JsonResponse(json!({"error": "dserv_artifact_invalid", "findings": findings_json(&findings)})),
        ),
        DeclareDServArtifactError::Conflict(message) => {
            (StatusCode::CONFLICT, JsonResponse(json!({"error": "dserv_artifact_conflict", "message": message})))
        }
    })?;

    if !already_present {
        persist_dcode_state(&db_path, &state).map_err(internal_error)?;
    }

    Ok(JsonResponse(DeclareDServArtifactResponse { artifact_id, inserted: !already_present }))
}

pub async fn get_dserv_artifact(
    axum::extract::Path((application_id, dserv_id)): axum::extract::Path<(String, String)>,
) -> Result<JsonResponse<CanonicalDServArtifactV1>, ApiError> {
    let state = load_dcode_state(&db_path()).map_err(internal_error)?;
    state
        .get(&application_id, &dserv_id)
        .cloned()
        .map(JsonResponse)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                JsonResponse(json!({"error": format!(
                    "no canonical D-Serv artifact is registered for application '{application_id}' service '{dserv_id}'"
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

    let _ddeploy_transaction = crate::api::ddeploy::ddeploy_transaction().lock().map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction().lock().map_err(lock_error)?;
    let _dcode_transaction = dcode_transaction().lock().map_err(lock_error)?;

    let db_path = db_path();
    let ddeploy_state = crate::core::ddeploy::load_state(&db_path).map_err(internal_error)?;
    let dnode_registry_state = crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let dcode_registry_state: DCodeArtifactRegistryStateV1 = load_dcode_state(&db_path).map_err(internal_error)?;

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
