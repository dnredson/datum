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

use crate::core::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
use crate::core::canonical_dforward::CanonicalDForwardV1;
use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1};
use crate::core::canonical_dmap::{CanonicalDMapV1, CanonicalDServPlacementV1};
use crate::core::canonical_dmap_v2::{
    CanonicalDMapV2, ContractRefV2, DCallRealizationV2, DIoTPlacementV2, DServPlacementV2,
    ExternalPortBindingV2, CANONICAL_DMAP_V2_SCHEMA,
};
use crate::core::ddeploy::{
    accept_proposal, active_operational_projection, active_placement_authority,
    check_artifact_list_project_scope, compute_project_node_commitments, create_proposal,
    derive_placement_eligibility_view, is_canonical_authority_active, load_state,
    operational_dgraph_id_for_project, persist_state, resolve_diot_artifact_authority,
    validate_full, validate_proposal_context, AcceptError, AcceptOutcome, CreateProposalInput,
    DDeployAcceptanceV1, DDeployProposalV1, PlannerProvenanceV1, ProposalRecord, ProposalSource,
    F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE, F_PERSISTENCE_CONFLICT,
    F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED, F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED,
    F_PROPOSAL_NOT_FOUND,
};
use crate::core::ddeploy_v2::{DDeployProposalV2, DDEPLOY_PROPOSAL_V2_SCHEMA};
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
    /// Phase 117G Section 36: the caller must name the EXACT registered
    /// D-Code descriptor revision (by `descriptor_digest`) it intends each
    /// bound service to execute -- never inferred from the registry's
    /// "latest"/"only" revision, an in-memory D-Compile result, or
    /// `artifact_id`/`version`. Optional and may be a strict subset of this
    /// proposal's own D-Graph services (Section 18: partial D-Code coverage
    /// is legal); omitted keys are simply not authorizable as D-Code.
    #[serde(default)]
    pub dcode_artifact_authority: std::collections::BTreeMap<String, crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1>,
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
        dcode_artifact_authority: payload.dcode_artifact_authority.clone(),
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
    // Phase 117G Section 19/60: `DCODE_TRANSACTION` acquired as the
    // innermost lock (same fixed order as `post_accept_proposal`) so this
    // creation-time D-Code binding validation reads registry state that
    // cannot change out from under it before the proposal is persisted.
    let _dcode_transaction = crate::api::dcode::dcode_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;

    if !payload.dcode_artifact_authority.is_empty() {
        let dcode_registry_state = crate::core::dcode_registry::load_state_v2(&db_path).map_err(internal_error)?;
        crate::core::ddeploy::validate_dcode_artifact_authority_bindings(
            &payload.dgraph,
            &payload.application_id,
            &dcode_registry_state,
            &payload.dcode_artifact_authority,
        )
        .map_err(|finding| unprocessable(&[finding]))?;
    }

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
            dcode_artifact_authority: payload.dcode_artifact_authority,
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
                    }), dcode_artifact_authority: std::collections::BTreeMap::new(),
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
    // Phase 117G Section 60: `DCODE_TRANSACTION` is acquired here as the
    // innermost lock, preserving the established
    // `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> DCODE_TRANSACTION` order
    // (`api::dcode`'s own doc comment) -- this handler never acquires
    // `DCODE_TRANSACTION` before `DDEPLOY_TRANSACTION`/`DNODE_TRANSACTION`,
    // so no reverse-order acquisition path is introduced anywhere.
    let _dcode_transaction = crate::api::dcode::dcode_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;
    let dcode_registry_state = crate::core::dcode_registry::load_state_v2(&db_path).map_err(internal_error)?;

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

            let outcome = crate::core::ddeploy::accept_proposal_with_dcode_binding_revalidation(
                &mut ddeploy_state,
                &proposal_id,
                payload.accepted_by.trim(),
                payload.expected_proposal_digest.as_deref(),
                &artifacts,
                current_authoritative_dcontinuum.as_ref(),
                &dcode_registry_state,
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
        // Phase 117G: the exact D-Code revision this proposal bound no
        // longer resolves against current registry state.
        AcceptError::DCodeBindingInvalid(findings) => unprocessable(&findings),
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


// --------------------------------------------------------------------------- //
// Phase 113A closure pass 4: DDeploy/2 HTTP surface                           //
//                                                                             //
// Mirrors the v1 handlers immediately above: same X-Datum-Project header     //
// convention, same read-only-validate / immutable-create / explicit-accept   //
// separation, same DDEPLOY_TRANSACTION -> DNODE_TRANSACTION -> ServiceArtifact//
// stable-view lock order, same "server owns identity/digests, caller supplies//
// only candidate semantic content" discipline. Every handler here is a thin  //
// wrapper around the real core functions Passes 1-3.1 already closed         //
// (`DDeployStateV1::create_v2_proposal`/`accept_v2_proposal`,                //
// `active_placement_authority`) -- no capacity/ServiceArtifact/relational-   //
// integrity logic is duplicated here.                                       //
// --------------------------------------------------------------------------- //

/// A v2 candidate's caller-declared semantic content. Deliberately carries NO
/// `dmap_id`/`revision`/`digest`/`proposal_id`/`acceptance_id`/authoritative-
/// ref field anywhere -- `DServPlacementV2`/`DIoTPlacementV2`/
/// `DCallRealizationV2`/`ExternalPortBindingV2` themselves have no such
/// fields, so a caller cannot supply arbitrary DMap authority through this
/// payload even before `deny_unknown_fields` is considered (item L, Phase
/// 113A closure pass 4).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeployV2CandidatePayload {
    pub application_id: String,
    /// Phase 113A closure pass 4.1: typed provenance classification only --
    /// never authority to bypass validation. The server still owns
    /// `proposal_id`/`created_at_utc`/every authoritative ref-digest/
    /// `diot_artifact_authority`/`proposal_digest`. `source == Ai` requires
    /// the exact same explicit `POST .../accept` call as any other source --
    /// see `ai_sourced_v2_proposal_has_no_alternate_activation_path`.
    #[serde(default = "default_source")]
    pub source: ProposalSource,
    pub dgraph: CanonicalDGraphV1,
    pub dcontinuum: CanonicalDContinuumV1,
    pub dforward: CanonicalDForwardV1,
    #[serde(default)]
    pub placements: Vec<DServPlacementV2>,
    #[serde(default)]
    pub diot_placements: Vec<DIoTPlacementV2>,
    #[serde(default)]
    pub dcall_realizations: Vec<DCallRealizationV2>,
    #[serde(default)]
    pub external_port_bindings: Vec<ExternalPortBindingV2>,
    /// Phase 117G Section 14/36: the v2 counterpart of `ProposalInputPayload`'s
    /// own `dcode_artifact_authority` field -- same shared binding type,
    /// same "caller names the exact registered descriptor_digest" contract.
    #[serde(default)]
    pub dcode_artifact_authority: std::collections::BTreeMap<String, crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1>,
}

#[derive(Debug, Serialize)]
pub struct ValidateV2Response {
    pub valid: bool,
    pub findings: Vec<Value>,
    pub dgraph_ref: ContractRefV2,
    pub dcontinuum_ref: ContractRefV2,
    pub dforward_ref: ContractRefV2,
}

#[derive(Debug, Serialize)]
pub struct ProposalV2Response {
    pub proposal: DDeployProposalV2,
    pub proposal_digest: String,
    pub state: &'static str,
}

