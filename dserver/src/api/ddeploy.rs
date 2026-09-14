//! Stage 111 (ADR-0018) HTTP surface: D-Deploy proposal validation,
//! creation, fetch, explicit acceptance, and the authoritative active-
//! deployment read. All routes are project-scoped via `X-Datum-Project`,
//! matching every other DServer API module's convention.
//!
//! `POST .../validate` is READ-ONLY: no proposal persistence, no D-Map, no
//! operational update, no host side effect. `POST .../proposals` persists
//! an immutable PENDING proposal only -- never an active D-Map. Only
//! `POST .../:proposal_id/accept` may materialize and activate one.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use axum::{
    extract::{Json, Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json as JsonResponse,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::canonical_dcontinuum::CanonicalDContinuumV1;
use crate::core::canonical_dgraph::CanonicalDGraphV1;
use crate::core::canonical_dmap::{CanonicalDMapV1, CanonicalDServPlacementV1};
use crate::core::ddeploy::{
    accept_proposal, active_operational_projection, check_artifact_list_project_scope,
    create_proposal, is_canonical_authority_active, load_state, operational_dgraph_id_for_project,
    persist_state, validate_full, validate_proposal_context, AcceptError, AcceptOutcome,
    CreateProposalInput, DDeployAcceptanceV1, DDeployProposalV1, ProposalRecord, ProposalSource,
    F_PERSISTENCE_CONFLICT, F_PROPOSAL_NOT_FOUND,
};
use crate::core::service_artifact_registry::ServiceArtifact;
use crate::storage::control_state::default_control_state_db_path;
use crate::AppState;

type ApiError = (StatusCode, JsonResponse<Value>);

static DDEPLOY_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

fn ddeploy_transaction() -> &'static Mutex<()> {
    DDEPLOY_TRANSACTION.get_or_init(|| Mutex::new(()))
}

fn ddeploy_db_path() -> PathBuf {
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

fn unprocessable(findings: &[crate::core::canonical_dgraph::CanonicalValidationFinding]) -> ApiError {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        JsonResponse(json!({"error": "candidate_dmap_invalid", "findings": findings_json(findings)})),
    )
}

fn not_found_proposal(proposal_id: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        JsonResponse(json!({"error": F_PROPOSAL_NOT_FOUND, "message": format!("proposal '{proposal_id}' not found")})),
    )
}

fn conflict(code: &str, message: impl Into<String>) -> ApiError {
    (StatusCode::CONFLICT, JsonResponse(json!({"error": code, "message": message.into()})))
}

fn internal_error(message: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": message.into()})))
}

/// Persistence-layer failure during a write (500, per spec: "persistence
/// failure"). Uses the stable `F_PERSISTENCE_CONFLICT` finding code as the
/// wire-level `error` field so this failure mode is greppable independently
/// of its free-text message, while still surfacing as a server error rather
/// than a client-correctable 409 -- the caller did nothing wrong here.
fn persistence_failure(message: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, JsonResponse(json!({"error": F_PERSISTENCE_CONFLICT, "message": message.into()})))
}

fn findings_json(findings: &[crate::core::canonical_dgraph::CanonicalValidationFinding]) -> Vec<Value> {
    findings.iter().map(|finding| json!({"code": finding.code, "message": finding.message})).collect()
}

fn lock_error<T>(_: T) -> ApiError {
    internal_error("ddeploy transaction lock poisoned")
}

/// Fetches the project-scoped `ServiceArtifact` list and defensively verifies
/// it is actually scoped to the requested project (`F_ARTIFACT_PROJECT_MISMATCH`)
/// before handing it to any Stage 111 validation/acceptance path.
fn fetch_project_artifacts(state: &AppState, project_id: &str) -> Result<Vec<ServiceArtifact>, ApiError> {
    let list = state.service_artifacts.list(project_id).map_err(internal_error)?;
    if let Some(f) = check_artifact_list_project_scope(project_id, &list.project_id) {
        return Err(unprocessable(&[f]));
    }
    Ok(list.artifacts)
}

// --------------------------------------------------------------------------- //
// Request / response DTOs                                                     //
// --------------------------------------------------------------------------- //

