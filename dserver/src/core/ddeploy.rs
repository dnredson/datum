//! Stage 111 (ADR-0018): D-Deploy proposal / validation / explicit acceptance
//! / canonical `datum.dmap/1` materialization / derived operational
//! projection.
//!
//! This module is the ONLY place a canonical `datum.dmap/1` is ever
//! constructed with an AUTHORITATIVE `dmap_id`/`revision` and persisted as
//! active. A proposal (`DDeployProposalV1`) is NEVER a D-Map: it never
//! serializes with `"schema": "datum.dmap/1"`, and no proposal-time code
//! path activates one. Only `accept_proposal` materializes and activates a
//! `CanonicalDMapV1` -- and only after `CanonicalDMapV1::validate_against`
//! (Stage 110, reused verbatim, never duplicated) returns zero findings.
//!
//! ADR-0002 remains authoritative: the accepted `datum.dmap/1` is the SINGLE
//! placement authority. `ServiceArtifact.control_plane.target_nodes`/
//! `target_stages` remain ELIGIBILITY constraints checked at proposal/
//! acceptance time (`check_node_eligibility`/`check_target_stages`) -- they
//! are NEVER read again to determine an ALREADY-ACCEPTED placement. This
//! module never writes `target_nodes` to reflect an accepted placement.
//!
//! No Stage 112 placement algorithm exists here: every function in this
//! module either VALIDATES a caller-supplied candidate placement or
//! MATERIALIZES it verbatim. Nothing selects, scores, or optimizes a node.
//! No Stage 113 telemetry is read: only Stage 110's structural
//! capacity/capability fields participate in any decision.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1, CanonicalValidationFinding};
use crate::core::canonical_dmap::{
    recompute_dmap_digest, CanonicalContractReferenceV1, CanonicalDMapV1, CanonicalDServPlacementV1,
    CANONICAL_DMAP_SCHEMA,
};
use crate::core::jcs;
use crate::core::operational_dgraph::{
    OperationalDGraph, OperationalDesiredService, OperationalNodeDesiredState,
    OPERATIONAL_DGRAPH_SCHEMA_VERSION,
};
use crate::core::service_artifact_registry::ServiceArtifact;
use crate::storage::control_state::{
    default_control_state_db_path, load_domain_json, persist_domain_json, DOMAIN_DDEPLOY_STATE,
};

pub const DDEPLOY_PROPOSAL_SCHEMA: &str = "datum.ddeploy-proposal/1";
pub const DDEPLOY_ACCEPTANCE_SCHEMA: &str = "datum.ddeploy-acceptance/1";
pub const DDEPLOY_STATE_SCHEMA_VERSION: &str = "0.1.0";

// --------------------------------------------------------------------------- //
// Finding codes (stable, mirrored by the Python checker).                     //
// --------------------------------------------------------------------------- //
pub const F_PROPOSAL_SCHEMA_INVALID: &str = "proposal_schema_invalid";
pub const F_MISSING_FIELD: &str = "missing_field";
pub const F_INVALID_DIGEST_FORMAT: &str = "invalid_digest_format";
pub const F_PROPOSAL_PROJECT_MISMATCH: &str = "proposal_project_mismatch";
/// Part of the stable finding-code contract (mirrored by the Stage 111
/// Python checker) but not currently reachable: the Stage 111 API has no
/// endpoint that supplies an "expected application_id" independent of the
/// proposal's own `application_id` field, so no genuine mismatch can occur
/// yet. Reserved for a future application-id-scoped route (e.g. nesting
/// proposal lookups under `/applications/:application_id/...`) rather than
/// forcing an artificial check with no real defensive value today.
#[allow(dead_code)]
pub const F_PROPOSAL_APPLICATION_MISMATCH: &str = "proposal_application_mismatch";
pub const F_PROPOSAL_DIGEST_MISMATCH: &str = "proposal_digest_mismatch";
pub const F_STALE_ACTIVE_DMAP_REVISION: &str = "stale_active_dmap_revision";
pub const F_ARTIFACT_NOT_FOUND: &str = "artifact_not_found";
pub const F_ARTIFACT_AMBIGUOUS: &str = "artifact_ambiguous";
pub const F_NODE_NOT_ELIGIBLE: &str = "node_not_eligible";
pub const F_TARGET_STAGE_UNVERIFIABLE: &str = "target_stage_constraint_unverifiable";
pub const F_OPERATIONAL_SERVICE_ID_COLLISION: &str = "operational_service_id_collision";
pub const F_PROPOSAL_NOT_FOUND: &str = "proposal_not_found";
pub const F_ARTIFACT_PROJECT_MISMATCH: &str = "artifact_project_mismatch";
pub const F_PERSISTENCE_CONFLICT: &str = "persistence_conflict";
/// Independent review v3: the project-wide operational projection an
/// acceptance would produce (or a persisted state's projection, on reload)
/// fails the existing `OperationalDGraph` contract's own validator, or is
/// otherwise internally inconsistent (e.g. a D-Map-placed service missing
/// from its own application's `operational_services` snapshot).
pub const F_OPERATIONAL_PROJECTION_INVALID: &str = "operational_projection_invalid";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

/// Stage 111 (ADR-0018 independent review v2, identifier-composition
/// audit): a LENGTH-PREFIXED encoding of `value`, so concatenating several
/// of these together is unambiguous regardless of what characters `value`
/// itself contains (including the concatenation's own separators). A plain
/// `"{a}{sep}{b}"` join is NOT collision-free here: `project_id` and
/// `application_id` are arbitrary caller-supplied strings (only checked
/// non-empty, `validate_proposal_shape`), and this repository's OWN real
/// fixtures already use colon-containing application ids (e.g.
/// `"app:smart-sentinel-lab"`, ADR-0017) -- a naive `"dmap:{project}:{app}"`
/// join can genuinely produce the same string for two DIFFERENT
/// `(project_id, application_id)` pairs (e.g. `("p", "a:b")` and
/// `("p:a", "b")` both join to `"dmap:p:a:b"`). Prefixing each component
/// with its own byte length closes this: the length tells a (hypothetical)
/// parser exactly where one component ends, independent of its content.
fn length_prefixed_component(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

pub fn application_key(project_id: &str, application_id: &str) -> String {
    format!("{}{}", length_prefixed_component(project_id), length_prefixed_component(application_id))
}

pub fn canonical_dmap_id(project_id: &str, application_id: &str) -> String {
    format!("dmap:{}{}", length_prefixed_component(project_id), length_prefixed_component(application_id))
}

/// Deliberately NOT the canonical D-Graph's own `dgraph_id` (independent
/// review requirement): this is a SEPARATE, stable, operational-deployment-
/// document identity, owned by Stage 111, never confused with the
/// application-plane `datum.dgraph/1` identity it was derived from.
pub fn operational_dgraph_id_for_project(project_id: &str) -> String {
    format!("ddeploy-operational:{project_id}")
}

// --------------------------------------------------------------------------- //
// Wire types                                                                   //
// --------------------------------------------------------------------------- //

/// Provenance only -- records WHO/WHAT produced a proposal. Never implements
/// placement selection itself (Stage 112) and never bypasses validation or
/// acceptance (AI must never directly produce an active D-Map).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalSource {
    Operator,
    Deterministic,
    Ai,
}

/// `datum.ddeploy-proposal/1`. NEVER a D-Map: carries no `dmap_id`/
/// `dmap_revision` (those are DServer-owned, assigned only at acceptance).
/// Immutable once created -- there is no proposal update endpoint; a changed
/// placement is always a NEW proposal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DDeployProposalV1 {
    pub schema: String,
    pub proposal_id: String,
    pub project_id: String,
    pub application_id: String,
    pub source: ProposalSource,
    pub created_at_utc: String,
    /// Snapshot, taken at proposal-creation time, of the active D-Map
    /// revision for this project/application (`None` for a first
    /// deployment). Used as the compare-and-swap guard at acceptance time
    /// (`F_STALE_ACTIVE_DMAP_REVISION`): if the active revision has since
    /// changed, acceptance fails closed rather than silently activating
    /// against a revision the proposal was never actually validated
    /// against.
    #[serde(default)]
    pub expected_active_dmap_revision: Option<u64>,
    pub dgraph_ref: CanonicalContractReferenceV1,
    pub dcontinuum_ref: CanonicalContractReferenceV1,
    #[serde(default)]
    pub placements: Vec<CanonicalDServPlacementV1>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalLifecycleState {
    Pending,
    Accepted,
}

/// Internal, persisted record: the public proposal PLUS the immutable exact
/// D-Graph/D-Continuum snapshots it was validated against (independent
/// review requirement: proposal validation must be reproducible later
/// without trusting a caller-supplied digest as authoritative) and its
/// server-computed digest and lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalRecord {
    pub proposal: DDeployProposalV1,
    /// Server-owned, computed by `compute_proposal_digest` over the
    /// immutable public proposal fields only -- never over `state`/
    /// `accepted_acceptance_id`, which are mutable lifecycle metadata.
    pub proposal_digest: String,
    pub dgraph_snapshot: CanonicalDGraphV1,
    pub dcontinuum_snapshot: CanonicalDContinuumV1,
    pub state: ProposalLifecycleState,
    #[serde(default)]
    pub accepted_acceptance_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalProjectionRefV1 {
    pub operational_dgraph_id: String,
    pub revision: u64,
}

/// `datum.ddeploy-acceptance/1`. `accepted_by` is a CALLER-SUPPLIED AUDIT
/// LABEL, exactly like `ResolvedImageIdentityV1.resolved_by` -- DServer has
/// no application-layer authentication (ADR-0013 §6), so this is provenance
/// for a human reading the record later, never a verified/authenticated
/// operator identity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DDeployAcceptanceV1 {
    pub schema: String,
    pub acceptance_id: String,
    pub proposal_id: String,
    pub proposal_digest: String,
    pub project_id: String,
    pub application_id: String,
    pub accepted_at_utc: String,
    pub accepted_by: String,
    pub materialized_dmap_ref: CanonicalContractReferenceV1,
    pub operational_projection_ref: OperationalProjectionRefV1,
}

/// Internal, persisted: the currently active canonical D-Map for one
/// (project_id, application_id), plus the operational content precomputed
/// AT ACCEPTANCE TIME from the transitional `ServiceArtifact` bridge. This
/// precomputation (not a live re-join on every read) is deliberate: it
/// makes the accepted projection a frozen snapshot exactly like every other
/// Stage 110/111 acceptance artifact -- a later artifact CONTENT edit
/// cannot silently change what an already-accepted deployment reports as
/// its desired runtime; only a NEW accepted proposal can.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActiveApplicationDeployment {
    pub application_id: String,
    pub active_dmap: CanonicalDMapV1,
    pub dmap_digest: String,
    pub proposal_id: String,
    pub acceptance_id: String,
    pub accepted_at_utc: String,
    /// Precomputed at acceptance time by `derive_application_operational_services`
    /// -- one entry per D-Map-placed service, KEYED BY `service_id`.
    /// Independent review v3, single-persisted-placement-authority fix:
    /// this map deliberately carries NO node identity anywhere (not as a
    /// key, not as a field on `OperationalDesiredService`). `active_dmap`
    /// (specifically `active_dmap.placements`) is the ONLY place this
    /// struct records which node a service is placed on -- there is no
    /// second, independently-persisted service->node index that could
    /// silently drift from it. `union_project_operational_graph` derives
    /// every node assignment by reading `active_dmap.placements` directly
    /// and using this map only to look up the ALREADY-PLACED service's
    /// runtime/desired_state payload.
    pub operational_services: BTreeMap<String, OperationalDesiredService>,
}

/// The ONE Stage 111 control-state domain blob (independent review
/// requirement: proposal lifecycle + active D-Map + derived operational
/// projection persist as ONE atomic domain update, never as separate
/// independent writes).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DDeployStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub proposals: BTreeMap<String, ProposalRecord>,
    #[serde(default)]
    pub acceptances: BTreeMap<String, DDeployAcceptanceV1>,
    /// Keyed by `application_key(project_id, application_id)`.
    #[serde(default)]
    pub active_by_application: BTreeMap<String, ActiveApplicationDeployment>,
    /// Project-level operational-projection revision (independent review
    /// requirement: increments only when an ACCEPTED deployment changes the
    /// EFFECTIVE operational projection for the project; an idempotent
    /// re-acceptance of the same proposal never increments it).
    #[serde(default)]
    pub project_operational_revision: BTreeMap<String, u64>,
}

impl DDeployStateV1 {
    pub fn empty() -> Self {
        Self {
            state_version: DDEPLOY_STATE_SCHEMA_VERSION.to_string(),
            proposals: BTreeMap::new(),
            acceptances: BTreeMap::new(),
            active_by_application: BTreeMap::new(),
            project_operational_revision: BTreeMap::new(),
        }
    }

    pub fn active_for(&self, project_id: &str, application_id: &str) -> Option<&ActiveApplicationDeployment> {
        self.active_by_application.get(&application_key(project_id, application_id))
    }

    pub fn has_active_authority(&self, project_id: &str) -> bool {
        self.active_by_application.values().any(|a| a.active_dmap.project_id == project_id)
    }
}

// --------------------------------------------------------------------------- //
// Persistence                                                                  //
// --------------------------------------------------------------------------- //

pub fn load_state(db_path: &std::path::Path) -> Result<DDeployStateV1, String> {
    match load_domain_json::<DDeployStateV1>(db_path, DOMAIN_DDEPLOY_STATE)? {
        Some(state) => {
            if state.state_version != DDEPLOY_STATE_SCHEMA_VERSION {
                return Err(format!(
                    "unsupported ddeploy state_version '{}'; expected '{}' -- fail closed rather than silently reinterpreting an unknown future schema",
                    state.state_version, DDEPLOY_STATE_SCHEMA_VERSION
                ));
            }
            validate_persisted_state_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DDeployStateV1::empty()),
    }
}