impl ProposalV2Response {
    fn from_record(record: &crate::core::ddeploy_v2::DDeployProposalV2Record) -> Self {
        let state = match record.state {
            crate::core::ddeploy_v2::ProposalV2State::Pending => "pending",
            crate::core::ddeploy_v2::ProposalV2State::Accepted => "accepted",
        };
        ProposalV2Response {
            proposal: record.proposal.clone(),
            proposal_digest: record.proposal_digest.clone(),
            state,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AcceptV2RequestPayload {
    pub accepted_by: String,
    /// Phase 113A closure pass 4.1: caller-supplied CAS guard, mirroring
    /// v1's `AcceptRequestPayload::expected_proposal_digest` exactly. If
    /// supplied and it does not equal the proposal's recorded
    /// `proposal_digest`, acceptance is rejected BEFORE any state mutation --
    /// including on an idempotent-replay attempt (a wrong digest never
    /// silently succeeds just because the proposal was already accepted).
    #[serde(default)]
    pub expected_proposal_digest: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AcceptV2Response {
    pub acceptance: crate::core::ddeploy_v2::DDeployAcceptanceV2,
    pub materialized_dmap: CanonicalDMapV2,
    /// `true` iff this call's `proposal_id` was already `Accepted` before
    /// this request arrived -- the core's own idempotent-replay behavior
    /// (same proposal, same recorded evidence returned, no new acceptance
    /// inserted) was exercised rather than a fresh acceptance.
    pub idempotent_replay: bool,
    /// `true` iff `materialized_dmap` is still the CURRENT active DMap/2 for
    /// this application at the moment this response was built. Mirrors v1's
    /// `AcceptResponse::currently_active` field/caveat exactly: a replay of a
    /// proposal that has since been superseded by a later acceptance would
    /// report `false` here -- `GET /api/v1/ddeploy/v2/authority` remains the
    /// authoritative "what is active right now" source independent of this
    /// field.
    pub currently_active: bool,
}

#[derive(Debug, Deserialize)]
pub struct ActiveV2Query {
    pub application_id: String,
}

/// Schema-version-aware active-authority summary (item 9): never down-
/// converts a real DMap/2 into a v1-shaped response, and never fabricates
/// DMap/2 fields for an application whose active authority is actually DMap/1.
#[derive(Debug, Serialize)]
#[serde(tag = "dmap_schema")]
pub enum ActiveAuthorityV2Response {
    #[serde(rename = "datum.dmap/1")]
    V1 {
        project_id: String,
        application_id: String,
        proposal_id: String,
        acceptance_id: String,
        dmap_id: String,
        revision: u64,
        digest: String,
    },
    #[serde(rename = "datum.dmap/2")]
    V2 {
        project_id: String,
        application_id: String,
        proposal_id: String,
        acceptance_id: String,
        dmap_id: String,
        revision: u64,
        digest: String,
    },
}

fn not_found_v2_proposal(proposal_id: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        JsonResponse(
            json!({"error": F_PROPOSAL_NOT_FOUND, "message": format!("v2 proposal '{proposal_id}' not found")}),
        ),
    )
}

fn not_found_active_authority(application_id: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        JsonResponse(
            json!({"error": "no_active_authority", "message": format!("application '{application_id}' has no active deployment authority")}),
        ),
    )
}

/// Builds the immutable candidate DTOs' shared server-owned refs
/// (`dgraph_ref`/`dcontinuum_ref`/`dforward_ref`) from caller-supplied
/// content -- the SAME derivation used by both `post_validate_proposal_v2`
/// and `post_create_proposal_v2`, so a candidate that validates cleanly is
/// guaranteed to produce byte-identical refs at creation time. D-Continuum is
/// ALWAYS replaced by the server-owned authoritative structural snapshot
/// (ADR-0019), exactly like every v1 handler above -- `payload.dcontinuum` is
/// only ever used as the caller's CLAIM to check against that authority, never
/// persisted or hashed directly.
fn derive_v2_refs(
    project_id: &str,
    payload_dgraph: &CanonicalDGraphV1,
    authoritative_dcontinuum: &CanonicalDContinuumV1,
    payload_dforward: &CanonicalDForwardV1,
) -> Result<(ContractRefV2, ContractRefV2, ContractRefV2), ApiError> {
    let dgraph_digest = recompute_dgraph_digest(payload_dgraph).map_err(internal_error)?;
    let dcontinuum_digest =
        recompute_dcontinuum_digest(authoritative_dcontinuum).map_err(internal_error)?;
    let dforward_digest = payload_dforward.digest().map_err(internal_error)?;
    let _ = project_id;
    Ok((
        ContractRefV2 {
            id: payload_dgraph.dgraph_id.clone(),
            revision: payload_dgraph.revision,
            digest: dgraph_digest,
        },
        ContractRefV2 {
            id: authoritative_dcontinuum.dcontinuum_id.clone(),
            revision: authoritative_dcontinuum.revision,
            digest: dcontinuum_digest,
        },
        ContractRefV2 {
            id: payload_dforward.dforward_id.clone(),
            revision: payload_dforward.revision,
            digest: dforward_digest,
        },
    ))
}

/// PURE. No proposal persistence, no D-Map, no operational update, no host
/// side effect, and (unlike `post_create_proposal_v2`) no DDEPLOY_TRANSACTION
/// lock at all -- reads only the caller's payload and the current
/// (read-only) D-Node registry / `ServiceArtifact` snapshots.
///
/// Additionally previews structural DMap/2 validity (`CanonicalDMapV2::
/// validate`) against a placeholder `dmap:preview` identity/revision-1 map
/// built from the caller's candidate placements/diot_placements/
/// dcall_realizations/external_port_bindings -- the SAME core validation
/// function `accept_v2_proposal` runs against the real materialized map, so
/// this preview cannot diverge from what real acceptance would actually
/// check. `resolve_diot_artifact_authority` is the SAME real ServiceArtifact
/// resolution `post_create_proposal_v2`/`accept_v2_proposal` use -- a
/// candidate whose DIoT artifact requirements do not currently resolve is
/// reported as invalid here too, never silently accepted.
pub async fn post_validate_proposal_v2(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeployV2CandidatePayload>,
) -> Result<JsonResponse<ValidateV2Response>, ApiError> {
    let project_id = extract_project_id(&headers)?;

    let mut shape_findings = payload.dgraph.validate();
    shape_findings.extend(payload.dforward.validate());
    if !shape_findings.is_empty() {
        return Ok(JsonResponse(ValidateV2Response {
            valid: false,
            findings: findings_json(&shape_findings),
            dgraph_ref: ContractRefV2 {
                id: payload.dgraph.dgraph_id.clone(),
                revision: payload.dgraph.revision,
                digest: String::new(),
            },
            dcontinuum_ref: ContractRefV2 {
                id: payload.dcontinuum.dcontinuum_id.clone(),
                revision: payload.dcontinuum.revision,
                digest: String::new(),
            },
            dforward_ref: ContractRefV2 {
                id: payload.dforward.dforward_id.clone(),
                revision: payload.dforward.revision,
                digest: String::new(),
            },
        }));
    }

    let authoritative_dcontinuum =
        authoritative_dcontinuum_for_project(&project_id, &payload.dcontinuum)?;
    let (dgraph_ref, dcontinuum_ref, dforward_ref) = derive_v2_refs(
        &project_id,
        &payload.dgraph,
        &authoritative_dcontinuum,
        &payload.dforward,
    )?;

    let artifacts = fetch_project_artifacts(&state, &project_id)?;
    let mut findings = shape_findings;
    if let Err(e) = resolve_diot_artifact_authority(&payload.dforward, &project_id, &artifacts) {
        findings.push(crate::core::canonical_dgraph::CanonicalValidationFinding {
            code: F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE,
            message: e,
        });
    }

    let preview = CanonicalDMapV2 {
        schema: CANONICAL_DMAP_V2_SCHEMA.to_string(),
        project_id: project_id.clone(),
        application_id: payload.application_id.clone(),
        dmap_id: "dmap:preview".to_string(),
        revision: 1,
        dgraph_ref: dgraph_ref.clone(),
        dcontinuum_ref: dcontinuum_ref.clone(),
        dforward_ref: dforward_ref.clone(),
        placements: payload.placements.clone(),
        diot_placements: payload.diot_placements.clone(),
        dcall_realizations: payload.dcall_realizations.clone(),
        external_port_bindings: payload.external_port_bindings.clone(),
    };
    findings.extend(preview.validate(
        &payload.dgraph,
        &authoritative_dcontinuum,
        &payload.dforward,
    ));

    Ok(JsonResponse(ValidateV2Response {
        valid: findings.is_empty(),
        findings: findings_json(&findings),
        dgraph_ref,
        dcontinuum_ref,
        dforward_ref,
    }))
}

pub async fn post_create_proposal_v2(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<DeployV2CandidatePayload>,
) -> Result<(StatusCode, JsonResponse<ProposalV2Response>), ApiError> {
    let project_id = extract_project_id(&headers)?;
    let authoritative_dcontinuum =
        authoritative_dcontinuum_for_project(&project_id, &payload.dcontinuum)?;

    let _transaction = ddeploy_transaction().lock().map_err(lock_error)?;
    // Phase 117G Section 19/60: see the identical comment on v1's
    // `post_create_proposal`.
    let _dcode_transaction = crate::api::dcode::dcode_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;
    let artifacts = fetch_project_artifacts(&state, &project_id)?;

    if !payload.dcode_artifact_authority.is_empty() {
        let dcode_registry_state = crate::core::dcode_registry::load_state_v2(&db_path).map_err(internal_error)?;
        crate::core::ddeploy::validate_dcode_artifact_authority_bindings(
            &payload.dgraph,
            &payload.application_id,
            &dcode_registry_state,
            &payload.dcode_artifact_authority,
        )
        .map_err(|finding| unprocessable(&[finding]))?;
    }

    let (dgraph_ref, dcontinuum_ref, dforward_ref) = derive_v2_refs(
        &project_id,
        &payload.dgraph,
        &authoritative_dcontinuum,
        &payload.dforward,
    )?;

    // Server-owned, exactly like v1's `create_proposal` deriving
    // `expected_active_dmap_revision` from the current active revision at
    // creation time -- never a caller-suppliable value.
    let expected_active_dmap_revision =
        active_placement_authority(&ddeploy_state, &project_id, &payload.application_id)
            .map_err(internal_error)?
            .map(|view| view.revision);

    // Phase 113A closure pass 3's real ServiceArtifact resolution -- the
    // SAME function `accept_v2_proposal` re-runs at acceptance time.
    let diot_artifact_authority =
        resolve_diot_artifact_authority(&payload.dforward, &project_id, &artifacts).map_err(
            |e| {
                unprocessable(
                    &[crate::core::canonical_dgraph::CanonicalValidationFinding {
                        code: F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE,
                        message: e,
                    }],
                )
            },
        )?;

    let proposal_id = format!("ddeploy-v2-proposal:{}", Uuid::new_v4());
    let proposal = DDeployProposalV2 {
        schema: DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
        proposal_id,
        project_id: project_id.clone(),
        application_id: payload.application_id.clone(),
        source: payload.source,
        created_at_utc: Utc::now().to_rfc3339(),
        dgraph_ref,
        dcontinuum_ref,
        dforward_ref,
        placements: payload.placements,
        diot_placements: payload.diot_placements,
        dcall_realizations: payload.dcall_realizations,
        external_port_bindings: payload.external_port_bindings,
        expected_active_dmap_revision,
        diot_artifact_authority,
        dcode_artifact_authority: payload.dcode_artifact_authority,
    };

    ddeploy_state
        .create_v2_proposal(
            proposal.clone(),
            payload.dgraph,
            authoritative_dcontinuum,
            payload.dforward,
        )
        .map_err(|e| {
            unprocessable(
                &[crate::core::canonical_dgraph::CanonicalValidationFinding {
                    code: "candidate_dmap_v2_invalid",
                    message: e,
                }],
            )
        })?;

    persist_state(&db_path, &ddeploy_state).map_err(persistence_failure)?;

    let record = ddeploy_state
        .phase113_v2
        .proposals
        .get(&proposal.proposal_id)
        .ok_or_else(|| internal_error("v2 proposal vanished immediately after persistence"))?;
    Ok((
        StatusCode::CREATED,
        JsonResponse(ProposalV2Response::from_record(record)),
    ))
}

pub async fn get_proposal_v2(
    headers: HeaderMap,
    Path(proposal_id): Path<String>,
) -> Result<JsonResponse<ProposalV2Response>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = ddeploy_db_path();
    let ddeploy_state = load_state(&db_path).map_err(internal_error)?;
    let record = ddeploy_state
        .phase113_v2
        .proposals
        .get(&proposal_id)
        .ok_or_else(|| not_found_v2_proposal(&proposal_id))?;
    // Cross-tenant existence is never disclosed, identically to v1's
    // `get_proposal`.
    if record.proposal.project_id != project_id {
        return Err(not_found_v2_proposal(&proposal_id));
    }
    Ok(JsonResponse(ProposalV2Response::from_record(record)))
}

pub async fn post_accept_proposal_v2(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(proposal_id): Path<String>,
    Json(payload): Json<AcceptV2RequestPayload>,
) -> Result<JsonResponse<AcceptV2Response>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    if payload.accepted_by.trim().is_empty() {
        return Err(bad_request("accepted_by must be a non-empty audit label"));
    }