#[derive(Debug, Deserialize)]
pub struct ProposalInputPayload {
    pub application_id: String,
    #[serde(default = "default_source")]
    pub source: ProposalSource,
    pub dgraph: CanonicalDGraphV1,
    pub dcontinuum: CanonicalDContinuumV1,
    #[serde(default)]
    pub placements: Vec<CanonicalDServPlacementV1>,
}

fn default_source() -> ProposalSource {
    ProposalSource::Operator
}

#[derive(Debug, Serialize)]
pub struct ValidateResponse {
    pub valid: bool,
    pub findings: Vec<Value>,
    pub dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1,
    pub dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1,
    pub candidate_placements: Vec<CanonicalDServPlacementV1>,
}

#[derive(Debug, Serialize)]
pub struct ProposalResponse {
    pub proposal: DDeployProposalV1,
    pub proposal_digest: String,
    pub state: &'static str,
}

impl From<&ProposalRecord> for ProposalResponse {
    fn from(record: &ProposalRecord) -> Self {
        let state = match record.state {
            crate::core::ddeploy::ProposalLifecycleState::Pending => "pending",
            crate::core::ddeploy::ProposalLifecycleState::Accepted => "accepted",
        };
        ProposalResponse { proposal: record.proposal.clone(), proposal_digest: record.proposal_digest.clone(), state }
    }
}

#[derive(Debug, Deserialize)]
pub struct AcceptRequestPayload {
    pub accepted_by: String,
    #[serde(default)]
    pub expected_proposal_digest: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AcceptResponse {
    pub acceptance: DDeployAcceptanceV1,
    /// The D-Map this call's `acceptance` record materializes. On a replay
    /// of a SUPERSEDED proposal (independent review v2, P1 fix) this is
    /// that proposal's OWN original materialization, which may no longer be
    /// the current active D-Map -- see `currently_active`.
    pub materialized_dmap: CanonicalDMapV1,
    pub operational_dgraph_id: String,
    pub operational_revision: u64,
    pub idempotent_replay: bool,
    /// `true` iff `materialized_dmap` is still the CURRENT active D-Map for
    /// this application. `GET /api/v1/ddeploy/active` remains the
    /// authoritative source for "what is active right now" independent of
    /// this field.
    pub currently_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct ActiveQuery {
    pub application_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ActiveDeploymentSummary {
    pub project_id: String,
    pub application_id: String,
    pub active_dmap: CanonicalDMapV1,
    pub dmap_digest: String,
    pub proposal_id: String,
    pub acceptance_id: String,
    pub accepted_at_utc: String,
    pub operational_dgraph_id: String,
    pub operational_revision: u64,
}

#[derive(Debug, Serialize)]
pub struct ActiveResponse {
    pub project_id: String,
    pub deployments: Vec<ActiveDeploymentSummary>,
}

// --------------------------------------------------------------------------- //
// Handlers                                                                     //
// --------------------------------------------------------------------------- //

/// PURE. No proposal persistence, no D-Map, no operational update, no host
/// side effect -- this handler never touches the ddeploy control-state
/// domain at all, only the caller-supplied payload and the CURRENT
/// (read-only) `ServiceArtifactRegistry` snapshot.
pub async fn post_validate_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ProposalInputPayload>,
) -> Result<JsonResponse<ValidateResponse>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let mut shape_findings = payload.dgraph.validate();
    shape_findings.extend(payload.dcontinuum.validate());
    if !shape_findings.is_empty() {
        return Ok(JsonResponse(ValidateResponse {
            valid: false,
            findings: findings_json(&shape_findings),
            dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 { id: payload.dgraph.dgraph_id.clone(), revision: payload.dgraph.revision, digest: String::new() },
            dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 { id: payload.dcontinuum.dcontinuum_id.clone(), revision: payload.dcontinuum.revision, digest: String::new() },
            candidate_placements: payload.placements,
        }));
    }

    let dgraph_digest = crate::core::canonical_dgraph::recompute_dgraph_digest(&payload.dgraph).map_err(internal_error)?;
    let dcontinuum_digest = crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&payload.dcontinuum).map_err(internal_error)?;

