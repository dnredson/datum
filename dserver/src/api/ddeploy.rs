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
    compute_project_node_commitments, create_proposal, derive_placement_eligibility_view,
    is_canonical_authority_active, load_state, operational_dgraph_id_for_project, persist_state,
    validate_full, validate_proposal_context, AcceptError, AcceptOutcome, CreateProposalInput,
    DDeployAcceptanceV1, DDeployProposalV1, PlannerProvenanceV1, ProposalRecord, ProposalSource,
    F_PERSISTENCE_CONFLICT, F_PROPOSAL_NOT_FOUND,
};
use crate::core::service_artifact_registry::ServiceArtifact;
use crate::storage::control_state::default_control_state_db_path;
use crate::AppState;

type ApiError = (StatusCode, JsonResponse<Value>);

static DDEPLOY_TRANSACTION: OnceLock<Mutex<()>> = OnceLock::new();

/// `pub(crate)` (D-Code authorization, ADR-0021): `api::dcode`'s
/// `post_authorize_dcode_invocation` also acquires this same lock, as the
/// OUTERMOST lock in its own `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
/// DCODE_TRANSACTION` chain, for the same reason `post_accept_proposal`
/// already nests `DNODE_TRANSACTION` inside it: a consistent snapshot
/// across D-Deploy active-authority state and the D-Node/D-Code registries
/// requires one fixed, never-reversed lock order.
pub(crate) fn ddeploy_transaction() -> &'static Mutex<()> {
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

/// D-Node/D-Continuum authority transition (D-Node/D-Continuum foundation
/// closure, ADR-0019): derives DServer's own server-owned authoritative
/// `datum.dcontinuum/1` for `project_id` from the structural D-Node registry
/// (`core::dnode_registry`) and verifies the caller-supplied `claimed`
/// continuum's nodes genuinely match it. Returns the AUTHORITATIVE
/// continuum on success -- callers must use THIS, never `claimed`, for every
/// downstream validation/persistence step, so a proposal's `dcontinuum_ref`/
/// `dcontinuum_snapshot` is always server-derived even when the caller's
/// claim happened to be internally consistent. Fails closed with
/// `authoritative_dcontinuum_unavailable` if no structural D-Node has ever
/// been declared, and with `dcontinuum_node_unregistered` /
/// `dnode_structural_conflict` / `dnode_capability_invalid` if `claimed`
/// names a node the registry does not recognise or disagrees with.
fn authoritative_dcontinuum_for_project(
    project_id: &str,
    claimed: &CanonicalDContinuumV1,
) -> Result<CanonicalDContinuumV1, ApiError> {
    let db_path = default_control_state_db_path();
    let registry_state = crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let authoritative = crate::core::dnode_registry::build_canonical_dcontinuum(&registry_state, project_id)
        .map_err(|findings| unprocessable(&findings))?;
    let authority_findings = crate::core::dnode_registry::check_dcontinuum_authority(claimed, &authoritative);
    if !authority_findings.is_empty() {
        return Err(unprocessable(&authority_findings));
    }
    Ok(authoritative)
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

/// Canonical deterministic placement v1 (roadmap conceptual phase 120,
/// ADR-0020). Deliberately carries NO `source`/`dcontinuum`/`placements`
/// field at all -- unlike `ProposalInputPayload`, this endpoint's entire
/// point is that the caller never supplies (and is never asked to
/// reconstruct) a proposal source, a D-Continuum claim, or a placement
/// decision; all three are derived server-side. Foundation closure v2 (R1
/// fix): a proposal produced by THIS endpoint is unconditionally
/// `ProposalSource::Deterministic` -- `post_plan_proposal` never reads a
/// `source` field from any payload, so there is no caller-suppliable value
/// that could reach `proposal.source`. `deny_unknown_fields` (unlike every
/// other DTO in this module) is deliberate here: a caller attempting to
/// send `source`/`dcontinuum`/`placements` to THIS endpoint gets an
/// immediate 400 rather than having the field silently ignored, which could
/// otherwise look (to that caller) like it had taken effect.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanRequestPayload {
    pub application_id: String,
    pub dgraph: CanonicalDGraphV1,
}