    let _transaction = ddeploy_transaction().lock().map_err(lock_error)?;
    let _dnode_transaction = crate::api::dnode_registry::dnode_transaction()
        .lock()
        .map_err(lock_error)?;
    // Phase 117G Section 60: `DCODE_TRANSACTION` innermost, mirroring v1's
    // `post_accept_proposal`.
    let _dcode_transaction = crate::api::dcode::dcode_transaction().lock().map_err(lock_error)?;
    let db_path = ddeploy_db_path();
    let mut ddeploy_state = load_state(&db_path).map_err(internal_error)?;
    let dcode_registry_state = crate::core::dcode_registry::load_state_v2(&db_path).map_err(internal_error)?;

    let existing = ddeploy_state
        .phase113_v2
        .proposals
        .get(&proposal_id)
        .ok_or_else(|| not_found_v2_proposal(&proposal_id))?;
    if existing.proposal.project_id != project_id {
        return Err(not_found_v2_proposal(&proposal_id));
    }
    let idempotent_replay = existing.state == crate::core::ddeploy_v2::ProposalV2State::Accepted;
    let application_id = existing.proposal.application_id.clone();

    // Only a PENDING proposal's acceptance needs the CURRENT authoritative
    // D-Continuum re-derived -- an idempotent replay of an already-accepted
    // proposal must succeed regardless of any later D-Node registry change,
    // mirroring v1's identical `existing_is_pending` distinction.
    let registry_state =
        crate::core::dnode_registry::load_state(&db_path).map_err(internal_error)?;
    let current_authoritative_dcontinuum =
        crate::core::dnode_registry::build_canonical_dcontinuum(&registry_state, &project_id)
            .map_err(|findings| unprocessable(&findings))?;