/// Stage 111 (ADR-0018 independent review v2/v3, bounded persisted-state
/// integrity audit): a MINIMAL, fail-closed check that a persisted --
/// therefore already schema-parseable -- `DDeployStateV1` blob forms one
/// CLOSED, bidirectionally-consistent provenance chain
/// (proposal <-> acceptance <-> active application <-> operational
/// projection), not merely that each record hashes itself correctly. This
/// is deliberately NOT a migration or repair framework: on any
/// inconsistency this returns `Err` (via `load_state`, which propagates it
/// to every caller), never attempts to reconcile, drop, or "best-effort"
/// repair the offending record.
///
/// Independent review v3 correction: v2's version of this function checked
/// each record's OWN digest but did not verify the RELATIONSHIPS between
/// records were mutually consistent -- e.g. two independently
/// self-consistent objects (`active_dmap` recomputing to its own
/// `dmap_digest`) could still jointly lie about which accepted proposal
/// actually produced them. Every check below is now either a same-record
/// digest check OR a cross-record relational check.
///
/// Independent review v4 correction (attribution fix): the known
/// self-consistent `active_dmap` tamper exercised by
/// `self_consistent_tamper_is_still_detected` is actually rejected EARLIER,
/// by category C's active-D-Map-vs-`acceptance.materialized_dmap_ref`
/// cross-reference/digest relation -- not by category C's LATER exact
/// structural-equality comparison against the historically-reconstructed
/// D-Map, which an earlier version of this comment incorrectly credited.
/// Category B independently reconstructs that same historical D-Map and
/// verifies its digest, so by the time the later structural-equality
/// branch would run, the preceding digest relations (B's reconstruction
/// check and C's acceptance cross-reference) already establish digest
/// equality between the reconstruction and `active_dmap`; under ordinary
/// SHA-256 collision-resistance, the structural-equality branch is then
/// logically implied by those relations and is not the first failure
/// `self_consistent_tamper_is_still_detected` actually reaches. It is
/// retained as defense-in-depth, not as the mechanism that test proves.
fn validate_persisted_state_integrity(state: &DDeployStateV1) -> Result<(), String> {
    fn violation(message: impl Into<String>) -> String {
        format!("persisted state integrity violation: {} -- refusing to load potentially corrupted controller state", message.into())
    }

    // ---- category A: proposal records ---- //
    for (proposal_key, record) in &state.proposals {
        let proposal = &record.proposal;
        if *proposal_key != proposal.proposal_id {
            return Err(violation(format!(
                "proposals map key '{proposal_key}' does not match its own record's proposal_id '{}'", proposal.proposal_id
            )));
        }
        let actual_digest = compute_proposal_digest(proposal)?;
        if actual_digest != record.proposal_digest {
            return Err(violation(format!(
                "proposal '{proposal_key}' recomputed digest '{actual_digest}' does not match its recorded proposal_digest '{}'", record.proposal_digest
            )));
        }
        if !record.dgraph_snapshot.validate().is_empty() {
            return Err(violation(format!("proposal '{proposal_key}' dgraph_snapshot does not validate cleanly")));
        }
        if !record.dcontinuum_snapshot.validate().is_empty() {
            return Err(violation(format!("proposal '{proposal_key}' dcontinuum_snapshot does not validate cleanly")));
        }
        let expected_dgraph_digest = recompute_dgraph_digest(&record.dgraph_snapshot)?;
        if proposal.dgraph_ref.id != record.dgraph_snapshot.dgraph_id
            || proposal.dgraph_ref.revision != record.dgraph_snapshot.revision
            || proposal.dgraph_ref.digest != expected_dgraph_digest
        {
            return Err(violation(format!(
                "proposal '{proposal_key}' dgraph_ref {:?} does not match its own persisted dgraph_snapshot (id='{}', revision={}, recomputed_digest='{expected_dgraph_digest}')",
                proposal.dgraph_ref, record.dgraph_snapshot.dgraph_id, record.dgraph_snapshot.revision
            )));
        }
        let expected_dcontinuum_digest = recompute_dcontinuum_digest(&record.dcontinuum_snapshot)?;
        if proposal.dcontinuum_ref.id != record.dcontinuum_snapshot.dcontinuum_id
            || proposal.dcontinuum_ref.revision != record.dcontinuum_snapshot.revision
            || proposal.dcontinuum_ref.digest != expected_dcontinuum_digest
        {
            return Err(violation(format!(
                "proposal '{proposal_key}' dcontinuum_ref {:?} does not match its own persisted dcontinuum_snapshot (id='{}', revision={}, recomputed_digest='{expected_dcontinuum_digest}')",
                proposal.dcontinuum_ref, record.dcontinuum_snapshot.dcontinuum_id, record.dcontinuum_snapshot.revision
            )));
        }
        if proposal.application_id != record.dgraph_snapshot.application_id {
            return Err(violation(format!(
                "proposal '{proposal_key}' application_id '{}' does not match its own persisted dgraph_snapshot.application_id '{}'",
                proposal.application_id, record.dgraph_snapshot.application_id
            )));
        }
        if proposal.project_id != record.dcontinuum_snapshot.project_id {
            return Err(violation(format!(
                "proposal '{proposal_key}' project_id '{}' does not match its own persisted dcontinuum_snapshot.project_id '{}'",
                proposal.project_id, record.dcontinuum_snapshot.project_id
            )));
        }
        match record.state {
            ProposalLifecycleState::Pending => {
                if record.accepted_acceptance_id.is_some() {
                    return Err(violation(format!("proposal '{proposal_key}' is Pending but carries a non-None accepted_acceptance_id")));
                }
            }
            ProposalLifecycleState::Accepted => {
                let acceptance_id = record.accepted_acceptance_id.as_ref().ok_or_else(|| violation(format!(
                    "proposal '{proposal_key}' is Accepted but has no accepted_acceptance_id"
                )))?;
                let acceptance = state.acceptances.get(acceptance_id).ok_or_else(|| violation(format!(
                    "proposal '{proposal_key}' references acceptance '{acceptance_id}', which does not exist in state.acceptances"
                )))?;
                // Independent review v4, R2: an Accepted proposal must
                // point to its OWN matching acceptance, not merely to SOME
                // acceptance that happens to exist -- otherwise a falsely-
                // Accepted proposal could claim a real acceptance that
                // actually belongs to a DIFFERENT proposal. Category B
                // (below) only verifies, for each acceptance, that ITS OWN
                // referenced proposal points back to it -- it never checks
                // whether some OTHER proposal also (incorrectly) claims
                // that same acceptance, which is exactly this check's job.
                if acceptance.proposal_id != *proposal_key {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' claims accepted_acceptance_id '{acceptance_id}', but that acceptance's own proposal_id is '{}', not '{proposal_key}' -- an Accepted proposal must point to its OWN matching acceptance, never another proposal's",
                        acceptance.proposal_id
                    )));
                }
            }
        }
    }

    // ---- category B: acceptance records ---- //
    for (acceptance_key, acceptance) in &state.acceptances {
        if *acceptance_key != acceptance.acceptance_id {
            return Err(violation(format!(
                "acceptances map key '{acceptance_key}' does not match its own record's acceptance_id '{}'", acceptance.acceptance_id
            )));
        }
        if acceptance.schema != DDEPLOY_ACCEPTANCE_SCHEMA {
            return Err(violation(format!("acceptance '{acceptance_key}' schema '{}' is not '{DDEPLOY_ACCEPTANCE_SCHEMA}'", acceptance.schema)));
        }
        // Proposals are never deleted by this module -- a dangling
        // `proposal_id` is corruption, not a tolerated gap (independent
        // review v3 correction: v2 tolerated this).
        let referenced_proposal = state.proposals.get(&acceptance.proposal_id).ok_or_else(|| violation(format!(
            "acceptance '{acceptance_key}' references proposal '{}', which does not exist in state.proposals", acceptance.proposal_id
        )))?;
        if acceptance.proposal_digest != referenced_proposal.proposal_digest {
            return Err(violation(format!(
                "acceptance '{acceptance_key}' proposal_digest '{}' does not match referenced proposal '{}''s recorded proposal_digest '{}'",
                acceptance.proposal_digest, acceptance.proposal_id, referenced_proposal.proposal_digest
            )));
        }
        if acceptance.project_id != referenced_proposal.proposal.project_id || acceptance.application_id != referenced_proposal.proposal.application_id {
            return Err(violation(format!(
                "acceptance '{acceptance_key}' project/application ('{}','{}') does not match referenced proposal '{}''s own ('{}','{}')",
                acceptance.project_id, acceptance.application_id, acceptance.proposal_id, referenced_proposal.proposal.project_id, referenced_proposal.proposal.application_id
            )));
        }
        if referenced_proposal.state != ProposalLifecycleState::Accepted {
            return Err(violation(format!(
                "acceptance '{acceptance_key}' references proposal '{}', which is not in state Accepted", acceptance.proposal_id
            )));
        }
        if referenced_proposal.accepted_acceptance_id.as_deref() != Some(acceptance_key.as_str()) {
            return Err(violation(format!(
                "acceptance '{acceptance_key}' references proposal '{}', but that proposal's own accepted_acceptance_id does not point back to '{acceptance_key}'",
                acceptance.proposal_id
            )));
        }
        let expected_dmap_id = canonical_dmap_id(&acceptance.project_id, &acceptance.application_id);
        if acceptance.materialized_dmap_ref.id != expected_dmap_id {
            return Err(violation(format!(
                "acceptance '{acceptance_key}' materialized_dmap_ref.id '{}' does not match canonical_dmap_id('{}', '{}') = '{expected_dmap_id}'",
                acceptance.materialized_dmap_ref.id, acceptance.project_id, acceptance.application_id
            )));
        }
        let historical_dmap = build_candidate_dmap(&referenced_proposal.proposal, &acceptance.materialized_dmap_ref.id, acceptance.materialized_dmap_ref.revision);
        if !historical_dmap.validate_against(&referenced_proposal.dgraph_snapshot, &referenced_proposal.dcontinuum_snapshot).is_empty() {
            return Err(violation(format!(
                "acceptance '{acceptance_key}''s historically-reconstructed D-Map does not validate against its own proposal's persisted D-Graph/D-Continuum snapshots"
            )));
        }
        let historical_digest = recompute_dmap_digest(&historical_dmap)?;
        if historical_digest != acceptance.materialized_dmap_ref.digest {
            return Err(violation(format!(
                "acceptance '{acceptance_key}' recomputed historical D-Map digest '{historical_digest}' does not match its own materialized_dmap_ref.digest '{}'",
                acceptance.materialized_dmap_ref.digest
            )));
        }
    }

    // ---- category C: active application deployments ---- //
    for (key, active) in &state.active_by_application {
        let expected_key = application_key(&active.active_dmap.project_id, &active.active_dmap.application_id);
        if *key != expected_key {
            return Err(violation(format!(
                "active_by_application entry stored under key '{key}' does not match application_key('{}', '{}') = '{expected_key}'",
                active.active_dmap.project_id, active.active_dmap.application_id
            )));
        }
        if active.application_id != active.active_dmap.application_id {
            return Err(violation(format!(
                "active_by_application entry '{key}' application_id '{}' does not match its own active_dmap.application_id '{}'",
                active.application_id, active.active_dmap.application_id
            )));
        }
        let actual_dmap_digest = recompute_dmap_digest(&active.active_dmap)?;
        if actual_dmap_digest != active.dmap_digest {
            return Err(violation(format!(
                "active_by_application entry '{key}' recomputed D-Map digest '{actual_dmap_digest}' does not match its recorded dmap_digest '{}'",
                active.dmap_digest
            )));
        }
        let referenced_proposal = state.proposals.get(&active.proposal_id).ok_or_else(|| violation(format!(
            "active_by_application entry '{key}' references proposal '{}', which does not exist", active.proposal_id
        )))?;
        let referenced_acceptance = state.acceptances.get(&active.acceptance_id).ok_or_else(|| violation(format!(
            "active_by_application entry '{key}' references acceptance '{}', which does not exist", active.acceptance_id
        )))?;
        if referenced_proposal.state != ProposalLifecycleState::Accepted || referenced_proposal.accepted_acceptance_id.as_deref() != Some(active.acceptance_id.as_str()) {
            return Err(violation(format!(
                "active_by_application entry '{key}''s proposal '{}' is not consistently Accepted with acceptance '{}'", active.proposal_id, active.acceptance_id
            )));
        }
        if referenced_acceptance.proposal_id != active.proposal_id {
            return Err(violation(format!(
                "active_by_application entry '{key}''s acceptance '{}' references a DIFFERENT proposal ('{}') than the entry itself claims ('{}')",
                active.acceptance_id, referenced_acceptance.proposal_id, active.proposal_id
            )));
        }
        if referenced_acceptance.project_id != active.active_dmap.project_id || referenced_acceptance.application_id != active.active_dmap.application_id {
            return Err(violation(format!(
                "active_by_application entry '{key}''s acceptance '{}' project/application does not match the entry's own active_dmap", active.acceptance_id
            )));
        }
        if referenced_acceptance.materialized_dmap_ref.id != active.active_dmap.dmap_id
            || referenced_acceptance.materialized_dmap_ref.revision != active.active_dmap.revision
            || referenced_acceptance.materialized_dmap_ref.digest != active.dmap_digest
        {
            return Err(violation(format!(
                "active_by_application entry '{key}''s active_dmap (id='{}', revision={}, digest='{}') does not match its own acceptance '{}''s materialized_dmap_ref",
                active.active_dmap.dmap_id, active.active_dmap.revision, active.dmap_digest, active.acceptance_id
            )));
        }
        // The relational closure (independent review v3, PASS U): the
        // historically-reconstructed D-Map -- built ONLY from the
        // referenced proposal's immutable content plus the referenced
        // acceptance's own recorded ref -- must be EXACTLY, STRUCTURALLY
        // equal to the persisted `active_dmap`, not merely digest-equal.
        // This is what prevents a self-consistent (own-digest-correct)
        // `active_dmap` from ever being forged without a real accepted
        // proposal actually having materialized it.
        let historical_dmap = build_candidate_dmap(&referenced_proposal.proposal, &referenced_acceptance.materialized_dmap_ref.id, referenced_acceptance.materialized_dmap_ref.revision);
        if historical_dmap != active.active_dmap {
            return Err(violation(format!(
                "active_by_application entry '{key}''s active_dmap is NOT the D-Map its referenced accepted proposal/acceptance actually materializes -- a self-consistent but unauthorized D-Map was detected"
            )));
        }

        // ---- category D: operational service snapshot ---- //
        let placement_service_ids: std::collections::BTreeSet<&str> = active.active_dmap.placements.iter().map(|p| p.service_id.as_str()).collect();
        let snapshot_service_ids: std::collections::BTreeSet<&str> = active.operational_services.keys().map(String::as_str).collect();
        if placement_service_ids != snapshot_service_ids {
            return Err(violation(format!(
                "active_by_application entry '{key}''s operational_services service-ID set {snapshot_service_ids:?} does not exactly match its own active_dmap.placements service-ID set {placement_service_ids:?}"
            )));
        }
        for (service_key, service) in &active.operational_services {
            if service_key != &service.service_id {
                return Err(violation(format!(
                    "active_by_application entry '{key}''s operational_services map key '{service_key}' does not match its own entry's service_id '{}'", service.service_id
                )));
            }
            if service.desired_state != "running" {
                return Err(violation(format!(
                    "active_by_application entry '{key}''s operational service '{service_key}' has desired_state '{}', expected 'running'", service.desired_state
                )));
            }
            if crate::core::runtime_kind::RuntimeKind::parse(&service.runtime).is_err() {
                return Err(violation(format!(
                    "active_by_application entry '{key}''s operational service '{service_key}' has an invalid runtime '{}'", service.runtime
                )));
            }
        }
    }

    // ---- category E: project operational revision ---- //
    // Independent review v4, R2: derived from ACCEPTED HISTORY
    // (`state.acceptances`, never deleted by this module), never from
    // `active_by_application`'s own key set -- deleting every active entry
    // for a project must NOT skip this check (v3's version iterated
    // `active_by_application`, so an empty `active_by_application` made
    // this loop silently run zero times).
    let mut max_projection_revision_by_project: BTreeMap<&str, u64> = BTreeMap::new();
    for acceptance in state.acceptances.values() {
        let expected_operational_dgraph_id = operational_dgraph_id_for_project(&acceptance.project_id);
        if acceptance.operational_projection_ref.operational_dgraph_id != expected_operational_dgraph_id {
            return Err(violation(format!(
                "acceptance '{}' operational_projection_ref.operational_dgraph_id '{}' does not match operational_dgraph_id_for_project('{}') = '{expected_operational_dgraph_id}'",
                acceptance.acceptance_id, acceptance.operational_projection_ref.operational_dgraph_id, acceptance.project_id
            )));
        }
        let entry = max_projection_revision_by_project.entry(acceptance.project_id.as_str()).or_insert(0);
        *entry = (*entry).max(acceptance.operational_projection_ref.revision);
    }
    let accepted_project_ids: std::collections::BTreeSet<&str> = state.acceptances.values().map(|a| a.project_id.as_str()).collect();
    for project_id in &accepted_project_ids {
        let recorded_revision = *state.project_operational_revision.get(*project_id).ok_or_else(|| violation(format!(
            "project '{project_id}' has accepted history but no project_operational_revision entry"
        )))?;
        if recorded_revision == 0 {
            return Err(violation(format!("project '{project_id}' has accepted history but project_operational_revision is 0")));
        }
        let max_from_acceptances = max_projection_revision_by_project.get(project_id).copied().unwrap_or(0);
        if recorded_revision != max_from_acceptances {
            return Err(violation(format!(
                "project '{project_id}' project_operational_revision ({recorded_revision}) does not equal the maximum operational_projection_ref.revision among its acceptances ({max_from_acceptances})"
            )));
        }
    }

    // ---- category F: active authority completeness (independent review --
    // ---- v4, R2) -- every application with accepted history must have --
    // ---- an active entry, and that entry must be the LATEST consistent -
    // ---- accepted materialization, never an older restored one. --------
    //
    // Derived entirely from `state.acceptances` (never deleted), so
    // deleting an `active_by_application` entry cannot silently make this
    // loop skip it -- this is what catches BOTH the "active authority
    // silently lost" case (an accepted application with NO active entry)
    // and the "stale active entry restored from an earlier acceptance"
    // case (an active entry present, but not at the latest server-owned
    // D-Map revision this application's own accepted history reached).
    // Ordering is by the server-owned `materialized_dmap_ref.revision`
    // (monotonic per application, assigned only by `accept_proposal`'s own
    // commit) -- never by acceptance_id or any other caller-controlled or
    // lexical label.
    let mut latest_acceptance_by_app: BTreeMap<String, (u64, &str)> = BTreeMap::new();
    for acceptance in state.acceptances.values() {
        let app_key = application_key(&acceptance.project_id, &acceptance.application_id);
        let revision = acceptance.materialized_dmap_ref.revision;
        match latest_acceptance_by_app.get(&app_key) {
            None => {
                latest_acceptance_by_app.insert(app_key, (revision, acceptance.acceptance_id.as_str()));
            }
            Some((existing_revision, existing_acceptance_id)) => {
                if revision > *existing_revision {
                    latest_acceptance_by_app.insert(app_key, (revision, acceptance.acceptance_id.as_str()));
                } else if revision == *existing_revision && *existing_acceptance_id != acceptance.acceptance_id {
                    // Two DIFFERENT acceptances claiming the same D-Map
                    // revision for the same application: server-owned
                    // per-application revisions must be unique, so this
                    // ambiguity is itself corruption -- never silently
                    // resolved by acceptance_id ordering.
                    return Err(violation(format!(
                        "application '{app_key}' has two DIFFERENT accepted acceptances ('{existing_acceptance_id}' and '{}') both claiming D-Map revision {revision} -- ambiguous accepted history",
                        acceptance.acceptance_id
                    )));
                }
            }
        }
    }
    for (app_key, (latest_revision, latest_acceptance_id)) in &latest_acceptance_by_app {
        let active = state.active_by_application.get(app_key).ok_or_else(|| violation(format!(
            "application '{app_key}' has accepted history reaching D-Map revision {latest_revision} (acceptance '{latest_acceptance_id}') but has NO active_by_application entry -- canonical placement authority must not be silently lost"
        )))?;
        if active.active_dmap.revision != *latest_revision {
            return Err(violation(format!(
                "application '{app_key}' active entry is at D-Map revision {}, but its accepted history's LATEST revision is {latest_revision} (acceptance '{latest_acceptance_id}') -- the active entry is not the latest accepted materialization for this application",
                active.active_dmap.revision
            )));
        }
        if active.acceptance_id != *latest_acceptance_id {
            return Err(violation(format!(
                "application '{app_key}' active entry's acceptance_id '{}' does not match the latest accepted acceptance '{latest_acceptance_id}' at the SAME revision {latest_revision} -- inconsistent accepted history",
                active.acceptance_id
            )));
        }
    }

    // ---- category G: reload projection validity (independent review v4, --
    // ---- R1) -- reconstruct every canonical project's operational union -
    // ---- and validate it against the SAME contract acceptance already --
    // ---- checks pre-commit. Per-record checks (A-F) alone cannot catch, --
    // ---- for example, two independently self-consistent accepted -------
    // ---- applications that jointly claim the same service_id -- only ---
    // ---- reconstructing the actual union surfaces that collision. ------
    for project_id in &accepted_project_ids {
        let revision = *state.project_operational_revision.get(*project_id).unwrap_or(&0);
        let projection = union_project_operational_graph(project_id, &state.active_by_application, revision)
            .map_err(|findings| violation(format!(
                "project '{project_id}' reconstructed operational projection failed union (e.g. a cross-application service_id collision): {findings:?}"
            )))?;
        if let Err(error) = crate::core::operational_dgraph::validate_operational_dgraph(&projection) {
            return Err(violation(format!(
                "project '{project_id}' reconstructed operational projection failed validate_operational_dgraph: {error}"
            )));
        }
    }

    Ok(())
}

pub fn persist_state(db_path: &std::path::Path, state: &DDeployStateV1) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DDEPLOY_STATE, state)
}

fn resolved_db_path() -> std::path::PathBuf {
    default_control_state_db_path()
}

/// Is an accepted canonical D-Map currently the placement authority for
/// `project_id`? Used by the legacy operational-graph write guard
/// (`api::operational_dgraph::put_operational_dgraph`) to fail closed rather
/// than let a legacy write silently override an active canonical placement.
pub fn is_canonical_authority_active(project_id: &str) -> Result<bool, String> {
    let state = load_state(&resolved_db_path())?;
    Ok(state.has_active_authority(project_id))
}

/// The centralized authority accessor (independent review requirement): the
/// project's canonical-derived operational graph if canonical authority is
/// active for it, else `None` (caller falls back to the legacy graph).
/// Reads are computed PURELY from persisted state -- no artifact registry
/// access at read time (see `ActiveApplicationDeployment`'s doc comment).
pub fn active_operational_projection(project_id: &str) -> Result<Option<OperationalDGraph>, String> {
    let state = load_state(&resolved_db_path())?;
    if !state.has_active_authority(project_id) {
        return Ok(None);
    }
    let revision = *state.project_operational_revision.get(project_id).unwrap_or(&0);
    union_project_operational_graph(project_id, &state.active_by_application, revision)
        .map(Some)
        .map_err(|findings| format!("operational projection union failed (should be unreachable -- collisions are rejected at acceptance time): {findings:?}"))
}

// --------------------------------------------------------------------------- //
// Proposal digest                                                              //
// --------------------------------------------------------------------------- //

/// Server-owned canonical identity of the IMMUTABLE proposal semantics only
/// -- never mutable lifecycle state (`ProposalRecord.state`/
/// `accepted_acceptance_id` are excluded because they are not part of this
/// type at all).
pub fn compute_proposal_digest(proposal: &DDeployProposalV1) -> Result<String, String> {
    let value = serde_json::to_value(proposal).map_err(|error| format!("serialize DDeployProposalV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

// --------------------------------------------------------------------------- //
// Pure validation                                                              //
// --------------------------------------------------------------------------- //

pub fn validate_proposal_shape(proposal: &DDeployProposalV1) -> Vec<CanonicalValidationFinding> {
    let mut findings = Vec::new();
    if proposal.schema != DDEPLOY_PROPOSAL_SCHEMA {
        findings.push(finding(F_PROPOSAL_SCHEMA_INVALID, format!("schema must be '{DDEPLOY_PROPOSAL_SCHEMA}', got '{}'", proposal.schema)));
    }
    if proposal.proposal_id.trim().is_empty() {
        findings.push(finding(F_MISSING_FIELD, "proposal_id must be a non-empty string"));
    }
    if proposal.project_id.trim().is_empty() {
        findings.push(finding(F_MISSING_FIELD, "project_id must be a non-empty string"));
    }
    if proposal.application_id.trim().is_empty() {
        findings.push(finding(F_MISSING_FIELD, "application_id must be a non-empty string"));
    }
    if proposal.created_at_utc.trim().is_empty()
        || chrono::DateTime::parse_from_rfc3339(&proposal.created_at_utc).is_err()
    {
        findings.push(finding(F_MISSING_FIELD, "created_at_utc must be a valid RFC3339 timestamp"));
    }
    if proposal.dgraph_ref.id.trim().is_empty() || proposal.dgraph_ref.revision < 1 {
        findings.push(finding(F_MISSING_FIELD, "dgraph_ref.id/revision must be present and revision >= 1"));
    }
    if !jcs::is_canonical_digest_format(&proposal.dgraph_ref.digest) {
        findings.push(finding(F_INVALID_DIGEST_FORMAT, format!("dgraph_ref.digest '{}' is not the canonical 'sha256:<64 lowercase hex>' form", proposal.dgraph_ref.digest)));
    }
    if proposal.dcontinuum_ref.id.trim().is_empty() || proposal.dcontinuum_ref.revision < 1 {
        findings.push(finding(F_MISSING_FIELD, "dcontinuum_ref.id/revision must be present and revision >= 1"));
    }
    if !jcs::is_canonical_digest_format(&proposal.dcontinuum_ref.digest) {
        findings.push(finding(F_INVALID_DIGEST_FORMAT, format!("dcontinuum_ref.digest '{}' is not the canonical 'sha256:<64 lowercase hex>' form", proposal.dcontinuum_ref.digest)));
    }
    findings
}

/// Proposal-level context checks: does this proposal actually belong to the
/// project it is being created/accepted under?
pub fn validate_proposal_context(proposal: &DDeployProposalV1, expected_project_id: &str) -> Vec<CanonicalValidationFinding> {
    let mut findings = Vec::new();
    if proposal.project_id.trim() != expected_project_id.trim() {
        findings.push(finding(F_PROPOSAL_PROJECT_MISMATCH, format!(
            "proposal project_id '{}' does not match request context project_id '{}'",
            proposal.project_id, expected_project_id
        )));
    }
    findings
}

/// Defensive boundary check: the `ServiceArtifactList` the API layer fetched
/// (via `ServiceArtifactRegistry::list(project_id)`) must actually be scoped
/// to the SAME project as the proposal/acceptance being validated. The
/// registry already partitions artifacts by project internally, so this can
/// only fire if a caller passes a mismatched list into `validate_full`/
/// `create_proposal`/`accept_proposal` -- it exists so such a caller bug
/// fails closed with a stable, greppable code instead of silently resolving
/// services against another project's artifacts.
pub fn check_artifact_list_project_scope(expected_project_id: &str, artifact_list_project_id: &str) -> Option<CanonicalValidationFinding> {
    if artifact_list_project_id == expected_project_id {
        None
    } else {
        Some(finding(F_ARTIFACT_PROJECT_MISMATCH, format!(
            "service artifact list is scoped to project '{artifact_list_project_id}' but expected project '{expected_project_id}'"
        )))
    }
}

/// The join semantics MIRROR (deliberately, byte-for-byte in spirit)
/// `artifact_reconciliation_context::resolve_assignments_from_registry`'s
/// direct/alias match -- but WITHOUT the `target_nodes` filter, since that
/// filter is proposal/acceptance-time ELIGIBILITY, never post-acceptance
/// placement truth (ADR-0002). Exactly one eligible artifact is required;
/// zero or multiple both fail closed.
pub fn resolve_artifact_for_service<'a>(
    service_id: &str,
    artifacts: &'a [ServiceArtifact],
) -> Result<&'a ServiceArtifact, CanonicalValidationFinding> {
    let matches: Vec<&ServiceArtifact> = artifacts
        .iter()
        .filter(|artifact| {
            let direct_match = artifact.artifact_id == service_id || artifact.service_id == service_id;
            let alias_match = artifact
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("service_aliases"))
                .and_then(|aliases| aliases.as_array())
                .is_some_and(|aliases| aliases.iter().any(|alias| alias.as_str() == Some(service_id)));
            direct_match || alias_match
        })
        .collect();
    match matches.as_slice() {
        [artifact] => Ok(*artifact),
        [] => Err(finding(F_ARTIFACT_NOT_FOUND, format!("no service artifact resolves canonical service '{service_id}'"))),
        _ => Err(finding(F_ARTIFACT_AMBIGUOUS, format!("multiple service artifacts resolve canonical service '{service_id}'"))),
    }
}