#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub proposal: DDeployProposalV1,
    pub proposal_digest: String,
    pub state: &'static str,
    /// Full decision evidence from `core::canonical_placement` -- exposed
    /// directly so a caller need not re-run the (deterministic, so
    /// reproducible anyway) policy to see WHY each D-Serv landed where it
    /// did.
    pub placement_provenance: crate::core::canonical_placement::PlacementProvenance,
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

    // D-Node/D-Continuum authority transition (ADR-0019): the caller's own
    // `dcontinuum` shape is valid, but it is NEVER trusted as the record of
    // what D-Nodes genuinely exist -- only the server-owned structural
    // registry is. `authoritative_dcontinuum` REPLACES `payload.dcontinuum`
    // for every downstream use below.
    let authoritative_dcontinuum = authoritative_dcontinuum_for_project(&project_id, &payload.dcontinuum)?;

    let dgraph_digest = crate::core::canonical_dgraph::recompute_dgraph_digest(&payload.dgraph).map_err(internal_error)?;
    let dcontinuum_digest = crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&authoritative_dcontinuum).map_err(internal_error)?;

    let proposal = DDeployProposalV1 {
        schema: crate::core::ddeploy::DDEPLOY_PROPOSAL_SCHEMA.to_string(),
        proposal_id: "preview".to_string(),
        project_id: project_id.clone(),
        application_id: payload.application_id.clone(),
        source: payload.source,
        created_at_utc: Utc::now().to_rfc3339(),
        expected_active_dmap_revision: None,
        dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 { id: payload.dgraph.dgraph_id.clone(), revision: payload.dgraph.revision, digest: dgraph_digest },
        dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 { id: authoritative_dcontinuum.dcontinuum_id.clone(), revision: authoritative_dcontinuum.revision, digest: dcontinuum_digest },
        placements: payload.placements.clone(),
    };

    let artifacts = fetch_project_artifacts(&state, &project_id)?;
    let outcome = validate_full(&proposal, &payload.dgraph, &authoritative_dcontinuum, &artifacts, "dmap:preview", 1);

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

    // D-Node/D-Continuum authority transition (ADR-0019): see the identical
    // comment in `post_validate_proposal`. `authoritative_dcontinuum`
    // REPLACES `payload.dcontinuum` -- the persisted `ProposalRecord`'s
    // `dcontinuum_snapshot` is therefore always server-derived, never a
    // verbatim client payload.
    let authoritative_dcontinuum = authoritative_dcontinuum_for_project(&project_id, &payload.dcontinuum)?;

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
            dcontinuum: authoritative_dcontinuum,
            placements: payload.placements,
            planner_provenance: None,
        },
        proposal_id,
        Utc::now().to_rfc3339(),
        &artifacts,
    )
    .map_err(|findings| unprocessable(&findings))?;

    persist_state(&db_path, &ddeploy_state).map_err(persistence_failure)?;

    Ok((StatusCode::CREATED, JsonResponse(ProposalResponse::from(&record))))
}