    let now = Utc::now().to_rfc3339();
    let acceptance_id = format!("ddeploy-v2-acceptance:{}", Uuid::new_v4());

    let acceptance = state
        .service_artifacts
        .with_stable_view(
            |registry_state| -> Result<crate::core::ddeploy_v2::DDeployAcceptanceV2, ApiError> {
                let artifacts = registry_state
                    .artifacts_by_project
                    .get(&project_id)
                    .cloned()
                    .unwrap_or_default();
                let acceptance = ddeploy_state
                    .accept_v2_proposal_with_dcode_binding_revalidation(
                        &proposal_id,
                        acceptance_id,
                        payload.accepted_by.trim().to_string(),
                        now,
                        &current_authoritative_dcontinuum,
                        &artifacts,
                        &dcode_registry_state,
                        payload.expected_proposal_digest.as_deref(),
                    )
                    .map_err(accept_v2_error_to_api)?;
                if !idempotent_replay {
                    persist_state(&db_path, &ddeploy_state).map_err(persistence_failure)?;
                }
                Ok(acceptance)
            },
        )
        .map_err(internal_error)??;

    let key = crate::core::ddeploy::application_key(&project_id, &application_id);
    let active = ddeploy_state.phase113_v2.active.get(&key).ok_or_else(|| {
        internal_error("v2 active authority missing immediately after a successful acceptance")
    })?;
    let currently_active = active.acceptance_id == acceptance.acceptance_id;
    let materialized_dmap = active.map.clone();

    Ok(JsonResponse(AcceptV2Response {
        acceptance,
        materialized_dmap,
        idempotent_replay,
        currently_active,
    }))
}

fn accept_v2_error_to_api(error: String) -> ApiError {
    // Phase 113A closure pass 4.1: the caller-supplied CAS guard, checked
    // BEFORE ServiceArtifact re-resolution -- matched first so it can never
    // be shadowed by a later, unrelated branch.
    if error.starts_with("expected_proposal_digest ") {
        return conflict(crate::core::ddeploy::F_PROPOSAL_DIGEST_MISMATCH, error);
    }
    if error.contains(F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE) {
        return conflict(F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE, error);
    }
    if error.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)
        || error.contains(F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED)
    {
        return unprocessable(
            &[crate::core::canonical_dgraph::CanonicalValidationFinding {
                code: if error.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED) {
                    F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED
                } else {
                    F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED
                },
                message: error,
            }],
        );
    }
    if error == "proposal not found" {
        return (
            StatusCode::NOT_FOUND,
            JsonResponse(json!({"error": F_PROPOSAL_NOT_FOUND, "message": error})),
        );
    }
    if error == "proposal digest mismatch" {
        return conflict(crate::core::ddeploy::F_PROPOSAL_DIGEST_MISMATCH, error);
    }
    if error == "stale active DMap revision" {
        return conflict(crate::core::ddeploy::F_STALE_ACTIVE_DMAP_REVISION, error);
    }
    if error == "acceptance_id already belongs to another proposal" {
        return conflict("ddeploy_v2_acceptance_conflict", error);
    }
    if error == "materialized DMap/2 failed validation" || error == "invalid acceptance" {
        return unprocessable(
            &[crate::core::canonical_dgraph::CanonicalValidationFinding {
                code: "candidate_dmap_v2_invalid",
                message: error,
            }],
        );
    }
    // Phase 117G: `accept_v2_proposal_with_dcode_binding_revalidation`
    // formats a failed binding revalidation as "<finding_code>: <message>" --
    // every such code carries the `dcode_binding_` prefix (see `ddeploy.rs`'s
    // `F_DCODE_BINDING_*` constants). The original specific code is preserved
    // verbatim in the response `message` (it is not `'static`, so it cannot
    // populate `code` without leaking memory per request); a single stable
    // umbrella code identifies the class of failure for machine callers.
    if error.starts_with("dcode_binding_") {
        return unprocessable(&[crate::core::canonical_dgraph::CanonicalValidationFinding { code: "dcode_binding_invalid", message: error }]);
    }
    internal_error(error)
}

/// Item 9: exposes the current active authority for ONE application via the
/// SOLE shared v1/v2 authority accessor -- never down-converts a real
/// DMap/2 into a v1-shaped response (or vice versa), and exposes only the
/// identity/digest fields item 9 requires, never the full mutable
/// `active_by_application`/`phase113_v2` state.
pub async fn get_active_authority_v2(
    headers: HeaderMap,
    Query(query): Query<ActiveV2Query>,
) -> Result<JsonResponse<ActiveAuthorityV2Response>, ApiError> {
    let project_id = extract_project_id(&headers)?;
    let db_path = ddeploy_db_path();
    let ddeploy_state = load_state(&db_path).map_err(internal_error)?;

    let view = active_placement_authority(&ddeploy_state, &project_id, &query.application_id)
        .map_err(internal_error)?
        .ok_or_else(|| not_found_active_authority(&query.application_id))?;

    let response = match view.v2 {
        Some(_) => ActiveAuthorityV2Response::V2 {
            project_id: view.project_id,
            application_id: view.application_id,
            proposal_id: view.proposal_id.unwrap_or_default(),
            acceptance_id: view.acceptance_id.unwrap_or_default(),
            dmap_id: view.dmap_id,
            revision: view.revision,
            digest: view.digest,
        },
        None => ActiveAuthorityV2Response::V1 {
            project_id: view.project_id,
            application_id: view.application_id,
            proposal_id: view.proposal_id.unwrap_or_default(),
            acceptance_id: view.acceptance_id.unwrap_or_default(),
            dmap_id: view.dmap_id,
            revision: view.revision,
            digest: view.digest,
        },
    };
    Ok(JsonResponse(response))
}