    let proposal = DDeployProposalV1 {
        schema: crate::core::ddeploy::DDEPLOY_PROPOSAL_SCHEMA.to_string(),
        proposal_id: "preview".to_string(),
        project_id: project_id.clone(),
        application_id: payload.application_id.clone(),
        source: payload.source,
        created_at_utc: Utc::now().to_rfc3339(),
        expected_active_dmap_revision: None,
        dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 { id: payload.dgraph.dgraph_id.clone(), revision: payload.dgraph.revision, digest: dgraph_digest },
        dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 { id: payload.dcontinuum.dcontinuum_id.clone(), revision: payload.dcontinuum.revision, digest: dcontinuum_digest },
        placements: payload.placements.clone(),
    };

    let artifacts = fetch_project_artifacts(&state, &project_id)?;
    let outcome = validate_full(&proposal, &payload.dgraph, &payload.dcontinuum, &artifacts, "dmap:preview", 1);

    Ok(JsonResponse(ValidateResponse {
        valid: outcome.findings.is_empty(),
        findings: findings_json(&outcome.findings),
        dgraph_ref: proposal.dgraph_ref,
        dcontinuum_ref: proposal.dcontinuum_ref,
        candidate_placements: outcome.candidate_dmap.placements,
    }))
}

pub async fn post_create_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<ProposalInputPayload>,
) -> Result<(StatusCode, JsonResponse<ProposalResponse>), ApiError> {
    let project_id = extract_project_id(&headers)?;
    let artifacts = fetch_project_artifacts(&state, &project_id)?;

    let _transaction = ddeploy_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;

    let proposal_id = format!("ddeploy-proposal:{}", Uuid::new_v4());
    let record = create_proposal(
        &mut ddeploy_state,
        CreateProposalInput {
            project_id,
            application_id: payload.application_id,
            source: payload.source,
            dgraph: payload.dgraph,
            dcontinuum: payload.dcontinuum,
            placements: payload.placements,
        },
        proposal_id,
        Utc::now().to_rfc3339(),
        &artifacts,
    )
    .map_err(|findings| unprocessable(&findings))?;

    persist_state(&db_path, &ddeploy_state).map_err(persistence_failure)?;

    Ok((StatusCode::CREATED, JsonResponse(ProposalResponse::from(&record))))
}

pub async fn get_proposal(
    headers: HeaderMap,
    Path(proposal_id): Path<String>,
) -> Result<JsonResponse<ProposalResponse>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = ddeploy_db_path();
    let ddeploy_state = load_state(&db_path).map_err(internal_error)?;
    let record = ddeploy_state
        .proposals
        .get(&proposal_id)
        .ok_or_else(|| not_found_proposal(&proposal_id))?;
    // Cross-tenant existence is never disclosed: a project_id mismatch (the
    // SAME condition `validate_proposal_context` would flag as
    // `proposal_project_mismatch` at creation/acceptance time) reports
    // identically to a genuinely missing proposal here, never as a
    // differentiated 422/409 -- otherwise the response itself would leak
    // "a proposal with this id exists, just not in your project" to a
    // caller probing another project's proposal ids.
    if !validate_proposal_context(&record.proposal, &project_id).is_empty() {
        return Err(not_found_proposal(&proposal_id));
    }
    Ok(JsonResponse(ProposalResponse::from(record)))
}