/// Canonical deterministic placement v1 (roadmap conceptual phase 120,
/// ADR-0020): `canonical DGraph + authoritative structural DContinuum +
/// CURRENT project-wide structural commitment + CURRENT ServiceArtifact
/// eligibility + deterministic placement policy -> candidate placements ->
/// ordinary D-Deploy create_proposal -> validate -> explicit acceptance
/// later`. This handler NEVER calls `accept_proposal` -- it produces a
/// PENDING proposal through the exact same `create_proposal` function
/// `post_create_proposal` uses, with zero special-cased validation logic of
/// its own. The caller supplies only `application_id`/`dgraph`;
/// `source`/D-Continuum/placements are ALL entirely server-derived, so
/// there is no caller-suppliable field this endpoint could use to fabricate
/// a node, a capacity, a placement authority, or a false provenance claim
/// (foundation closure v2, R1 fix).
///
/// Foundation closure v2 (R2/R3B fix): unlike v1, this handler now acquires
/// the SAME lock order `accept_proposal` uses --
/// `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> ServiceArtifact stable view`
/// -- and holds it across planning itself, not merely across persistence.
/// This is necessary (not merely defensive) because the plan now genuinely
/// depends on the CURRENT `DDeployStateV1` (for project-wide structural
/// commitment, R3B) and the CURRENT `ServiceArtifact` snapshot (for
/// eligibility, R2): a plan computed against a detached, possibly-stale
/// snapshot of either could recommend a placement that a concurrent
/// acceptance elsewhere has just made infeasible or ineligible. Holding the
/// same locks used by acceptance for the ENTIRE plan-and-create sequence
/// means the facts this plan is built from cannot change out from under it
/// before its own resulting proposal is persisted. No lock is ever held
/// across network I/O -- there is none in this path.
pub async fn post_plan_proposal(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PlanRequestPayload>,
) -> Result<(StatusCode, JsonResponse<PlanResponse>), ApiError> {
    let project_id = extract_project_id(&headers)?;

    let _transaction = ddeploy_transaction().lock().map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction().lock().map_err(lock_error)?;
    let db_path = default_control_state_db_path();

    let registry_state = crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let authoritative_dcontinuum = crate::core::dnode_registry::build_canonical_dcontinuum(&registry_state, &project_id)
        .map_err(|findings| unprocessable(&findings))?;

    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;
    // R3B: exclude this application's OWN current active D-Map from the
    // "other active applications" baseline -- a successful acceptance
    // REPLACES it rather than adding a second, simultaneously-authoritative
    // one (transient dual-running migration headroom is a future phase-119
    // concern; this model is POST-ACCEPTANCE steady state only).
    let commitments = compute_project_node_commitments(&project_id, &ddeploy_state.active_by_application, &ddeploy_state.proposals, Some(&payload.application_id))
        .map_err(internal_error)?;

    let (plan, record) = state
        .service_artifacts
        .with_stable_view(|registry_state| -> Result<(crate::core::canonical_placement::CanonicalPlacementPlanV1, ProposalRecord), ApiError> {
            let artifacts = registry_state.artifacts_by_project.get(&project_id).cloned().unwrap_or_default();

            // R2: candidate nodes are filtered against the CURRENT,
            // just-snapshotted-under-lock ServiceArtifact eligibility view --
            // never a detached fetch taken before this lock was acquired.
            let eligibility = derive_placement_eligibility_view(&payload.dgraph, &artifacts);
            let plan = crate::core::canonical_placement::plan_canonical_placements(&payload.dgraph, &authoritative_dcontinuum, &commitments, &eligibility)
                .map_err(|findings| unprocessable(&findings))?;

            let proposal_id = format!("ddeploy-proposal:{}", Uuid::new_v4());
            let record = create_proposal(
                &mut ddeploy_state,
                CreateProposalInput {
                    project_id: project_id.clone(),
                    application_id: payload.application_id.clone(),
                    source: ProposalSource::Deterministic,
                    dgraph: payload.dgraph.clone(),
                    dcontinuum: authoritative_dcontinuum.clone(),
                    placements: plan.placements.clone(),
                    // C1A/C1B/C1C fix: persist the EXACT canonical input
                    // refs, commitment baseline, and eligibility view this
                    // successful call actually used -- `commitments`/
                    // `eligibility` are the very values just passed into
                    // `plan_canonical_placements` above, not a
                    // reconstruction; `dgraph_ref`/`dcontinuum_ref` are built
                    // from `plan.provenance`'s own already-recomputed
                    // id/revision/digest fields (identical to what
                    // `create_proposal` will independently recompute and
                    // assign to `proposal.dgraph_ref`/`dcontinuum_ref` below,
                    // since both derive from the same deterministic digest
                    // functions over the same `payload.dgraph`/
                    // `authoritative_dcontinuum` values).
                    planner_provenance: Some(PlannerProvenanceV1 {
                        planner_version: plan.provenance.planner_version.clone(),
                        policy_name: plan.provenance.policy_name.clone(),
                        policy_version: plan.provenance.policy_version.clone(),
                        dgraph_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                            id: plan.provenance.dgraph_id.clone(),
                            revision: plan.provenance.dgraph_revision,
                            digest: plan.provenance.dgraph_digest.clone(),
                        },
                        dcontinuum_ref: crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                            id: plan.provenance.dcontinuum_id.clone(),
                            revision: plan.provenance.dcontinuum_revision,
                            digest: plan.provenance.dcontinuum_digest.clone(),
                        },
                        committed_by_node: commitments.clone(),
                        eligibility: eligibility.clone(),
                        decisions: plan.provenance.decisions.clone(),
                    }),
                },
                proposal_id,
                Utc::now().to_rfc3339(),
                &artifacts,
            )
            .map_err(|findings| unprocessable(&findings))?;

            persist_state(&db_path, &ddeploy_state).map_err(persistence_failure)?;

            Ok((plan, record))
        })
        .map_err(internal_error)??;

    Ok((
        StatusCode::CREATED,
        JsonResponse(PlanResponse {
            proposal: record.proposal.clone(),
            proposal_digest: record.proposal_digest.clone(),
            state: "pending",
            placement_provenance: plan.provenance,
        }),
    ))
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
    // Foundation closure v2 (ADR-0019, R1 TOCTOU requirement): acquire the
    // D-Node registry's own transaction lock too, NESTED inside
    // `DDEPLOY_TRANSACTION` (fixed order, documented on `dnode_transaction`
    // itself) and held for the ENTIRE remainder of this call -- through the
    // authority revalidation below, the `ServiceArtifact` stable view, the
    // acceptance mutation, and the final SQLite persist. A concurrent
    // `PUT`/`DELETE` on a D-Node cannot commit anywhere in that window, so
    // the structural facts this call validates against cannot change out
    // from under it between the check and the persist.
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;

    let existing = ddeploy_state
        .proposals
        .get(&proposal_id)
        .ok_or_else(|| not_found_proposal(&proposal_id))?;
    if !validate_proposal_context(&existing.proposal, &project_id).is_empty() {
        return Err(not_found_proposal(&proposal_id));
    }
    let existing_is_pending = existing.state == crate::core::ddeploy::ProposalLifecycleState::Pending;

    // Only a PENDING proposal's acceptance needs the CURRENT authoritative
    // D-Continuum -- an idempotent replay of an ALREADY-accepted proposal
    // must succeed regardless of any later D-Node registry change (R1
    // scope is the PENDING -> ACCEPTED transition only; historical
    // acceptances are immutable, see `core::ddeploy::accept_proposal`'s doc
    // comment on this parameter).
    let current_authoritative_dcontinuum = if existing_is_pending {
        let registry_state = crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
        let authoritative = crate::core::dnode_registry::build_canonical_dcontinuum(&registry_state, &project_id)
            .map_err(|findings| unprocessable(&findings))?;
        Some(authoritative)
    } else {
        None
    };

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
                current_authoritative_dcontinuum.as_ref(),
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
        // Foundation closure v2 (ADR-0019, R1): a CONFLICT (409), not a 422
        // -- the proposal's own shape was fine; the world it depended on
        // (the server-owned D-Node structural registry) has since changed.
        AcceptError::StaleAuthority(findings) => (
            StatusCode::CONFLICT,
            JsonResponse(json!({"error": crate::core::ddeploy::F_STALE_AUTHORITATIVE_DCONTINUUM, "findings": findings_json(&findings)})),
        ),
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