/// Proposal/acceptance-time ELIGIBILITY (ADR-0002): the CURRENT repository
/// semantics for `control_plane.target_nodes` -- an empty list means no
/// node is eligible, mirroring `resolve_assignments_from_registry` exactly.
/// Never consulted again once a placement is accepted.
pub fn check_node_eligibility(artifact: &ServiceArtifact, node_id: &str) -> Option<CanonicalValidationFinding> {
    if artifact.control_plane.target_nodes.iter().any(|candidate| candidate == node_id) {
        None
    } else {
        Some(finding(F_NODE_NOT_ELIGIBLE, format!(
            "service '{}' artifact '{}' control_plane.target_nodes {:?} does not include node '{}'",
            artifact.service_id, artifact.artifact_id, artifact.control_plane.target_nodes, node_id
        )))
    }
}

/// Focused audit conclusion (independent review, recorded here and in
/// ADR-0018): no already-authoritative repository source maps a candidate
/// node to a D-Stage without inventing new semantics.
/// `dserver::core::capabilities::NodeCapabilities.stage` exists but is
/// self-reported, Firebase-backed, mutable, and defaults to `"unknown"` for
/// any node that never explicitly reported one -- it cannot serve as a
/// complete, authoritative mapping for every candidate node, and
/// `datum.dcontinuum/1` deliberately does not carry D-Stage (Stage 110).
/// `resolve_assignments_from_registry` itself never enforces
/// `target_stages` today. A non-empty `target_stages` therefore fails
/// closed rather than being silently ignored; every real checked-in
/// `ServiceArtifact` fixture declares `target_stages: []`.
pub fn check_target_stages(artifact: &ServiceArtifact) -> Option<CanonicalValidationFinding> {
    if artifact.control_plane.target_stages.is_empty() {
        None
    } else {
        Some(finding(F_TARGET_STAGE_UNVERIFIABLE, format!(
            "service artifact '{}' declares non-empty control_plane.target_stages {:?}; Stage 111 has no authoritative node-stage mapping to verify it against and rejects fail-closed rather than silently ignoring it",
            artifact.artifact_id, artifact.control_plane.target_stages
        )))
    }
}

pub fn build_candidate_dmap(proposal: &DDeployProposalV1, dmap_id: &str, revision: u64) -> CanonicalDMapV1 {
    CanonicalDMapV1 {
        schema: CANONICAL_DMAP_SCHEMA.to_string(),
        project_id: proposal.project_id.clone(),
        application_id: proposal.application_id.clone(),
        dmap_id: dmap_id.to_string(),
        revision,
        dgraph_ref: proposal.dgraph_ref.clone(),
        dcontinuum_ref: proposal.dcontinuum_ref.clone(),
        placements: proposal.placements.clone(),
    }
}

pub struct ProposalValidationOutcome {
    pub findings: Vec<CanonicalValidationFinding>,
    /// EPHEMERAL only -- never persisted or activated by this function.
    /// Only `accept_proposal` may turn a zero-finding outcome's candidate
    /// into the authoritative active D-Map.
    pub candidate_dmap: CanonicalDMapV1,
}

/// PURE. No active D-Map, no operational graph change, no host mutation, no
/// `target_nodes` mutation, no dispatch, no authorization issuance. Reuses
/// `CanonicalDMapV1::validate_against` verbatim (Stage 110) for schema/
/// digest/identity/ABI/resource-feasibility/cardinality checks -- this
/// function never re-implements any of that logic, it only ADDS the
/// Stage 111-specific artifact-eligibility layer on top.
pub fn validate_full(
    proposal: &DDeployProposalV1,
    dgraph: &CanonicalDGraphV1,
    dcontinuum: &CanonicalDContinuumV1,
    artifacts: &[ServiceArtifact],
    candidate_dmap_id: &str,
    candidate_revision: u64,
) -> ProposalValidationOutcome {
    let mut findings = validate_proposal_shape(proposal);
    let candidate = build_candidate_dmap(proposal, candidate_dmap_id, candidate_revision);
    findings.extend(candidate.validate_against(dgraph, dcontinuum));

    for placement in &proposal.placements {
        match resolve_artifact_for_service(&placement.service_id, artifacts) {
            Ok(artifact) => {
                if let Some(f) = check_node_eligibility(artifact, &placement.node_id) {
                    findings.push(f);
                }
                if let Some(f) = check_target_stages(artifact) {
                    findings.push(f);
                }
            }
            Err(f) => findings.push(f),
        }
    }

    ProposalValidationOutcome { findings, candidate_dmap: candidate }
}

/// Precompute, from an ALREADY-VALID candidate D-Map (every placement's
/// artifact already resolved without ambiguity -- see `validate_full`), the
/// per-node desired-service content this deployment will report once
/// active. Deterministic: `placements` sorted by `service_id`, output nodes
/// keyed and sorted by `node_id`, each node's services sorted by
/// `service_id`.
/// Stage 111 (independent review v3, single-placement-authority fix):
/// precompute, from an ALREADY-VALID candidate D-Map, the placement-
/// INDEPENDENT operational semantics (runtime, desired_state) for every
/// D-Map-placed service -- keyed by `service_id`, deliberately carrying NO
/// node identity anywhere. Node placement is supplied LATER, exclusively
/// by reading `active_dmap.placements` (see `union_project_operational_graph`)
/// -- this function's return value is never itself a placement carrier.
pub fn derive_application_operational_services(
    dmap: &CanonicalDMapV1,
    artifacts: &[ServiceArtifact],
) -> Result<BTreeMap<String, OperationalDesiredService>, Vec<CanonicalValidationFinding>> {
    let mut findings = Vec::new();
    let mut services: BTreeMap<String, OperationalDesiredService> = BTreeMap::new();

    for placement in &dmap.placements {
        match resolve_artifact_for_service(&placement.service_id, artifacts) {
            Ok(artifact) => {
                services.insert(placement.service_id.clone(), OperationalDesiredService {
                    service_id: placement.service_id.clone(),
                    runtime: artifact.runtime.kind.clone(),
                    desired_state: "running".to_string(),
                });
            }
            Err(f) => findings.push(f),
        }
    }

    if !findings.is_empty() {
        return Err(findings);
    }
    Ok(services)
}

/// Set-based, artifact-independent: two active applications in the same
/// project must never claim the same canonical `service_id` (the legacy
/// operational wire has no way to disambiguate them). Checked BEFORE
/// committing an acceptance -- by construction, `union_project_operational_graph`
/// (the read path) can never observe a collision once this has passed.
fn detect_service_id_collisions(
    project_id: &str,
    active_by_application: &BTreeMap<String, ActiveApplicationDeployment>,
) -> Vec<CanonicalValidationFinding> {
    let mut owner: BTreeMap<&str, &str> = BTreeMap::new();
    let mut findings = Vec::new();
    let mut apps: Vec<&ActiveApplicationDeployment> = active_by_application
        .values()
        .filter(|a| a.active_dmap.project_id == project_id)
        .collect();
    apps.sort_by(|a, b| a.application_id.cmp(&b.application_id));

    for app in apps {
        let mut service_ids: Vec<&str> = app.active_dmap.placements.iter().map(|p| p.service_id.as_str()).collect();
        service_ids.sort_unstable();
        for service_id in service_ids {
            match owner.get(service_id) {
                Some(existing_app) if *existing_app != app.application_id.as_str() => {
                    findings.push(finding(F_OPERATIONAL_SERVICE_ID_COLLISION, format!(
                        "service_id '{service_id}' is placed by both application '{existing_app}' and application '{}'",
                        app.application_id
                    )));
                }
                _ => {
                    owner.insert(service_id, &app.application_id);
                }
            }
        }
    }
    findings
}

/// The project-level UNION of every active application's operational
/// content (independent review's preferred v0.1 multi-application design).
/// Deterministic ordering: nodes and each node's services sorted by id.
///
/// Independent review v3, single-persisted-placement-authority fix: node
/// assignment is read EXCLUSIVELY from each application's own
/// `active_dmap.placements` -- `operational_services` (keyed by
/// `service_id`, no node identity) supplies only the runtime/desired_state
/// payload for a service the D-Map has ALREADY placed. There is no other
/// source of node assignment anywhere in this function.
pub fn union_project_operational_graph(
    project_id: &str,
    active_by_application: &BTreeMap<String, ActiveApplicationDeployment>,
    operational_revision: u64,
) -> Result<OperationalDGraph, Vec<CanonicalValidationFinding>> {
    let collisions = detect_service_id_collisions(project_id, active_by_application);
    if !collisions.is_empty() {
        return Err(collisions);
    }

    let mut apps: Vec<&ActiveApplicationDeployment> = active_by_application
        .values()
        .filter(|a| a.active_dmap.project_id == project_id)
        .collect();
    apps.sort_by(|a, b| a.application_id.cmp(&b.application_id));

    let mut merged_by_node: BTreeMap<String, Vec<OperationalDesiredService>> = BTreeMap::new();
    let mut application_provenance = Vec::new();
    let mut findings = Vec::new();
    for app in &apps {
        // The ONE and ONLY source of service->node assignment: the
        // accepted D-Map's own placements. `operational_services` is
        // consulted ONLY to fetch the runtime/desired_state payload for a
        // service this loop already knows (from the D-Map) is placed here.
        for placement in &app.active_dmap.placements {
            match app.operational_services.get(&placement.service_id) {
                Some(service) => {
                    merged_by_node.entry(placement.node_id.clone()).or_default().push(service.clone());
                }
                None => {
                    // Fail closed rather than silently omitting the
                    // service: this shape can only arise from corrupted/
                    // hand-edited state (accept_proposal always derives
                    // `operational_services` from the SAME `dmap.placements`
                    // it stores), and this is exactly the corruption a
                    // persisted-state reload must never silently tolerate.
                    findings.push(finding(F_OPERATIONAL_PROJECTION_INVALID, format!(
                        "application '{}' operational_services is missing service '{}', which its own active_dmap places on node '{}'",
                        app.application_id, placement.service_id, placement.node_id
                    )));
                }
            }
        }
        application_provenance.push(serde_json::json!({
            "application_id": app.application_id,
            "dmap_id": app.active_dmap.dmap_id,
            "dmap_revision": app.active_dmap.revision,
            "dmap_digest": app.dmap_digest,
            "dgraph_ref": app.active_dmap.dgraph_ref,
            "dcontinuum_ref": app.active_dmap.dcontinuum_ref,
            "proposal_id": app.proposal_id,
            "acceptance_id": app.acceptance_id,
        }));
    }
    if !findings.is_empty() {
        return Err(findings);
    }

    let mut nodes: Vec<OperationalNodeDesiredState> = merged_by_node
        .into_iter()
        .map(|(node_id, mut desired_services)| {
            desired_services.sort_by(|a, b| a.service_id.cmp(&b.service_id));
            OperationalNodeDesiredState { node_id, desired_services }
        })
        .collect();
    nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    Ok(OperationalDGraph {
        schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
        dgraph_id: operational_dgraph_id_for_project(project_id),
        revision: operational_revision,
        project_id: project_id.to_string(),
        nodes,
        metadata: Some(serde_json::json!({
            "placement_authority": "canonical_dmap",
            "applications": application_provenance,
        })),
    })
}

// --------------------------------------------------------------------------- //
// Proposal creation                                                            //
// --------------------------------------------------------------------------- //

pub struct CreateProposalInput {
    pub project_id: String,
    pub application_id: String,
    pub source: ProposalSource,
    pub dgraph: CanonicalDGraphV1,
    pub dcontinuum: CanonicalDContinuumV1,
    pub placements: Vec<CanonicalDServPlacementV1>,
}

/// DServer validates both contracts, recomputes their digests itself, and
/// constructs the proposal's exact refs itself -- it NEVER trusts a caller-
/// supplied digest as authoritative (there is no such field on
/// `CreateProposalInput` to trust in the first place).
pub fn create_proposal(
    state: &mut DDeployStateV1,
    input: CreateProposalInput,
    proposal_id: String,
    created_at_utc: String,
    artifacts: &[ServiceArtifact],
) -> Result<ProposalRecord, Vec<CanonicalValidationFinding>> {
    let mut findings = input.dgraph.validate();
    findings.extend(input.dcontinuum.validate());
    if !findings.is_empty() {
        return Err(findings);
    }

    let dgraph_digest = recompute_dgraph_digest(&input.dgraph).map_err(|error| vec![finding("internal_error", error)])?;
    let dcontinuum_digest = recompute_dcontinuum_digest(&input.dcontinuum).map_err(|error| vec![finding("internal_error", error)])?;

    let expected_active_dmap_revision = state
        .active_for(&input.project_id, &input.application_id)
        .map(|active| active.active_dmap.revision);

    let proposal = DDeployProposalV1 {
        schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
        proposal_id: proposal_id.clone(),
        project_id: input.project_id.clone(),
        application_id: input.application_id.clone(),
        source: input.source,
        created_at_utc,
        expected_active_dmap_revision,
        dgraph_ref: CanonicalContractReferenceV1 {
            id: input.dgraph.dgraph_id.clone(),
            revision: input.dgraph.revision,
            digest: dgraph_digest,
        },
        dcontinuum_ref: CanonicalContractReferenceV1 {
            id: input.dcontinuum.dcontinuum_id.clone(),
            revision: input.dcontinuum.revision,
            digest: dcontinuum_digest,
        },
        placements: input.placements,
    };

    let dmap_id_hint = canonical_dmap_id(&input.project_id, &input.application_id);
    let next_revision_hint = expected_active_dmap_revision.unwrap_or(0) + 1;
    let outcome = validate_full(&proposal, &input.dgraph, &input.dcontinuum, artifacts, &dmap_id_hint, next_revision_hint);
    if !outcome.findings.is_empty() {
        return Err(outcome.findings);
    }

    let proposal_digest = compute_proposal_digest(&proposal).map_err(|error| vec![finding("internal_error", error)])?;

    let record = ProposalRecord {
        proposal,
        proposal_digest,
        dgraph_snapshot: input.dgraph,
        dcontinuum_snapshot: input.dcontinuum,
        state: ProposalLifecycleState::Pending,
        accepted_acceptance_id: None,
    };
    state.proposals.insert(proposal_id, record.clone());
    Ok(record)
}

// --------------------------------------------------------------------------- //
// Acceptance                                                                   //
// --------------------------------------------------------------------------- //

#[derive(Debug, Clone)]
pub struct AcceptOutcome {
    pub acceptance: DDeployAcceptanceV1,
    /// The D-Map THIS acceptance call materialized -- for a fresh
    /// acceptance, always the newly active one; for an idempotent replay of
    /// an ALREADY-accepted proposal, the ORIGINAL D-Map that proposal's OWN
    /// acceptance produced (independent review v2, P1 fix), which may or
    /// may not still be the CURRENT active D-Map for the application if a
    /// different, later proposal has since been accepted. Check
    /// `currently_active` -- never assume this field reflects the current
    /// state; `GET /api/v1/ddeploy/active` is the source for that.
    pub materialized_dmap: CanonicalDMapV1,
    pub operational_dgraph_id: String,
    pub operational_revision: u64,
    /// `true` iff this call replayed an ALREADY-accepted proposal
    /// idempotently -- no new D-Map revision, no new operational revision,
    /// no duplicated acceptance record was created.
    pub idempotent_replay: bool,
    /// `true` iff `materialized_dmap` is STILL the current active D-Map for
    /// this `(project_id, application_id)` at the moment this call
    /// returned. Always `true` for a fresh acceptance and for an
    /// idempotent replay of the MOST RECENTLY accepted proposal; `false`
    /// when replaying a proposal that has since been SUPERSEDED by a later
    /// accepted proposal for the same application (independent review v2,
    /// P1 fix) -- `materialized_dmap` is still that proposal's own
    /// historically-correct materialization, just no longer authoritative.
    pub currently_active: bool,
}

#[derive(Debug, Clone)]
pub enum AcceptError {
    NotFound(String),
    DigestMismatch(String),
    Invalid(Vec<CanonicalValidationFinding>),
    Stale(String),
    Collision(Vec<CanonicalValidationFinding>),
    Internal(String),
}