pub async fn post_accept_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<String>,
    Json(payload): Json<AcceptRequestPayload>,
) -> Result<JsonResponse<AcceptResponse>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    if payload.accepted_by.trim().is_empty() {
        return Err(bad_request("accepted_by must be a non-empty audit label"));
    }

    let _transaction = ddeploy_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;

    let existing = ddeploy_state
        .proposals
        .get(&proposal_id)
        .ok_or_else(|| not_found_proposal(&proposal_id))?;
    if !validate_proposal_context(&existing.proposal, &project_id).is_empty() {
        return Err(not_found_proposal(&proposal_id));
    }

    let now = Utc::now().to_rfc3339();
    let acceptance_id = format!("ddeploy-acceptance:{}", Uuid::new_v4());

    // Stage 111 (ADR-0018 independent review v2, P0-2): hold a STABLE
    // `ServiceArtifactRegistry` read view across acceptance-critical
    // eligibility revalidation AND the SQLite persist commit. Lock order:
    // `DDEPLOY_TRANSACTION` (already held above) -> registry stable view ->
    // SQLite persistence -- matching Stage 107A's own documented order
    // (`authorize_operational_reconciliation`), so no reverse-order
    // deadlock is introduced. A concurrent `ServiceArtifactRegistry::create`/
    // `update`/`delete`/`set_pinning_mode` cannot acquire its write lock
    // (and therefore cannot commit a `target_nodes`/artifact change) while
    // this closure still holds the read lock -- the artifacts this call
    // validates against are provably still current at the instant the
    // acceptance is persisted.
    let outcome = state
        .service_artifacts
        .with_stable_view(|registry_state| -> Result<AcceptOutcome, ApiError> {
            let artifacts = registry_state
                .artifacts_by_project
                .get(&project_id)
                .cloned()
                .unwrap_or_default();

            let outcome = accept_proposal(
                &mut ddeploy_state,
                &proposal_id,
                payload.accepted_by.trim(),
                payload.expected_proposal_digest.as_deref(),
                &artifacts,
                &now,
                acceptance_id,
            )
            .map_err(accept_error_to_api)?;

            // Only persist when this call actually mutated state -- an
            // idempotent replay leaves `ddeploy_state` byte-identical to
            // what was just loaded, so re-persisting it is a harmless no-op
            // we skip for clarity, not a correctness requirement.
            if !outcome.idempotent_replay {
                persist_state(&db_path, &ddeploy_state).map_err(persistence_failure)?;
            }

            Ok(outcome)
        })
        .map_err(internal_error)??;

    Ok(JsonResponse(AcceptResponse {
        acceptance: outcome.acceptance,
        materialized_dmap: outcome.materialized_dmap,
        operational_dgraph_id: outcome.operational_dgraph_id,
        operational_revision: outcome.operational_revision,
        idempotent_replay: outcome.idempotent_replay,
        currently_active: outcome.currently_active,
    }))
}

fn accept_error_to_api(error: AcceptError) -> ApiError {
    match error {
        AcceptError::NotFound(message) => (StatusCode::NOT_FOUND, JsonResponse(json!({"error": F_PROPOSAL_NOT_FOUND, "message": message}))),
        AcceptError::DigestMismatch(message) => conflict(crate::core::ddeploy::F_PROPOSAL_DIGEST_MISMATCH, message),
        AcceptError::Invalid(findings) => unprocessable(&findings),
        AcceptError::Stale(message) => conflict(crate::core::ddeploy::F_STALE_ACTIVE_DMAP_REVISION, message),
        AcceptError::Collision(findings) => unprocessable(&findings),
        AcceptError::Internal(message) => internal_error(message),
    }
}

pub async fn get_active(
    headers: HeaderMap,
    Query(query): Query<ActiveQuery>,
) -> Result<JsonResponse<ActiveResponse>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = ddeploy_db_path();
    let ddeploy_state = load_state(&db_path).map_err(internal_error)?;

    let mut deployments: Vec<ActiveDeploymentSummary> = ddeploy_state
        .active_by_application
        .values()
        .filter(|active| active.active_dmap.project_id == project_id)
        .filter(|active| query.application_id.as_deref().is_none_or(|requested| requested == active.application_id))
        .map(|active| ActiveDeploymentSummary {
            project_id: project_id.clone(),
            application_id: active.application_id.clone(),
            active_dmap: active.active_dmap.clone(),
            dmap_digest: active.dmap_digest.clone(),
            proposal_id: active.proposal_id.clone(),
            acceptance_id: active.acceptance_id.clone(),
            accepted_at_utc: active.accepted_at_utc.clone(),
            operational_dgraph_id: operational_dgraph_id_for_project(&project_id),
            operational_revision: *ddeploy_state.project_operational_revision.get(&project_id).unwrap_or(&0),
        })
        .collect();
    deployments.sort_by(|a, b| a.application_id.cmp(&b.application_id));

    Ok(JsonResponse(ActiveResponse { project_id, deployments }))
}

// --------------------------------------------------------------------------- //
// Cross-module accessor (used by `api::operational_dgraph`)                   //
// --------------------------------------------------------------------------- //

/// Thin wrapper so `api::operational_dgraph` never has to import
/// `core::ddeploy` internals directly -- the effective-authority accessor
/// lives here, next to every other D-Deploy read.
pub fn effective_canonical_operational_graph(project_id: &str) -> Result<Option<crate::core::operational_dgraph::OperationalDGraph>, String> {
    active_operational_projection(project_id)
}

pub fn canonical_authority_active(project_id: &str) -> Result<bool, String> {
    is_canonical_authority_active(project_id)
}