#[cfg(test)]
mod tests {
    // Foundation closure v2 (R1 fix): this crate has no established
    // precedent anywhere for constructing a real `AppState`/SQLite-backed
    // `ServiceArtifactRegistry` inside a unit test to invoke an Axum handler
    // directly (`post_plan_proposal` needs both) -- runtime HTTP-level
    // coverage for R1 is therefore MANUAL (see `MANUAL_VALIDATION_COMMANDS.md`).
    // What IS provable at this level, cheaply and reliably, is the
    // STRUCTURAL guarantee the fix actually rests on: `PlanRequestPayload`
    // has no field a caller could use to influence `proposal.source` at
    // all, and `post_plan_proposal`'s body hardcodes
    // `ProposalSource::Deterministic` unconditionally. Combined with the
    // core-level `planner_output_is_accepted_by_create_proposal_unmodified`
    // test (`core::ddeploy`), which proves a proposal built exactly the way
    // this handler builds one really does end up with
    // `proposal.source == ProposalSource::Deterministic`, these two facts
    // together are the real proof of item 1/2's mandatory regressions: it
    // is not merely that the handler CURRENTLY happens to pass
    // `ProposalSource::Deterministic` -- it is that there exists no
    // deserializable field through which a caller's own value could ever
    // reach that call.
    fn production_source() -> String {
        let source = include_str!("ddeploy.rs");
        let production_source: String = source.lines().take_while(|line| line.trim() != "#[cfg(test)]").collect::<Vec<_>>().join("\n");
        assert!(
            production_source.len() < source.len(),
            "the '#[cfg(test)] mod tests' boundary was not found -- this test would otherwise scan its own forbidden-string literals"
        );
        production_source
    }