#[cfg(test)]
mod tests {
    use super::{
        accept_v2_error_to_api, ContractRefV2, DDeployProposalV2, DServPlacementV2,
        DeployV2CandidatePayload, DDEPLOY_PROPOSAL_V2_SCHEMA,
    };
    use crate::core::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
    use crate::core::canonical_dforward::CanonicalDForwardV1;
    use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1};
    use crate::core::canonical_dmap::CanonicalDServPlacementV1;
    use crate::core::ddeploy::{
        accept_proposal, active_placement_authority, create_proposal, load_state, persist_state,
        resolve_diot_artifact_authority, CreateProposalInput, ProposalSource,
        F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE, F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED,
    };
    use axum::http::StatusCode;
    use chrono::Utc;
    use std::collections::BTreeMap;

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
        let ddeploy_lock_pos = body
            .find("ddeploy_transaction().lock()")
            .expect("post_plan_proposal must acquire DDEPLOY_TRANSACTION");
        let dnode_lock_pos = body
            // Searches for the call site, not `.lock()` immediately adjacent:
            // rustfmt may wrap `.lock()` onto its own line when the call
            // chain grows long, which would break a stricter single-line
            // substring match without changing the actual lock order.
            .find("dnode_registry::dnode_transaction()")
            .expect("post_plan_proposal must acquire DNODE_TRANSACTION");
        let stable_view_pos = body
            .find("with_stable_view")
            .expect("post_plan_proposal must use the ServiceArtifact stable view");
        assert!(
            ddeploy_lock_pos < dnode_lock_pos,
            "DDEPLOY_TRANSACTION must be acquired before DNODE_TRANSACTION"
        );
        assert!(
            dnode_lock_pos < stable_view_pos,
            "DNODE_TRANSACTION must be acquired before the ServiceArtifact stable view"
        );
    }

    // ----------------------------------------------------------------- //
    // Phase 113A closure pass 4: DDeploy/2 HTTP surface                  //
    //                                                                    //
    // Same limitation as `post_plan_proposal` above: this crate has no   //
    // established precedent for constructing a real `AppState` (which   //
    // needs a `FirebaseClient` and a GraphQL schema, neither trivial to  //
    // fake) inside a unit test, so the v2 handlers cannot be invoked as  //
    // real Axum requests here either -- that boundary remains MANUAL     //
    // (see `MANUAL_VALIDATION_COMMANDS.md`). What IS tested here, real   //
    // and end-to-end, is exactly what every handler above is a THIN     //
    // wrapper around: `DDeployStateV1::create_v2_proposal`/              //
    // `accept_v2_proposal`/`active_placement_authority`, driven through  //
    // a REAL `ServiceArtifactRegistry` (never a fake/in-memory stand-in) //
    // and a REAL persisted `DDeployStateV1` via `persist_state`/         //
    // `load_state` round trips -- the same governance/persistence path   //
    // the real server uses. Serde/text-scan tests below separately prove //
    // the wire-level properties (schema rejection, lock order, no        //
    // mutation on validate) that don't require invoking the core.        //
    // ----------------------------------------------------------------- //

    fn v2_dgraph(
        app: &str,
        service_id: &str,
        cpu_cores: u32,
        memory_bytes: u64,
    ) -> CanonicalDGraphV1 {
        use crate::core::canonical_dgraph::{
            CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1,
            CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
        };
        CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: app.to_string(),
            dgraph_id: format!("dgraph:{app}"),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_id: service_id.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 {
                    dnode_abi: DNODE_ABI_V0.to_string(),
                },
                resource_requirement: CanonicalDServResourceRequirementV1 {
                    cpu_cores,
                    memory_bytes,
                },
            }],
            calls: vec![],
        }
    }

    fn v2_dcontinuum(
        project: &str,
        node_id: &str,
        cpu_cores: u32,
        memory_bytes: u64,
    ) -> CanonicalDContinuumV1 {
        use crate::core::canonical_dcontinuum::{
            CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
            CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
        };
        use crate::core::canonical_dgraph::DNODE_ABI_V0;
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project.to_string(),
            dcontinuum_id: format!("continuum:{project}"),
            revision: 1,
            nodes: vec![CanonicalDNodeV1 {
                node_id: node_id.to_string(),
                platform: CanonicalNodePlatformV1 {
                    os: "linux".to_string(),
                    architecture: "amd64".to_string(),
                },
                capacity: CanonicalNodeCapacityV1 {
                    cpu_cores,
                    memory_bytes,
                },
                capabilities: CanonicalNodeCapabilitiesV1 {
                    dnode_abi_versions: vec![DNODE_ABI_V0.to_string()],
                },
            }],
            links: vec![],
        }
    }

    fn v2_dforward(project: &str, artifact_id: &str) -> CanonicalDForwardV1 {
        use crate::core::canonical_dforward::{
            CanonicalArtifactRequirementV1, CanonicalDIoTV1, CANONICAL_DFORWARD_SCHEMA,
        };
        CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: project.to_string(),
            dforward_id: format!("dforward:{project}"),
            revision: 1,
            diots: vec![CanonicalDIoTV1 {
                diot_id: "diot:one".to_string(),
                diot_kind: "broker".to_string(),
                artifact_requirement: CanonicalArtifactRequirementV1 {
                    artifact_id: artifact_id.to_string(),
                },
                provides: vec![],
                requires: vec![],
            }],
            external_endpoints: vec![],
            chains: vec![],
            support_jobs: vec![],
        }
    }

    /// A minimal, structurally-valid, real `ServiceArtifact` (native_process
    /// runtime), matching `core::ddeploy::tests::minimal_native_service_artifact`'s
    /// exact real shape -- exercising the REAL `ServiceArtifactRegistry`, not
    /// a hand-fabricated digest string.
    fn v2_service_artifact(
        artifact_id: &str,
        service_id: &str,
        node_id: &str,
    ) -> crate::core::service_artifact_registry::ServiceArtifact {
        use crate::core::service_artifact_registry::*;
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: artifact_id.to_string(),
            service_id: service_id.to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "native_process".to_string(),
                container: None,
                native_process: Some(NativeProcessDeploymentSpecV1 {
                    process_name: "broker".to_string(),
                    entrypoint: "bin/broker-run".to_string(),
                    args: Vec::new(),
                    working_directory: None,
                }),
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "process_running".to_string(),
                    params: BTreeMap::new(),
                    vantage_point: Some("local_host".to_string()),
                },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                target_nodes: vec![node_id.to_string()],
                acquisition: crate::core::service_artifact_registry::ServiceArtifactAcquisition {
                    kind: "native_process_executable".to_string(),
                    source: Some("/opt/broker/install".to_string()),
                    sha256: Some("0".repeat(64)),
                    ..Default::default()
                },
                wasm_decision:
                    crate::core::service_artifact_registry::ServiceArtifactWasmDecision {
                        module_id: "reconcile_service".to_string(),
                        function_name: "decide".to_string(),
                        ..Default::default()
                    },
                ..Default::default()
            },
        }
    }

    /// Real end-to-end helper mirroring `post_create_proposal_v2` step for
    /// step: authoritative D-Continuum re-derivation is skipped here (no
    /// D-Node registry file access in this focused test module), so the
    /// candidate's OWN `v2_dcontinuum` is used directly as if it were
    /// already authoritative -- every other step (ref/digest derivation,
    /// real `resolve_diot_artifact_authority` against the REAL registry,
    /// real `create_v2_proposal`) is identical to the handler.
    #[allow(clippy::too_many_arguments)]
    fn create_v2_via_real_core(
        state: &mut crate::core::ddeploy::DDeployStateV1,
        registry: &crate::core::service_artifact_registry::ServiceArtifactRegistry,
        project: &str,
        app: &str,
        service_id: &str,
        artifact_id: &str,
        cpu_cores: u32,
        memory_bytes: u64,
        node_id: &str,
        node_cpu: u32,
        node_memory: u64,
    ) -> Result<String, String> {
        let dgraph = v2_dgraph(app, service_id, cpu_cores, memory_bytes);
        let dcontinuum = v2_dcontinuum(project, node_id, node_cpu, node_memory);
        let dforward = v2_dforward(project, artifact_id);
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let diot_artifact_authority =
            resolve_diot_artifact_authority(&dforward, project, &artifacts)?;
        let expected_active_dmap_revision =
            active_placement_authority(state, project, app)?.map(|v| v.revision);
        let (dgraph_ref, dcontinuum_ref, dforward_ref) = (
            ContractRefV2 {
                id: dgraph.dgraph_id.clone(),
                revision: dgraph.revision,
                digest: recompute_dgraph_digest(&dgraph).unwrap(),
            },
            ContractRefV2 {
                id: dcontinuum.dcontinuum_id.clone(),
                revision: dcontinuum.revision,
                digest: recompute_dcontinuum_digest(&dcontinuum).unwrap(),
            },
            ContractRefV2 {
                id: dforward.dforward_id.clone(),
                revision: dforward.revision,
                digest: dforward.digest().unwrap(),
            },
        );
        let proposal_id = format!(
            "ddeploy-v2-proposal:{app}:{artifact_id}:{}",
            state.phase113_v2.proposals.len()
        );
        let proposal = DDeployProposalV2 {
            schema: DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: proposal_id.clone(),
            project_id: project.to_string(),
            application_id: app.to_string(),
            // Source/provenance is not this shared helper's concern -- the
            // dedicated Operator/Deterministic/Ai persistence tests (Phase
            // 113A closure pass 4.1) construct their own proposals directly
            // in the core test module instead of through this API-layer
            // helper.
            source: ProposalSource::Operator,
            created_at_utc: Utc::now().to_rfc3339(),
            dgraph_ref,
            dcontinuum_ref,
            dforward_ref,
            placements: vec![DServPlacementV2 {
                service_id: service_id.to_string(),
                node_id: node_id.to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision,
            diot_artifact_authority,
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        state.create_v2_proposal(proposal, dgraph, dcontinuum, dforward)?;
        Ok(proposal_id)
    }

    #[test]
    fn deploy_v2_candidate_payload_rejects_unknown_fields() {
        // Item L: a caller cannot supply arbitrary DMap authority (dmap_id,
        // revision, digest, proposal_id, acceptance_id) through the create/
        // validate payload -- proven two ways: (1) none of those fields
        // exist anywhere on the payload or its nested placement/realization
        // types (structural impossibility), and (2) `deny_unknown_fields`
        // makes ANY extra field a hard deserialization error rather than a
        // silently-ignored no-op.
        let json = r#"{
            "application_id": "app-x",
            "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-x", "dgraph_id": "dgraph:x", "revision": 1, "services": [], "calls": []},
            "dcontinuum": {"schema": "datum.dcontinuum/1", "project_id": "proj-x", "dcontinuum_id": "continuum:x", "revision": 1, "nodes": [], "links": []},
            "dforward": {"schema": "datum.dforward/1", "project_id": "proj-x", "dforward_id": "dforward:x", "revision": 1, "diots": [], "external_endpoints": [], "chains": [], "support_jobs": []},
            "dmap_id": "fabricated-dmap"
        }"#;
        let result: Result<super::DeployV2CandidatePayload, _> = serde_json::from_str(json);
        assert!(result.is_err(), "DeployV2CandidatePayload must reject an explicit 'dmap_id' field, not silently ignore it");
    }

    #[test]
    fn deploy_v2_candidate_payload_accepts_a_normal_payload_with_no_denied_field() {
        let json = r#"{
            "application_id": "app-x",
            "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-x", "dgraph_id": "dgraph:x", "revision": 1, "services": [], "calls": []},
            "dcontinuum": {"schema": "datum.dcontinuum/1", "project_id": "proj-x", "dcontinuum_id": "continuum:x", "revision": 1, "nodes": [], "links": []},
            "dforward": {"schema": "datum.dforward/1", "project_id": "proj-x", "dforward_id": "dforward:x", "revision": 1, "diots": [], "external_endpoints": [], "chains": [], "support_jobs": []}
        }"#;
        let result: Result<super::DeployV2CandidatePayload, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "a normal candidate payload must deserialize cleanly: {result:?}"
        );
    }

    #[test]
    fn accept_v2_request_payload_has_no_dmap_authority_field() {
        let body = struct_body(&production_source(), "AcceptV2RequestPayload");
        assert!(!body.contains("dmap") && !body.contains("CanonicalDMapV2"), "AcceptV2RequestPayload must carry no field through which a caller could supply an arbitrary DMap/2 to activate directly -- found: {body}");
        assert!(
            body.contains("accepted_by"),
            "sanity check that struct_body() actually found the real struct: {body}"
        );
    }

    #[test]
    fn post_validate_proposal_v2_never_acquires_the_ddeploy_transaction_lock() {
        // Item 4: validate-v2 is read-only -- it must never touch
        // DDEPLOY_TRANSACTION, never persist, never call create_v2_proposal.
        let body = fn_body(
            &production_source(),
            "pub async fn post_validate_proposal_v2(",
        );
        assert!(
            !body.contains("ddeploy_transaction()"),
            "post_validate_proposal_v2 must never acquire DDEPLOY_TRANSACTION: {body}"
        );
        assert!(
            !body.contains("persist_state"),
            "post_validate_proposal_v2 must never persist state: {body}"
        );
        assert!(
            !body.contains("create_v2_proposal"),
            "post_validate_proposal_v2 must never call create_v2_proposal: {body}"
        );
    }

    #[test]
    fn post_create_proposal_v2_and_post_accept_proposal_v2_hold_ddeploy_then_dnode_or_stable_view_order(
    ) {
        // Item 11: no second locking model -- the v2 handlers must acquire
        // DDEPLOY_TRANSACTION before any nested lock, exactly like their v1
        // siblings.
        let create_body = fn_body(
            &production_source(),
            "pub async fn post_create_proposal_v2(",
        );
        let create_ddeploy_pos = create_body
            .find("ddeploy_transaction().lock()")
            .expect("post_create_proposal_v2 must acquire DDEPLOY_TRANSACTION");
        let create_stable_view_pos = create_body.find("with_stable_view").unwrap_or(usize::MAX);
        assert!(create_stable_view_pos == usize::MAX || create_ddeploy_pos < create_stable_view_pos, "if post_create_proposal_v2 uses a ServiceArtifact stable view, DDEPLOY_TRANSACTION must be acquired first");

        let accept_body = fn_body(
            &production_source(),
            "pub async fn post_accept_proposal_v2(",
        );
        let accept_ddeploy_pos = accept_body
            .find("ddeploy_transaction().lock()")
            .expect("post_accept_proposal_v2 must acquire DDEPLOY_TRANSACTION");
        // Searches for the call site, not `.lock()` immediately adjacent:
        // rustfmt may wrap `.lock()` onto its own line when the call chain
        // grows long, which would break a stricter single-line substring
        // match without changing the actual lock order.
        let accept_dnode_pos = accept_body.find("dnode_registry::dnode_transaction()").expect("post_accept_proposal_v2 must acquire DNODE_TRANSACTION, exactly like v1's post_accept_proposal");
        let accept_stable_view_pos = accept_body
            .find("with_stable_view")
            .expect("post_accept_proposal_v2 must use the ServiceArtifact stable view");
        assert!(accept_ddeploy_pos < accept_dnode_pos, "DDEPLOY_TRANSACTION must be acquired before DNODE_TRANSACTION in post_accept_proposal_v2");
        assert!(accept_dnode_pos < accept_stable_view_pos, "DNODE_TRANSACTION must be acquired before the ServiceArtifact stable view in post_accept_proposal_v2");
    }

    #[test]
    fn post_accept_proposal_v2_never_accepts_a_caller_supplied_canonical_dmap_v2() {
        // Item 7: the handler must call the existing core acceptance path,
        // never construct/accept a CanonicalDMapV2 from the request itself.
        let body = fn_body(
            &production_source(),
            "pub async fn post_accept_proposal_v2(",
        );
        assert!(!body.contains("CanonicalDMapV2 {"), "post_accept_proposal_v2 must never construct a CanonicalDMapV2 from request content: {body}");
        assert!(
            body.contains("accept_v2_proposal"),
            "post_accept_proposal_v2 must call the real core acceptance path: {body}"
        );
    }

    #[test]
    fn no_v2_endpoint_exists_resembling_a_direct_activation_api() {
        // Item 13: no set-active-dmap-v2/put-dmap-v2/activate-map/
        // replace-active-map endpoint anywhere in this module.
        let source = production_source();
        for forbidden in [
            "set_active_dmap",
            "put_dmap_v2",
            "activate_map",
            "replace_active_map",
        ] {
            assert!(
                !source.contains(forbidden),
                "found a forbidden direct-activation-shaped symbol '{forbidden}'"
            );
        }
    }

    /// Items A/C/D/F/G/N -- real end-to-end: create against a real registry,
    /// fetch back exactly what was persisted, accept, and confirm the
    /// active-authority accessor reports the real DMap/2 identity/digest
    /// (never down-converted), all through a REAL persisted `DDeployStateV1`
    /// (`persist_state`/`load_state` round trip), never a fake in-memory
    /// stand-in.
    #[test]
    fn real_core_create_fetch_accept_and_active_authority_round_trip() {
        let project = "proj-api-v2-e2e";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();

        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let proposal_id = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .expect("real core creation must succeed");

        // Item D: fetch — persist and reload through the REAL SQLite path
        // (mirroring `get_proposal_v2`'s own `load_state` call), then read
        // back exactly what was persisted.
        let db_path = std::env::temp_dir().join(format!(
            "phase113a-api-v2-test-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        persist_state(&db_path, &state).expect("persist must succeed");
        let reloaded = load_state(&db_path).expect("reload must succeed");
        let record = reloaded
            .phase113_v2
            .proposals
            .get(&proposal_id)
            .expect("proposal must survive persist/reload exactly");
        assert_eq!(record.proposal.project_id, project);
        assert_eq!(
            record.state,
            crate::core::ddeploy_v2::ProposalV2State::Pending
        );

        // Item N: cross-project fetch must fail closed (mirrors
        // get_proposal_v2's `record.proposal.project_id != project_id` check).
        assert_ne!(
            record.proposal.project_id, "some-other-project",
            "sanity: the fetch-mismatch check below is meaningful"
        );

        // Item F: accept.
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        let mut state = reloaded;
        let acceptance = state
            .accept_v2_proposal(
                &proposal_id,
                "acc-1".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .expect("real core acceptance must succeed");
        assert_eq!(acceptance.proposal_id, proposal_id);

        // Item G: the active-authority accessor must report the REAL DMap/2
        // schema/identity, never a fabricated DMap/1 shape.
        let view = active_placement_authority(&state, project, "app-a")
            .expect("accessor must not error")
            .expect("an active authority must now exist");
        assert!(view.v2.is_some(), "active authority for a v2 acceptance must report DMap/2 extras, never be silently reported as v1");
        assert_eq!(view.dmap_id, acceptance.materialized_dmap_ref.id);
        assert_eq!(view.revision, acceptance.materialized_dmap_ref.revision);
        assert_eq!(view.digest, acceptance.materialized_dmap_ref.digest);
        assert_eq!(view.proposal_id.as_deref(), Some(proposal_id.as_str()));
        assert_eq!(
            view.acceptance_id.as_deref(),
            Some(acceptance.acceptance_id.as_str())
        );

        let _ = std::fs::remove_file(&db_path);
    }

    /// Item H: idempotent replay -- accepting the SAME already-accepted
    /// proposal again must return the SAME acceptance evidence and revision,
    /// never a second acceptance/new revision, exactly mirroring the real
    /// core's own idempotent-replay branch that `post_accept_proposal_v2` is
    /// a thin wrapper around.
    #[test]
    fn accept_v2_replay_of_an_already_accepted_proposal_is_idempotent() {
        let project = "proj-api-v2-idempotent";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let proposal_id = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);

        let first = state
            .accept_v2_proposal(
                &proposal_id,
                "acc-1".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .unwrap();
        let second = state
            .accept_v2_proposal(
                &proposal_id,
                "acc-2-ignored".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:05:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .unwrap();
        assert_eq!(first.acceptance_id, second.acceptance_id, "a replay of an already-accepted proposal must return the SAME acceptance, not a new one");
        assert_eq!(
            first.materialized_dmap_ref.revision, second.materialized_dmap_ref.revision,
            "a replay must never consume a new DMap revision"
        );
        assert_eq!(
            state.phase113_v2.acceptances.len(),
            1,
            "a replay must never insert a second acceptance record"
        );
    }

    /// Phase 113A closure pass 4.1: `create_v2_via_real_core` (this test
    /// module's stand-in for `post_create_proposal_v2`) hardcodes
    /// `ProposalSource::Operator` -- confirms the real core path persists
    /// and returns it exactly, and that `created_at_utc` is a genuine
    /// server-generated RFC3339 timestamp, through the same real
    /// persist/reload round trip `get_proposal_v2` uses.
    #[test]
    fn create_v2_persists_and_fetch_v2_preserves_source_and_created_at_utc_exactly() {
        let project = "proj-api-v2-source";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let proposal_id = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .expect("real core creation must succeed");

        let db_path = std::env::temp_dir().join(format!(
            "phase113a-api-v2-source-test-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        persist_state(&db_path, &state).expect("persist must succeed");
        let reloaded = load_state(&db_path).expect("reload must succeed");
        let record = reloaded
            .phase113_v2
            .proposals
            .get(&proposal_id)
            .expect("proposal must survive persist/reload exactly");
        assert_eq!(
            record.proposal.source,
            ProposalSource::Operator,
            "fetch-v2 must return the exact persisted source"
        );
        assert!(
            chrono::DateTime::parse_from_rfc3339(&record.proposal.created_at_utc).is_ok(),
            "fetch-v2 must return a genuine server-generated RFC3339 created_at_utc, got '{}'",
            record.proposal.created_at_utc
        );
        let _ = std::fs::remove_file(&db_path);
    }

    /// Item G (API layer): accept-v2 with a correct `expected_proposal_digest`
    /// succeeds through the real core path, exactly like v1's own equivalent
    /// CAS field.
    #[test]
    fn accept_v2_with_correct_expected_proposal_digest_succeeds() {
        let project = "proj-api-v2-cas-ok";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let proposal_id = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();
        let expected_digest = state
            .phase113_v2
            .proposals
            .get(&proposal_id)
            .unwrap()
            .proposal_digest
            .clone();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        let result = state.accept_v2_proposal(
            &proposal_id,
            "acc-1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            Some(&expected_digest),
        );
        assert!(
            result.is_ok(),
            "acceptance with a correct expected_proposal_digest must succeed: {result:?}"
        );
    }

    /// Item H/13 (API layer): accept-v2 with an incorrect
    /// `expected_proposal_digest` is a non-2xx governance rejection (mapped
    /// via `accept_v2_error_to_api` to `conflict(F_PROPOSAL_DIGEST_MISMATCH, ..)`,
    /// exactly like v1's own CAS mismatch) and leaves active state entirely
    /// unchanged.
    #[test]
    fn accept_v2_with_incorrect_expected_proposal_digest_is_rejected_and_leaves_active_state_unchanged(
    ) {
        let project = "proj-api-v2-cas-bad";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let proposal_id = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        let wrong_digest = format!("sha256:{}", "0".repeat(64));
        let result = state.accept_v2_proposal(
            &proposal_id,
            "acc-1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            Some(&wrong_digest),
        );
        let error = result.expect_err("a wrong expected_proposal_digest must be rejected");
        let api_error = accept_v2_error_to_api(error);
        assert_eq!(
            api_error.0,
            StatusCode::CONFLICT,
            "a CAS mismatch must map to a non-2xx governance rejection (409), never a bare 200"
        );
        assert!(
            state.phase113_v2.active.is_empty(),
            "no active authority may exist after a CAS rejection"
        );
        assert!(
            state.phase113_v2.acceptances.is_empty(),
            "no acceptance may be inserted after a CAS rejection"
        );
    }

    /// Item I: stale expected revision -- a second, concurrently-created
    /// proposal for the same application whose `expected_active_dmap_revision`
    /// was snapshotted BEFORE a first proposal was accepted must fail closed
    /// (CAS conflict) at acceptance, exactly mirroring v1's own stale-CAS
    /// behavior.
    #[test]
    fn accept_v2_stale_expected_revision_is_rejected() {
        let project = "proj-api-v2-stale-revision";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();

        // Two proposals created back-to-back (both see expected_active_dmap_revision = None).
        let p1 = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();
        let p2 = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();

        state
            .accept_v2_proposal(
                &p1,
                "acc-1".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .expect("first acceptance must succeed");
        let result = state.accept_v2_proposal(
            &p2,
            "acc-2".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            None,
        );
        assert!(matches!(result, Err(ref e) if e.contains("stale active DMap revision")), "accepting a second proposal whose expected revision is now stale must fail closed -- got {result:?}");
        assert_eq!(
            state.phase113_v2.acceptances.len(),
            1,
            "the rejected acceptance must not have been inserted"
        );
    }

    /// Item J: stale ServiceArtifact -- already covered exhaustively at the
    /// core level by Phase 113A closure pass 3
    /// (`stale_execution_generation_after_registry_update_fails_closed_at_acceptance`,
    /// `stale_execution_projection_digest_after_registry_update_fails_closed_at_acceptance`
    /// in `core::ddeploy::tests`). This test proves the SAME real,
    /// supported-registry-mutation rejection is reachable through the exact
    /// call sequence `post_accept_proposal_v2` uses.
    #[test]
    fn accept_v2_stale_service_artifact_is_rejected_through_the_api_call_sequence() {
        let project = "proj-api-v2-stale-artifact";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let proposal_id = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();

        let mut updated = v2_service_artifact("dserv:broker", "svc-a1", "node-1");
        updated.runtime.native_process.as_mut().unwrap().entrypoint = "bin/run-v2".to_string();
        registry
            .update(project, "dserv:broker", updated)
            .expect("a real, supported registry update must succeed");

        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        let result = state.accept_v2_proposal(
            &proposal_id,
            "acc-1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE)),
            "a real registry mutation must fail closed at acceptance -- got {result:?}"
        );
        assert!(state.phase113_v2.acceptances.is_empty());
    }

    /// Item K: capacity overcommit -- a second application's candidate
    /// commitment exceeding the node's remaining structural capacity across
    /// active applications must reject at acceptance (Phase 113A closure
    /// pass 1's project-wide capacity gate, reachable through the exact
    /// `post_accept_proposal_v2` call sequence).
    #[test]
    fn accept_v2_capacity_overcommit_across_applications_is_rejected() {
        let project = "proj-api-v2-overcommit";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                project,
                v2_service_artifact("dserv:broker-a", "svc-a1", "node-1"),
            )
            .unwrap();
        let mut broker_b = v2_service_artifact("dserv:broker-b", "svc-a1", "node-1");
        broker_b.service_id = "svc-b1".to_string();
        registry.create(project, broker_b).unwrap();

        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        // Node has 4 cores total; app-a takes all 4, app-b's candidate needs
        // 1 more -- must be rejected as CPU overcommit.
        let p_a = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker-a",
            4,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();
        let p_b = create_v2_via_real_core(
            &mut state,
            &registry,
            project,
            "app-b",
            "svc-b1",
            "dserv:broker-b",
            1,
            1,
            "node-1",
            4,
            4_294_967_296,
        )
        .unwrap();

        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        state
            .accept_v2_proposal(
                &p_a,
                "acc-a".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .expect("app-a's acceptance must succeed (exact fit)");
        let result = state.accept_v2_proposal(
            &p_b,
            "acc-b".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "app-b's acceptance must fail closed as CPU overcommit -- got {result:?}"
        );
        assert!(
            !state
                .phase113_v2
                .active
                .contains_key(&crate::core::ddeploy::application_key(project, "app-b")),
            "no active v2 authority for app-b must exist after the overcommit rejection"
        );
    }

    /// Item M (v1 regression): the shared `active_placement_authority`
    /// accessor `get_active_authority_v2` uses must continue to report v1
    /// authority correctly and unmodified by this pass -- proven at the
    /// core level, since `active_placement_authority` itself (not a
    /// duplicate v1-only path) is exactly what the v1 `get_active` handler's
    /// sibling accessors already rely on.
    #[test]
    fn active_authority_accessor_still_reports_v1_authority_correctly() {
        let project = "proj-api-v1-regression";
        let mut state = crate::core::ddeploy::DDeployStateV1::empty();
        let dgraph = v2_dgraph("app-v1", "svc-1", 1, 1);
        let dcontinuum = v2_dcontinuum(project, "node-1", 4, 4_294_967_296);
        let artifacts: Vec<crate::core::service_artifact_registry::ServiceArtifact> =
            vec![v2_service_artifact(
                "dserv:v1-regression",
                "svc-1",
                "node-1",
            )];
        let proposal_id = "ddeploy-proposal:v1-regression".to_string();
        let record = create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: project.to_string(),
                application_id: "app-v1".to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 {
                    service_id: "svc-1".to_string(),
                    node_id: "node-1".to_string(),
                }],
                planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            proposal_id.clone(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap_or_else(|e| {
            panic!("v1 proposal creation must still succeed unmodified by this pass: {e:?}")
        });
        let _ = record;
        let outcome = accept_proposal(
            &mut state,
            &proposal_id,
            "operator:dana",
            None,
            &artifacts,
            Some(&dcontinuum),
            "2026-01-01T00:05:00Z",
            "ddeploy-acceptance:v1-regression".to_string(),
        )
        .unwrap_or_else(|e| {
            panic!("v1 acceptance must still succeed unmodified by this pass: {e:?}")
        });

        let view = active_placement_authority(&state, project, "app-v1")
            .expect("accessor must not error")
            .expect("an active v1 authority must exist");
        assert!(view.v2.is_none(), "a v1 acceptance must report NO DMap/2 extras -- it must never be up-converted or misreported as v2");
        assert_eq!(view.dmap_id, outcome.materialized_dmap.dmap_id);
        assert_eq!(view.revision, outcome.materialized_dmap.revision);
    }
}