/// Mutates `state` IN MEMORY ONLY on the success path -- every validation
/// step runs against clones/reads before any field of `state` is written.
/// The caller (the accept API handler) is responsible for persisting the
/// WHOLE mutated `state` in exactly ONE `persist_state` call after this
/// returns `Ok`; if that persist fails, the caller must discard the
/// mutated in-memory copy and leave the on-disk state (and therefore the
/// previously active deployment) untouched. This function itself never
/// touches disk, so a failure here can never leave a half-written domain.
pub fn accept_proposal(
    state: &mut DDeployStateV1,
    proposal_id: &str,
    accepted_by: &str,
    expected_proposal_digest: Option<&str>,
    artifacts: &[ServiceArtifact],
    now_utc: &str,
    new_acceptance_id: String,
) -> Result<AcceptOutcome, AcceptError> {
    let record = state
        .proposals
        .get(proposal_id)
        .cloned()
        .ok_or_else(|| AcceptError::NotFound(format!("proposal '{proposal_id}' not found")))?;

    // P0-3 (independent review v2): recompute the digest of the ACTUAL
    // persisted proposal bytes and verify it against the recorded
    // `proposal_digest` BEFORE anything else -- including the idempotent
    // replay branch below. Without this, a persisted `ProposalRecord` whose
    // `proposal` field was altered after creation (e.g. `placements`
    // changed) while `proposal_digest` was left stale would silently pass
    // every later check, since nothing else in this function ever
    // recomputes the digest from the CURRENT proposal bytes. This closes
    // that gap unconditionally, for every code path below.
    let actual_proposal_digest = compute_proposal_digest(&record.proposal).map_err(AcceptError::Internal)?;
    if actual_proposal_digest != record.proposal_digest {
        return Err(AcceptError::DigestMismatch(format!(
            "persisted proposal '{proposal_id}' is internally inconsistent: its recomputed digest '{actual_proposal_digest}' does not match its recorded proposal_digest '{}'",
            record.proposal_digest
        )));
    }
    if let Some(expected) = expected_proposal_digest {
        if expected != actual_proposal_digest {
            return Err(AcceptError::DigestMismatch(format!(
                "expected_proposal_digest '{expected}' does not match proposal's verified digest '{actual_proposal_digest}'"
            )));
        }
    }

    // Idempotency: replay the existing result, mutate nothing.
    if record.state == ProposalLifecycleState::Accepted {
        let acceptance_id = record
            .accepted_acceptance_id
            .clone()
            .ok_or_else(|| AcceptError::Internal("accepted proposal is missing its acceptance_id".to_string()))?;
        let acceptance = state
            .acceptances
            .get(&acceptance_id)
            .cloned()
            .ok_or_else(|| AcceptError::Internal("accepted proposal's acceptance record is missing".to_string()))?;

        // P1 fix (independent review v2): reconstruct THIS proposal's OWN
        // original materialization deterministically from the immutable
        // proposal + the acceptance's own recorded ref -- NEVER from
        // `state.active_by_application`, which reflects whichever proposal
        // was accepted MOST RECENTLY for this application and may since be
        // a DIFFERENT proposal entirely. `build_candidate_dmap` is a pure
        // function of `record.proposal` plus the two id/revision values
        // already recorded in `acceptance.materialized_dmap_ref` -- it
        // reproduces exactly what this proposal's own acceptance produced.
        let historical_dmap = build_candidate_dmap(
            &record.proposal,
            &acceptance.materialized_dmap_ref.id,
            acceptance.materialized_dmap_ref.revision,
        );
        let historical_digest = recompute_dmap_digest(&historical_dmap).map_err(AcceptError::Internal)?;
        if historical_digest != acceptance.materialized_dmap_ref.digest {
            return Err(AcceptError::Internal(format!(
                "persisted acceptance '{acceptance_id}' is internally inconsistent: recomputed historical D-Map digest '{historical_digest}' does not match its own materialized_dmap_ref.digest '{}'",
                acceptance.materialized_dmap_ref.digest
            )));
        }

        let key = application_key(&record.proposal.project_id, &record.proposal.application_id);
        let currently_active = state
            .active_by_application
            .get(&key)
            .is_some_and(|active| active.active_dmap.dmap_id == historical_dmap.dmap_id && active.active_dmap.revision == historical_dmap.revision);

        return Ok(AcceptOutcome {
            // P1 fix: `operational_dgraph_id`/`operational_revision` come
            // from THIS acceptance's own recorded `operational_projection_ref`
            // -- the projection revision that was current WHEN THIS
            // PROPOSAL was accepted -- never from the live
            // `project_operational_revision` counter, which may since have
            // been bumped by a later, different proposal's acceptance.
            operational_dgraph_id: acceptance.operational_projection_ref.operational_dgraph_id.clone(),
            operational_revision: acceptance.operational_projection_ref.revision,
            acceptance,
            materialized_dmap: historical_dmap,
            idempotent_replay: true,
            currently_active,
        });
    }

    // Compare-and-swap against the CURRENT active revision (re-checked now,
    // not merely at proposal-creation time).
    let key = application_key(&record.proposal.project_id, &record.proposal.application_id);
    let current_active_revision = state.active_by_application.get(&key).map(|active| active.active_dmap.revision);
    if record.proposal.expected_active_dmap_revision != current_active_revision {
        return Err(AcceptError::Stale(format!(
            "expected_active_dmap_revision {:?} does not match the current active revision {:?} for application '{}': another proposal was accepted first",
            record.proposal.expected_active_dmap_revision, current_active_revision, record.proposal.application_id
        )));
    }

    let next_revision = current_active_revision.unwrap_or(0) + 1;
    let dmap_id = canonical_dmap_id(&record.proposal.project_id, &record.proposal.application_id);

    // Acceptance-time REVALIDATION -- a proposal valid yesterday is not
    // automatically acceptable today. Re-runs the identical pure validator
    // against the immutable snapshot and the CURRENT artifact registry.
    let outcome = validate_full(&record.proposal, &record.dgraph_snapshot, &record.dcontinuum_snapshot, artifacts, &dmap_id, next_revision);
    if !outcome.findings.is_empty() {
        return Err(AcceptError::Invalid(outcome.findings));
    }

    let dmap_digest = recompute_dmap_digest(&outcome.candidate_dmap).map_err(AcceptError::Internal)?;
    let operational_services = derive_application_operational_services(&outcome.candidate_dmap, artifacts).map_err(AcceptError::Invalid)?;

    let mut updated_active = state.active_by_application.clone();
    updated_active.insert(
        key,
        ActiveApplicationDeployment {
            application_id: record.proposal.application_id.clone(),
            active_dmap: outcome.candidate_dmap.clone(),
            dmap_digest: dmap_digest.clone(),
            proposal_id: proposal_id.to_string(),
            acceptance_id: new_acceptance_id.clone(),
            accepted_at_utc: now_utc.to_string(),
            operational_services,
        },
    );

    let previous_operational_revision = *state.project_operational_revision.get(&record.proposal.project_id).unwrap_or(&0);
    let new_operational_revision = previous_operational_revision + 1;

    // Independent review v3: build the CANDIDATE project-wide projection
    // (this also performs cross-application service_id collision
    // detection, superseding the standalone `detect_service_id_collisions`
    // call v1/v2 made here) and validate it against the existing
    // `OperationalDGraph` contract BEFORE committing anything. An
    // acceptance that cannot currently be projected into a valid
    // `OperationalDGraph` (e.g. a zero-service/zero-placement application --
    // legal at the Stage 110 canonical level, ADR-0017, but not
    // operationally activatable through this v0.1 bridge) must never
    // become authoritative state.
    let candidate_projection = union_project_operational_graph(&record.proposal.project_id, &updated_active, new_operational_revision)
        .map_err(AcceptError::Collision)?;
    if let Err(error) = crate::core::operational_dgraph::validate_operational_dgraph(&candidate_projection) {
        return Err(AcceptError::Invalid(vec![finding(F_OPERATIONAL_PROJECTION_INVALID, error)]));
    }
    let operational_dgraph_id = candidate_projection.dgraph_id.clone();

    let acceptance = DDeployAcceptanceV1 {
        schema: DDEPLOY_ACCEPTANCE_SCHEMA.to_string(),
        acceptance_id: new_acceptance_id.clone(),
        proposal_id: proposal_id.to_string(),
        proposal_digest: record.proposal_digest.clone(),
        project_id: record.proposal.project_id.clone(),
        application_id: record.proposal.application_id.clone(),
        accepted_at_utc: now_utc.to_string(),
        accepted_by: accepted_by.to_string(),
        materialized_dmap_ref: CanonicalContractReferenceV1 {
            id: dmap_id,
            revision: next_revision,
            digest: dmap_digest,
        },
        operational_projection_ref: OperationalProjectionRefV1 {
            operational_dgraph_id: operational_dgraph_id.clone(),
            revision: new_operational_revision,
        },
    };

    // Commit -- every prior step only read or cloned; this is the ONLY
    // place `state`'s fields are mutated, and only after every check above
    // passed.
    state.active_by_application = updated_active;
    state.acceptances.insert(new_acceptance_id.clone(), acceptance.clone());
    state.project_operational_revision.insert(record.proposal.project_id.clone(), new_operational_revision);
    if let Some(persisted_proposal) = state.proposals.get_mut(proposal_id) {
        persisted_proposal.state = ProposalLifecycleState::Accepted;
        persisted_proposal.accepted_acceptance_id = Some(new_acceptance_id);
    }

    Ok(AcceptOutcome {
        acceptance,
        materialized_dmap: outcome.candidate_dmap,
        operational_dgraph_id,
        operational_revision: new_operational_revision,
        idempotent_replay: false,
        // A fresh acceptance always makes its own materialization the
        // current active D-Map for this application, at the instant it
        // commits.
        currently_active: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
        CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        CanonicalDGraphV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1,
        CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
    };
    use crate::core::service_artifact_registry::{
        ServiceArtifactControlPlane, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
        SERVICE_ARTIFACT_SCHEMA_VERSION,
    };

    fn sample_dgraph(app: &str, revision: u64) -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: app.to_string(),
            dgraph_id: format!("dgraph:{app}"),
            revision,
            services: vec![
                CanonicalDServV1 {
                    service_id: "svc-a".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 134_217_728 },
                },
                CanonicalDServV1 {
                    service_id: "svc-b".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 134_217_728 },
                },
            ],
            calls: vec![],
        }
    }

    fn sample_dcontinuum(project: &str, revision: u64) -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project.to_string(),
            dcontinuum_id: format!("continuum:{project}"),
            revision,
            nodes: vec![
                CanonicalDNodeV1 {
                    node_id: "fog-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                    capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_294_967_296 },
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
                },
                CanonicalDNodeV1 {
                    node_id: "cloud-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                    capacity: CanonicalNodeCapacityV1 { cpu_cores: 8, memory_bytes: 17_179_869_184 },
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
                },
            ],
            links: vec![],
        }
    }

    fn sample_artifact(service_id: &str, target_nodes: Vec<&str>) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: format!("artifact:{service_id}"),
            service_id: service_id.to_string(),
            runtime: ServiceRuntimeSpec { kind: "container".to_string(), container: None, native_process: None },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec { kind: "none".to_string(), params: Default::default(), vantage_point: None },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                target_nodes: target_nodes.into_iter().map(str::to_string).collect(),
                ..Default::default()
            },
        }
    }

    fn sample_placements() -> Vec<CanonicalDServPlacementV1> {
        vec![
            CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() },
            CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "cloud-01".to_string() },
        ]
    }

    fn sample_artifacts() -> Vec<ServiceArtifact> {
        vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["cloud-01"])]
    }

    // ---- identifier composition (independent review v2) -----------------

    /// The naive, PRE-FIX join this test proves would have collided --
    /// kept here ONLY as a local reference for the regression proof below,
    /// never used by production code.
    fn naive_join(project_id: &str, application_id: &str) -> String {
        format!("dmap:{project_id}:{application_id}")
    }

    #[test]
    fn naive_colon_join_would_have_collided_but_length_prefixed_encoding_does_not() {
        // Two DIFFERENT (project_id, application_id) pairs that a naive
        // "dmap:{project}:{app}" join maps to the SAME string, because
        // `application_id` itself legitimately contains a ':' (exactly the
        // shape this repository's own real fixtures use, e.g.
        // "app:smart-sentinel-lab", ADR-0017).
        let pair_a = ("p", "a:b");
        let pair_b = ("p:a", "b");
        assert_eq!(
            naive_join(pair_a.0, pair_a.1),
            naive_join(pair_b.0, pair_b.1),
            "sanity check: the OLD naive join must actually collide for this pair, or this test proves nothing"
        );

        // The REAL, fixed `canonical_dmap_id` must NOT collide for the same pair.
        assert_ne!(
            canonical_dmap_id(pair_a.0, pair_a.1),
            canonical_dmap_id(pair_b.0, pair_b.1),
            "canonical_dmap_id must not collide for distinct (project_id, application_id) pairs, even when application_id contains ':'"
        );
        assert_ne!(
            application_key(pair_a.0, pair_a.1),
            application_key(pair_b.0, pair_b.1),
            "application_key must not collide for distinct (project_id, application_id) pairs"
        );
    }

    #[test]
    fn identifier_composition_is_collision_free_across_many_adversarial_pairs() {
        // A broader sweep: any two DIFFERENT pairs drawn from this set
        // (deliberately including empty-ish, colon-heavy, and
        // length-prefix-lookalike strings) must never produce the same
        // `canonical_dmap_id` or `application_key`.
        let candidates: Vec<(&str, &str)> = vec![
            ("p", "a:b"),
            ("p:a", "b"),
            ("1:x", "y"),
            ("1", "x:y"),
            ("a", "b:c:d"),
            ("a:b", "c:d"),
            ("a:b:c", "d"),
            ("", "x"),
            ("x", ""),
            ("proj", "app"),
            ("pro:j", "app"),
        ];
        for i in 0..candidates.len() {
            for j in 0..candidates.len() {
                if i == j {
                    continue;
                }
                let (p1, a1) = candidates[i];
                let (p2, a2) = candidates[j];
                if p1 == p2 && a1 == a2 {
                    continue;
                }
                assert_ne!(
                    canonical_dmap_id(p1, a1),
                    canonical_dmap_id(p2, a2),
                    "canonical_dmap_id collision between ({p1:?}, {a1:?}) and ({p2:?}, {a2:?})"
                );
                assert_ne!(
                    application_key(p1, a1),
                    application_key(p2, a2),
                    "application_key collision between ({p1:?}, {a1:?}) and ({p2:?}, {a2:?})"
                );
            }
        }
    }

    // ---- proposal contract ---- //

    #[test]
    fn proposal_digest_is_deterministic() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut state = DDeployStateV1::empty();
        let record = create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum,
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        let recomputed = compute_proposal_digest(&record.proposal).unwrap();
        assert_eq!(recomputed, record.proposal_digest);
        assert!(recomputed.starts_with("sha256:"));
    }

    #[test]
    fn proposal_digest_changes_when_placement_changes() {
        let mut state1 = DDeployStateV1::empty();
        let record1 = create_proposal(
            &mut state1,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        let mut swapped = sample_placements();
        swapped[0].node_id = "cloud-01".to_string();
        swapped[1].node_id = "cloud-01".to_string();
        let swapped_artifacts = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        let mut state2 = DDeployStateV1::empty();
        let record2 = create_proposal(
            &mut state2,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: swapped,
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &swapped_artifacts,
        )
        .unwrap();

        assert_ne!(record1.proposal_digest, record2.proposal_digest);
    }

    #[test]
    fn proposal_digest_changes_when_source_refs_change() {
        let mut state1 = DDeployStateV1::empty();
        let record1 = create_proposal(
            &mut state1,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        let mut state2 = DDeployStateV1::empty();
        let record2 = create_proposal(
            &mut state2,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 2), // different D-Graph revision -> different ref
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        assert_ne!(record1.proposal_digest, record2.proposal_digest);
    }

    #[test]
    fn wrong_schema_rejected() {
        let mut proposal_state = DDeployStateV1::empty();
        let record = create_proposal(
            &mut proposal_state,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        let mut proposal = record.proposal;
        proposal.schema = "wrong/1".to_string();
        assert!(validate_proposal_shape(&proposal).iter().any(|f| f.code == F_PROPOSAL_SCHEMA_INVALID));
    }

    #[test]
    fn empty_proposal_id_rejected() {
        let mut proposal = valid_proposal_for_shape_tests();
        proposal.proposal_id = "  ".to_string();
        assert!(validate_proposal_shape(&proposal).iter().any(|f| f.code == F_MISSING_FIELD));
    }

    fn valid_proposal_for_shape_tests() -> DDeployProposalV1 {
        DDeployProposalV1 {
            schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
            proposal_id: "proposal-1".to_string(),
            project_id: "proj-x".to_string(),
            application_id: "app-x".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            expected_active_dmap_revision: None,
            dgraph_ref: CanonicalContractReferenceV1 { id: "dgraph:app-x".to_string(), revision: 1, digest: format!("sha256:{}", "a".repeat(64)) },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: "continuum:proj-x".to_string(), revision: 1, digest: format!("sha256:{}", "b".repeat(64)) },
            placements: sample_placements(),
        }
    }

    #[test]
    fn wrong_project_rejected_by_context_check() {
        let proposal = valid_proposal_for_shape_tests();
        let findings = validate_proposal_context(&proposal, "some-other-project");
        assert!(findings.iter().any(|f| f.code == F_PROPOSAL_PROJECT_MISMATCH));
    }

    #[test]
    fn malformed_canonical_ref_digest_rejected() {
        let mut proposal = valid_proposal_for_shape_tests();
        proposal.dgraph_ref.digest = "not-a-digest".to_string();
        assert!(validate_proposal_shape(&proposal).iter().any(|f| f.code == F_INVALID_DIGEST_FORMAT));
    }

    #[test]
    fn no_dmap_materialized_merely_by_proposal_creation() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        assert!(state.active_by_application.is_empty(), "proposal creation alone must never materialize an active D-Map");
        assert!(!state.has_active_authority("proj-x"));
    }

    #[test]
    fn immutable_snapshot_reload_round_trips() {
        let mut state = DDeployStateV1::empty();
        let record = create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        let json = serde_json::to_string(&record).unwrap();
        let reloaded: ProposalRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(reloaded, record);
    }

    // ---- pure validation ---- //

    #[test]
    fn validate_endpoint_style_call_has_no_side_effects() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let dgraph_digest = recompute_dgraph_digest(&dgraph).unwrap();
        let dcontinuum_digest = recompute_dcontinuum_digest(&dcontinuum).unwrap();
        let proposal = DDeployProposalV1 {
            schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
            proposal_id: "preview".to_string(),
            project_id: "proj-x".to_string(),
            application_id: "app-x".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            expected_active_dmap_revision: None,
            dgraph_ref: CanonicalContractReferenceV1 { id: dgraph.dgraph_id.clone(), revision: dgraph.revision, digest: dgraph_digest },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: dcontinuum_digest },
            placements: sample_placements(),
        };
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:preview", 1);
        assert_eq!(outcome.findings, Vec::new());
        // No global/mutable state exists for this pure function to have
        // touched; the absence of any `state: &mut ...` parameter here is
        // itself the proof.
    }

    #[test]
    fn invalid_abi_mapping_rejected() {
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services[0].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let proposal = proposal_for(&dgraph, &dcontinuum, sample_placements());
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "invalid_dnode_abi"));
    }

    fn proposal_for(dgraph: &CanonicalDGraphV1, dcontinuum: &CanonicalDContinuumV1, placements: Vec<CanonicalDServPlacementV1>) -> DDeployProposalV1 {
        DDeployProposalV1 {
            schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
            proposal_id: "p".to_string(),
            project_id: dcontinuum.project_id.clone(),
            application_id: dgraph.application_id.clone(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            expected_active_dmap_revision: None,
            dgraph_ref: CanonicalContractReferenceV1 { id: dgraph.dgraph_id.clone(), revision: dgraph.revision, digest: recompute_dgraph_digest(dgraph).unwrap() },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: recompute_dcontinuum_digest(dcontinuum).unwrap() },
            placements,
        }
    }

    #[test]
    fn individual_cpu_over_capacity_rejected() {
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services[0].resource_requirement.cpu_cores = 100;
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let proposal = proposal_for(&dgraph, &dcontinuum, sample_placements());
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "insufficient_node_cpu"));
    }

    #[test]
    fn aggregate_cpu_over_capacity_rejected() {
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services[0].resource_requirement.cpu_cores = 3;
        dgraph.services[1].resource_requirement.cpu_cores = 3;
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        // both on fog-01 (capacity 4)
        let placements = vec![
            CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() },
            CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "fog-01".to_string() },
        ];
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];
        let proposal = proposal_for(&dgraph, &dcontinuum, placements);
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &artifacts, "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "aggregate_node_cpu_exceeded"));
    }

    #[test]
    fn individual_memory_over_capacity_rejected() {
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services[0].resource_requirement.memory_bytes = 8 * 1024 * 1024 * 1024;
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let proposal = proposal_for(&dgraph, &dcontinuum, sample_placements());
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "insufficient_node_memory"));
    }

    #[test]
    fn aggregate_memory_over_capacity_rejected() {
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services[0].resource_requirement.memory_bytes = 3_221_225_472;
        dgraph.services[1].resource_requirement.memory_bytes = 3_221_225_472;
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let placements = vec![
            CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() },
            CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "fog-01".to_string() },
        ];
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];
        let proposal = proposal_for(&dgraph, &dcontinuum, placements);
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &artifacts, "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "aggregate_node_memory_exceeded"));
    }

    #[test]
    fn missing_placement_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let placements = vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }];
        let proposal = proposal_for(&dgraph, &dcontinuum, placements);
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "missing_placement"));
    }

    #[test]
    fn duplicate_placement_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut placements = sample_placements();
        let dup = placements[0].clone();
        placements.push(dup);
        let proposal = proposal_for(&dgraph, &dcontinuum, placements);
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "duplicate_placement"));
    }

    #[test]
    fn unknown_service_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut placements = sample_placements();
        placements.push(CanonicalDServPlacementV1 { service_id: "does-not-exist".to_string(), node_id: "fog-01".to_string() });
        let proposal = proposal_for(&dgraph, &dcontinuum, placements);
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "unknown_service"));
    }

    #[test]
    fn unknown_node_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut placements = sample_placements();
        placements[0].node_id = "does-not-exist".to_string();
        let proposal = proposal_for(&dgraph, &dcontinuum, placements);
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "unknown_node"));
    }

    #[test]
    fn source_digest_mismatch_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut proposal = proposal_for(&dgraph, &dcontinuum, sample_placements());
        proposal.dgraph_ref.digest = format!("sha256:{}", "a".repeat(64));
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert!(outcome.findings.iter().any(|f| f.code == "dgraph_digest_mismatch"));
    }

    #[test]
    fn dcall_cycles_remain_legal_in_proposal_validation() {
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.calls = vec![
            crate::core::canonical_dgraph::CanonicalDCallV1 { call_id: "a-to-b".to_string(), source_service_id: "svc-a".to_string(), target_service_id: "svc-b".to_string() },
            crate::core::canonical_dgraph::CanonicalDCallV1 { call_id: "b-to-a".to_string(), source_service_id: "svc-b".to_string(), target_service_id: "svc-a".to_string() },
        ];
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let proposal = proposal_for(&dgraph, &dcontinuum, sample_placements());
        let outcome = validate_full(&proposal, &dgraph, &dcontinuum, &sample_artifacts(), "dmap:x", 1);
        assert_eq!(outcome.findings, Vec::new(), "a legal D-Call cycle must never affect placement validation: {:?}", outcome.findings);
    }

    // ---- artifact eligibility ---- //

    #[test]
    fn alias_match_resolves_artifact() {
        let mut artifact = sample_artifact("real-artifact-id", vec!["fog-01"]);
        artifact.metadata = Some(serde_json::json!({"service_aliases": ["svc-a"]}));
        let resolved = resolve_artifact_for_service("svc-a", std::slice::from_ref(&artifact)).unwrap();
        assert_eq!(resolved.artifact_id, "artifact:real-artifact-id");
    }

    #[test]
    fn zero_artifact_match_rejected() {
        let result = resolve_artifact_for_service("svc-a", &[]);
        assert!(matches!(result, Err(f) if f.code == F_ARTIFACT_NOT_FOUND));
    }

    #[test]
    fn ambiguous_artifact_match_rejected() {
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), {
            let mut a = sample_artifact("other", vec!["fog-01"]);
            a.metadata = Some(serde_json::json!({"service_aliases": ["svc-a"]}));
            a
        }];
        let result = resolve_artifact_for_service("svc-a", &artifacts);
        assert!(matches!(result, Err(f) if f.code == F_ARTIFACT_AMBIGUOUS));
    }

    #[test]
    fn candidate_node_absent_from_target_nodes_rejected() {
        let artifact = sample_artifact("svc-a", vec!["cloud-01"]);
        let finding = check_node_eligibility(&artifact, "fog-01");
        assert!(finding.is_some_and(|f| f.code == F_NODE_NOT_ELIGIBLE));
    }

    #[test]
    fn empty_target_nodes_means_no_eligible_node() {
        let artifact = sample_artifact("svc-a", vec![]);
        let finding = check_node_eligibility(&artifact, "fog-01");
        assert!(finding.is_some_and(|f| f.code == F_NODE_NOT_ELIGIBLE));
    }

    #[test]
    fn non_empty_target_stages_is_unverifiable() {
        let mut artifact = sample_artifact("svc-a", vec!["fog-01"]);
        artifact.control_plane.target_stages = vec!["fog".to_string()];
        let finding = check_target_stages(&artifact);
        assert!(finding.is_some_and(|f| f.code == F_TARGET_STAGE_UNVERIFIABLE));
    }

    #[test]
    fn empty_target_stages_is_fine() {
        let artifact = sample_artifact("svc-a", vec!["fog-01"]);
        assert!(check_target_stages(&artifact).is_none());
    }

    // ---- acceptance ---- //

    fn create_and_accept(
        state: &mut DDeployStateV1,
        project_id: &str,
        application_id: &str,
        proposal_id: &str,
        placements: Vec<CanonicalDServPlacementV1>,
        artifacts: &[ServiceArtifact],
        accepted_by: &str,
        acceptance_id: &str,
        now: &str,
    ) -> Result<AcceptOutcome, AcceptError> {
        create_and_accept_with_dgraph(state, project_id, application_id, sample_dgraph(application_id, 1), proposal_id, placements, artifacts, accepted_by, acceptance_id, now)
    }

    #[allow(clippy::too_many_arguments)]
    fn create_and_accept_with_dgraph(
        state: &mut DDeployStateV1,
        project_id: &str,
        application_id: &str,
        dgraph: CanonicalDGraphV1,
        proposal_id: &str,
        placements: Vec<CanonicalDServPlacementV1>,
        artifacts: &[ServiceArtifact],
        accepted_by: &str,
        acceptance_id: &str,
        now: &str,
    ) -> Result<AcceptOutcome, AcceptError> {
        create_proposal(
            state,
            CreateProposalInput {
                project_id: project_id.to_string(),
                application_id: application_id.to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum: sample_dcontinuum(project_id, 1),
                placements,
            },
            proposal_id.to_string(),
            now.to_string(),
            artifacts,
        )
        .map_err(AcceptError::Invalid)?;
        accept_proposal(state, proposal_id, accepted_by, None, artifacts, now, acceptance_id.to_string())
    }

    #[test]
    fn valid_pending_proposal_accepted_and_materializes_revision_1() {
        let mut state = DDeployStateV1::empty();
        let outcome = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(outcome.materialized_dmap.revision, 1);
        assert!(!outcome.idempotent_replay);
        assert_eq!(outcome.acceptance.accepted_by, "operator:alice");
    }

    #[test]
    fn next_valid_acceptance_reaches_revision_2() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        let mut swapped = sample_placements();
        swapped[0].node_id = "cloud-01".to_string();
        let artifacts2 = vec![sample_artifact("svc-a", vec!["cloud-01", "fog-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: swapped,
            },
            "p2".to_string(),
            "2026-01-01T01:00:00Z".to_string(),
            &artifacts2,
        )
        .unwrap();
        let outcome2 = accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts2, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        assert_eq!(outcome2.materialized_dmap.revision, 2);
    }

    #[test]
    fn accepting_same_proposal_twice_is_idempotent() {
        let mut state = DDeployStateV1::empty();
        let first = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let second = accept_proposal(&mut state, "p1", "operator:bob", None, &sample_artifacts(), "2026-01-01T02:00:00Z", "should-not-be-used".to_string()).unwrap();
        assert!(second.idempotent_replay);
        assert_eq!(second.materialized_dmap.revision, first.materialized_dmap.revision);
        assert_eq!(second.acceptance.acceptance_id, first.acceptance.acceptance_id);
        assert_eq!(second.acceptance.accepted_by, "operator:alice", "idempotent replay must not overwrite the original acceptance record");
        assert_eq!(state.acceptances.len(), 1, "idempotent re-accept must not create a duplicate acceptance record");
        assert!(second.currently_active, "replaying the MOST RECENTLY accepted proposal must report currently_active=true");
    }

    /// P1 fix (independent review v2): replaying an OLD, since-superseded
    /// proposal's acceptance must NEVER report a NEWER proposal's D-Map --
    /// each replay must be historically correct to the proposal actually
    /// being replayed.
    #[test]
    fn superseded_proposal_replay_returns_its_own_historical_materialization() {
        let mut state = DDeployStateV1::empty();

        // Accept p1 -> revision 1.
        let outcome_p1 = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(outcome_p1.materialized_dmap.revision, 1);

        // Accept p2 -> revision 2 (a placement change for the SAME application).
        let mut swapped = sample_placements();
        swapped[0].node_id = "cloud-01".to_string();
        let swapped_artifacts = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: swapped },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &swapped_artifacts,
        ).unwrap();
        let outcome_p2 = accept_proposal(&mut state, "p2", "operator:alice", None, &swapped_artifacts, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        assert_eq!(outcome_p2.materialized_dmap.revision, 2);

        // Replay p1 -- must return p1's OWN original materialization
        // (revision 1, digest matching a1's own recorded ref), reported as
        // NOT currently active, with ZERO further state mutation.
        let state_before_replay = state.clone();
        let replay_p1 = accept_proposal(&mut state, "p1", "operator:carol", None, &sample_artifacts(), "2026-01-01T03:00:00Z", "should-not-be-used".to_string()).unwrap();
        assert!(replay_p1.idempotent_replay);
        assert_eq!(replay_p1.acceptance.acceptance_id, "a1", "replaying p1 must return p1's OWN acceptance record, never p2's");
        assert_eq!(replay_p1.materialized_dmap.revision, 1, "replaying p1 must return p1's OWN revision (1), never p2's current revision (2)");
        assert_eq!(replay_p1.materialized_dmap.placements[0].node_id, "fog-01", "replaying p1 must return p1's OWN placement, never p2's");
        let expected_p1_digest = recompute_dmap_digest(&replay_p1.materialized_dmap).unwrap();
        assert_eq!(expected_p1_digest, replay_p1.acceptance.materialized_dmap_ref.digest);
        assert!(!replay_p1.currently_active, "p1 has been SUPERSEDED by p2 -- replaying it must report currently_active=false");
        assert_eq!(state, state_before_replay, "replaying a superseded proposal must not mutate state at all");

        // Replay p2 -- must return p2's own (current) materialization,
        // reported as currently active.
        let replay_p2 = accept_proposal(&mut state, "p2", "operator:carol", None, &swapped_artifacts, "2026-01-01T03:00:00Z", "should-not-be-used-either".to_string()).unwrap();
        assert!(replay_p2.idempotent_replay);
        assert_eq!(replay_p2.materialized_dmap.revision, 2);
        assert!(replay_p2.currently_active, "p2 is the CURRENTLY active proposal -- replaying it must report currently_active=true");
    }

    #[test]
    fn two_proposals_same_active_revision_first_succeeds_second_is_stale() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        // Two more proposals, BOTH created expecting revision 1 as active.
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p3".to_string(), "2026-01-01T01:00:01Z".to_string(), &sample_artifacts(),
        ).unwrap();

        let accept_p2 = accept_proposal(&mut state, "p2", "operator:alice", None, &sample_artifacts(), "2026-01-01T02:00:00Z", "a2".to_string());
        assert!(accept_p2.is_ok());
        assert_eq!(state.active_by_application.get(&application_key("proj-x", "app-x")).unwrap().active_dmap.revision, 2);

        let accept_p3 = accept_proposal(&mut state, "p3", "operator:alice", None, &sample_artifacts(), "2026-01-01T02:00:01Z", "a3".to_string());
        assert!(matches!(accept_p3, Err(AcceptError::Stale(_))));
        // p3 must NOT have moved the active revision to 3.
        assert_eq!(state.active_by_application.get(&application_key("proj-x", "app-x")).unwrap().active_dmap.revision, 2);
    }

    #[test]
    fn dmap_refs_and_digest_are_exact() {
        let mut state = DDeployStateV1::empty();
        let outcome = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let recomputed = recompute_dmap_digest(&outcome.materialized_dmap).unwrap();
        assert_eq!(recomputed, outcome.acceptance.materialized_dmap_ref.digest);
        assert_eq!(outcome.materialized_dmap.dmap_id, outcome.acceptance.materialized_dmap_ref.id);
        assert_eq!(outcome.materialized_dmap.revision, outcome.acceptance.materialized_dmap_ref.revision);
    }

    #[test]
    fn invalid_proposal_cannot_be_accepted() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut placements = sample_placements();
        placements[0].node_id = "does-not-exist".to_string();
        let mut state = DDeployStateV1::empty();
        // Creation itself must reject this (create_proposal validates too).
        let created = create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph, dcontinuum, placements },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        );
        assert!(created.is_err());
        assert!(state.proposals.is_empty(), "an invalid proposal must never be persisted");
    }

    #[test]
    fn tampered_proposal_snapshot_cannot_be_accepted() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        // Simulate tampering: mutate the persisted snapshot without updating the digest.
        state.proposals.get_mut("p1").unwrap().dgraph_snapshot.services[0].resource_requirement.cpu_cores = 999;
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::Invalid(_))), "a tampered snapshot must fail acceptance-time revalidation (dgraph_digest_mismatch): {result:?}");
    }

    // ---- proposal integrity (P0-3, independent review v2) ---------------
    // ---- acceptance must recompute and verify the ACTUAL persisted ------
    // ---- proposal's digest, not merely trust the recorded value ---------

    #[test]
    fn placement_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        // Attacker/bug scenario: mutate the persisted PROPOSAL's placements
        // (svc-a: fog-01 -> cloud-01) WITHOUT updating `proposal_digest`.
        state.proposals.get_mut("p1").unwrap().proposal.placements[0].node_id = "cloud-01".to_string();
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose persisted placements were changed without updating its recorded digest must be rejected as proposal_digest_mismatch, not silently accepted with the mutated placement: {result:?}");
        assert!(!state.has_active_authority("proj-x"));
    }

    #[test]
    fn source_ref_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.dgraph_ref.revision = 2;
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose dgraph_ref was changed without updating its recorded digest must be rejected as proposal_digest_mismatch: {result:?}");
    }

    #[test]
    fn dcontinuum_ref_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.dcontinuum_ref.digest = format!("sha256:{}", "0".repeat(64));
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose dcontinuum_ref was changed without updating its recorded digest must be rejected as proposal_digest_mismatch: {result:?}");
    }

    #[test]
    fn source_and_provenance_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.source = ProposalSource::Ai;
        state.proposals.get_mut("p1").unwrap().proposal.created_at_utc = "2026-06-01T00:00:00Z".to_string();
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose source/created_at_utc were changed without updating its recorded digest must be rejected as proposal_digest_mismatch: {result:?}");
    }

    #[test]
    fn expected_active_dmap_revision_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.expected_active_dmap_revision = Some(41);
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose expected_active_dmap_revision was changed without updating its recorded digest must be rejected as proposal_digest_mismatch (never merely evaluated as a CAS mismatch against the tampered value): {result:?}");
    }

    #[test]
    fn accepted_proposal_replay_also_verifies_digest_integrity_before_replaying() {
        // The digest-integrity check must run BEFORE the idempotent-replay
        // branch too -- an already-Accepted proposal record that was
        // somehow corrupted after acceptance must not be silently replayed.
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Accepted);

        // Corrupt the now-Accepted proposal's placements without updating its digest.
        state.proposals.get_mut("p1").unwrap().proposal.placements[0].node_id = "cloud-01".to_string();
        let replay = accept_proposal(&mut state, "p1", "operator:bob", None, &sample_artifacts(), "2026-01-01T02:00:00Z", "should-not-be-used".to_string());
        assert!(matches!(replay, Err(AcceptError::DigestMismatch(_))), "even an ALREADY-Accepted proposal must satisfy its own digest integrity before an idempotent replay is served: {replay:?}");
    }

    #[test]
    fn wrong_expected_digest_after_verification_is_still_rejected() {
        // `expected_proposal_digest` is compared against the VERIFIED
        // actual digest, not the (also verified, so identical in the
        // non-tampered case) recorded one -- this test just confirms the
        // ordinary "caller supplied the wrong digest" path still works
        // after the P0-3 restructuring.
        let mut state = DDeployStateV1::empty();
        let record = create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let wrong = format!("sha256:{}", if record.proposal_digest.ends_with('0') { "1".repeat(64) } else { "0".repeat(64) });
        let result = accept_proposal(&mut state, "p1", "operator:alice", Some(&wrong), &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))));
    }

    #[test]
    fn acceptance_time_eligibility_is_rechecked() {
        let mut state = DDeployStateV1::empty();
        let artifacts_at_creation = sample_artifacts();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &artifacts_at_creation,
        ).unwrap();
        // Artifact eligibility changes AFTER proposal creation, BEFORE acceptance.
        let artifacts_at_acceptance = vec![sample_artifact("svc-a", vec!["cloud-01"]) /* fog-01 no longer eligible */, sample_artifact("svc-b", vec!["cloud-01"])];
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &artifacts_at_acceptance, "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_NODE_NOT_ELIGIBLE)), "acceptance must re-check eligibility against CURRENT artifacts, not the creation-time snapshot: {result:?}");
        assert!(!state.has_active_authority("proj-x"));
    }

    #[test]
    fn accept_nonexistent_proposal_not_found() {
        let mut state = DDeployStateV1::empty();
        let result = accept_proposal(&mut state, "does-not-exist", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::NotFound(_))));
    }

    #[test]
    fn accept_with_wrong_expected_digest_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let result = accept_proposal(&mut state, "p1", "operator:alice", Some("sha256:wrong"), &sample_artifacts(), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))));
        assert!(!state.has_active_authority("proj-x"));
    }

    // ---- operational projection ---- //

    #[test]
    fn projection_places_service_only_on_its_target_node() {
        let mut state = DDeployStateV1::empty();
        let outcome = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let _ = outcome;
        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        let fog = projection.nodes.iter().find(|n| n.node_id == "fog-01").unwrap();
        assert!(fog.desired_services.iter().any(|s| s.service_id == "svc-a"));
        assert!(!fog.desired_services.iter().any(|s| s.service_id == "svc-b"));
        let cloud = projection.nodes.iter().find(|n| n.node_id == "cloud-01").unwrap();
        assert!(cloud.desired_services.iter().any(|s| s.service_id == "svc-b"));
    }

    #[test]
    fn moving_service_removes_it_from_old_node_and_adds_to_new_one() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        let mut moved = sample_placements();
        moved[0].node_id = "cloud-01".to_string(); // svc-a: fog-01 -> cloud-01
        let artifacts2 = vec![sample_artifact("svc-a", vec!["cloud-01", "fog-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: moved },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts2,
        ).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts2, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();

        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        let fog = projection.nodes.iter().find(|n| n.node_id == "fog-01");
        assert!(fog.is_none_or(|n| !n.desired_services.iter().any(|s| s.service_id == "svc-a")), "svc-a must no longer be desired on fog-01 after moving");
        let cloud = projection.nodes.iter().find(|n| n.node_id == "cloud-01").unwrap();
        assert_eq!(cloud.desired_services.iter().filter(|s| s.service_id == "svc-a").count(), 1, "svc-a must be desired exactly once on cloud-01, never dual-running");
    }

    #[test]
    fn nodes_and_services_are_sorted_deterministically() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        let node_ids: Vec<&str> = projection.nodes.iter().map(|n| n.node_id.as_str()).collect();
        let mut sorted = node_ids.clone();
        sorted.sort_unstable();
        assert_eq!(node_ids, sorted);
    }

    #[test]
    fn desired_state_is_running() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        for node in &projection.nodes {
            for service in &node.desired_services {
                assert_eq!(service.desired_state, "running");
            }
        }
    }

    #[test]
    fn operational_runtime_comes_from_service_artifact_not_canonical_dgraph() {
        let mut artifacts = sample_artifacts();
        artifacts[0].runtime.kind = "native_process".to_string();
        artifacts[0].runtime.container = None;
        artifacts[0].runtime.native_process = Some(crate::core::service_artifact_registry::NativeProcessDeploymentSpecV1 {
            process_name: "svc-a".to_string(), entrypoint: "/bin/svc-a".to_string(), args: vec![], working_directory: None,
        });
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &artifacts, "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        let fog = projection.nodes.iter().find(|n| n.node_id == "fog-01").unwrap();
        let svc_a = fog.desired_services.iter().find(|s| s.service_id == "svc-a").unwrap();
        assert_eq!(svc_a.runtime, "native_process", "runtime must come from the ServiceArtifact binding, not the canonical D-Graph (which has no runtime-kind field at all)");
    }

    #[test]
    fn canonical_dgraph_remains_free_of_runtime_kind() {
        let dgraph = sample_dgraph("app-x", 1);
        let value = serde_json::to_value(&dgraph).unwrap();
        let text = value.to_string();
        assert!(!text.contains("runtime_kind"));
        assert!(!text.contains("\"container\""));
        assert!(!text.contains("native_process"));
    }

    /// Test-only helper mirroring `active_operational_projection`, but
    /// operating on an in-memory `DDeployStateV1` directly (the real
    /// function reads from the SQLite-backed control-state domain, exercised
    /// separately by the restart/persistence tests below).
    fn active_operational_projection_from_state(state: &DDeployStateV1, project_id: &str) -> Option<OperationalDGraph> {
        if !state.has_active_authority(project_id) {
            return None;
        }
        let revision = *state.project_operational_revision.get(project_id).unwrap_or(&0);
        union_project_operational_graph(project_id, &state.active_by_application, revision).ok()
    }

    // ---- target_nodes authority transition ---- //

    #[test]
    fn mutating_target_nodes_after_acceptance_does_not_move_active_dmap() {
        let mut state = DDeployStateV1::empty();
        let outcome = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let before = outcome.materialized_dmap.clone();

        // Simulate an artifact.target_nodes edit AWAY from the active node --
        // this must have NO effect on the already-accepted D-Map.
        let mutated_artifacts = vec![sample_artifact("svc-a", vec!["cloud-01"] /* fog-01 removed */), sample_artifact("svc-b", vec!["cloud-01"])];
        let _ = mutated_artifacts; // artifact mutation alone touches nothing in `state`

        let active = state.active_for("proj-x", "app-x").unwrap();
        assert_eq!(active.active_dmap, before, "accepted D-Map must be completely unaffected by a later target_nodes edit");
    }

    #[test]
    fn a_new_proposal_targeting_a_now_ineligible_node_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        // fog-01 is no longer eligible for svc-a.
        let mutated_artifacts = vec![sample_artifact("svc-a", vec!["cloud-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        let result = create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() /* still targets fog-01 */ },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &mutated_artifacts,
        );
        assert!(result.is_err_and(|findings| findings.iter().any(|f| f.code == F_NODE_NOT_ELIGIBLE)));
    }

    #[test]
    fn stage111_never_writes_target_nodes_equal_to_accepted_node() {
        // Structural proof: ServiceArtifact is never even a parameter type
        // any mutating function in this module accepts by `&mut` -- only
        // `&[ServiceArtifact]` (shared reference) appears anywhere in this
        // module's public signatures, so this module cannot write to
        // `control_plane.target_nodes` at all.
        let mut state = DDeployStateV1::empty();
        let artifacts = sample_artifacts();
        let before = artifacts.clone();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &artifacts, "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        assert_eq!(artifacts, before, "acceptance must never mutate the artifacts it was given");
    }

    // ---- multi-application ---- //

    #[test]
    fn two_applications_both_active_in_one_project() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-a", "p1", vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }, CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "cloud-01".to_string() }], &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        let dgraph_b = sample_dgraph("app-b", 1);
        let mut dgraph_b = dgraph_b;
        dgraph_b.services[0].service_id = "svc-c".to_string();
        dgraph_b.services[1].service_id = "svc-d".to_string();
        let artifacts_b = vec![sample_artifact("svc-c", vec!["fog-01"]), sample_artifact("svc-d", vec!["cloud-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: dgraph_b, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "fog-01".to_string() }, CanonicalDServPlacementV1 { service_id: "svc-d".to_string(), node_id: "cloud-01".to_string() }] },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_b,
        ).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts_b, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();

        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        let all_services: Vec<&str> = projection.nodes.iter().flat_map(|n| n.desired_services.iter().map(|s| s.service_id.as_str())).collect();
        assert!(all_services.contains(&"svc-a"));
        assert!(all_services.contains(&"svc-c"), "both applications must be present in the project union");
    }

    #[test]
    fn accepting_new_revision_of_a_does_not_remove_b() {
        let mut state = DDeployStateV1::empty();
        let mut dgraph_a = sample_dgraph("app-a", 1);
        dgraph_a.services.truncate(1);
        dgraph_a.services[0].service_id = "svc-a".to_string();
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-a", dgraph_a, "p1", vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }], &[sample_artifact("svc-a", vec!["fog-01", "cloud-01"])], "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "svc-c".to_string();
        let artifacts_b = vec![sample_artifact("svc-c", vec!["cloud-01"])];
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: dgraph_b, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "cloud-01".to_string() }] }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_b).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts_b, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();

        // Re-accept a new revision of app-a.
        let artifacts_a = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"])];
        let mut dgraph_a2 = sample_dgraph("app-a", 1);
        dgraph_a2.services.truncate(1);
        dgraph_a2.services[0].service_id = "svc-a".to_string();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: dgraph_a2, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "cloud-01".to_string() }] }, "p3".to_string(), "2026-01-01T02:00:00Z".to_string(), &artifacts_a).unwrap();
        accept_proposal(&mut state, "p3", "operator:alice", None, &artifacts_a, "2026-01-01T02:00:00Z", "a3".to_string()).unwrap();

        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        let all_services: Vec<&str> = projection.nodes.iter().flat_map(|n| n.desired_services.iter().map(|s| s.service_id.as_str())).collect();
        assert!(all_services.contains(&"svc-c"), "app-b must remain active after app-a is re-accepted");
        assert!(all_services.contains(&"svc-a"));
    }

    #[test]
    fn same_service_id_across_two_applications_is_rejected() {
        let mut state = DDeployStateV1::empty();
        let mut dgraph_a = sample_dgraph("app-a", 1);
        dgraph_a.services.truncate(1);
        dgraph_a.services[0].service_id = "shared-svc".to_string();
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-a", dgraph_a, "p1", vec![CanonicalDServPlacementV1 { service_id: "shared-svc".to_string(), node_id: "fog-01".to_string() }], &[sample_artifact("shared-svc", vec!["fog-01", "cloud-01"])], "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "shared-svc".to_string();
        let artifacts_b = vec![sample_artifact("shared-svc", vec!["cloud-01"])];
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: dgraph_b, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "shared-svc".to_string(), node_id: "cloud-01".to_string() }] }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_b).unwrap();
        let result = accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts_b, "2026-01-01T01:00:00Z", "a2".to_string());
        assert!(matches!(result, Err(AcceptError::Collision(ref f)) if f.iter().any(|finding| finding.code == F_OPERATIONAL_SERVICE_ID_COLLISION)));
        // app-a must remain untouched.
        assert!(state.active_for("proj-x", "app-b").is_none());
        assert!(state.active_for("proj-x", "app-a").is_some());
    }

    // ---- atomicity (in-memory boundary) ---- //

    #[test]
    fn failed_validation_leaves_state_unchanged() {
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let before = state.clone();

        // Proposal whose target-node eligibility fails at acceptance time.
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &sample_artifacts()).unwrap();
        let bad_artifacts = vec![sample_artifact("svc-a", vec![]), sample_artifact("svc-b", vec!["cloud-01"])];
        let result = accept_proposal(&mut state, "p2", "operator:alice", None, &bad_artifacts, "2026-01-01T01:00:00Z", "a2".to_string());
        assert!(result.is_err());

        assert_eq!(state.active_by_application, before.active_by_application);
        assert_eq!(state.acceptances.len(), before.acceptances.len());
        assert_eq!(state.project_operational_revision, before.project_operational_revision);
    }

    #[test]
    fn successful_acceptance_activates_proposal_dmap_and_projection_together() {
        let mut state = DDeployStateV1::empty();
        let outcome = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        assert!(state.proposals.get("p1").unwrap().state == ProposalLifecycleState::Accepted);
        assert!(state.acceptances.contains_key(&outcome.acceptance.acceptance_id));
        assert!(state.active_for("proj-x", "app-x").is_some());
        assert!(active_operational_projection_from_state(&state, "proj-x").is_some());
    }

    // ---- host-mutation source guard ---- //

    #[test]
    fn source_contains_no_host_mutation_calls() {
        // Scan only the NON-TEST portion of this file -- the test module
        // itself necessarily quotes these forbidden strings (as literals in
        // this very assertion list), so including it would make this test
        // detect itself. Split line-by-line (`str::lines`, which handles
        // both `\n` and `\r\n` uniformly) rather than on a raw `"\n...\n"`
        // substring -- a raw substring match is silently defeated if this
        // file's on-disk line endings are ever CRLF (this repository's git
        // config normalizes several files to CRLF on Windows checkouts),
        // which would make `production_source` equal the WHOLE file and
        // cause this test to detect its own forbidden-string literals.
        let source = include_str!("ddeploy.rs");
        let production_source: String = source
            .lines()
            .take_while(|line| line.trim() != "#[cfg(test)]")
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            production_source.len() < source.len(),
            "the '#[cfg(test)] mod tests' boundary was not found -- this test would otherwise scan its own forbidden-string literals"
        );
        for forbidden in [
            "std::process::Command",
            "Command::new",
            "docker",
            "native_process_runtime::",
            "dispatch::execute",
            "reconcile_service",
            "OperationalReconciliationAuthorization {",
        ] {
            assert!(!production_source.contains(forbidden), "ddeploy.rs (excluding its own test module) must never reference '{forbidden}' -- acceptance is control-state mutation only");
        }
    }

    // ---- persistence / restart ---- //

    fn temp_db_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-ddeploy-test-{tag}-{}.db", uuid::Uuid::new_v4()))
    }

    fn cleanup_db(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn state_survives_restart() {
        let path = temp_db_path("restart");

        let mut state = load_state(&path).unwrap();
        assert_eq!(state, DDeployStateV1::empty(), "no previous state -> bootstrap empty v1 state");

        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let outcome = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        persist_state(&path, &state).unwrap();

        // Drop everything and reload as if the process had restarted.
        drop(state);
        let reloaded = load_state(&path).unwrap();

        assert_eq!(reloaded.proposals.get("p1").unwrap().state, ProposalLifecycleState::Accepted, "pending->accepted proposal must survive restart");
        assert_eq!(reloaded.acceptances.get(&outcome.acceptance.acceptance_id).unwrap(), &outcome.acceptance, "accepted acceptance record must survive restart");
        let active = reloaded.active_for("proj-x", "app-x").unwrap();
        assert_eq!(active.active_dmap, outcome.materialized_dmap, "active D-Map must survive restart");
        assert_eq!(active.dmap_digest, outcome.acceptance.materialized_dmap_ref.digest, "D-Map digest must survive restart");
        let projection = active_operational_projection_from_state(&reloaded, "proj-x").unwrap();
        assert!(!projection.nodes.is_empty(), "operational projection must survive restart");
        assert_eq!(
            *reloaded.project_operational_revision.get("proj-x").unwrap(),
            outcome.operational_revision,
            "expected active revision must survive restart"
        );

        // Idempotent accept must remain idempotent after reload.
        let mut reloaded_mut = reloaded;
        let replay = accept_proposal(&mut reloaded_mut, "p1", "operator:bob", None, &sample_artifacts(), "2026-01-01T05:00:00Z", "should-not-be-used".to_string()).unwrap();
        assert!(replay.idempotent_replay);
        assert_eq!(replay.materialized_dmap.revision, outcome.materialized_dmap.revision);
        assert!(replay.currently_active, "p1 is still the only/most-recently-accepted proposal after restart");

        cleanup_db(&path);
    }

    #[test]
    fn no_previous_state_bootstraps_empty_v1_state() {
        let path = temp_db_path("bootstrap-empty");
        let state = load_state(&path).unwrap();
        assert_eq!(state.state_version, DDEPLOY_STATE_SCHEMA_VERSION);
        assert!(state.proposals.is_empty());
        assert!(state.active_by_application.is_empty());
        cleanup_db(&path);
    }

    #[test]
    fn unknown_future_state_version_fails_closed() {
        let path = temp_db_path("unknown-version");
        let mut future_state = DDeployStateV1::empty();
        future_state.state_version = "99.0.0".to_string();
        persist_state(&path, &future_state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "an unrecognised future state_version must fail closed, never be silently reinterpreted");
        cleanup_db(&path);
    }

    // ---- persisted-state integrity (independent review v2, bounded ------
    // ---- persisted-state integrity audit) --------------------------------

    #[test]
    fn corrupted_proposal_digest_fails_closed_on_reload() {
        let path = temp_db_path("corrupt-proposal-digest");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        // Corrupt the persisted record directly (bypassing every in-process
        // invariant, simulating a hand-edited or externally corrupted blob).
        state.proposals.get_mut("p1").unwrap().proposal.placements[0].node_id = "cloud-01".to_string();
        persist_state(&path, &state).unwrap();

        let result = load_state(&path);
        assert!(result.is_err(), "a persisted proposal whose recorded digest no longer matches its own content must fail closed on reload, never silently load as valid state");
        cleanup_db(&path);
    }

    #[test]
    fn corrupted_active_dmap_digest_fails_closed_on_reload() {
        let path = temp_db_path("corrupt-active-digest");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        // Corrupt the persisted ACTIVE D-Map's placement without updating
        // its recorded dmap_digest -- this is exactly the shape of
        // corruption that would otherwise let a stale/tampered placement
        // silently become "the current active D-Map" after restart.
        let key = application_key("proj-x", "app-x");
        state.active_by_application.get_mut(&key).unwrap().active_dmap.placements[0].node_id = "cloud-01".to_string();
        persist_state(&path, &state).unwrap();

        let result = load_state(&path);
        assert!(result.is_err(), "an active D-Map whose recorded digest no longer matches its own content must fail closed on reload, never silently become placement authority");
        cleanup_db(&path);
    }

    #[test]
    fn dangling_accepted_acceptance_id_fails_closed_on_reload() {
        let path = temp_db_path("dangling-acceptance-id");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        // Corrupt: the acceptance record itself is deleted, but the
        // proposal still claims to be Accepted and still points at it.
        state.acceptances.remove("a1");
        persist_state(&path, &state).unwrap();

        let result = load_state(&path);
        assert!(result.is_err(), "a proposal marked Accepted whose acceptance record is missing must fail closed on reload");
        cleanup_db(&path);
    }

    #[test]
    fn mismatched_active_by_application_key_fails_closed_on_reload() {
        let path = temp_db_path("mismatched-active-key");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        // Corrupt: move the entry to a key that does not correspond to its
        // own content's (project_id, application_id).
        let real_key = application_key("proj-x", "app-x");
        let entry = state.active_by_application.remove(&real_key).unwrap();
        state.active_by_application.insert("some-other-key".to_string(), entry);
        persist_state(&path, &state).unwrap();

        let result = load_state(&path);
        assert!(result.is_err(), "an active_by_application entry stored under a key that does not match its own content must fail closed on reload");
        cleanup_db(&path);
    }

    #[test]
    fn self_consistent_state_loads_cleanly() {
        // Sanity/negative-of-negative: a normal, uncorrupted lifecycle must
        // NOT be rejected by the integrity audit above.
        let path = temp_db_path("self-consistent");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_ok(), "an uncorrupted, self-consistent state must load cleanly");
        cleanup_db(&path);
    }

    // ---- independent review v3: single persisted placement authority ---- //

    #[test]
    fn active_application_deployment_carries_no_persisted_node_placement_outside_active_dmap() {
        // Structural proof: `ActiveApplicationDeployment` has exactly one
        // field that can name a node (`active_dmap`, itself a
        // `CanonicalDMapV1` whose `placements` are the sole node-bearing
        // field); `operational_services` is `BTreeMap<String,
        // OperationalDesiredService>` keyed by service_id, and
        // `OperationalDesiredService` itself has fields `service_id`,
        // `runtime`, `desired_state` -- no `node_id`/`target_node`/
        // `target_nodes` field exists anywhere on it. This is enforced by
        // the type system (the struct literally has no such field to
        // populate), not merely by convention -- if a future edit added
        // one, every construction site in this file would need to supply
        // it, at which point the struct definition itself is the review
        // surface.
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let active = state.active_for("proj-x", "app-x").unwrap();
        // operational_services carries ONLY service semantics, keyed by service_id.
        for (service_id, service) in &active.operational_services {
            assert_eq!(service_id, &service.service_id);
        }
        // The only node identities anywhere in this struct come from active_dmap.placements.
        let node_ids_from_dmap: Vec<&str> = active.active_dmap.placements.iter().map(|p| p.node_id.as_str()).collect();
        assert!(!node_ids_from_dmap.is_empty());
    }

    #[test]
    fn moving_a_service_via_new_dmap_is_the_only_way_node_placement_changes() {
        // Functional companion to the structural proof above: accept svc-a
        // on fog-01, then accept a NEW proposal moving it to cloud-01 --
        // the projection must move BECAUSE active_dmap.placements changed,
        // never because operational_services was independently edited (it
        // cannot be, since accept_proposal always derives it fresh from the
        // SAME dmap.placements it stores).
        let mut state = DDeployStateV1::empty();
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services.truncate(1);
        dgraph.services[0].service_id = "svc-a".to_string();
        let placements_v1 = vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }];
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-x", dgraph.clone(), "p1", placements_v1, &artifacts, "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let projection_v1 = active_operational_projection_from_state(&state, "proj-x").unwrap();
        assert!(projection_v1.nodes.iter().any(|n| n.node_id == "fog-01" && n.desired_services.iter().any(|s| s.service_id == "svc-a")));

        let placements_v2 = vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "cloud-01".to_string() }];
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph, dcontinuum: sample_dcontinuum("proj-x", 1), placements: placements_v2 }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        let projection_v2 = active_operational_projection_from_state(&state, "proj-x").unwrap();
        assert!(!projection_v2.nodes.iter().any(|n| n.node_id == "fog-01" && n.desired_services.iter().any(|s| s.service_id == "svc-a")), "svc-a must no longer be on fog-01");
        assert!(projection_v2.nodes.iter().any(|n| n.node_id == "cloud-01" && n.desired_services.iter().any(|s| s.service_id == "svc-a")), "svc-a must now be on cloud-01, via the NEW D-Map's placements");
    }

    // ---- independent review v3: acceptance validates the derived --------
    // ---- operational projection before commit ----------------------------

    #[test]
    fn empty_application_in_an_otherwise_empty_project_is_rejected_as_operational_projection_invalid() {
        // Legal at the Stage 110 canonical level (a D-Graph with zero
        // services, ADR-0017 permits this), but not operationally
        // activatable through the v0.1 Stage111 bridge WHEN it is the
        // project's only application: the derived OperationalDGraph would
        // have zero nodes at a non-bootstrap revision, which the existing
        // `validate_operational_dgraph` contract rejects. This must be
        // caught BEFORE any state mutation.
        //
        // Independent review v4 documentation-precision correction: THIS
        // rejection is a property of the PROJECT-WIDE UNION having zero
        // nodes, not of "empty applications" in isolation -- acceptance
        // validates `union_project_operational_graph`'s OUTPUT, which
        // merges every active application in the project. An empty
        // application accepted into a project that ALREADY has another
        // active application supplying nodes produces a valid (non-empty)
        // union and is accepted -- see
        // `empty_application_with_a_sibling_application_supplying_nodes_is_accepted`
        // immediately below, which is the necessary companion proof this
        // ADR/audit language was previously missing.
        let mut state = DDeployStateV1::empty();
        let mut empty_dgraph = sample_dgraph("app-empty", 1);
        empty_dgraph.services.clear();
        empty_dgraph.calls.clear();
        let result = create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-empty".to_string(), application_id: "app-empty".to_string(), source: ProposalSource::Operator, dgraph: empty_dgraph, dcontinuum: sample_dcontinuum("proj-empty", 1), placements: vec![] },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &[],
        );
        // create_proposal itself runs validate_full (schema/identity/
        // cardinality -- a zero-service/zero-placement D-Map is
        // structurally valid there, per Stage 110's own rules), so it must
        // succeed at CREATION time; the operational-projection rejection
        // is specifically an ACCEPTANCE-time bridge concern.
        let record = result.expect("a structurally valid zero-service proposal must still be creatable");
        let outcome = accept_proposal(&mut state, &record.proposal.proposal_id, "operator:alice", None, &[], "2026-01-01T00:01:00Z", "a1".to_string());
        assert!(
            matches!(outcome, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_OPERATIONAL_PROJECTION_INVALID)),
            "an empty application must be rejected at acceptance time with operational_projection_invalid, never silently activated: {outcome:?}"
        );
        assert!(!state.has_active_authority("proj-empty"), "a rejected acceptance must leave no active authority behind");
        assert_eq!(state.proposals.get(&record.proposal.proposal_id).unwrap().state, ProposalLifecycleState::Pending, "the proposal must remain Pending, never silently marked Accepted");
    }

    #[test]
    fn empty_application_with_a_sibling_application_supplying_nodes_is_accepted() {
        // Independent review v4, documentation-precision correction:
        // proves the union-level (not application-level) nature of the
        // operational-projection-validity check. app-a is accepted first,
        // supplying real nodes to the project; a SECOND, zero-service
        // application (app-empty) is then accepted into the SAME project.
        // The project-wide union still has nodes (from app-a), so the
        // union validates and the empty application's acceptance SUCCEEDS
        // -- it must never be rejected merely for being empty in
        // isolation.
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-a", "p-a", sample_placements(), &sample_artifacts(), "operator:alice", "a-a", "2026-01-01T00:00:00Z").unwrap();

        let mut empty_dgraph = sample_dgraph("app-empty", 1);
        empty_dgraph.services.clear();
        empty_dgraph.calls.clear();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-empty".to_string(), source: ProposalSource::Operator, dgraph: empty_dgraph, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![] },
            "p-empty".to_string(), "2026-01-01T01:00:00Z".to_string(), &[],
        ).expect("a structurally valid zero-service proposal must still be creatable");
        let outcome = accept_proposal(&mut state, "p-empty", "operator:bob", None, &[], "2026-01-01T01:00:00Z", "a-empty".to_string());
        assert!(outcome.is_ok(), "an empty application must be ACCEPTED when a sibling application in the same project already supplies nodes to the union: {outcome:?}");
        assert!(state.has_active_authority("proj-x"));
        let projection = active_operational_projection_from_state(&state, "proj-x").unwrap();
        assert!(!projection.nodes.is_empty(), "the project union must still have app-a's nodes");
    }

    // ---- independent review v3: relational persisted-state integrity ---- //

    #[test]
    fn self_consistent_tamper_is_still_detected() {
        // PASS U: prove the integrity layer is RELATIONAL, not merely
        // "every object hashes itself." Accept p1 (svc-a -> fog-01), then
        // tamper `active_dmap` to a DIFFERENT (but internally valid)
        // placement AND recompute `dmap_digest` to match the tampered
        // value -- both objects are now individually self-consistent, but
        // this tampered D-Map was never actually materialized by ANY
        // accepted proposal.
        //
        // Independent review v4 correction: this test is caught by
        // category C's acceptance-cross-reference check (`active_dmap`'s
        // own id/revision/digest must match its acceptance's
        // `materialized_dmap_ref` -- here, `active.dmap_digest` was
        // recomputed to match the TAMPERED placement, but
        // `acceptance.materialized_dmap_ref.digest` still records the
        // ORIGINAL, untampered digest, so they now disagree), verified
        // below by asserting on the actual error text -- NOT by the
        // separate historical-structural-equality reconstruction check
        // this test's v3 description incorrectly credited. That second
        // check is logically implied by (a) category B's own reconstruction-
        // vs-`materialized_dmap_ref.digest` check and (b) this same
        // acceptance-cross-reference check, given SHA-256 collision
        // resistance -- it is kept as defense-in-depth against a future
        // change that weakens one of those without weakening the other,
        // but no currently-constructible test reaches it as the FIRST
        // failure without an actual hash collision. See
        // `self-consistent-tamper-audit.txt` for the full trace.
        let path = temp_db_path("self-consistent-tamper");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();

        let key = application_key("proj-x", "app-x");
        let active = state.active_by_application.get_mut(&key).unwrap();
        active.active_dmap.placements[0].node_id = "cloud-01".to_string();
        // Recompute the digest so the tampered D-Map is, BY ITSELF, fully
        // self-consistent (this is the crux of the test: a same-record-only
        // digest check would NOT catch this).
        active.dmap_digest = recompute_dmap_digest(&active.active_dmap).unwrap();
        // operational_services still matches the (unchanged) service-ID set,
        // so category D alone would not catch this either.

        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a self-consistent but unauthorized active_dmap (never actually materialized by its referenced accepted proposal) must still fail closed on reload");
        assert!(
            error.contains("does not match its own acceptance") && error.contains("materialized_dmap_ref"),
            "expected the acceptance-cross-reference check to fire (active_dmap vs materialized_dmap_ref), got a DIFFERENT error instead -- this would mean the test is no longer verifying the intended check: {error}"
        );
        cleanup_db(&path);
    }

    #[test]
    fn proposal_map_key_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("proposal-key-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        let record = state.proposals.remove("p1").unwrap();
        state.proposals.insert("p1-renamed".to_string(), record);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a proposals map key that disagrees with its own record's proposal_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn proposal_dgraph_ref_mismatch_fails_closed_on_reload() {
        // Independent review v4 correction (R3): this negative case, as
        // originally written, mutated `proposal.dgraph_ref` WITHOUT
        // recomputing `proposal_digest` -- meaning it actually exercised
        // the EARLIER proposal-digest guard, never the LATER dgraph_ref-
        // vs-dgraph_snapshot cross-check this test's name claims. Fixed to
        // ALSO recompute `proposal_digest` to match the tampered proposal
        // (explicitly documented, per the review's suggestion), so the
        // digest guard passes and the intended ref-vs-snapshot check is
        // what actually fires -- verified via the asserted error text, not
        // merely `is_err()`.
        let path = temp_db_path("proposal-dgraph-ref-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        let record = state.proposals.get_mut("p1").unwrap();
        record.proposal.dgraph_ref.revision = 42;
        record.proposal_digest = compute_proposal_digest(&record.proposal).unwrap();
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a proposal.dgraph_ref that disagrees with its own persisted dgraph_snapshot must fail closed, even when the proposal_digest guard is satisfied");
        assert!(error.contains("dgraph_ref") && error.contains("dgraph_snapshot"), "expected the dgraph_ref-vs-dgraph_snapshot check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn proposal_dgraph_snapshot_mismatch_fails_closed_on_reload() {
        // A SECOND, independent way to reach the SAME dgraph_ref-vs-
        // dgraph_snapshot check (R3): mutate the persisted SNAPSHOT itself
        // (structurally valid on its own -- `CanonicalDGraphV1::validate()`
        // still passes) rather than the reference. `proposal_digest` is
        // computed only over `proposal` (never `dgraph_snapshot`), so it
        // remains valid untouched -- proving this check is reachable via a
        // completely different corruption vector than the ref-mutation
        // test above.
        let path = temp_db_path("proposal-dgraph-snapshot-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        state.proposals.get_mut("p1").unwrap().dgraph_snapshot.revision = 7;
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a persisted dgraph_snapshot that disagrees with the proposal's own dgraph_ref must fail closed");
        assert!(error.contains("dgraph_ref") && error.contains("dgraph_snapshot"), "expected the dgraph_ref-vs-dgraph_snapshot check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn proposal_dcontinuum_ref_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("proposal-dcontinuum-ref-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        let record = state.proposals.get_mut("p1").unwrap();
        record.proposal.dcontinuum_ref.revision = 42;
        record.proposal_digest = compute_proposal_digest(&record.proposal).unwrap();
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a proposal.dcontinuum_ref that disagrees with its own persisted dcontinuum_snapshot must fail closed, even when the proposal_digest guard is satisfied");
        assert!(error.contains("dcontinuum_ref") && error.contains("dcontinuum_snapshot"), "expected the dcontinuum_ref-vs-dcontinuum_snapshot check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn proposal_dcontinuum_snapshot_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("proposal-dcontinuum-snapshot-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        state.proposals.get_mut("p1").unwrap().dcontinuum_snapshot.revision = 7;
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a persisted dcontinuum_snapshot that disagrees with the proposal's own dcontinuum_ref must fail closed");
        assert!(error.contains("dcontinuum_ref") && error.contains("dcontinuum_snapshot"), "expected the dcontinuum_ref-vs-dcontinuum_snapshot check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_proposal_digest_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-proposal-digest-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.acceptances.get_mut("a1").unwrap().proposal_digest = format!("sha256:{}", "0".repeat(64));
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("an acceptance whose proposal_digest disagrees with its referenced proposal's recorded proposal_digest must fail closed");
        assert!(error.contains("proposal_digest") && error.contains("referenced proposal"), "expected the acceptance-proposal_digest cross-check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_application_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-application-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.acceptances.get_mut("a1").unwrap().application_id = "app-y".to_string();
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("an acceptance whose application_id disagrees with its referenced proposal's application_id must fail closed");
        assert!(error.contains("project/application"), "expected the acceptance project/application cross-check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_materialized_dmap_ref_corruption_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-materialized-ref-corruption");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        // Corrupt the acceptance's OWN recorded materialized_dmap_ref
        // revision, WITHOUT touching active_by_application at all -- this
        // must be caught by category B's historical-reconstruction check
        // against the (untouched) proposal, independent of category C.
        state.acceptances.get_mut("a1").unwrap().materialized_dmap_ref.revision = 99;
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("an acceptance whose materialized_dmap_ref does not reproduce from its own referenced proposal must fail closed");
        assert!(error.contains("historical"), "expected the acceptance historical-reconstruction check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn active_referencing_missing_acceptance_fails_closed_on_reload() {
        let path = temp_db_path("active-missing-acceptance");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        state.active_by_application.get_mut(&key).unwrap().acceptance_id = "phantom-acceptance".to_string();
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("an active_by_application entry referencing a missing acceptance must fail closed");
        assert!(error.contains("acceptance") && error.contains("does not exist"), "expected the active-references-missing-acceptance check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn project_operational_revision_zero_fails_closed_on_reload() {
        let path = temp_db_path("project-revision-zero");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.project_operational_revision.insert("proj-x".to_string(), 0);
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a project_operational_revision explicitly set to exactly 0 for a project with accepted history must fail closed");
        assert!(error.contains("project_operational_revision is 0"), "expected the explicit zero-revision check to fire, got a different error: {error}");
        cleanup_db(&path);
    }

    #[test]
    fn pending_proposal_with_accepted_acceptance_id_fails_closed_on_reload() {
        let path = temp_db_path("pending-with-acceptance-id");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        state.proposals.get_mut("p1").unwrap().accepted_acceptance_id = Some("phantom-acceptance".to_string());
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a Pending proposal carrying a non-None accepted_acceptance_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_map_key_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-key-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let acceptance = state.acceptances.remove("a1").unwrap();
        state.acceptances.insert("a1-renamed".to_string(), acceptance);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptances map key that disagrees with its own record's acceptance_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_referencing_missing_proposal_fails_closed_on_reload() {
        // Independent review v3 correction: v2 tolerated this as a
        // "dangling reference, not fatal" -- v3 treats it as corruption,
        // since proposals are never deleted by this module.
        let path = temp_db_path("acceptance-missing-proposal");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.proposals.remove("p1");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance referencing a missing proposal must now fail closed (v3), never be tolerated as a dangling reference (v2's behavior)");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_project_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-project-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.acceptances.get_mut("a1").unwrap().project_id = "some-other-project".to_string();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance whose project_id disagrees with its referenced proposal's project_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn active_application_id_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("active-application-id-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        state.active_by_application.get_mut(&key).unwrap().application_id = "app-y".to_string();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an active_by_application entry whose own application_id disagrees with its active_dmap.application_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn active_referencing_missing_proposal_fails_closed_on_reload() {
        let path = temp_db_path("active-missing-proposal");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        state.active_by_application.get_mut(&key).unwrap().proposal_id = "phantom-proposal".to_string();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an active_by_application entry referencing a missing proposal must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn operational_service_snapshot_missing_a_dmap_service_fails_closed_on_reload() {
        let path = temp_db_path("snapshot-missing-service");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        state.active_by_application.get_mut(&key).unwrap().operational_services.remove("svc-a");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an operational_services snapshot missing a service the active_dmap places must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn operational_service_snapshot_extra_service_fails_closed_on_reload() {
        let path = temp_db_path("snapshot-extra-service");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        let extra = OperationalDesiredService { service_id: "svc-ghost".to_string(), runtime: "container".to_string(), desired_state: "running".to_string() };
        state.active_by_application.get_mut(&key).unwrap().operational_services.insert("svc-ghost".to_string(), extra);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an operational_services snapshot with an EXTRA service the active_dmap does not place must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn operational_service_snapshot_key_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("snapshot-key-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        let active = state.active_by_application.get_mut(&key).unwrap();
        let mut service = active.operational_services.get("svc-a").unwrap().clone();
        service.service_id = "svc-b".to_string(); // the VALUE's service_id no longer matches the map KEY "svc-a"
        active.operational_services.insert("svc-a".to_string(), service);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an operational_services entry whose map key disagrees with its own value's service_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn operational_service_invalid_runtime_fails_closed_on_reload() {
        let path = temp_db_path("snapshot-invalid-runtime");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        state.active_by_application.get_mut(&key).unwrap().operational_services.get_mut("svc-a").unwrap().runtime = "not-a-real-runtime".to_string();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an operational service with an unparseable runtime must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn project_operational_revision_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("project-revision-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.project_operational_revision.insert("proj-x".to_string(), 99);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a project_operational_revision that disagrees with the maximum recorded acceptance projection revision must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn project_operational_revision_missing_fails_closed_on_reload() {
        let path = temp_db_path("project-revision-missing");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.project_operational_revision.remove("proj-x");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an active project with no project_operational_revision entry at all must fail closed");
        cleanup_db(&path);
    }

    // ---- independent review v4: R1 -- reload reconstructs and validates -
    // ---- the project operational union, not merely per-record checks ---

    #[test]
    fn cross_application_collision_undetectable_per_record_is_caught_by_reload_union_reconstruction() {
        // Constructs TWO applications that are each, INDIVIDUALLY,
        // perfectly self-consistent under every category-A-F check (their
        // own proposal/acceptance/active triple cross-references correctly
        // and every digest recomputes correctly) -- but which jointly claim
        // the SAME canonical service_id. `accept_proposal`'s own pre-commit
        // collision check would never allow this to arise through the real
        // API, so app-b's proposal/acceptance/active triple is constructed
        // BY HAND here (never through `accept_proposal`) specifically to
        // prove that per-record relational checks alone (R1's gap) cannot
        // catch this -- only reconstructing the actual project union can.
        let path = temp_db_path("reload-union-collision");
        let mut state = DDeployStateV1::empty();

        let mut dgraph_a = sample_dgraph("app-a", 1);
        dgraph_a.services.truncate(1);
        dgraph_a.services[0].service_id = "svc-shared".to_string();
        let placements_a = vec![CanonicalDServPlacementV1 { service_id: "svc-shared".to_string(), node_id: "fog-01".to_string() }];
        let artifacts_a = vec![sample_artifact("svc-shared", vec!["fog-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-a", dgraph_a, "p-a", placements_a, &artifacts_a, "operator:alice", "a-a", "2026-01-01T00:00:00Z").unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "svc-shared".to_string();
        let dcontinuum_b = sample_dcontinuum("proj-x", 1);
        let dgraph_digest_b = recompute_dgraph_digest(&dgraph_b).unwrap();
        let dcontinuum_digest_b = recompute_dcontinuum_digest(&dcontinuum_b).unwrap();
        let placements_b = vec![CanonicalDServPlacementV1 { service_id: "svc-shared".to_string(), node_id: "cloud-01".to_string() }];
        let proposal_b = DDeployProposalV1 {
            schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
            proposal_id: "p-b".to_string(),
            project_id: "proj-x".to_string(),
            application_id: "app-b".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            expected_active_dmap_revision: None,
            dgraph_ref: CanonicalContractReferenceV1 { id: dgraph_b.dgraph_id.clone(), revision: dgraph_b.revision, digest: dgraph_digest_b },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: dcontinuum_b.dcontinuum_id.clone(), revision: dcontinuum_b.revision, digest: dcontinuum_digest_b },
            placements: placements_b,
        };
        let proposal_digest_b = compute_proposal_digest(&proposal_b).unwrap();
        state.proposals.insert("p-b".to_string(), ProposalRecord {
            proposal: proposal_b.clone(),
            proposal_digest: proposal_digest_b.clone(),
            dgraph_snapshot: dgraph_b,
            dcontinuum_snapshot: dcontinuum_b,
            state: ProposalLifecycleState::Accepted,
            accepted_acceptance_id: Some("a-b".to_string()),
        });

        let dmap_id_b = canonical_dmap_id("proj-x", "app-b");
        let candidate_dmap_b = build_candidate_dmap(&proposal_b, &dmap_id_b, 1);
        let dmap_digest_b = recompute_dmap_digest(&candidate_dmap_b).unwrap();
        state.acceptances.insert("a-b".to_string(), DDeployAcceptanceV1 {
            schema: DDEPLOY_ACCEPTANCE_SCHEMA.to_string(),
            acceptance_id: "a-b".to_string(),
            proposal_id: "p-b".to_string(),
            proposal_digest: proposal_digest_b,
            project_id: "proj-x".to_string(),
            application_id: "app-b".to_string(),
            accepted_at_utc: "2026-01-01T00:00:00Z".to_string(),
            accepted_by: "operator:bob".to_string(),
            materialized_dmap_ref: CanonicalContractReferenceV1 { id: dmap_id_b.clone(), revision: 1, digest: dmap_digest_b.clone() },
            operational_projection_ref: OperationalProjectionRefV1 { operational_dgraph_id: operational_dgraph_id_for_project("proj-x"), revision: 2 },
        });

        let artifacts_b = vec![sample_artifact("svc-shared", vec!["cloud-01"])];
        let operational_services_b = derive_application_operational_services(&candidate_dmap_b, &artifacts_b).unwrap();
        state.active_by_application.insert(application_key("proj-x", "app-b"), ActiveApplicationDeployment {
            application_id: "app-b".to_string(),
            active_dmap: candidate_dmap_b,
            dmap_digest: dmap_digest_b,
            proposal_id: "p-b".to_string(),
            acceptance_id: "a-b".to_string(),
            accepted_at_utc: "2026-01-01T00:00:00Z".to_string(),
            operational_services: operational_services_b,
        });
        state.project_operational_revision.insert("proj-x".to_string(), 2);

        // Sanity: every per-record category (A-F) individually accepts
        // BOTH triples -- app-b's hand-built records are internally
        // coherent and correctly cross-referenced, exactly as if they had
        // been produced by a real (but never-actually-possible, due to the
        // pre-commit collision check) acceptance.
        persist_state(&path, &state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "two individually self-consistent accepted applications that jointly collide on service_id must be caught by RECONSTRUCTING the project union at reload, even though no single record's own checks can see it: {result:?}");
        cleanup_db(&path);
    }

    #[test]
    fn operationally_invalid_reconstructed_projection_is_caught_at_reload_even_when_records_are_individually_valid() {
        // A single application whose proposal/acceptance/active triple is
        // hand-built to be individually self-consistent under every
        // category-A-F check (zero placements matches zero
        // operational_services; all digests recompute correctly; all
        // cross-references match) -- but whose reconstructed project union
        // has zero nodes at a non-bootstrap revision, which
        // `validate_operational_dgraph` rejects. `accept_proposal`'s own
        // pre-commit check (operational-projection-validity-audit.txt)
        // would never let this be COMMITTED through the real API; this
        // proves the SAME contract is enforced again at reload.
        let path = temp_db_path("reload-invalid-projection");
        let mut state = DDeployStateV1::empty();

        let mut empty_dgraph = sample_dgraph("app-empty", 1);
        empty_dgraph.services.clear();
        empty_dgraph.calls.clear();
        let dcontinuum = sample_dcontinuum("proj-empty", 1);
        let dgraph_digest = recompute_dgraph_digest(&empty_dgraph).unwrap();
        let dcontinuum_digest = recompute_dcontinuum_digest(&dcontinuum).unwrap();
        let proposal = DDeployProposalV1 {
            schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
            proposal_id: "p-empty".to_string(),
            project_id: "proj-empty".to_string(),
            application_id: "app-empty".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            expected_active_dmap_revision: None,
            dgraph_ref: CanonicalContractReferenceV1 { id: empty_dgraph.dgraph_id.clone(), revision: empty_dgraph.revision, digest: dgraph_digest },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: dcontinuum_digest },
            placements: vec![],
        };
        let proposal_digest = compute_proposal_digest(&proposal).unwrap();
        state.proposals.insert("p-empty".to_string(), ProposalRecord {
            proposal: proposal.clone(),
            proposal_digest: proposal_digest.clone(),
            dgraph_snapshot: empty_dgraph,
            dcontinuum_snapshot: dcontinuum,
            state: ProposalLifecycleState::Accepted,
            accepted_acceptance_id: Some("a-empty".to_string()),
        });

        let dmap_id = canonical_dmap_id("proj-empty", "app-empty");
        let candidate_dmap = build_candidate_dmap(&proposal, &dmap_id, 1);
        let dmap_digest = recompute_dmap_digest(&candidate_dmap).unwrap();
        state.acceptances.insert("a-empty".to_string(), DDeployAcceptanceV1 {
            schema: DDEPLOY_ACCEPTANCE_SCHEMA.to_string(),
            acceptance_id: "a-empty".to_string(),
            proposal_id: "p-empty".to_string(),
            proposal_digest,
            project_id: "proj-empty".to_string(),
            application_id: "app-empty".to_string(),
            accepted_at_utc: "2026-01-01T00:00:00Z".to_string(),
            accepted_by: "operator:alice".to_string(),
            materialized_dmap_ref: CanonicalContractReferenceV1 { id: dmap_id.clone(), revision: 1, digest: dmap_digest.clone() },
            operational_projection_ref: OperationalProjectionRefV1 { operational_dgraph_id: operational_dgraph_id_for_project("proj-empty"), revision: 1 },
        });
        state.active_by_application.insert(application_key("proj-empty", "app-empty"), ActiveApplicationDeployment {
            application_id: "app-empty".to_string(),
            active_dmap: candidate_dmap,
            dmap_digest,
            proposal_id: "p-empty".to_string(),
            acceptance_id: "a-empty".to_string(),
            accepted_at_utc: "2026-01-01T00:00:00Z".to_string(),
            operational_services: BTreeMap::new(),
        });
        state.project_operational_revision.insert("proj-empty".to_string(), 1);

        persist_state(&path, &state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "a reconstructed project union with zero nodes at a non-bootstrap revision must fail closed at reload, even though every individual record is internally self-consistent: {result:?}");
        cleanup_db(&path);
    }

    #[test]
    fn positive_multi_application_multi_revision_state_reloads_cleanly() {
        // Positive control for R1/R2/category-G: a real, uncorrupted
        // multi-application state, including a SECOND accepted revision
        // for one application, must NOT be rejected by the union
        // reconstruction or by the accepted-history completeness checks.
        let path = temp_db_path("positive-multi-app-multi-revision");
        let mut state = DDeployStateV1::empty();

        let mut dgraph_a = sample_dgraph("app-a", 1);
        dgraph_a.services.truncate(1);
        dgraph_a.services[0].service_id = "svc-a".to_string();
        let artifacts_a = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-a", dgraph_a.clone(), "p-a1", vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }], &artifacts_a, "operator:alice", "a-a1", "2026-01-01T00:00:00Z").unwrap();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: dgraph_a, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "cloud-01".to_string() }] }, "p-a2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_a).unwrap();
        accept_proposal(&mut state, "p-a2", "operator:alice", None, &artifacts_a, "2026-01-01T01:00:00Z", "a-a2".to_string()).unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "svc-b".to_string();
        let artifacts_b = vec![sample_artifact("svc-b", vec!["fog-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-b", dgraph_b, "p-b1", vec![CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "fog-01".to_string() }], &artifacts_b, "operator:bob", "a-b1", "2026-01-01T02:00:00Z").unwrap();

        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path);
        assert!(reloaded.is_ok(), "a real, uncorrupted multi-application/multi-revision state must reload cleanly: {reloaded:?}");
        let reloaded = reloaded.unwrap();
        assert_eq!(reloaded.active_for("proj-x", "app-a").unwrap().active_dmap.revision, 2);
        assert_eq!(reloaded.active_for("proj-x", "app-b").unwrap().active_dmap.revision, 1);
        cleanup_db(&path);
    }

    // ---- independent review v4: R2 -- accepted-history authority --------
    // ---- completeness (active entry must exist, and must be the LATEST -
    // ---- consistent accepted materialization) ----------------------------

    #[test]
    fn accepted_application_with_no_active_entry_fails_closed_on_reload() {
        // The R2 "concrete deletion case": accept normally, then remove
        // ONLY the active_by_application entry, keeping the proposal and
        // acceptance untouched. Stage 111 exposes no undeploy operation
        // that could legitimately produce this shape.
        let path = temp_db_path("accepted-app-no-active-entry");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        state.active_by_application.remove(&key);
        persist_state(&path, &state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "an application with accepted history but NO active_by_application entry must fail closed -- canonical authority must not be silently lost: {result:?}");
        cleanup_db(&path);
    }

    #[test]
    fn removing_one_of_two_active_applications_fails_closed_on_reload() {
        let path = temp_db_path("remove-one-of-two-active");
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-a", "p-a", vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }, CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "cloud-01".to_string() }], &sample_artifacts(), "operator:alice", "a-a", "2026-01-01T00:00:00Z").unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "svc-c".to_string();
        let artifacts_c = vec![sample_artifact("svc-c", vec!["fog-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-b", dgraph_b, "p-b", vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "fog-01".to_string() }], &artifacts_c, "operator:bob", "a-b", "2026-01-01T01:00:00Z").unwrap();

        let key_b = application_key("proj-x", "app-b");
        state.active_by_application.remove(&key_b);
        persist_state(&path, &state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "removing ONE of two active applications (leaving the other intact) must still fail closed for the removed one: {result:?}");
        cleanup_db(&path);
    }

    #[test]
    fn restoring_an_older_active_entry_after_a_later_acceptance_fails_closed_on_reload() {
        // The R2 "concrete stale-active case": accept p1 (revision 1),
        // save that active entry, accept p2 (revision 2, same
        // application), then REPLACE the active entry with the SAVED
        // (older, but individually still self-consistent -- it still
        // matches p1's own real accepted history) revision-1 entry. Both
        // histories (p1 and p2) remain valid and present; only the choice
        // of WHICH is currently active is stale.
        let path = temp_db_path("restore-older-active-entry");
        let mut state = DDeployStateV1::empty();
        create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let key = application_key("proj-x", "app-x");
        let revision_1_entry = state.active_by_application.get(&key).unwrap().clone();

        let mut swapped = sample_placements();
        swapped[0].node_id = "cloud-01".to_string();
        let swapped_artifacts = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: swapped }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &swapped_artifacts).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &swapped_artifacts, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        assert_eq!(state.active_by_application.get(&key).unwrap().active_dmap.revision, 2, "sanity: p2 is now active at revision 2");

        // Restore the SAVED revision-1 entry over the current revision-2 one.
        state.active_by_application.insert(key, revision_1_entry);

        persist_state(&path, &state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "an active entry restored to an OLDER accepted revision, while a NEWER accepted revision exists in history, must fail closed -- the active entry must be the LATEST accepted materialization: {result:?}");
        cleanup_db(&path);
    }

    #[test]
    fn accepted_proposal_pointing_to_another_proposals_acceptance_fails_closed_on_reload() {
        // Independent review v4, R2: category A must check the Accepted-
        // proposal-to-acceptance link BIDIRECTIONALLY. Proposal p2 falsely
        // claims p1's REAL acceptance (a1) as its own.
        let path = temp_db_path("accepted-proposal-wrong-acceptance");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();

        // A second, genuinely PENDING proposal for a different application
        // (so it does not itself collide with p1's placements).
        let mut dgraph_c = sample_dgraph("app-c", 1);
        dgraph_c.services.truncate(1);
        dgraph_c.services[0].service_id = "svc-c".to_string();
        create_proposal(&mut state, CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-c".to_string(), source: ProposalSource::Operator, dgraph: dgraph_c, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "fog-01".to_string() }] }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &[sample_artifact("svc-c", vec!["fog-01"])]).unwrap();

        // Corrupt: falsely mark p2 Accepted, pointing at p1's REAL acceptance.
        let p2 = state.proposals.get_mut("p2").unwrap();
        p2.state = ProposalLifecycleState::Accepted;
        p2.accepted_acceptance_id = Some("a1".to_string());

        persist_state(&path, &state).unwrap();
        let result = load_state(&path);
        assert!(result.is_err(), "a proposal falsely marked Accepted and pointing at ANOTHER proposal's real acceptance must fail closed: {result:?}");
        cleanup_db(&path);
    }

    #[test]
    fn crash_during_acceptance_leaves_previous_active_deployment_unchanged() {
        let path = temp_db_path("crash-consistency");

        // 1. Accept revision N successfully and persist it.
        let mut state = load_state(&path).unwrap();
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let outcome_n = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        persist_state(&path, &state).unwrap();
        assert_eq!(outcome_n.materialized_dmap.revision, 1);

        // 2. Construct a proposal for N+1 and accept it IN MEMORY ONLY --
        // simulate a persistence failure by deliberately never calling
        // `persist_state` for this step (the exact failure mode a real I/O
        // error would produce: the mutated in-memory copy is discarded,
        // the on-disk state is untouched).
        let mut swapped = sample_placements();
        swapped[0].node_id = "cloud-01".to_string();
        let artifacts2 = vec![sample_artifact("svc-a", vec!["cloud-01", "fog-01"]), sample_artifact("svc-b", vec!["cloud-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: swapped },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts2,
        ).unwrap();
        // The proposal itself IS successfully persisted (a real caller
        // persists after every successful mutation)...
        persist_state(&path, &state).unwrap();
        let outcome_n1 = accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts2, "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        assert_eq!(outcome_n1.materialized_dmap.revision, 2, "in-memory acceptance succeeded (this is what a real caller would then try, and fail, to persist)");
        // ...but the ACCEPTANCE's persist is the one that "fails": deliberately
        // never call persist_state for the accepted mutation, and discard the
        // in-memory copy exactly as a real caller must on a persistence error.
        drop(state);

        // 3. Reload state -- must reflect the LAST SUCCESSFULLY PERSISTED
        // state (revision N), never the discarded in-memory N+1.
        let reloaded = load_state(&path).unwrap();
        let active = reloaded.active_for("proj-x", "app-x").unwrap();
        assert_eq!(active.active_dmap.revision, 1, "active D-Map must still be N after a simulated persistence failure during N+1's acceptance");
        assert_eq!(reloaded.proposals.get("p2").unwrap().state, ProposalLifecycleState::Pending, "proposal N+1 must NOT be accepted after a simulated persistence failure");
        let projection = active_operational_projection_from_state(&reloaded, "proj-x").unwrap();
        let fog = projection.nodes.iter().find(|n| n.node_id == "fog-01").unwrap();
        assert!(fog.desired_services.iter().any(|s| s.service_id == "svc-a"), "operational projection must still correspond to revision N (svc-a still on fog-01)");

        cleanup_db(&path);
    }

    // ---- authorization separation: old authorization cannot authorize a --
    // ---- new (canonical-derived) operational graph revision -------------

    /// EXPLICIT regression proof (independent review requirement): an
    /// `OperationalReconciliationAuthorization` issued against the LEGACY
    /// operational graph's `(dgraph_id, revision)` must NEVER validate
    /// against the canonical-derived `OperationalDGraph` a D-Deploy
    /// acceptance produces -- even for the identical project, node, and
    /// revision number -- because `operational_dgraph_id_for_project`
    /// deliberately never equals the legacy graph's own `dgraph_id`
    /// (`format!("dgraph:{project_id}")` in this test, matching real
    /// callers such as `api::operational_dgraph::post_operational_project`'s
    /// default). `authorization_is_valid_for_context`
    /// (`core::operational_dgraph.rs`) checks `authorization.dgraph_id !=
    /// graph.dgraph_id` unconditionally, so this holds with zero additional
    /// matching-logic changes in Stage 111.
    #[test]
    fn old_legacy_authorization_cannot_authorize_canonical_derived_graph() {
        use crate::core::operational_dgraph::{
            authorization_is_valid_for_context, ArtifactBindingV1, OperationalReconciliationAuthorization,
            AUTHORIZATION_SCOPE_PROCESS,
        };

        let legacy_dgraph_id = "dgraph:proj-x".to_string();
        let revision = 1u64;

        let legacy_graph = OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: legacy_dgraph_id.clone(),
            revision,
            project_id: "proj-x".to_string(),
            nodes: vec![OperationalNodeDesiredState { node_id: "fog-01".to_string(), desired_services: vec![] }],
            metadata: None,
        };

        let authorization = OperationalReconciliationAuthorization {
            authorization_id: "auth-1".to_string(),
            project_id: "proj-x".to_string(),
            dgraph_id: legacy_dgraph_id,
            dgraph_revision: revision,
            node_id: "fog-01".to_string(),
            binding_id: Some("binding-1".to_string()),
            resource_id: Some("resource-1".to_string()),
            authorized_by: "operator:alice".to_string(),
            authorized_at_utc: "2026-01-01T00:00:00Z".to_string(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            persistence_scope: AUTHORIZATION_SCOPE_PROCESS.to_string(),
            expires_at_utc: None,
            artifact_bindings: Vec::<ArtifactBindingV1>::new(),
            execution_pinning_mode: "legacy_partial".to_string(),
            snapshot: Vec::new(),
        };

        // Sanity: the authorization DOES match the legacy graph it was
        // actually issued against (proves the test setup itself is valid,
        // not vacuously false due to an unrelated field mismatch).
        assert!(authorization_is_valid_for_context(&authorization, &legacy_graph, "fog-01", "binding-1", chrono::Utc::now()));

        // Build the canonical-derived operational graph for the SAME
        // project/node/revision via the real acceptance path.
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        let outcome = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let canonical_graph = union_project_operational_graph(
            "proj-x",
            &state.active_by_application,
            outcome.operational_revision,
        )
        .unwrap();

        // The canonical-derived graph happens to carry the SAME revision
        // number (1) the legacy authorization was issued against.
        assert_eq!(canonical_graph.revision, revision);
        assert_ne!(canonical_graph.dgraph_id, "dgraph:proj-x", "canonical-derived operational graph id must never equal the legacy graph's own dgraph_id");

        assert!(
            !authorization_is_valid_for_context(&authorization, &canonical_graph, "fog-01", "binding-1", chrono::Utc::now()),
            "a legacy authorization must NEVER validate against the canonical-derived operational graph, even at the identical revision number"
        );
    }

    // ---- Stage 111 ddeploy-v1 fixture cross-check (mirrors ADR-0017's -----
    // ---- canonical-v1 cross-crate fixture proof pattern) ------------------

    /// Real fixture proof: constructs the SAME dgraph/dcontinuum/proposal/D-Map
    /// content `scripts/fixtures/datum-contracts/ddeploy-v1/proposal.valid.json`
    /// / `acceptance.valid.json` / `active-deployment.valid.json` describe,
    /// and asserts this crate's own digest functions reproduce the exact
    /// values recorded in those fixtures (`expected_digest_sha256`,
    /// `expected_dmap_digest_sha256`) -- the same discipline
    /// `agent_cross_crate_fixture_produces_the_expected_digest_and_validates`
    /// (ADR-0017, `canonical_dmap.rs`) uses for Stage 110. `DDeployProposalV1`
    /// is not mirrored into the DATUM agent crate (it is a DServer/D-Controller
    /// concern, per this ADR's non-goals), so this proof lives only in
    /// `dserver`; `scripts/check_ddeploy_contracts_v1.py` independently
    /// recomputes the same digests in Python and cross-checks the fixture
    /// files directly.
    #[test]
    fn ddeploy_v1_fixtures_reproduce_expected_digests_and_validate() {
        use crate::core::canonical_dcontinuum::{CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1, CanonicalDLinkV1, CANONICAL_DCONTINUUM_SCHEMA};
        use crate::core::canonical_dgraph::{CanonicalDCallV1, CanonicalDGraphV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0};

        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:ddeploy-lab".to_string(),
            dgraph_id: "dgraph:ddeploy-lab".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 { service_id: "svc-ingest".to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 268435456 } },
                CanonicalDServV1 { service_id: "svc-process".to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 268435456 } },
            ],
            calls: vec![CanonicalDCallV1 { call_id: "ingest-to-process".to_string(), source_service_id: "svc-ingest".to_string(), target_service_id: "svc-process".to_string() }],
        };
        assert_eq!(dgraph.validate(), Vec::new());

        let dcontinuum = CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: "monitor-f49c8".to_string(),
            dcontinuum_id: "continuum:ddeploy-lab".to_string(),
            revision: 1,
            nodes: vec![
                CanonicalDNodeV1 { node_id: "fog-01".to_string(), platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() }, capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4294967296 }, capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] } },
                CanonicalDNodeV1 { node_id: "cloud-01".to_string(), platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() }, capacity: CanonicalNodeCapacityV1 { cpu_cores: 8, memory_bytes: 17179869184 }, capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] } },
            ],
            links: vec![CanonicalDLinkV1 { link_id: "fog-to-cloud".to_string(), from_node_id: "fog-01".to_string(), to_node_id: "cloud-01".to_string() }],
        };
        assert_eq!(dcontinuum.validate(), Vec::new());

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("ddeploy-v1");

        let proposal_text = std::fs::read_to_string(dir.join("proposal.valid.json")).unwrap();
        let proposal_raw: serde_json::Value = serde_json::from_str(&proposal_text).unwrap();
        let expected_proposal_digest = proposal_raw["expected_digest_sha256"].as_str().unwrap().to_string();
        let proposal: DDeployProposalV1 = serde_json::from_str(&proposal_text).expect("fixture must deserialize as DDeployProposalV1");

        assert_eq!(proposal.dgraph_ref.digest, recompute_dgraph_digest(&dgraph).unwrap(), "fixture dgraph_ref.digest must match this crate's recomputed dgraph digest");
        assert_eq!(proposal.dcontinuum_ref.digest, recompute_dcontinuum_digest(&dcontinuum).unwrap(), "fixture dcontinuum_ref.digest must match this crate's recomputed dcontinuum digest");
        assert_eq!(validate_proposal_shape(&proposal), Vec::new(), "the positive proposal fixture must pass shape validation");

        let proposal_digest = compute_proposal_digest(&proposal).unwrap();
        assert_eq!(proposal_digest, expected_proposal_digest, "this crate's recomputed proposal digest must match proposal.valid.json's expected_digest_sha256");

        // Use the REAL `canonical_dmap_id` function (not a hardcoded
        // literal) so this fixture proof actually exercises current
        // dmap_id-composition behavior (independent review v2,
        // identifier-composition audit).
        let dmap_id = canonical_dmap_id(&proposal.project_id, &proposal.application_id);
        let dmap = build_candidate_dmap(&proposal, &dmap_id, 1);
        assert_eq!(dmap.validate_against(&dgraph, &dcontinuum), Vec::new(), "the candidate D-Map materialized from the fixture proposal must validate cleanly against its own dgraph/dcontinuum");
        let dmap_digest = recompute_dmap_digest(&dmap).unwrap();

        let acceptance_text = std::fs::read_to_string(dir.join("acceptance.valid.json")).unwrap();
        let acceptance_raw: serde_json::Value = serde_json::from_str(&acceptance_text).unwrap();
        assert_eq!(acceptance_raw["proposal_digest"].as_str().unwrap(), proposal_digest, "acceptance.valid.json's proposal_digest must match the real recomputed proposal digest");
        assert_eq!(acceptance_raw["materialized_dmap_ref"]["digest"].as_str().unwrap(), dmap_digest, "acceptance.valid.json's materialized_dmap_ref.digest must match the real recomputed D-Map digest");

        let active_text = std::fs::read_to_string(dir.join("active-deployment.valid.json")).unwrap();
        let active_raw: serde_json::Value = serde_json::from_str(&active_text).unwrap();
        assert_eq!(active_raw["dmap_digest"].as_str().unwrap(), dmap_digest, "active-deployment.valid.json's dmap_digest must match the real recomputed D-Map digest");
        let active_dmap: CanonicalDMapV1 = serde_json::from_value(active_raw["active_dmap"].clone()).expect("active_dmap must deserialize as CanonicalDMapV1");
        assert_eq!(active_dmap, dmap, "active-deployment.valid.json's embedded active_dmap must equal the D-Map this crate materializes from the fixture proposal");
    }

}