    fn struct_body(source: &str, struct_name: &str) -> String {
        let needle = format!("struct {struct_name} {{");
        let start = source.find(&needle).unwrap_or_else(|| panic!("'{needle}' not found in source")) + needle.len();
        let end = source[start..].find('}').unwrap_or_else(|| panic!("no closing brace found for struct {struct_name}"));
        source[start..start + end].to_string()
    }

    fn fn_body(source: &str, fn_signature_prefix: &str) -> String {
        let start = source.find(fn_signature_prefix).unwrap_or_else(|| panic!("'{fn_signature_prefix}' not found in source"));
        // Every handler in this file is followed by either another `pub
        // async fn` or `fn accept_error_to_api` -- either is a safe
        // end-of-function marker for this simple text scan (no nested
        // `fn` definitions exist inside any handler body in this file).
        let rest = &source[start..];
        let next_fn = rest[fn_signature_prefix.len()..]
            .find("\npub async fn ")
            .or_else(|| rest[fn_signature_prefix.len()..].find("\nfn accept_error_to_api"))
            .unwrap_or(rest.len() - fn_signature_prefix.len());
        rest[..fn_signature_prefix.len() + next_fn].to_string()
    }

    #[test]
    fn plan_request_payload_has_no_caller_controllable_source_field() {
        let body = struct_body(&production_source(), "PlanRequestPayload");
        assert!(!body.contains("source"), "PlanRequestPayload must carry no field through which a caller could influence proposal.source -- found: {body}");
        assert!(body.contains("application_id") && body.contains("dgraph"), "sanity check that struct_body() actually found the real struct: {body}");
    }

    /// Independent review v3 (C2): a cheap, DIRECT `serde_json`
    /// deserialization proof -- not merely a text-scan of the source, and
    /// not an assertion about which exact HTTP status Axum's JSON extractor
    /// maps a `serde` error to for the pinned Axum version (that was never
    /// executed end-to-end in this pass; see `MANUAL_VALIDATION_COMMANDS.md`
    /// for the corrected "HTTP 4xx, never 201" phrasing). The only property
    /// this test needs to prove is the one that actually determines the
    /// product behavior: `PlanRequestPayload` cannot be deserialized at all
    /// when an extra `source` field is present.
    #[test]
    fn plan_request_payload_serde_rejects_an_explicit_source_field() {
        let json = r#"{
            "application_id": "app-x",
            "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-x", "dgraph_id": "dgraph:x", "revision": 1, "services": [], "calls": []},
            "source": "operator"
        }"#;
        let result: Result<super::PlanRequestPayload, _> = serde_json::from_str(json);
        assert!(result.is_err(), "PlanRequestPayload must reject an explicit 'source' field, not silently ignore it");
    }

    #[test]
    fn plan_request_payload_serde_rejects_an_explicit_dcontinuum_field() {
        let json = r#"{
            "application_id": "app-x",
            "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-x", "dgraph_id": "dgraph:x", "revision": 1, "services": [], "calls": []},
            "dcontinuum": {"schema": "datum.dcontinuum/1", "project_id": "proj-x", "dcontinuum_id": "continuum:x", "revision": 1, "nodes": [], "links": []}
        }"#;
        let result: Result<super::PlanRequestPayload, _> = serde_json::from_str(json);
        assert!(result.is_err(), "PlanRequestPayload must reject an explicit 'dcontinuum' field, not silently ignore it");
    }

    #[test]
    fn plan_request_payload_serde_rejects_an_explicit_placements_field() {
        let json = r#"{
            "application_id": "app-x",
            "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-x", "dgraph_id": "dgraph:x", "revision": 1, "services": [], "calls": []},
            "placements": [{"service_id": "svc-a", "node_id": "fabricated-node"}]
        }"#;
        let result: Result<super::PlanRequestPayload, _> = serde_json::from_str(json);
        assert!(result.is_err(), "PlanRequestPayload must reject an explicit 'placements' field, not silently ignore it");
    }

    #[test]
    fn plan_request_payload_serde_accepts_a_normal_payload_with_no_denied_field() {
        // Sanity control for the three tests above: proves their failure is
        // specifically caused by the extra field, not by an unrelated schema
        // mismatch in this test's own JSON fixture.
        let json = r#"{
            "application_id": "app-x",
            "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-x", "dgraph_id": "dgraph:x", "revision": 1, "services": [], "calls": []}
        }"#;
        let result: Result<super::PlanRequestPayload, _> = serde_json::from_str(json);
        assert!(result.is_ok(), "a normal payload with no denied field must deserialize cleanly: {result:?}");
    }

    #[test]
    fn plan_request_payload_rejects_unknown_fields() {
        let body = production_source();
        let struct_decl_start = body.find("pub struct PlanRequestPayload").unwrap();
        let preceding = &body[..struct_decl_start];
        let last_attr_start = preceding.rfind("#[derive").unwrap();
        let header = &preceding[last_attr_start..];
        assert!(header.contains("deny_unknown_fields"), "PlanRequestPayload must reject an explicit source/dcontinuum/placements field with a 400 rather than silently ignoring it: {header}");
    }

    #[test]
    fn post_plan_proposal_hardcodes_deterministic_source_and_never_reads_a_payload_source() {
        let body = fn_body(&production_source(), "pub async fn post_plan_proposal(");
        assert!(body.contains("ProposalSource::Deterministic"), "post_plan_proposal must unconditionally set ProposalSource::Deterministic");
        assert!(!body.contains("payload.source"), "post_plan_proposal must never read a caller-supplied source field (none exists on PlanRequestPayload, but this also guards against a future field being added and wired up incorrectly)");
    }

    #[test]
    fn post_plan_proposal_holds_the_same_lock_order_accept_proposal_uses() {
        // R2/R3B fix: planning now genuinely depends on live DDeployStateV1/
        // ServiceArtifact state, so it must hold the SAME lock order as
        // acceptance (DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
        // ServiceArtifact stable view) across planning itself, not merely
        // across the final persist.
        let body = fn_body(&production_source(), "pub async fn post_plan_proposal(");
        let ddeploy_lock_pos = body.find("ddeploy_transaction().lock()").expect("post_plan_proposal must acquire DDEPLOY_TRANSACTION");
        let dnode_lock_pos = body.find("dnode_registry::dnode_transaction().lock()").expect("post_plan_proposal must acquire DNODE_TRANSACTION");
        let stable_view_pos = body.find("with_stable_view").expect("post_plan_proposal must use the ServiceArtifact stable view");
        assert!(ddeploy_lock_pos < dnode_lock_pos, "DDEPLOY_TRANSACTION must be acquired before DNODE_TRANSACTION");
        assert!(dnode_lock_pos < stable_view_pos, "DNODE_TRANSACTION must be acquired before the ServiceArtifact stable view");
    }
}
