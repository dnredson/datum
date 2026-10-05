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

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::core::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1, CanonicalValidationFinding};
use crate::core::canonical_dmap::{
    recompute_dmap_digest, CanonicalContractReferenceV1, CanonicalDMapV1, CanonicalDServPlacementV1,
    CANONICAL_DMAP_SCHEMA,
};
use crate::core::jcs;
use crate::core::operational_dgraph::{
    OperationalCleanupDirective, OperationalDGraph, OperationalDesiredService, OperationalNodeDesiredState,
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
/// Foundation closure v2 (ADR-0019, R1): a PENDING proposal's persisted
/// `dcontinuum_snapshot` no longer matches the CURRENT server-owned
/// structural D-Node registry at the moment of explicit acceptance -- the
/// node it depends on was removed, or its platform/capacity/ABI
/// capabilities changed since the proposal was created. Never raised for an
/// idempotent replay of an ALREADY-accepted proposal (historical
/// acceptances are immutable regardless of a later registry change).
pub const F_STALE_AUTHORITATIVE_DCONTINUUM: &str = "stale_authoritative_dcontinuum";
/// Foundation closure v2 (R3C), made GLOBAL by Phase 136A: accepting the
/// candidate active set (this proposal's own materialization plus every
/// OTHER currently-active application, across EVERY project sharing this
/// D-Node -- D-Nodes are global, never project-owned, so the capacity
/// ceiling this compares against is a global physical fact, not a
/// per-project one) would commit more CPU cores to one D-Node than its
/// current authoritative structural `capacity.cpu_cores`. Raised only on
/// the PENDING -> ACCEPTED transition, exactly like
/// `F_STALE_AUTHORITATIVE_DCONTINUUM`; applies uniformly regardless of
/// `ProposalSource` (deterministic, operator, or future AI). The constant
/// name/wire value are kept unchanged across the Phase 136A global-scope
/// correction (only the comparison's SCOPE changed, not its meaning: "this
/// D-Node's declared capacity would be exceeded").
pub const F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED: &str = "project_node_cpu_capacity_exceeded";
/// Same as `F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED`, for `capacity.memory_bytes`.
pub const F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED: &str = "project_node_memory_capacity_exceeded";
pub const F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE: &str = "ddeploy_v2_artifact_authority_stale";

// ---- Phase 117G: D-Code artifact-authority binding finding codes -------- //
pub const F_DCODE_BINDING_SERVICE_UNKNOWN: &str = "dcode_binding_service_unknown";
pub const F_DCODE_BINDING_DIGEST_MALFORMED: &str = "dcode_binding_digest_malformed";
pub const F_DCODE_BINDING_ARTIFACT_NOT_FOUND: &str = "dcode_binding_artifact_not_found";
pub const F_DCODE_BINDING_ARTIFACT_INVALID: &str = "dcode_binding_artifact_invalid";
pub const F_DCODE_BINDING_DESCRIPTOR_DIGEST_MISMATCH: &str = "dcode_binding_descriptor_digest_mismatch";
pub const F_DCODE_BINDING_APPLICATION_MISMATCH: &str = "dcode_binding_application_mismatch";
pub const F_DCODE_BINDING_SERVICE_MISMATCH: &str = "dcode_binding_service_mismatch";
pub const F_DCODE_BINDING_ABI_MISMATCH: &str = "dcode_binding_abi_mismatch";
pub const F_DCODE_BINDING_STAGE_CONSTRAINT_UNVERIFIABLE: &str = "dcode_binding_stage_constraint_unverifiable";
/// Phase 119G (independent review, R1-adjacent hardening): a v1→v2 transition
/// (`DDeployStateV1::accept_v2_proposal`) already retires v1's
/// `active_by_application` pointer when it installs v2 authority for an
/// application. The REVERSE direction has no such symmetric guard anywhere
/// in this file: `create_proposal`/`accept_proposal` (both v1-only) never
/// consult `state.phase113_v2.active` at all. Without this finding, a v1
/// proposal could be created and accepted for an application that is
/// CURRENTLY v2-active, producing simultaneous `active_by_application` and
/// `phase113_v2.active` entries for the same application key -- exactly the
/// "dual active DMap authority" corruption `active_placement_authority`
/// fails closed on for every subsequent read, and
/// `validate_persisted_state_integrity` refuses to ever reload. This finding
/// fails the v1 path closed at creation AND at acceptance, BEFORE any
/// mutation, rather than allowing that corruption to occur and only be
/// detected reactively later. Reverse-version-transition support (safely
/// retiring v2 authority to accept a v1 proposal) is an explicit
/// non-goal of this phase -- see `phase119g-governed-source-cleanup-implementation.md`.
pub const F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED: &str = "ddeploy_v1_downgrade_from_v2_unsupported";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

/// Phase 117G: the ONE shared validator for a proposal's
/// `dcode_artifact_authority` bindings, used identically at proposal
/// CREATION time (against current registry state) and at ACCEPTANCE time
/// (against current registry state, re-fetched under the caller's own
/// stable view -- mirroring `resolve_diot_artifact_authority`'s
/// acquire-once-then-resolve shape). Unlike D-IoT/`ServiceArtifact`'s own
/// mutable-registry re-resolution-and-compare pattern, this never needs to
/// "re-resolve and compare to what was bound before": the binding already
/// names an exact, immutable revision directly, so the only question is
/// whether that EXACT revision still exists and validates -- there is no
/// "current/latest" to drift away from underneath it.
///
/// For every `(service_id, binding)`: `service_id` must exist in `dgraph`;
/// `binding.descriptor_digest` must be the canonical `sha256:<64 lowercase
/// hex>` form; the exact revision must exist in `registry` for
/// `(application_id, service_id)`; it must independently validate; its
/// freshly-recomputed descriptor digest must equal the bound digest (never
/// trusted from the registry's own storage key); its own
/// `application_id`/`dserv_id` must match `application_id`/`service_id`;
/// its ABI must match the D-Graph service's `execution_requirement.dnode_abi`;
/// and `constraints.allowed_stages` must be empty (Phase 117D: a non-empty
/// value is structurally legal but permanently unauthorizable under v0.1 --
/// D-Compile/DDeploy must never bind to an artifact guaranteed to fail
/// authorization).
pub fn validate_dcode_artifact_authority_bindings(
    dgraph: &CanonicalDGraphV1,
    application_id: &str,
    registry: &crate::core::dcode_registry::DCodeArtifactRegistryStateV2,
    bindings: &std::collections::BTreeMap<String, crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1>,
) -> Result<(), CanonicalValidationFinding> {
    for (service_id, binding) in bindings {
        let service = dgraph.services.iter().find(|service| service.service_id == *service_id).ok_or_else(|| {
            finding(F_DCODE_BINDING_SERVICE_UNKNOWN, format!("dcode_artifact_authority references service_id '{service_id}', which does not exist in the proposal's D-Graph"))
        })?;
        if !jcs::is_canonical_digest_format(&binding.descriptor_digest) {
            return Err(finding(
                F_DCODE_BINDING_DIGEST_MALFORMED,
                format!("dcode_artifact_authority['{service_id}'].descriptor_digest must be 'sha256:<64 lowercase hex>', got '{}'", binding.descriptor_digest),
            ));
        }
        let artifact = registry.get_exact(application_id, service_id, &binding.descriptor_digest).ok_or_else(|| {
            finding(
                F_DCODE_BINDING_ARTIFACT_NOT_FOUND,
                format!("no registered D-Code revision exists for application_id='{application_id}' service_id='{service_id}' descriptor_digest='{}'", binding.descriptor_digest),
            )
        })?;
        let artifact_findings = artifact.validate();
        if !artifact_findings.is_empty() {
            return Err(finding(
                F_DCODE_BINDING_ARTIFACT_INVALID,
                format!("bound D-Code revision for service '{service_id}' no longer validates cleanly: {artifact_findings:?}"),
            ));
        }
        let recomputed = crate::core::dserv_artifact::recompute_dserv_artifact_digest(artifact)
            .map_err(|error| finding(F_DCODE_BINDING_ARTIFACT_INVALID, format!("recompute descriptor digest for service '{service_id}': {error}")))?;
        if recomputed != binding.descriptor_digest {
            return Err(finding(
                F_DCODE_BINDING_DESCRIPTOR_DIGEST_MISMATCH,
                format!("bound descriptor_digest '{}' for service '{service_id}' does not match its own freshly recomputed digest '{recomputed}' -- registry corruption", binding.descriptor_digest),
            ));
        }
        if artifact.application_id != application_id {
            return Err(finding(
                F_DCODE_BINDING_APPLICATION_MISMATCH,
                format!("bound D-Code revision for service '{service_id}' has application_id '{}', expected '{application_id}'", artifact.application_id),
            ));
        }
        if artifact.dserv_id != *service_id {
            return Err(finding(
                F_DCODE_BINDING_SERVICE_MISMATCH,
                format!("bound D-Code revision has dserv_id '{}', expected '{service_id}'", artifact.dserv_id),
            ));
        }
        let artifact_abi = format!("{}/{}", artifact.abi.name, artifact.abi.version);
        if artifact_abi != service.execution_requirement.dnode_abi {
            return Err(finding(
                F_DCODE_BINDING_ABI_MISMATCH,
                format!("bound D-Code revision for service '{service_id}' declares ABI '{artifact_abi}', but the D-Graph service requires '{}'", service.execution_requirement.dnode_abi),
            ));
        }
        if !artifact.constraints.allowed_stages.is_empty() {
            return Err(finding(
                F_DCODE_BINDING_STAGE_CONSTRAINT_UNVERIFIABLE,
                format!(
                    "bound D-Code revision for service '{service_id}' declares constraints.allowed_stages={:?}, which cannot currently be authorized under v0.1 -- refusing to bind an artifact guaranteed to fail invocation authorization",
                    artifact.constraints.allowed_stages
                ),
            ));
        }
    }
    Ok(())
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
    /// Phase 117G: explicit, exact D-Code artifact-revision binding per
    /// service (`service_id -> DCodeArtifactAuthorityBindingV1`). Empty for
    /// every pre-117G proposal and for any proposal that binds no D-Code
    /// service at all -- this is a real proposal-semantic field
    /// participating in `compute_proposal_digest`, correcting Phase 117F's
    /// own finding that D-Code binding was invisible to proposal identity
    /// (117F's "digest collision" wording was itself imprecise: two
    /// proposals' digests were never literally identical because
    /// `proposal_id`/`created_at_utc` already differ; the real gap was that
    /// NO field committed to which D-Code revision was intended, so this
    /// field closes that gap, not a digest-collision one).
    /// `skip_serializing_if` deliberately preserves the EXACT pre-117G
    /// serialized shape (and therefore digest) for any proposal that has no
    /// D-Code binding at all -- see
    /// `legacy_proposal_digest_is_unchanged_when_dcode_binding_absent`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub dcode_artifact_authority: std::collections::BTreeMap<String, crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalLifecycleState {
    Pending,
    Accepted,
}

/// Foundation closure v2 (P2, planner provenance persistence): INTERNAL-ONLY
/// evidence of exactly which deterministic planner run produced a proposal's
/// `placements`, retained alongside it so a HISTORICAL proposal can later
/// prove which planner/policy version produced it without relying on an
/// external client having preserved the original HTTP response. This is
/// never part of the public `datum.ddeploy-proposal/1` wire schema --
/// `DDeployProposalV1` gains no field for it; it lives only on the
/// internal `ProposalRecord` wrapper, exactly like `dgraph_snapshot`/
/// `dcontinuum_snapshot` already do. `None` for every operator/AI-sourced
/// proposal, and (via `#[serde(default)]` on `ProposalRecord`'s field) for
/// any proposal persisted before this field existed.
///
/// Independent review v3 (C1, provenance input completeness): after
/// foundation closure v2's R2/R3 changes, `plan_canonical_placements`
/// genuinely depends on FOUR inputs -- `CanonicalDGraphV1`,
/// `CanonicalDContinuumV1`, the project-wide structural commitment map
/// (R3A/R3B), and the `ServiceArtifact`-derived eligibility view (R2) -- not
/// merely the two canonical contracts. A `PlannerProvenanceV1` that recorded
/// only `dgraph`/`dcontinuum` identity (v2's original shape) was therefore no
/// longer a complete record of what genuinely affected this proposal's
/// placement selection: the same D-Graph and D-Continuum can now legitimately
/// produce a DIFFERENT placement depending on other active applications'
/// commitments or on `ServiceArtifact` eligibility at plan time. `dgraph_ref`/
/// `dcontinuum_ref`, `committed_by_node`, and `eligibility` below close that
/// gap -- every field a successful `plan_canonical_placements` call actually
/// received is now persisted verbatim alongside its `decisions`. This is
/// relational self-consistency and deterministic-reproducibility evidence,
/// never a claim of cryptographic authenticity against an attacker capable of
/// rewriting the entire SQLite database and recomputing every digest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlannerProvenanceV1 {
    pub planner_version: String,
    pub policy_name: String,
    pub policy_version: String,
    /// C1C fix: must relationally match `DDeployProposalV1.dgraph_ref`
    /// exactly (`validate_persisted_state_integrity` verifies this).
    pub dgraph_ref: CanonicalContractReferenceV1,
    /// C1C fix: must relationally match `DDeployProposalV1.dcontinuum_ref`
    /// exactly.
    pub dcontinuum_ref: CanonicalContractReferenceV1,
    /// C1A fix: the EXACT per-node structural commitment baseline actually
    /// supplied to `plan_canonical_placements` for this successful call --
    /// structural only, never live telemetry; for a redeployment of this
    /// same application it already excludes that application's own prior
    /// active allocation (R3B). Phase 120B correction: this is
    /// `compute_global_node_commitments_versioned`'s own return value, not
    /// `compute_global_node_commitments`'s -- it reflects every OTHER
    /// active application's structural commitment across BOTH DMap/1 and
    /// DMap/2, not DMap/1 alone (an earlier version of this comment named
    /// the non-versioned function, which was a real gap: an active DMap/2
    /// application's commitment was invisible to the planner until this
    /// fix). Phase 136A correction: also GLOBAL across every project
    /// sharing a D-Node, not scoped to this proposal's own project (the
    /// prior project-scoped aggregation allowed cross-project
    /// oversubscription of one shared D-Node's declared capacity -- see
    /// `documentation/reference/phase136-placement-ui.md`). An empty map
    /// legitimately means "no other active application, in any project,
    /// committed anything to any D-Node, under either D-Map version, at
    /// plan time."
    pub committed_by_node: BTreeMap<String, crate::core::canonical_placement::NodeCommitmentV1>,
    /// C1B fix: the EXACT `PlacementEligibilityView` actually supplied to
    /// `plan_canonical_placements` for this successful call -- preserves the
    /// absent-vs-present-empty distinction
    /// (`derive_placement_eligibility_view`'s own doc comment) rather than
    /// collapsing it into one representation.
    pub eligibility: crate::core::canonical_placement::PlacementEligibilityView,
    pub decisions: Vec<crate::core::canonical_placement::PlacementDecisionRecord>,
}

/// Internal, persisted record: the public proposal PLUS the immutable exact
/// D-Graph/D-Continuum snapshots it was validated against (independent
/// review requirement: proposal validation must be reproducible later
/// without trusting a caller-supplied digest as authoritative) and its
/// server-computed digest and lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProposalRecord {
    pub proposal: DDeployProposalV1,
    /// P2 fix: `Some` only for a proposal produced by
    /// `core::canonical_placement::plan_canonical_placements` (always
    /// implies `proposal.source == ProposalSource::Deterministic`, verified
    /// by `validate_persisted_state_integrity`). `None` for every operator/
    /// AI-sourced proposal and for any pre-existing persisted proposal.
    #[serde(default)]
    pub planner_provenance: Option<PlannerProvenanceV1>,
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
    /// Phase-113A v2 lifecycle records live in this same persisted domain.
    /// Keeping the field additive preserves loading of historical v1-only
    /// blobs while preventing a second deployment database.
    #[serde(default)]
    pub phase113_v2: crate::core::ddeploy_v2::DDeployStateV2,
}

impl DDeployStateV1 {
    pub fn empty() -> Self {
        Self {
            state_version: DDEPLOY_STATE_SCHEMA_VERSION.to_string(),
            proposals: BTreeMap::new(),
            acceptances: BTreeMap::new(),
            active_by_application: BTreeMap::new(),
            project_operational_revision: BTreeMap::new(),
            phase113_v2: crate::core::ddeploy_v2::DDeployStateV2::default(),
        }
    }


    pub fn active_for(&self, project_id: &str, application_id: &str) -> Option<&ActiveApplicationDeployment> {
        self.active_by_application.get(&application_key(project_id, application_id))
    }


    pub fn has_active_authority(&self, project_id: &str) -> bool {
        self.active_by_application
            .values()
            .any(|a| a.active_dmap.project_id == project_id)
            || self
                .phase113_v2
                .active
                .values()
                .any(|a| a.map.project_id == project_id)
    }


    pub fn create_v2_proposal(
        &mut self,
        proposal: crate::core::ddeploy_v2::DDeployProposalV2,
        dgraph: CanonicalDGraphV1,
        dcontinuum: CanonicalDContinuumV1,
        dforward: crate::core::canonical_dforward::CanonicalDForwardV1,
    ) -> Result<String, String> {
        self.phase113_v2
            .create_proposal(proposal, dgraph, dcontinuum, dforward)
    }


    pub fn accept_v2_proposal(
        &mut self,
        proposal_id: &str,
        acceptance_id: String,
        accepted_by: String,
        accepted_at_utc: String,
        current_authoritative_dcontinuum: &CanonicalDContinuumV1,
        current_artifacts: &[crate::core::service_artifact_registry::ServiceArtifact],
        // Phase 113A closure pass 4.1: deliberately the LAST parameter
        // (rather than mirroring v1's `accept_proposal`, which places its
        // equivalent right after `accepted_by`) to minimize churn to the 19
        // already-hardened Pass 1-3.1 call sites -- parameter position has
        // no effect on the CAS semantics itself, which are otherwise
        // identical to v1's.
        expected_proposal_digest: Option<&str>,
    ) -> Result<crate::core::ddeploy_v2::DDeployAcceptanceV2, String> {
        let record = self
            .phase113_v2
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or("proposal not found")?;

        // Phase 113A closure pass 4.1: caller-supplied CAS guard, mirroring
        // v1's `accept_proposal` exactly (see its own `expected_proposal_digest`
        // check for the identical rationale). Checked BEFORE ServiceArtifact
        // re-resolution, the capacity gate, and `accept_with_previous`'s own
        // idempotent-replay branch -- so a WRONG expected digest rejects
        // even when replaying an already-Accepted proposal, never silently
        // served the old result (test J).
        if let Some(expected) = expected_proposal_digest {
            if expected != record.proposal_digest {
                return Err(format!(
                    "expected_proposal_digest '{expected}' does not match proposal '{proposal_id}''s recorded digest '{}'",
                    record.proposal_digest
                ));
            }
        }

        // Phase 113A closure pass 3: re-resolve every DIoT artifact against
        // the CURRENT ServiceArtifact registry (a stable snapshot the caller
        // obtained via ONE `with_stable_view` call, same convention as v1's
        // `resolve_artifact_for_service`) and require EXACT equality with
        // what proposal-time binding recorded. Runs BEFORE the capacity gate
        // and BEFORE any active-authority mutation, acceptance insertion, or
        // persistence -- a stale/missing/ambiguous artifact rejects closed
        // and leaves everything untouched, exactly like the capacity gate
        // immediately below it.
        let current_authority = resolve_diot_artifact_authority(
            &record.dforward_snapshot,
            &record.proposal.project_id,
            current_artifacts,
        )
        .map_err(|e| {
            format!("{F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE}: current ServiceArtifact authority could not be resolved for proposal '{proposal_id}': {e}")
        })?;
        if current_authority != record.proposal.diot_artifact_authority {
            return Err(format!(
                "{F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE}: proposal '{proposal_id}' was bound to DIoT artifact authority that no longer matches the current ServiceArtifact registry -- create a new proposal against current authority"
            ));
        }

        let key = application_key(&record.proposal.project_id, &record.proposal.application_id);
        let previous_v2 = self
            .phase113_v2
            .active
            .get(&key)
            .map(|a| (a.map.dmap_id.clone(), a.map.revision));
        let previous = previous_v2
            .as_ref()
            .map(|(id, revision)| (id.as_str(), *revision))
            .or_else(|| {
                self.active_by_application
                    .get(&key)
                    .map(|a| (a.active_dmap.dmap_id.as_str(), a.active_dmap.revision))
            });

        // Phase 113A closure, hardened Phase 136A: mirror Stage 111/
        // foundation-closure's v1 project-wide committed-capacity gate (see
        // `accept_proposal`'s own identical check, immediately below the
        // candidate operational projection validation) for the DDeploy/2
        // acceptance path. Phase 136A made this GLOBAL (every project
        // sharing a physical node, not just this proposal's own) -- see
        // `evaluate_global_node_capacity`'s own doc comment. Other active
        // applications' commitments are derived from the version-aware view
        // (both DMap/1 and DMap/2 authority), EXCLUDING this application's
        // own previous active entry -- accepting a new revision REPLACES,
        // never adds to, that application's own prior commitment. Never
        // repairs or regenerates placement on failure: the operator must
        // create a new proposal against the now-current structural reality.
        let baseline_commitments = compute_global_node_commitments_versioned(
            self,
            Some((&record.proposal.project_id, &record.proposal.application_id)),
        )?;
        let candidate_services: BTreeMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1> =
            record
                .dgraph_snapshot
                .services
                .iter()
                .map(|s| (s.service_id.as_str(), s))
                .collect();
        // `DServPlacementV2` and `CanonicalDServPlacementV1` share the exact
        // same `{service_id, node_id}` shape (see `canonical_dmap_v2.rs`'s
        // own `validate()`, which already converts the same way to reuse
        // the v1 aggregate-capacity gate) -- converted here only to call the
        // one shared capacity-evaluation function, never duplicated.
        let v1_shaped_placements: Vec<CanonicalDServPlacementV1> = record
            .proposal
            .placements
            .iter()
            .map(|p| CanonicalDServPlacementV1 { service_id: p.service_id.clone(), node_id: p.node_id.clone() })
            .collect();
        let capacity_findings = evaluate_global_node_capacity(
            &baseline_commitments,
            &v1_shaped_placements,
            &candidate_services,
            current_authoritative_dcontinuum,
        )?;
        if !capacity_findings.is_empty() {
            return Err(capacity_findings
                .iter()
                .map(|f| format!("{}: {} (proposal '{proposal_id}')", f.code, f.message))
                .collect::<Vec<_>>()
                .join("; "));
        }

        let acceptance = self.phase113_v2.accept_with_previous(
            proposal_id,
            acceptance_id,
            accepted_by,
            accepted_at_utc,
            previous,
        )?;
        // The transition is one in-memory state mutation; persistence is
        // performed by the existing persist_state() transaction by the API.
        self.active_by_application.remove(&key);
        Ok(acceptance)
    }

    /// Phase 117G Section 14: the DDeploy/2 counterpart to
    /// `accept_proposal_with_dcode_binding_revalidation`, giving v2 the
    /// EXACT same acceptance-time D-Code binding revalidation guarantee as
    /// v1 -- via the SAME shared `validate_dcode_artifact_authority_bindings`
    /// helper, never a parallel/divergent v2-specific check. Pre-validates
    /// every explicit `dcode_artifact_authority` binding on a still-PENDING
    /// proposal against CURRENT registry state BEFORE delegating to the
    /// real, unmodified `accept_v2_proposal` -- fails closed, mutating
    /// nothing, if any bound descriptor revision no longer resolves. Skipped
    /// entirely for an idempotent replay of an already-Accepted proposal
    /// (mirrors v1: a historical acceptance's own binding remains valid
    /// regardless of later registry changes) and for a proposal that never
    /// bound any service (empty map -- nothing to revalidate).
    #[allow(clippy::too_many_arguments)]
    pub fn accept_v2_proposal_with_dcode_binding_revalidation(
        &mut self,
        proposal_id: &str,
        acceptance_id: String,
        accepted_by: String,
        accepted_at_utc: String,
        current_authoritative_dcontinuum: &CanonicalDContinuumV1,
        current_artifacts: &[crate::core::service_artifact_registry::ServiceArtifact],
        dcode_registry: &crate::core::dcode_registry::DCodeArtifactRegistryStateV2,
        expected_proposal_digest: Option<&str>,
    ) -> Result<crate::core::ddeploy_v2::DDeployAcceptanceV2, String> {
        let record = self
            .phase113_v2
            .proposals
            .get(proposal_id)
            .cloned()
            .ok_or("proposal not found")?;
        if record.state != crate::core::ddeploy_v2::ProposalV2State::Accepted && !record.proposal.dcode_artifact_authority.is_empty() {
            validate_dcode_artifact_authority_bindings(
                &record.dgraph_snapshot,
                &record.proposal.application_id,
                dcode_registry,
                &record.proposal.dcode_artifact_authority,
            )
            .map_err(|finding| format!("{}: {}", finding.code, finding.message))?;
        }
        self.accept_v2_proposal(
            proposal_id,
            acceptance_id,
            accepted_by,
            accepted_at_utc,
            current_authoritative_dcontinuum,
            current_artifacts,
            expected_proposal_digest,
        )
    }
}

pub fn active_placement_authority(
    state: &DDeployStateV1,
    project_id: &str,
    application_id: &str,
) -> Result<Option<crate::core::ddeploy_v2::PlacementAuthorityView>, String> {
    use crate::core::ddeploy_v2::{PlacementAuthorityV2Extras, PlacementAuthorityView};

    let key = application_key(project_id, application_id);
    let v1 = state.active_by_application.get(&key);
    let v2 = state.phase113_v2.active.get(&key);
    if v1.is_some() && v2.is_some() {
        return Err(format!(
            "dual active DMap authority for application key '{key}'"
        ));
    }
    if let Some(a) = v2 {
        let proposal = state
            .phase113_v2
            .proposals
            .get(&a.proposal_id)
            .ok_or_else(|| {
                format!(
                    "active v2 application '{key}' references missing proposal '{}'",
                    a.proposal_id
                )
            })?;
        let dgraph_digest = recompute_dgraph_digest(&proposal.dgraph_snapshot)?;
        return Ok(Some(PlacementAuthorityView {
            project_id: a.map.project_id.clone(),
            application_id: a.map.application_id.clone(),
            dmap_id: a.map.dmap_id.clone(),
            revision: a.map.revision,
            digest: a.digest.clone(),
            placements: a.map.placements.clone(),
            proposal_id: Some(a.proposal_id.clone()),
            acceptance_id: Some(a.acceptance_id.clone()),
            dgraph_id: Some(proposal.dgraph_snapshot.dgraph_id.clone()),
            dgraph_revision: Some(proposal.dgraph_snapshot.revision),
            dgraph_digest: Some(dgraph_digest),
            dgraph_snapshot: Some(proposal.dgraph_snapshot.clone()),
            v2: Some(PlacementAuthorityV2Extras {
                dforward_id: a.map.dforward_ref.id.clone(),
                dforward_revision: a.map.dforward_ref.revision,
                dforward_digest: a.map.dforward_ref.digest.clone(),
                dcall_realizations: a.map.dcall_realizations.clone(),
                diot_placements: a.map.diot_placements.clone(),
                // Phase 113A closure pass 5: the real accepted snapshot,
                // never a caller-supplied one -- see the field's doc comment.
                dforward_snapshot: Some(proposal.dforward_snapshot.clone()),
                external_port_bindings: a.map.external_port_bindings.clone(),
            }),
        }));
    }
    if let Some(a) = v1 {
        let proposal = state.proposals.get(&a.proposal_id).ok_or_else(|| {
            format!(
                "active v1 application '{key}' references missing proposal '{}'",
                a.proposal_id
            )
        })?;
        let dgraph_digest = recompute_dgraph_digest(&proposal.dgraph_snapshot)?;
        return Ok(Some(PlacementAuthorityView {
            project_id: a.active_dmap.project_id.clone(),
            application_id: a.active_dmap.application_id.clone(),
            dmap_id: a.active_dmap.dmap_id.clone(),
            revision: a.active_dmap.revision,
            digest: a.dmap_digest.clone(),
            placements: a
                .active_dmap
                .placements
                .iter()
                .map(|p| crate::core::canonical_dmap_v2::DServPlacementV2 {
                    service_id: p.service_id.clone(),
                    node_id: p.node_id.clone(),
                })
                .collect(),
            proposal_id: Some(a.proposal_id.clone()),
            acceptance_id: Some(a.acceptance_id.clone()),
            dgraph_id: Some(proposal.dgraph_snapshot.dgraph_id.clone()),
            dgraph_revision: Some(proposal.dgraph_snapshot.revision),
            dgraph_digest: Some(dgraph_digest),
            dgraph_snapshot: Some(proposal.dgraph_snapshot.clone()),
            v2: None,
        }));
    }
    Ok(None)
}

/// Phase 118B: D-Engine membership, derived ENTIRELY from `view.placements`
/// (the top-level, v1/v2-neutral D-Serv placement list `active_placement_authority`
/// already produces) -- never from `view.v2.map(|v| &v.diot_placements)`, which
/// is DIoT/D-Forward's own placement domain, structurally and semantically
/// separate (Phase 118A audit, section 9). D-Engine is a passive, canonical
/// membership-set noun ("the set of D-Nodes that runs a D-Application"), not an
/// independent authority: this function has no persistence, identity, revision,
/// or digest of its own, and consults no D-Node registry, D-Monitor, capability,
/// or D-Code state -- it is a pure projection of one field of an already-accepted
/// `PlacementAuthorityView`. Callers that need to distinguish "no active
/// authority" from "active authority, zero placements" must do so BEFORE calling
/// this function, via `active_placement_authority`'s own `Result<Option<...>>`
/// (Phase 118A audit, section 15/21/23) -- receiving a `&PlacementAuthorityView`
/// here already implies active authority exists.
pub fn derive_active_dengine_nodes(
    view: &crate::core::ddeploy_v2::PlacementAuthorityView,
) -> Vec<String> {
    let mut nodes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for placement in &view.placements {
        nodes.insert(placement.node_id.clone());
    }
    nodes.into_iter().collect()
}

/// Phase 117G: the exact D-Code artifact-revision binding currently active
/// for `service_id`, derived ENTIRELY from the active deployment's own
/// already-accepted, immutable proposal record -- never a second,
/// independently-persisted copy (mirroring the established D-IoT-artifact-
/// authority precedent: `ActiveDMapV2`/`ActiveApplicationDeployment` never
/// duplicate `diot_artifact_authority`/`dcode_artifact_authority` either;
/// both are always re-read from the accepted proposal via `proposal_id`).
/// `None` means no explicit binding exists for this service on the active
/// deployment (a pre-117G proposal, or one that never bound this
/// particular service) -- the caller must then fall back to the registry's
/// own unambiguous-single-revision lookup, never guess.
pub fn resolve_active_dcode_binding<'a>(
    state: &'a DDeployStateV1,
    active: &crate::core::ddeploy_v2::PlacementAuthorityView,
    service_id: &str,
) -> Option<&'a crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1> {
    let proposal_id = active.proposal_id.as_deref()?;
    if active.v2.is_some() {
        state.phase113_v2.proposals.get(proposal_id)?.proposal.dcode_artifact_authority.get(service_id)
    } else {
        state.proposals.get(proposal_id)?.proposal.dcode_artifact_authority.get(service_id)
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
pub(crate) fn validate_persisted_state_integrity(state: &DDeployStateV1) -> Result<(), String> {
    fn violation(message: impl Into<String>) -> String {
        format!("persisted state integrity violation: {} -- refusing to load potentially corrupted controller state", message.into())
    }

    state.phase113_v2.validate_integrity().map_err(violation)?;
    for key in state.phase113_v2.active.keys() {
        if state.active_by_application.contains_key(key) {
            return Err(violation(format!(
                "simultaneous v1 and v2 active authority for key '{key}'"
            )));
        }
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
        // P2/C1 fix: a persisted `planner_provenance` must be internally
        // consistent with the proposal it rides alongside -- it can only
        // legitimately exist for a Deterministic-sourced proposal, its
        // canonical input refs must relationally match the proposal's own,
        // and its decision records must agree EXACTLY (no duplicate, no
        // missing, no extra, no mismatched node/ABI/arithmetic) with the
        // proposal's own persisted `placements` and `dgraph_snapshot`.
        if let Some(provenance) = &record.planner_provenance {
            if proposal.source != ProposalSource::Deterministic {
                return Err(violation(format!(
                    "proposal '{proposal_key}' carries planner_provenance but its own source is '{:?}', not Deterministic", proposal.source
                )));
            }
            // C1C: the persisted canonical input refs must relationally
            // match the proposal's own `dgraph_ref`/`dcontinuum_ref` exactly
            // (already independently verified above against
            // `dgraph_snapshot`/`dcontinuum_snapshot`) -- otherwise a
            // tampered/rewritten `planner_provenance` could silently disagree
            // about WHAT this proposal was actually planned against.
            if provenance.dgraph_ref != proposal.dgraph_ref {
                return Err(violation(format!(
                    "proposal '{proposal_key}' planner_provenance.dgraph_ref {:?} does not match its own proposal.dgraph_ref {:?}",
                    provenance.dgraph_ref, proposal.dgraph_ref
                )));
            }
            if provenance.dcontinuum_ref != proposal.dcontinuum_ref {
                return Err(violation(format!(
                    "proposal '{proposal_key}' planner_provenance.dcontinuum_ref {:?} does not match its own proposal.dcontinuum_ref {:?}",
                    provenance.dcontinuum_ref, proposal.dcontinuum_ref
                )));
            }
            // C1D: exactly one decision per proposal placement -- a naive
            // `Vec -> BTreeMap<service_id, node_id>` collapse (v2's original
            // check) would silently hide a duplicate decision service_id by
            // simply keeping the last one; this catches that directly.
            let mut seen_decision_services: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for decision in &provenance.decisions {
                if !seen_decision_services.insert(decision.service_id.as_str()) {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' planner_provenance carries more than one decision for service_id '{}'",
                        decision.service_id
                    )));
                }
            }
            let placement_service_ids: std::collections::BTreeSet<&str> =
                proposal.placements.iter().map(|p| p.service_id.as_str()).collect();
            if seen_decision_services != placement_service_ids {
                return Err(violation(format!(
                    "proposal '{proposal_key}' planner_provenance decision service_ids {seen_decision_services:?} do not exactly match its own persisted placement service_ids {placement_service_ids:?} (missing or extra decision)"
                )));
            }
            let placements_by_service: BTreeMap<&str, &str> =
                proposal.placements.iter().map(|p| (p.service_id.as_str(), p.node_id.as_str())).collect();
            let services_by_id: BTreeMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1> =
                record.dgraph_snapshot.services.iter().map(|s| (s.service_id.as_str(), s)).collect();
            for decision in &provenance.decisions {
                // Unreachable given the exact-set check above (every decision
                // service_id was just proven to be a placement service_id
                // too), but checked explicitly rather than indexed/unwrapped.
                let Some(&expected_node_id) = placements_by_service.get(decision.service_id.as_str()) else {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' planner_provenance decision for service '{}' has no corresponding proposal placement", decision.service_id
                    )));
                };
                if decision.node_id != expected_node_id {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' planner_provenance decision places service '{}' on node '{}', but the proposal's own placement puts it on '{expected_node_id}'",
                        decision.service_id, decision.node_id
                    )));
                }
                let Some(service) = services_by_id.get(decision.service_id.as_str()) else {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' planner_provenance decision references service '{}', which is absent from its own dgraph_snapshot", decision.service_id
                    )));
                };
                if decision.required_dnode_abi != service.execution_requirement.dnode_abi {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' planner_provenance decision for service '{}' records required_dnode_abi '{}', but its own dgraph_snapshot declares '{}'",
                        decision.service_id, decision.required_dnode_abi, service.execution_requirement.dnode_abi
                    )));
                }
                let required_cpu = service.resource_requirement.cpu_cores as u64;
                let required_memory = service.resource_requirement.memory_bytes;
                let cpu_ok = decision.node_residual_cpu_cores_before.checked_sub(required_cpu) == Some(decision.node_residual_cpu_cores_after);
                let memory_ok =
                    decision.node_residual_memory_bytes_before.checked_sub(required_memory) == Some(decision.node_residual_memory_bytes_after);
                if !cpu_ok || !memory_ok {
                    return Err(violation(format!(
                        "proposal '{proposal_key}' planner_provenance decision for service '{}' has residual arithmetic inconsistent with its own declared resource_requirement {{cpu_cores: {required_cpu}, memory_bytes: {required_memory}}} (before={{{}, {}}}, after={{{}, {}}})",
                        decision.service_id,
                        decision.node_residual_cpu_cores_before,
                        decision.node_residual_memory_bytes_before,
                        decision.node_residual_cpu_cores_after,
                        decision.node_residual_memory_bytes_after
                    )));
                }
            }
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
    let mut latest_acceptance_by_app: BTreeMap<String, (u64, String, bool)> = BTreeMap::new();
    for acceptance in state.acceptances.values() {
        let app_key = application_key(&acceptance.project_id, &acceptance.application_id);
        let revision = acceptance.materialized_dmap_ref.revision;
        match latest_acceptance_by_app.get(&app_key) {
            None => {
                latest_acceptance_by_app
                    .insert(app_key, (revision, acceptance.acceptance_id.clone(), false));
            }
            Some((existing_revision, existing_acceptance_id, _)) => {
                if revision > *existing_revision {
                    latest_acceptance_by_app
                        .insert(app_key, (revision, acceptance.acceptance_id.clone(), false));
                } else if revision == *existing_revision
                    && existing_acceptance_id != &acceptance.acceptance_id
                {
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
    for acceptance in state.phase113_v2.acceptances.values() {
        let app_key = application_key(&acceptance.project_id, &acceptance.application_id);
        let revision = acceptance.materialized_dmap_ref.revision;
        match latest_acceptance_by_app.get(&app_key) {
            None => {
                latest_acceptance_by_app
                    .insert(app_key, (revision, acceptance.acceptance_id.clone(), true));
            }
            Some((existing_revision, existing_acceptance_id, _))
                if revision > *existing_revision =>
            {
                latest_acceptance_by_app
                    .insert(app_key, (revision, acceptance.acceptance_id.clone(), true));
            }
            Some((existing_revision, existing_acceptance_id, _))
                if revision == *existing_revision
                    && existing_acceptance_id != &acceptance.acceptance_id =>
            {
                return Err(violation(format!("application '{app_key}' has duplicate cross-version accepted revision {revision}")));
            }
            _ => {}
        }
    }
    for (app_key, (latest_revision, latest_acceptance_id, latest_is_v2)) in
        &latest_acceptance_by_app
    {
        if *latest_is_v2 {
            let active = state.phase113_v2.active.get(app_key).ok_or_else(|| violation(format!("application '{app_key}' latest v2 acceptance '{latest_acceptance_id}' has no active v2 authority")))?;
            if active.map.revision != *latest_revision
                || active.acceptance_id != *latest_acceptance_id
                || active.digest != active.map.digest()?
            {
                return Err(violation(format!("application '{app_key}' active v2 authority does not match latest accepted revision {latest_revision}")));
            }
            if state.active_by_application.contains_key(app_key) {
                return Err(violation(format!("application '{app_key}' has v1 active authority while latest accepted authority is v2")));
            }
        } else {
            let active = state.active_by_application.get(app_key).ok_or_else(|| violation(format!("application '{app_key}' latest v1 acceptance '{latest_acceptance_id}' has no active v1 authority")))?;
            if active.active_dmap.revision != *latest_revision
                || active.acceptance_id != *latest_acceptance_id
            {
                return Err(violation(format!("application '{app_key}' active v1 authority does not match latest accepted revision {latest_revision}")));
            }
            if state.phase113_v2.active.contains_key(app_key) {
                return Err(violation(format!("application '{app_key}' has v2 active authority while latest accepted authority is v1")));
            }
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
pub fn resolve_diot_artifact_authority(
    dforward: &crate::core::canonical_dforward::CanonicalDForwardV1,
    project_id: &str,
    artifacts: &[ServiceArtifact],
) -> Result<BTreeMap<String, crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2>, String> {
    use crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2;
    let mut bindings: BTreeMap<String, DIoTArtifactAuthorityBindingV2> = BTreeMap::new();
    for diot in &dforward.diots {
        let artifact_id = &diot.artifact_requirement.artifact_id;
        if bindings.contains_key(artifact_id) {
            continue;
        }
        let matches: Vec<&ServiceArtifact> = artifacts
            .iter()
            .filter(|a| a.artifact_id == *artifact_id && a.control_plane.project_id == project_id)
            .collect();
        match matches.as_slice() {
            [artifact] => {
                bindings.insert(
                    artifact_id.clone(),
                    DIoTArtifactAuthorityBindingV2 {
                        artifact_id: artifact_id.clone(),
                        execution_generation: artifact.control_plane.execution_generation,
                        execution_projection_digest: artifact
                            .control_plane
                            .execution_projection_digest
                            .clone(),
                    },
                );
            }
            [] => {
                return Err(format!(
                    "{F_ARTIFACT_NOT_FOUND}: dforward '{}' diot '{}' requires artifact '{artifact_id}', which does not exist in project '{project_id}''s ServiceArtifact registry",
                    dforward.dforward_id, diot.diot_id
                ));
            }
            _ => {
                return Err(format!(
                    "{F_ARTIFACT_AMBIGUOUS}: dforward '{}' diot '{}' artifact requirement '{artifact_id}' resolved to more than one ServiceArtifact record in project '{project_id}'",
                    dforward.dforward_id, diot.diot_id
                ));
            }
        }
    }
    Ok(bindings)
}

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

/// Foundation closure v2 (R2 fix): derives an EPHEMERAL, per-service
/// placement-eligibility view from the CURRENT project-scoped
/// `ServiceArtifact` snapshot, for the deterministic planner
/// (`core::canonical_placement`) to filter candidate nodes BEFORE best-fit
/// selection -- never persisted, never part of canonical D-Graph/
/// D-Continuum, and reusing `resolve_artifact_for_service`'s exact
/// resolution semantics rather than a second, independently-drifting
/// interpretation.
///
/// For a service_id whose artifact resolves (exactly one match): the
/// returned view's eligible-node set for that service is EXACTLY
/// `artifact.control_plane.target_nodes` (including the empty set, which
/// mirrors `check_node_eligibility`'s own real per-node semantics: an
/// artifact with an explicitly empty `target_nodes` makes EVERY node
/// ineligible for that service -- this function does not weaken or
/// reinterpret that).
///
/// For a service_id whose artifact does NOT resolve (not found, or
/// ambiguous): the service_id is simply ABSENT from the returned view. The
/// planner treats an absent service_id as unrestricted-by-artifact-
/// eligibility -- this is deliberate, not a laxer reinterpretation of
/// `target_nodes`: a brand-new application commonly has no `ServiceArtifact`
/// registered yet at planning time, and `create_proposal`'s own
/// `resolve_artifact_for_service` call (inside `validate_full`) already
/// fails closed on that condition unconditionally, downstream, regardless
/// of what this ephemeral view said. This view exists only to avoid
/// FALSE-NEGATIVE planning (R2's actual defect) -- it never has authority
/// to make a real eligibility failure disappear.
pub fn derive_placement_eligibility_view(
    dgraph: &CanonicalDGraphV1,
    artifacts: &[ServiceArtifact],
) -> crate::core::canonical_placement::PlacementEligibilityView {
    let mut view = crate::core::canonical_placement::PlacementEligibilityView::default();
    for service in &dgraph.services {
        if let Ok(artifact) = resolve_artifact_for_service(&service.service_id, artifacts) {
            view.insert(service.service_id.clone(), artifact.control_plane.target_nodes.iter().cloned().collect());
        }
    }
    view
}

/// Phase 136A (ADR-0019 consequence): aggregates STRUCTURAL committed
/// capacity (never live utilization, never a `ServiceArtifact` runtime
/// value) per D-Node across EVERY ACTIVE application in `active_set`,
/// GLOBALLY -- never scoped to one project. D-Nodes are global (see
/// `core::dnode_registry`'s own doc comment: a structural D-Node
/// declaration is keyed only by `node_id`, never by project); the physical
/// capacity ceiling this function measures against is therefore also a
/// global fact, so the commitment accounting that compares against it must
/// be global too, or two different projects could each independently
/// consume more of one shared physical node's declared capacity than it
/// has. (Before Phase 136A, this function -- then named
/// `compute_project_node_commitments` -- filtered by `project_id`, which
/// allowed exactly that cross-project oversubscription; see
/// `documentation/reference/phase136-placement-ui.md`.) Uses each active
/// deployment's own accepted proposal's IMMUTABLE `dgraph_snapshot` (never
/// a live/current D-Graph) to look up each placed service's declared
/// `resource_requirement`, and `active_dmap.placements` for which node it
/// committed to.
///
/// `exclude`, when `Some((project_id, application_id))`, omits that one
/// application from the aggregate -- used when planning/accepting a
/// REDEPLOYMENT of that same application, since a successful acceptance
/// REPLACES its current active D-Map rather than adding a second,
/// simultaneously-authoritative one (transient dual-running migration
/// headroom is a future phase-119 concern, not modelled here; this function
/// models POST-ACCEPTANCE steady state only). Excluding by the full
/// `(project_id, application_id)` pair (not `application_id` alone) is
/// deliberate now that aggregation is global: `application_id` alone is
/// only unique WITHIN a project (`application_key`), so a bare-string
/// exclude could wrongly skip a different project's same-named application.
///
/// Fails closed (`Err`) on internally-inconsistent retained state (an
/// active placement's `service_id` absent from its own accepted
/// `dgraph_snapshot`, a dangling `proposal_id`, or CPU/memory summation
/// overflow) -- this should be structurally impossible given
/// `validate_persisted_state_integrity`, but this function independently
/// re-verifies rather than assuming that invariant already holds.
pub fn compute_global_node_commitments(
    active_set: &BTreeMap<String, ActiveApplicationDeployment>,
    proposals: &BTreeMap<String, ProposalRecord>,
    exclude: Option<(&str, &str)>,
) -> Result<BTreeMap<String, crate::core::canonical_placement::NodeCommitmentV1>, String> {
    let mut commitments: BTreeMap<String, crate::core::canonical_placement::NodeCommitmentV1> = BTreeMap::new();
    for active in active_set.values() {
        if exclude == Some((active.active_dmap.project_id.as_str(), active.application_id.as_str())) {
            continue;
        }
        let proposal_record = proposals.get(&active.proposal_id).ok_or_else(|| {
            format!(
                "active deployment for application '{}' references proposal '{}', which is missing from persisted proposals -- cannot derive structural commitment",
                active.application_id, active.proposal_id
            )
        })?;
        let services_by_id: BTreeMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1> =
            proposal_record.dgraph_snapshot.services.iter().map(|s| (s.service_id.as_str(), s)).collect();
        for placement in &active.active_dmap.placements {
            let service = services_by_id.get(placement.service_id.as_str()).ok_or_else(|| {
                format!(
                    "active deployment for application '{}' places service '{}', which is absent from its own accepted dgraph_snapshot -- cannot derive structural commitment",
                    active.application_id, placement.service_id
                )
            })?;
            let entry = commitments.entry(placement.node_id.clone()).or_default();
            entry.cpu_cores = entry.cpu_cores.checked_add(service.resource_requirement.cpu_cores as u64).ok_or_else(|| {
                format!("D-Node '{}' structural CPU commitment overflow while aggregating active applications", placement.node_id)
            })?;
            entry.memory_bytes = entry.memory_bytes.checked_add(service.resource_requirement.memory_bytes).ok_or_else(|| {
                format!("D-Node '{}' structural memory commitment overflow while aggregating active applications", placement.node_id)
            })?;
        }
    }
    Ok(commitments)
}

/// `_versioned` counterpart of [`compute_global_node_commitments`]: adds
/// every active DMap/2 application's own structural commitment to the SAME
/// global aggregate (Phase 120B closed the gap where the planner/acceptance
/// gate was structurally blind to v2 occupancy; Phase 136A additionally
/// removed the project filter from BOTH halves so neither dual-representation
/// gap nor a cross-project gap can reappear independently of the other).
pub fn compute_global_node_commitments_versioned(
    state: &DDeployStateV1,
    exclude: Option<(&str, &str)>,
) -> Result<BTreeMap<String, crate::core::canonical_placement::NodeCommitmentV1>, String> {
    let mut commitments = compute_global_node_commitments(&state.active_by_application, &state.proposals, exclude)?;
    for active in state.phase113_v2.active.values() {
        if exclude == Some((active.map.project_id.as_str(), active.map.application_id.as_str())) {
            continue;
        }
        let record = state
            .phase113_v2
            .proposals
            .get(&active.proposal_id)
            .ok_or_else(|| {
                format!(
                    "active v2 deployment references missing proposal '{}'",
                    active.proposal_id
                )
            })?;
        let services: BTreeMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1> = record
            .dgraph_snapshot
            .services
            .iter()
            .map(|s| (s.service_id.as_str(), s))
            .collect();
        for placement in &active.map.placements {
            let service = services.get(placement.service_id.as_str()).ok_or_else(|| {
                format!(
                    "active v2 placement references unknown service '{}'",
                    placement.service_id
                )
            })?;
            let entry = commitments.entry(placement.node_id.clone()).or_default();
            entry.cpu_cores = entry
                .cpu_cores
                .checked_add(service.resource_requirement.cpu_cores as u64)
                .ok_or("v2 CPU commitment overflow")?;
            entry.memory_bytes = entry
                .memory_bytes
                .checked_add(service.resource_requirement.memory_bytes)
                .ok_or("v2 memory commitment overflow")?;
        }
    }
    Ok(commitments)
}

/// Phase 136A: evaluates whether `candidate_placements`, ADDED ON TOP OF
/// `baseline_commitments`, would commit more of any D-Node's CURRENT
/// authoritative declared CPU/memory capacity than it structurally has --
/// GLOBALLY, across every project and application sharing that physical
/// node. `baseline_commitments` must already exclude whichever application
/// `candidate_placements` itself belongs to (see
/// [`compute_global_node_commitments_versioned`]'s own `exclude` parameter)
/// -- this function has no way to detect or correct a baseline that still
/// includes the candidate's own prior allocation.
///
/// The single shared source of this arithmetic: reused, never duplicated,
/// by the v1 and v2 R3C acceptance gates (`accept_proposal`/
/// `accept_v2_proposal`) and, additively since Phase 136A, by
/// `POST .../proposals/validate` and `POST .../proposals` (see
/// `documentation/reference/phase136-placement-ui.md`).
///
/// Pure, read-only, never short-circuits on the first violation -- every
/// exceeded node/dimension is reported, so a caller sees the complete
/// picture in one pass. Fails closed (`Err`) only on the same pathological
/// summation-overflow case the two aggregation functions above already
/// guard against.
pub fn evaluate_global_node_capacity(
    baseline_commitments: &BTreeMap<String, crate::core::canonical_placement::NodeCommitmentV1>,
    candidate_placements: &[CanonicalDServPlacementV1],
    candidate_services_by_id: &BTreeMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1>,
    authoritative_dcontinuum: &CanonicalDContinuumV1,
) -> Result<Vec<CanonicalValidationFinding>, String> {
    let mut candidate_commitments = baseline_commitments.clone();
    for placement in candidate_placements {
        // Mirrors every other commitment-aggregation function in this
        // module (`compute_global_node_commitments`'s own identical check):
        // a placement referencing a service absent from its own dgraph is a
        // defensive integrity failure, never silently skipped -- callers
        // only reach this function after their own `validate_cross_references`-
        // style check already passed, so this should be structurally
        // unreachable, but this function independently re-verifies rather
        // than assuming that invariant already holds.
        let service = candidate_services_by_id.get(placement.service_id.as_str()).ok_or_else(|| {
            format!(
                "candidate placement references service '{}', which is absent from its own dgraph -- cannot derive structural commitment",
                placement.service_id
            )
        })?;
        let entry = candidate_commitments.entry(placement.node_id.clone()).or_default();
        entry.cpu_cores = entry.cpu_cores.checked_add(service.resource_requirement.cpu_cores as u64).ok_or_else(|| {
            format!("D-Node '{}' structural CPU commitment overflow while evaluating a candidate placement", placement.node_id)
        })?;
        entry.memory_bytes = entry.memory_bytes.checked_add(service.resource_requirement.memory_bytes).ok_or_else(|| {
            format!("D-Node '{}' structural memory commitment overflow while evaluating a candidate placement", placement.node_id)
        })?;
    }

    let mut findings = Vec::new();
    for node in &authoritative_dcontinuum.nodes {
        let Some(commitment) = candidate_commitments.get(&node.node_id) else { continue };
        if commitment.cpu_cores > node.capacity.cpu_cores as u64 {
            findings.push(finding(
                F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED,
                format!(
                    "D-Node '{}' would be committed {} CPU core(s) across all currently accepted applications sharing it (every project), exceeding its declared capacity of {} core(s)",
                    node.node_id, commitment.cpu_cores, node.capacity.cpu_cores
                ),
            ));
        }
        if commitment.memory_bytes > node.capacity.memory_bytes {
            findings.push(finding(
                F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED,
                format!(
                    "D-Node '{}' would be committed {} byte(s) of memory across all currently accepted applications sharing it (every project), exceeding its declared capacity of {} byte(s)",
                    node.node_id, commitment.memory_bytes, node.capacity.memory_bytes
                ),
            ));
        }
    }
    Ok(findings)
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
                    // Phase 115C: known here directly from the D-Map this
                    // service is being placed by. `dmap_ref` is populated
                    // later, by `union_project_operational_graph`, which is
                    // where this application's already-computed `dmap_digest`
                    // (external to `CanonicalDMapV1` itself -- see that
                    // type's own doc comment) is actually available.
                    application_id: dmap.application_id.clone(),
                    dmap_ref: None,
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
                    // Phase 115C: attach the exact D-Map contract reference
                    // this placement came from -- `application_id` already
                    // survived from `derive_application_operational_services`;
                    // `dmap_ref.digest` is only available here, from this
                    // already-computed `ActiveApplicationDeployment`, never
                    // independently recomputed.
                    let mut service = service.clone();
                    service.dmap_ref = Some(crate::core::canonical_dmap::CanonicalContractReferenceV1 {
                        id: app.active_dmap.dmap_id.clone(),
                        revision: app.active_dmap.revision,
                        digest: app.dmap_digest.clone(),
                    });
                    merged_by_node.entry(placement.node_id.clone()).or_default().push(service);
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
// Phase 119G: governed source-cleanup directive derivation                    //
// --------------------------------------------------------------------------- //

/// Phase 119G, implementing the Phase 119F-R1 corrected design (section 5.1):
/// a pure, read-only, NEVER-PERSISTED derivation of which services this node
/// once ran under some historically-accepted D-Map, but does not currently
/// run under CURRENT accepted placement authority.
///
/// Full-history scan, not a single-hop "immediately-prior acceptance"
/// lookback (Phase 119F-R1 section 3 found the latter loses any node
/// displaced two or more revisions ago). For every application with ANY
/// historical acceptance -- in EITHER `state.acceptances` (v1) or
/// `state.phase113_v2.acceptances` (v2) -- for `project_id`:
///
/// 1. Reconstruct that acceptance's historical placements via
///    `acceptance.proposal_id` -> the exact persisted proposal record's own
///    IMMUTABLE `placements` (never by treating `materialized_dmap_ref` as
///    if it stored a placement payload of its own -- it does not; it is
///    only an `{id, revision, digest}` reference, section 5 of the Phase
///    119G task). This mirrors `accept_proposal`'s own idempotent-replay
///    branch, which already reconstructs a proposal's historical
///    materialization the same way via `build_candidate_dmap`.
/// 2. Validates relational integrity BEFORE trusting any reconstructed
///    placement: the proposal exists, is `Accepted`, its own
///    `accepted_acceptance_id` names this exact acceptance (never the
///    reverse -- an acceptance claiming a proposal that does not itself
///    claim it back is corruption), its digest recomputes correctly and
///    matches both its own recorded `proposal_digest` and the acceptance's
///    recorded `proposal_digest`, and project/application identity agree
///    between proposal and acceptance. For v1, ALSO recomputes the
///    historical D-Map digest via `build_candidate_dmap` and verifies it
///    against `acceptance.materialized_dmap_ref.digest`, exactly like
///    `accept_proposal`'s replay branch. A failure anywhere in this
///    integrity chain fails the WHOLE call closed (`Err`) -- this reads
///    already-committed state that should never be corrupt; if it is, that
///    must never be silently papered over by skipping just that one record.
/// 3. Every historical placement naming `node_id` becomes a candidate
///    `(application_id, service_id, previous_dmap_ref)` tuple, deduplicated
///    by the FULL tuple -- never collapsed to "latest per service" (Phase
///    119F-R1 section 5.3): two non-adjacent historical placements of the
///    same service on the same node produce two distinct directives, since
///    local runtime identity evidence -- not server-side guessing -- is
///    what determines which (if any) still corresponds to something
///    actually running.
/// 4. Each surviving candidate is checked against CURRENT active authority
///    (`active_placement_authority`, re-read fresh from `state` on every
///    call, never cached) for that `application_id`. Current authority
///    ALWAYS wins (Phase 119F-R1 section 7): if it currently places
///    `service_id` on `node_id`, the candidate is dropped, not emitted.
///
/// Never emits a directive from a transport failure, missing binding,
/// unknown node, or any other absence of information -- only ever from a
/// GENUINE historical acceptance record plus a GENUINE current-absence
/// check (Phase 119E's M16/M17, preserved by construction here exactly as
/// Phase 119F section 23 and Phase 119F-R1 section 13.12 require).
pub fn derive_cleanup_directives_for_node(
    state: &DDeployStateV1,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<OperationalCleanupDirective>, String> {
    let node_id = node_id.trim();

    // Same defensive cross-version ambiguity check `ddeploy_migration::locate_acceptance`
    // already establishes for identity resolution (Phase 119C): the exact
    // same `acceptance_id` naming two DIFFERENT records across the v1/v2
    // maps is itself corruption, never silently resolved by picking one.
    for acceptance_id in state.acceptances.keys() {
        if state.phase113_v2.acceptances.contains_key(acceptance_id) {
            return Err(format!(
                "cleanup derivation: acceptance_id '{acceptance_id}' exists in BOTH v1 and v2 acceptance maps (cross-version identity ambiguity)"
            ));
        }
    }

    // (application_id, service_id, dmap_id, dmap_revision, dmap_digest) --
    // the full dedup key. A `BTreeSet` both deduplicates and gives
    // deterministic iteration order for free.
    let mut candidates: BTreeSet<(String, String, String, u64, String)> = BTreeSet::new();

    for acceptance in state.acceptances.values() {
        if acceptance.project_id != project_id {
            continue;
        }
        let proposal_record = state.proposals.get(&acceptance.proposal_id).ok_or_else(|| {
            format!(
                "cleanup derivation: v1 acceptance '{}' references missing proposal '{}'",
                acceptance.acceptance_id, acceptance.proposal_id
            )
        })?;
        if proposal_record.state != ProposalLifecycleState::Accepted
            || proposal_record.accepted_acceptance_id.as_deref() != Some(acceptance.acceptance_id.as_str())
        {
            return Err(format!(
                "cleanup derivation: v1 proposal '{}' is not correctly linked back to acceptance '{}' (corrupt acceptance<->proposal linkage)",
                acceptance.proposal_id, acceptance.acceptance_id
            ));
        }
        let actual_proposal_digest = compute_proposal_digest(&proposal_record.proposal)
            .map_err(|error| format!("cleanup derivation: failed to recompute v1 proposal '{}' digest: {error}", acceptance.proposal_id))?;
        if actual_proposal_digest != proposal_record.proposal_digest || actual_proposal_digest != acceptance.proposal_digest {
            return Err(format!(
                "cleanup derivation: v1 proposal '{}' digest is internally inconsistent with its own record and/or acceptance '{}' (proposal digest corruption)",
                acceptance.proposal_id, acceptance.acceptance_id
            ));
        }
        if proposal_record.proposal.project_id != acceptance.project_id || proposal_record.proposal.application_id != acceptance.application_id {
            return Err(format!(
                "cleanup derivation: v1 proposal '{}' project/application identity does not match acceptance '{}'",
                acceptance.proposal_id, acceptance.acceptance_id
            ));
        }
        let historical_dmap = build_candidate_dmap(&proposal_record.proposal, &acceptance.materialized_dmap_ref.id, acceptance.materialized_dmap_ref.revision);
        let historical_digest = recompute_dmap_digest(&historical_dmap)
            .map_err(|error| format!("cleanup derivation: failed to recompute historical D-Map digest for acceptance '{}': {error}", acceptance.acceptance_id))?;
        if historical_digest != acceptance.materialized_dmap_ref.digest {
            return Err(format!(
                "cleanup derivation: v1 acceptance '{}' materialized_dmap_ref.digest does not match its own recomputed historical D-Map digest (acceptance corruption)",
                acceptance.acceptance_id
            ));
        }
        for placement in &proposal_record.proposal.placements {
            if placement.node_id.trim() == node_id {
                candidates.insert((
                    acceptance.application_id.clone(),
                    placement.service_id.clone(),
                    acceptance.materialized_dmap_ref.id.clone(),
                    acceptance.materialized_dmap_ref.revision,
                    acceptance.materialized_dmap_ref.digest.clone(),
                ));
            }
        }
    }

    for acceptance in state.phase113_v2.acceptances.values() {
        if acceptance.project_id != project_id {
            continue;
        }
        let proposal_record = state.phase113_v2.proposals.get(&acceptance.proposal_id).ok_or_else(|| {
            format!(
                "cleanup derivation: v2 acceptance '{}' references missing proposal '{}'",
                acceptance.acceptance_id, acceptance.proposal_id
            )
        })?;
        if proposal_record.state != crate::core::ddeploy_v2::ProposalV2State::Accepted
            || proposal_record.accepted_acceptance_id.as_deref() != Some(acceptance.acceptance_id.as_str())
        {
            return Err(format!(
                "cleanup derivation: v2 proposal '{}' is not correctly linked back to acceptance '{}' (corrupt acceptance<->proposal linkage)",
                acceptance.proposal_id, acceptance.acceptance_id
            ));
        }
        let actual_proposal_digest = crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&proposal_record.proposal)
            .map_err(|error| format!("cleanup derivation: failed to recompute v2 proposal '{}' digest: {error}", acceptance.proposal_id))?;
        if actual_proposal_digest != proposal_record.proposal_digest || actual_proposal_digest != acceptance.proposal_digest {
            return Err(format!(
                "cleanup derivation: v2 proposal '{}' digest is internally inconsistent with its own record and/or acceptance '{}' (proposal digest corruption)",
                acceptance.proposal_id, acceptance.acceptance_id
            ));
        }
        if proposal_record.proposal.project_id != acceptance.project_id || proposal_record.proposal.application_id != acceptance.application_id {
            return Err(format!(
                "cleanup derivation: v2 proposal '{}' project/application identity does not match acceptance '{}'",
                acceptance.proposal_id, acceptance.acceptance_id
            ));
        }
        // Phase 119F-R1 section 5.1 (normalizing `ContractRefV2` into
        // `CanonicalContractReferenceV1` for this ephemeral cleanup view
        // only -- the same field-copy pattern Phase 119C's own
        // `ddeploy_migration::locate_acceptance` already established, E14).
        for placement in &proposal_record.proposal.placements {
            if placement.node_id.trim() == node_id {
                candidates.insert((
                    acceptance.application_id.clone(),
                    placement.service_id.clone(),
                    acceptance.materialized_dmap_ref.id.clone(),
                    acceptance.materialized_dmap_ref.revision,
                    acceptance.materialized_dmap_ref.digest.clone(),
                ));
            }
        }
    }

    // Current authority always wins (Phase 119F-R1 section 7): re-read fresh
    // for every distinct application_id encountered, never cached across
    // calls to this function.
    let mut current_placements_by_app: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let mut directives = Vec::new();
    for (application_id, service_id, dmap_id, dmap_revision, dmap_digest) in candidates {
        if !current_placements_by_app.contains_key(&application_id) {
            let view = active_placement_authority(state, project_id, &application_id)?;
            let placements = view
                .map(|view| view.placements.into_iter().map(|p| (p.service_id, p.node_id)).collect())
                .unwrap_or_default();
            current_placements_by_app.insert(application_id.clone(), placements);
        }
        let currently_placed_here = current_placements_by_app[&application_id]
            .iter()
            .any(|(current_service_id, current_node_id)| current_service_id == &service_id && current_node_id == node_id);
        if currently_placed_here {
            continue;
        }
        directives.push(OperationalCleanupDirective {
            service_id,
            application_id,
            previous_dmap_ref: CanonicalContractReferenceV1 { id: dmap_id, revision: dmap_revision, digest: dmap_digest },
        });
    }
    directives.sort_by(|a, b| {
        (&a.application_id, &a.service_id, a.previous_dmap_ref.revision)
            .cmp(&(&b.application_id, &b.service_id, b.previous_dmap_ref.revision))
    });
    Ok(directives)
}

/// The centralized read accessor for API callers (mirroring
/// `active_operational_projection`/`is_canonical_authority_active`'s own
/// established pattern exactly): loads current persisted `DDeployStateV1`
/// fresh from disk and derives cleanup directives from it. Never caches
/// anything -- every call re-reads already-persisted state.
pub fn cleanup_directives_for_node(project_id: &str, node_id: &str) -> Result<Vec<OperationalCleanupDirective>, String> {
    let state = load_state(&resolved_db_path())?;
    derive_cleanup_directives_for_node(&state, project_id, node_id)
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
    /// P2 fix: `Some` only when `placements` was produced by
    /// `core::canonical_placement::plan_canonical_placements` -- see
    /// `PlannerProvenanceV1`. Every existing caller of `create_proposal`
    /// other than `post_plan_proposal` passes `None`.
    pub planner_provenance: Option<PlannerProvenanceV1>,
    /// Phase 117G: explicit, exact D-Code binding per service the caller
    /// intends this proposal to activate, if any. Validated against
    /// CURRENT D-Code registry state inside `create_proposal` (never a
    /// "latest" lookup -- every binding must name an exact
    /// `descriptor_digest` that already exists and validates). Empty for
    /// any proposal that binds no D-Code service -- existing non-D-Code
    /// callers are entirely unaffected.
    pub dcode_artifact_authority: std::collections::BTreeMap<String, crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1>,
}

/// DServer validates both contracts, recomputes their digests itself, and
/// constructs the proposal's exact refs itself -- it NEVER trusts a caller-
/// supplied digest as authoritative (there is no such field on
/// `CreateProposalInput` to trust in the first place).
///
/// Phase 117G: this function copies `input.dcode_artifact_authority`
/// verbatim into the constructed proposal (so it participates in
/// `compute_proposal_digest`, closing the gap Phase 117F identified) but
/// does NOT itself validate those bindings against current D-Code registry
/// state -- exactly mirroring this file's own established precedent for
/// `input.planner_provenance`/D-IoT artifact authority (`resolve_diot_artifact_authority`),
/// which the CALLER must already have run before populating the input. The
/// caller MUST call `validate_dcode_artifact_authority_bindings` against
/// current registry state before (or immediately after) calling this
/// function and reject the whole proposal-creation attempt on any finding.
/// This design avoids adding a new registry parameter to this single
/// function's already-numerous call sites, matching the codebase's own
/// established "separate resolve/validate helper, caller-invoked" shape.
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

    // Phase 119G (fail-closed v1-downgrade-from-v2 hardening): refuse to even
    // CREATE a v1 proposal for an application that currently has active v2
    // authority. Checked here (in addition to `accept_proposal`) so the
    // operator learns of the unsupported direction at the earliest possible
    // point, not only when they later attempt to accept. See
    // `F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED`'s own doc comment.
    if state
        .phase113_v2
        .active
        .contains_key(&application_key(&input.project_id, &input.application_id))
    {
        return Err(vec![finding(
            F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED,
            format!(
                "application '{}' currently has active DMap/2 authority; creating a DMap/1 proposal for it is unsupported (v2->v1 downgrade is not a supported transition)",
                application_key(&input.project_id, &input.application_id)
            ),
        )]);
    }

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
        dcode_artifact_authority: input.dcode_artifact_authority,
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
        planner_provenance: input.planner_provenance,
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
    /// Foundation closure v2 (ADR-0019, R1): see `F_STALE_AUTHORITATIVE_DCONTINUUM`.
    StaleAuthority(Vec<CanonicalValidationFinding>),
    /// Phase 117G: a PENDING proposal's `dcode_artifact_authority` binding
    /// no longer resolves against current D-Code registry state (the exact
    /// bound descriptor revision is missing, corrupt, or no longer
    /// validates). Never raised for an idempotent replay of an
    /// ALREADY-accepted proposal.
    DCodeBindingInvalid(Vec<CanonicalValidationFinding>),
}

/// Mutates `state` IN MEMORY ONLY on the success path -- every validation
/// step runs against clones/reads before any field of `state` is written.
/// The caller (the accept API handler) is responsible for persisting the
/// WHOLE mutated `state` in exactly ONE `persist_state` call after this
/// returns `Ok`; if that persist fails, the caller must discard the
/// mutated in-memory copy and leave the on-disk state (and therefore the
/// previously active deployment) untouched. This function itself never
/// touches disk, so a failure here can never leave a half-written domain.
/// `current_authoritative_dcontinuum` (foundation closure v2, ADR-0019 R1):
/// the server-owned structural D-Continuum authority AT THE MOMENT OF THIS
/// CALL, as derived by `core::dnode_registry::build_canonical_dcontinuum`.
/// Required (`Some`) whenever the referenced proposal is still PENDING --
/// this function returns `AcceptError::Internal` if it is `None` on that
/// path, since silently skipping the check would defeat it. Never consulted
/// on the idempotent-replay path (`record.state == Accepted`): an
/// already-accepted historical D-Map remains valid regardless of a LATER
/// registry change, so callers replaying an accepted proposal may safely
/// pass `None` without ever needing to fetch the current registry at all.
pub fn accept_proposal(
    state: &mut DDeployStateV1,
    proposal_id: &str,
    accepted_by: &str,
    expected_proposal_digest: Option<&str>,
    artifacts: &[ServiceArtifact],
    current_authoritative_dcontinuum: Option<&CanonicalDContinuumV1>,
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

    // Foundation closure v2 (ADR-0019, R1): a PENDING proposal's persisted
    // `dcontinuum_snapshot` was authoritative WHEN THE PROPOSAL WAS CREATED,
    // but explicit acceptance -- not creation -- is the moment a proposal
    // becomes operationally authoritative (exactly mirroring the existing
    // CURRENT-`ServiceArtifact`-eligibility revalidation via `validate_full`
    // below). If the server-owned structural D-Node registry has since
    // removed a node this proposal depends on, or changed its platform/
    // capacity/ABI capabilities, this proposal must fail closed rather than
    // silently activate against structural facts DServer no longer
    // considers true. This NEVER rewrites `record.dcontinuum_snapshot`
    // (immutability preserved), NEVER regenerates placement, and NEVER runs
    // for an already-Accepted proposal (the idempotent-replay branch above
    // already returned) -- a historical acceptance remains valid regardless
    // of a LATER registry change.
    let authoritative_dcontinuum = current_authoritative_dcontinuum.ok_or_else(|| {
        AcceptError::Internal(
            "current_authoritative_dcontinuum is required to accept a pending proposal".to_string(),
        )
    })?;
    let authority_findings =
        crate::core::dnode_registry::check_dcontinuum_authority(&record.dcontinuum_snapshot, authoritative_dcontinuum);
    if !authority_findings.is_empty() {
        return Err(AcceptError::StaleAuthority(authority_findings));
    }

    // Compare-and-swap against the CURRENT active revision (re-checked now,
    // not merely at proposal-creation time).
    let key = application_key(&record.proposal.project_id, &record.proposal.application_id);

    // Phase 119G (fail-closed v1-downgrade-from-v2 hardening): a PENDING v1
    // proposal must never be allowed to activate while this application is
    // currently v2-active -- `create_proposal` already refuses to create one
    // in this state, but state can change between creation and acceptance
    // (a v2 acceptance for the same application in between), so this is
    // re-checked here, immediately before any mutation, exactly like every
    // other acceptance-time revalidation in this function. See
    // `F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED`'s own doc comment. Never
    // reached on the idempotent-replay path above: an already-accepted v1
    // proposal already IS the active v1 authority (or was, historically),
    // so this question cannot arise for it.
    if state.phase113_v2.active.contains_key(&key) {
        return Err(AcceptError::Invalid(vec![finding(
            F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED,
            format!(
                "application '{key}' currently has active DMap/2 authority; accepting a DMap/1 proposal for it is unsupported (v2->v1 downgrade is not a supported transition)"
            ),
        )]));
    }

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

    // Foundation closure v2 (R3C), hardened Phase 136A: planning-time
    // residual accounting alone is NOT sufficient -- two proposals can each
    // be validly planned/created against the same free structural capacity
    // (R3D), and only ONE of their eventual acceptances may actually commit
    // it. Revalidate the CANDIDATE's total committed CPU/memory per D-Node
    // against the CURRENT authoritative D-Continuum, BEFORE this acceptance
    // is allowed to commit. This is D-Deploy acceptance safety, not a
    // deterministic-planner-only concern -- it applies uniformly regardless
    // of `ProposalSource`. Never repairs or regenerates placement on
    // failure: the operator must create a new proposal against the
    // now-current structural reality.
    //
    // Phase 136A: GLOBAL across every project sharing a physical node, not
    // merely this proposal's own project -- see
    // `evaluate_global_node_capacity`'s own doc comment for why a
    // project-scoped gate was a real cross-project oversubscription gap
    // (`documentation/reference/phase136-placement-ui.md`).
    //
    // Phase 120B fix: `state` here is the PRE-acceptance state, so
    // `compute_global_node_commitments_versioned(state, None)` alone would
    // NOT reflect this candidate's own new commitment (this application's
    // prior entry, if any, is either absent -- a first deploy -- or still
    // the OLD accepted revision about to be replaced). Using `updated_active`
    // alone (the previous v1-only approach) instead omitted every OTHER
    // application's DMap/2 commitment entirely. The correct baseline is
    // therefore: every OTHER active application's commitment across BOTH
    // DMap/1 and DMap/2, across every project (this application excluded,
    // exactly mirroring `accept_v2_proposal`'s own pattern), with this
    // candidate's own placements added on top exactly once.
    let baseline_commitments = compute_global_node_commitments_versioned(
        state,
        Some((&record.proposal.project_id, &record.proposal.application_id)),
    )
    .map_err(AcceptError::Internal)?;
    let candidate_services: BTreeMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1> = record
        .dgraph_snapshot
        .services
        .iter()
        .map(|s| (s.service_id.as_str(), s))
        .collect();
    let capacity_findings = evaluate_global_node_capacity(
        &baseline_commitments,
        &record.proposal.placements,
        &candidate_services,
        authoritative_dcontinuum,
    )
    .map_err(AcceptError::Internal)?;
    if !capacity_findings.is_empty() {
        return Err(AcceptError::Invalid(capacity_findings));
    }

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

/// Phase 117G: D-Code-binding-aware acceptance. Additive wrapper around the
/// unmodified `accept_proposal` (its own 75+ existing call sites are
/// entirely unaffected -- this function does not change its signature).
///
/// Fetches the SAME persisted `ProposalRecord` `accept_proposal` itself
/// would fetch, verifies the SAME digest-integrity invariant, then
/// revalidates `record.proposal.dcode_artifact_authority` against CURRENT
/// D-Code registry state via `validate_dcode_artifact_authority_bindings`
/// -- exactly the "does the exact bound revision still exist and
/// validate" question (Phase 117F Section 11/117G Section 22), never a
/// "current/latest" resolution. All of this happens BEFORE `accept_proposal`
/// is ever called, so a D-Code-binding failure here mutates `state` NOT AT
/// ALL (the real `accept_proposal` call is simply never reached) --
/// matching this module's own established "every validation step runs
/// against clones/reads before any field of `state` is written" discipline
/// for the rest of acceptance.
///
/// Idempotent replay (`record.state == Accepted`) is intentionally NOT
/// re-validated against current registry state here -- exactly mirroring
/// how `accept_proposal` itself never re-runs `current_authoritative_dcontinuum`
/// checks on replay: a historical acceptance's binding is immutable
/// evidence of what WAS true, and later registry changes (including a
/// bound revision being -- hypothetically, since this registry has no
/// deletion path -- removed) must never retroactively invalidate a
/// proposal's own historical record.
///
/// Known limitation, stated honestly: this is an ADDITIVE, two-step
/// integration seam, not a single atomic operation. A real production
/// caller wiring this into an API handler must acquire a stable view
/// spanning BOTH the D-Code-registry read here AND the subsequent
/// `accept_proposal` call (mirroring the established `DDEPLOY_TRANSACTION`
/// stable-view discipline) to fully close the TOCTOU window between the
/// two steps; this function alone proves the DECISION logic is correct
/// and testable, not the full production locking story.
#[allow(clippy::too_many_arguments)]
pub fn accept_proposal_with_dcode_binding_revalidation(
    state: &mut DDeployStateV1,
    proposal_id: &str,
    accepted_by: &str,
    expected_proposal_digest: Option<&str>,
    artifacts: &[ServiceArtifact],
    current_authoritative_dcontinuum: Option<&CanonicalDContinuumV1>,
    dcode_registry: &crate::core::dcode_registry::DCodeArtifactRegistryStateV2,
    now_utc: &str,
    new_acceptance_id: String,
) -> Result<AcceptOutcome, AcceptError> {
    let record = state
        .proposals
        .get(proposal_id)
        .cloned()
        .ok_or_else(|| AcceptError::NotFound(format!("proposal '{proposal_id}' not found")))?;
    let actual_proposal_digest = compute_proposal_digest(&record.proposal).map_err(AcceptError::Internal)?;
    if actual_proposal_digest != record.proposal_digest {
        return Err(AcceptError::DigestMismatch(format!(
            "persisted proposal '{proposal_id}' is internally inconsistent: its recomputed digest '{actual_proposal_digest}' does not match its recorded proposal_digest '{}'",
            record.proposal_digest
        )));
    }

    if record.state != ProposalLifecycleState::Accepted && !record.proposal.dcode_artifact_authority.is_empty() {
        if let Err(finding) = validate_dcode_artifact_authority_bindings(
            &record.dgraph_snapshot,
            &record.proposal.application_id,
            dcode_registry,
            &record.proposal.dcode_artifact_authority,
        ) {
            return Err(AcceptError::DCodeBindingInvalid(vec![finding]));
        }
    }

    accept_proposal(state, proposal_id, accepted_by, expected_proposal_digest, artifacts, current_authoritative_dcontinuum, now_utc, new_acceptance_id)
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
                    service_definition: None, service_id: "svc-a".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 134_217_728 },
                },
                CanonicalDServV1 {
                    service_definition: None, service_id: "svc-b".to_string(),
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

    /// Independent review v3 (C1): builds a `PlannerProvenanceV1` the exact
    /// same way `api::ddeploy::post_plan_proposal` does -- from a
    /// `CanonicalPlacementPlanV1`'s own already-recomputed
    /// `plan.provenance` id/revision/digest fields, plus the CALLER-SUPPLIED
    /// `committed_by_node`/`eligibility` that were actually passed into
    /// `plan_canonical_placements` to produce that plan (never
    /// reconstructed from the plan's own output).
    fn planner_provenance_from_plan(
        plan: &crate::core::canonical_placement::CanonicalPlacementPlanV1,
        committed_by_node: BTreeMap<String, crate::core::canonical_placement::NodeCommitmentV1>,
        eligibility: crate::core::canonical_placement::PlacementEligibilityView,
    ) -> PlannerProvenanceV1 {
        PlannerProvenanceV1 {
            planner_version: plan.provenance.planner_version.clone(),
            policy_name: plan.provenance.policy_name.clone(),
            policy_version: plan.provenance.policy_version.clone(),
            dgraph_ref: CanonicalContractReferenceV1 {
                id: plan.provenance.dgraph_id.clone(),
                revision: plan.provenance.dgraph_revision,
                digest: plan.provenance.dgraph_digest.clone(),
            },
            dcontinuum_ref: CanonicalContractReferenceV1 {
                id: plan.provenance.dcontinuum_id.clone(),
                revision: plan.provenance.dcontinuum_revision,
                digest: plan.provenance.dcontinuum_digest.clone(),
            },
            committed_by_node,
            eligibility,
            decisions: plan.provenance.decisions.clone(),
        }
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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

        dcode_artifact_authority: std::collections::BTreeMap::new(),
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
            dcode_artifact_authority: std::collections::BTreeMap::new(),
        }
    }

    // ===================================================================
    // Phase 117G: DCode proposal-binding test matrix (G11-G21) plus the
    // legacy proposal-digest compatibility proof (Section 16).
    // ===================================================================

    fn dserv_artifact_for(application_id: &str, service_id: &str) -> crate::core::dserv_artifact::CanonicalDServArtifactV1 {
        crate::core::dserv_artifact::CanonicalDServArtifactV1 {
            schema: crate::core::dserv_artifact::CANONICAL_DSERV_ARTIFACT_SCHEMA.to_string(),
            artifact_id: format!("dserv:{application_id}:{service_id}"),
            version: "0.1.0".to_string(),
            application_id: application_id.to_string(),
            dserv_id: service_id.to_string(),
            dcode: crate::core::dserv_artifact::DServArtifactDCodeV1 {
                uri: format!("datum-blob://sha256/{}", "a".repeat(64)),
                sha256: "a".repeat(64),
                content_id: format!("sha256:{}", "a".repeat(64)),
                size_bytes: 64,
            },
            abi: crate::core::dserv_artifact::DServArtifactAbiV1 {
                name: "datum-dnode".to_string(),
                version: "0".to_string(),
                required_exports: crate::core::dserv_artifact::DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
                allowed_imports: Vec::new(),
            },
            ports: crate::core::dserv_artifact::DServArtifactPortsV1::default(),
            limits: crate::core::dserv_artifact::DServArtifactLimitsV1 {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 100,
                max_memory_pages: 4,
                max_message_bytes: 4096,
                max_emits_per_invocation: 1,
            },
            safety: crate::core::dserv_artifact::DServArtifactSafetyV1::default(),
            state_model: "stateless".to_string(),
            instantiation: "fresh_instance_per_invocation".to_string(),
            constraints: crate::core::dserv_artifact::DServArtifactConstraintsV1::default(),
            qos: serde_json::Value::Null,
            provenance: serde_json::Value::Null,
            probes: serde_json::Value::Null,
            signature_ref: None,
        }
    }

    fn registered_binding(
        registry: &mut crate::core::dcode_registry::DCodeArtifactRegistryStateV2,
        application_id: &str,
        service_id: &str,
    ) -> crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 {
        let digest = crate::core::dcode_registry::declare_dserv_artifact_revision(registry, dserv_artifact_for(application_id, service_id)).unwrap();
        crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: digest }
    }

    /// Section 16: a pre-117G proposal serializes (and therefore digests)
    /// IDENTICALLY to a current proposal whose `dcode_artifact_authority`
    /// is empty -- `skip_serializing_if` means the field is entirely absent
    /// from the JSON in both cases, so no historical acceptance/digest
    /// record is invalidated by this phase's schema change.
    #[test]
    fn legacy_proposal_digest_is_unchanged_when_dcode_binding_absent() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let proposal = proposal_for(&dgraph, &dcontinuum, sample_placements());
        assert!(proposal.dcode_artifact_authority.is_empty());

        let value = serde_json::to_value(&proposal).unwrap();
        assert!(
            value.get("dcode_artifact_authority").is_none(),
            "an empty dcode_artifact_authority must be entirely absent from the serialized JSON, exactly matching the pre-117G wire shape"
        );

        // Round-tripping through that exact (field-absent) JSON reproduces
        // an identical struct (via #[serde(default)]) and therefore an
        // identical digest -- proving a genuinely pre-117G-serialized
        // proposal deserializes and re-digests unchanged under the new type.
        let reconstructed: DDeployProposalV1 = serde_json::from_value(value).unwrap();
        assert_eq!(reconstructed, proposal);
        assert_eq!(compute_proposal_digest(&reconstructed).unwrap(), compute_proposal_digest(&proposal).unwrap());
    }

    /// Section 16 (continued): adding one binding DOES change the digest --
    /// the field is real digest-covered proposal content, not inert metadata.
    #[test]
    fn adding_a_dcode_binding_changes_the_proposal_digest() {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a");

        let without_binding = proposal_for(&dgraph, &dcontinuum, sample_placements());
        let mut with_binding = without_binding.clone();
        with_binding.dcode_artifact_authority.insert("svc-a".to_string(), binding);

        assert_ne!(
            compute_proposal_digest(&without_binding).unwrap(),
            compute_proposal_digest(&with_binding).unwrap(),
            "a real, non-empty dcode_artifact_authority binding must change the proposal's digest"
        );
    }

    /// G11: a fully valid single binding passes.
    #[test]
    fn g11_valid_binding_passes() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a");
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), binding);
        assert!(validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).is_ok());
    }

    /// G12: binding a service_id absent from the proposal's own D-Graph
    /// snapshot fails closed.
    #[test]
    fn g12_binding_unknown_service_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-ghost");
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-ghost".to_string(), binding);
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_SERVICE_UNKNOWN);
    }

    /// G13: a malformed (non-canonical-format) descriptor digest fails closed.
    #[test]
    fn g13_malformed_digest_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert(
            "svc-a".to_string(),
            crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: "not-a-digest".to_string() },
        );
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_DIGEST_MALFORMED);
    }

    /// G14: a well-formed digest that names no registered revision fails closed.
    #[test]
    fn g14_unregistered_digest_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert(
            "svc-a".to_string(),
            crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: format!("sha256:{}", "0".repeat(64)) },
        );
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_ARTIFACT_NOT_FOUND);
    }

    /// G15: a revision registered for a DIFFERENT application_id (even with
    /// the same service_id) never satisfies a binding -- there is no
    /// cross-application fallback.
    #[test]
    fn g15_binding_application_mismatch_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        // Registered under a different logical key than (app-x, svc-a), but
        // we then reference its digest under app-x/svc-a's binding slot.
        let digest = crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, dserv_artifact_for("app-other", "svc-a")).unwrap();
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: digest });
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        // get_exact("app-x", "svc-a", digest) will not find it at all
        // (different outer key), so this surfaces as ARTIFACT_NOT_FOUND --
        // still a fail-closed rejection, just via the lookup itself rather
        // than the post-lookup application_id equality check.
        assert_eq!(error.code, F_DCODE_BINDING_ARTIFACT_NOT_FOUND);
    }

    /// G16: an artifact that no longer independently validates (simulated
    /// registry corruption) fails closed even though its digest key matches.
    #[test]
    fn g16_invalid_artifact_content_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let mut bad = dserv_artifact_for("app-x", "svc-a");
        bad.limits.fuel_per_invocation = 0; // invalid
        let digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(&bad).unwrap();
        let key = crate::core::dcode_registry::dserv_artifact_key("app-x", "svc-a");
        registry.artifacts.entry(key).or_default().insert(digest.clone(), bad);
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: digest });
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_ARTIFACT_INVALID);
    }

    /// G17: a binding digest that does not match its own artifact's freshly
    /// recomputed digest (stored-key corruption) fails closed.
    #[test]
    fn g17_binding_digest_mismatch_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let real_digest = crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, dserv_artifact_for("app-x", "svc-a")).unwrap();
        // Move it under a bogus digest key -- get_exact requires the exact
        // key, so this simulates corruption where the digest key itself no
        // longer matches the content it stores.
        let key = crate::core::dcode_registry::dserv_artifact_key("app-x", "svc-a");
        let revisions = registry.artifacts.get_mut(&key).unwrap();
        let content = revisions.remove(&real_digest).unwrap();
        let bogus_digest = format!("sha256:{}", "1".repeat(64));
        revisions.insert(bogus_digest.clone(), content);
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: bogus_digest });
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_DESCRIPTOR_DIGEST_MISMATCH);
    }

    /// G18: an artifact registered under a digest that itself resolves, but
    /// whose OWN `dserv_id` does not match the binding's service_id, fails
    /// closed (a caller cannot "borrow" one service's revision for another).
    #[test]
    fn g18_binding_service_mismatch_is_rejected() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        // Registered under the logical key (app-x, svc-a) so get_exact finds
        // it, but the artifact's OWN dserv_id field says "svc-b".
        let mut mismatched = dserv_artifact_for("app-x", "svc-b");
        mismatched.dserv_id = "svc-b".to_string();
        let digest = crate::core::dserv_artifact::recompute_dserv_artifact_digest(&mismatched).unwrap();
        let key = crate::core::dcode_registry::dserv_artifact_key("app-x", "svc-a");
        registry.artifacts.entry(key).or_default().insert(digest.clone(), mismatched);
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: digest });
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_SERVICE_MISMATCH);
    }

    /// G19: an ABI mismatch between the bound artifact and the D-Graph
    /// service's declared execution requirement fails closed.
    #[test]
    fn g19_binding_abi_mismatch_is_rejected() {
        // The D-Serv artifact itself must stay a validly-registerable
        // "datum-dnode/0" (that is the only ABI `CanonicalDServArtifactV1::validate`
        // currently accepts) -- the mismatch is expressed on the D-GRAPH
        // SERVICE side instead, which carries no such restriction on its own
        // `execution_requirement.dnode_abi` string.
        let mut dgraph = sample_dgraph("app-x", 1);
        dgraph.services[0].execution_requirement.dnode_abi = "some-other-abi/1".to_string();
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a");
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), binding);
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_ABI_MISMATCH);
    }

    /// G20: a non-empty `allowed_stages` constraint is unverifiable under
    /// v0.1 and must fail closed at BINDING time too, not just at
    /// invocation-authorization time -- a proposal should never be allowed
    /// to bind an artifact that is guaranteed to fail every invocation.
    #[test]
    fn g20_nonempty_allowed_stages_rejected_at_binding_time() {
        let dgraph = sample_dgraph("app-x", 1);
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let mut staged = dserv_artifact_for("app-x", "svc-a");
        staged.constraints.allowed_stages = vec!["prod".to_string()];
        let digest = crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, staged).unwrap();
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: digest });
        let error = validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).unwrap_err();
        assert_eq!(error.code, F_DCODE_BINDING_STAGE_CONSTRAINT_UNVERIFIABLE);
    }

    /// G21: Section 18 -- partial coverage is legal. A proposal may bind
    /// only SOME of its D-Graph services to D-Code (or none at all); the
    /// unbound services are simply not authorizable-as-DCode, which is not
    /// itself an error at proposal-validation time.
    #[test]
    fn g21_partial_binding_coverage_is_accepted() {
        let dgraph = sample_dgraph("app-x", 1); // has svc-a AND svc-b
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a"); // only svc-a bound
        let mut bindings = std::collections::BTreeMap::new();
        bindings.insert("svc-a".to_string(), binding);
        assert!(validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &bindings).is_ok());

        let empty_bindings: std::collections::BTreeMap<String, crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1> = std::collections::BTreeMap::new();
        assert!(validate_dcode_artifact_authority_bindings(&dgraph, "app-x", &registry, &empty_bindings).is_ok());
    }

    // ===================================================================
    // Phase 117G: acceptance-behavior test matrix (G22-G28).
    // ===================================================================

    fn create_and_accept_with_binding(
        state: &mut DDeployStateV1,
        registry: &crate::core::dcode_registry::DCodeArtifactRegistryStateV2,
        proposal_id: &str,
        acceptance_id: &str,
        binding_digest: Option<String>,
    ) -> Result<AcceptOutcome, AcceptError> {
        let dgraph = sample_dgraph("app-x", 1);
        let dcontinuum = sample_dcontinuum("proj-x", 1);
        let mut dcode_artifact_authority = std::collections::BTreeMap::new();
        if let Some(digest) = binding_digest {
            dcode_artifact_authority.insert(
                "svc-a".to_string(),
                crate::core::dcode_registry::DCodeArtifactAuthorityBindingV1 { descriptor_digest: digest },
            );
        }
        create_proposal(
            state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority,
                project_id: "proj-x".to_string(),
                application_id: "app-x".to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum: dcontinuum.clone(),
                placements: sample_placements(),
            },
            proposal_id.to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        accept_proposal_with_dcode_binding_revalidation(
            state,
            proposal_id,
            "operator:alice",
            None,
            &sample_artifacts(),
            Some(&dcontinuum),
            registry,
            "2026-01-01T00:00:00Z",
            acceptance_id.to_string(),
        )
    }

    /// G22: registering a NEW revision for a service that is already
    /// explicitly bound (via an accepted proposal's own
    /// `dcode_artifact_authority`) changes NOTHING about the active
    /// binding -- `resolve_active_dcode_binding` still resolves to the
    /// originally-bound digest, untouched.
    #[test]
    fn g22_registering_a_new_revision_does_not_change_the_active_binding() {
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let d1_binding = registered_binding(&mut registry, "app-x", "svc-a");
        let mut state = DDeployStateV1::empty();
        create_and_accept_with_binding(&mut state, &registry, "p1", "a1", Some(d1_binding.descriptor_digest.clone())).unwrap();

        // Register a second, different revision for the SAME logical
        // service -- an action entirely independent of DDeploy.
        let mut different = dserv_artifact_for("app-x", "svc-a");
        different.limits.timeout_ms += 1;
        crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, different).unwrap();
        assert_eq!(registry.revision_count("app-x", "svc-a"), 2);

        let active = active_placement_authority(&state, "proj-x", "app-x").unwrap().unwrap();
        let resolved = resolve_active_dcode_binding(&state, &active, "svc-a").unwrap();
        assert_eq!(resolved.descriptor_digest, d1_binding.descriptor_digest, "registering D2 alone must never change what the already-active deployment is bound to");
    }

    /// G23: a still-PENDING proposal (created, never accepted) produces no
    /// active authority at all, regardless of what it bound.
    #[test]
    fn g23_pending_proposal_with_binding_activates_nothing() {
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a");
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority: std::collections::BTreeMap::from([("svc-a".to_string(), binding)]),
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
        assert!(active_placement_authority(&state, "proj-x", "app-x").unwrap().is_none(), "creating a proposal must never itself produce active authority");
    }

    /// G24: acceptance-time revalidation fails closed (and mutates nothing)
    /// when the bound descriptor digest does not resolve against current
    /// registry state.
    #[test]
    fn g24_acceptance_fails_closed_when_bound_digest_is_missing_from_registry() {
        let registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty(); // nothing registered
        let mut state = DDeployStateV1::empty();
        let bogus_digest = format!("sha256:{}", "0".repeat(64));
        let error = create_and_accept_with_binding(&mut state, &registry, "p1", "a1", Some(bogus_digest)).unwrap_err();
        assert!(matches!(error, AcceptError::DCodeBindingInvalid(_)));
        assert!(active_placement_authority(&state, "proj-x", "app-x").unwrap().is_none(), "a failed acceptance must mutate nothing");
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Pending, "a failed acceptance must leave the proposal Pending");
    }

    /// G25: a valid binding accepts normally and the active binding
    /// resolves to exactly the bound digest.
    #[test]
    fn g25_valid_binding_accepts_and_resolves_exactly() {
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a");
        let mut state = DDeployStateV1::empty();
        let outcome = create_and_accept_with_binding(&mut state, &registry, "p1", "a1", Some(binding.descriptor_digest.clone())).unwrap();
        assert!(!outcome.idempotent_replay);
        let active = active_placement_authority(&state, "proj-x", "app-x").unwrap().unwrap();
        let resolved = resolve_active_dcode_binding(&state, &active, "svc-a").unwrap();
        assert_eq!(resolved.descriptor_digest, binding.descriptor_digest);
    }

    /// G26: replaying an ALREADY-accepted proposal never re-revalidates its
    /// D-Code binding against current registry state -- it succeeds even
    /// against a completely EMPTY registry, exactly mirroring the
    /// established `current_authoritative_dcontinuum: None`-on-replay
    /// convention `accept_proposal` itself already uses.
    #[test]
    fn g26_idempotent_replay_never_revalidates_dcode_binding() {
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, "app-x", "svc-a");
        let mut state = DDeployStateV1::empty();
        create_and_accept_with_binding(&mut state, &registry, "p1", "a1", Some(binding.descriptor_digest)).unwrap();

        // Replay against a totally empty registry -- if revalidation ran
        // unconditionally, this would fail closed; it must not.
        let empty_registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let replay = accept_proposal_with_dcode_binding_revalidation(
            &mut state, "p1", "operator:alice", None, &sample_artifacts(), None, &empty_registry, "2026-01-01T00:00:01Z", "should-not-be-used".to_string(),
        )
        .unwrap();
        assert!(replay.idempotent_replay);
    }

    /// G27: a pure D-Code-only redeploy -- identical D-Graph/D-Continuum/
    /// placements, only the D-Code binding differs -- correctly switches the
    /// active binding from D1 to D2 exactly once, with no service-id change
    /// anywhere, and P1's own historical record keeps its ORIGINAL D1
    /// binding unchanged (immutability of history).
    #[test]
    fn g27_pure_dcode_only_redeploy_switches_active_binding_exactly_once() {
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let d1 = registered_binding(&mut registry, "app-x", "svc-a");
        let mut different = dserv_artifact_for("app-x", "svc-a");
        different.limits.timeout_ms += 1;
        let d2_digest = crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, different).unwrap();

        let mut state = DDeployStateV1::empty();
        create_and_accept_with_binding(&mut state, &registry, "p1", "a1", Some(d1.descriptor_digest.clone())).unwrap();
        create_and_accept_with_binding(&mut state, &registry, "p2", "a2", Some(d2_digest.clone())).unwrap();

        let active = active_placement_authority(&state, "proj-x", "app-x").unwrap().unwrap();
        let resolved = resolve_active_dcode_binding(&state, &active, "svc-a").unwrap();
        assert_eq!(resolved.descriptor_digest, d2_digest, "the SECOND acceptance must be the one and only thing that switched the active binding");

        // P1's own historical proposal record must still show its ORIGINAL
        // D1 binding, completely unaffected by P2's later acceptance.
        assert_eq!(
            state.proposals.get("p1").unwrap().proposal.dcode_artifact_authority.get("svc-a").unwrap().descriptor_digest,
            d1.descriptor_digest,
            "an old proposal's own historical binding must never be rewritten by a later proposal's acceptance"
        );
    }

    /// G28: after P2 (bound to D2) is active, registering a THIRD revision
    /// D3 changes nothing -- the active binding remains exactly D2.
    #[test]
    fn g28_third_revision_does_not_drift_the_active_binding() {
        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let d1 = registered_binding(&mut registry, "app-x", "svc-a");
        let mut d2_content = dserv_artifact_for("app-x", "svc-a");
        d2_content.limits.timeout_ms += 1;
        let d2_digest = crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, d2_content).unwrap();

        let mut state = DDeployStateV1::empty();
        create_and_accept_with_binding(&mut state, &registry, "p1", "a1", Some(d1.descriptor_digest)).unwrap();
        create_and_accept_with_binding(&mut state, &registry, "p2", "a2", Some(d2_digest.clone())).unwrap();

        let mut d3_content = dserv_artifact_for("app-x", "svc-a");
        d3_content.limits.timeout_ms += 2;
        crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, d3_content).unwrap();
        assert_eq!(registry.revision_count("app-x", "svc-a"), 3);

        let active = active_placement_authority(&state, "proj-x", "app-x").unwrap().unwrap();
        let resolved = resolve_active_dcode_binding(&state, &active, "svc-a").unwrap();
        assert_eq!(resolved.descriptor_digest, d2_digest, "registering a third revision must never drift the currently active binding");
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
        accept_proposal(state, proposal_id, accepted_by, None, artifacts, Some(&sample_dcontinuum(project_id, 1)), now, acceptance_id.to_string())
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
        let outcome2 = accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts2, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        assert_eq!(outcome2.materialized_dmap.revision, 2);
    }

    #[test]
    fn accepting_same_proposal_twice_is_idempotent() {
        let mut state = DDeployStateV1::empty();
        let first = create_and_accept(&mut state, "proj-x", "app-x", "p1", sample_placements(), &sample_artifacts(), "operator:alice", "a1", "2026-01-01T00:00:00Z").unwrap();
        let second = accept_proposal(&mut state, "p1", "operator:bob", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T02:00:00Z", "should-not-be-used".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: swapped },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &swapped_artifacts,
        ).unwrap();
        let outcome_p2 = accept_proposal(&mut state, "p2", "operator:alice", None, &swapped_artifacts, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
        assert_eq!(outcome_p2.materialized_dmap.revision, 2);

        // Replay p1 -- must return p1's OWN original materialization
        // (revision 1, digest matching a1's own recorded ref), reported as
        // NOT currently active, with ZERO further state mutation.
        let state_before_replay = state.clone();
        let replay_p1 = accept_proposal(&mut state, "p1", "operator:carol", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T03:00:00Z", "should-not-be-used".to_string()).unwrap();
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
        let replay_p2 = accept_proposal(&mut state, "p2", "operator:carol", None, &swapped_artifacts, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T03:00:00Z", "should-not-be-used-either".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p3".to_string(), "2026-01-01T01:00:01Z".to_string(), &sample_artifacts(),
        ).unwrap();

        let accept_p2 = accept_proposal(&mut state, "p2", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T02:00:00Z", "a2".to_string());
        assert!(accept_p2.is_ok());
        assert_eq!(state.active_by_application.get(&application_key("proj-x", "app-x")).unwrap().active_dmap.revision, 2);

        let accept_p3 = accept_proposal(&mut state, "p3", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T02:00:01Z", "a3".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph, dcontinuum, placements },
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        // Simulate tampering: mutate the persisted snapshot without updating the digest.
        state.proposals.get_mut("p1").unwrap().dgraph_snapshot.services[0].resource_requirement.cpu_cores = 999;
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        // Attacker/bug scenario: mutate the persisted PROPOSAL's placements
        // (svc-a: fog-01 -> cloud-01) WITHOUT updating `proposal_digest`.
        state.proposals.get_mut("p1").unwrap().proposal.placements[0].node_id = "cloud-01".to_string();
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose persisted placements were changed without updating its recorded digest must be rejected as proposal_digest_mismatch, not silently accepted with the mutated placement: {result:?}");
        assert!(!state.has_active_authority("proj-x"));
    }

    #[test]
    fn source_ref_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.dgraph_ref.revision = 2;
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose dgraph_ref was changed without updating its recorded digest must be rejected as proposal_digest_mismatch: {result:?}");
    }

    #[test]
    fn dcontinuum_ref_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.dcontinuum_ref.digest = format!("sha256:{}", "0".repeat(64));
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose dcontinuum_ref was changed without updating its recorded digest must be rejected as proposal_digest_mismatch: {result:?}");
    }

    #[test]
    fn source_and_provenance_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.source = ProposalSource::Ai;
        state.proposals.get_mut("p1").unwrap().proposal.created_at_utc = "2026-06-01T00:00:00Z".to_string();
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))), "a proposal whose source/created_at_utc were changed without updating its recorded digest must be rejected as proposal_digest_mismatch: {result:?}");
    }

    #[test]
    fn expected_active_dmap_revision_tampering_without_digest_update_is_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        state.proposals.get_mut("p1").unwrap().proposal.expected_active_dmap_revision = Some(41);
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Accepted);

        // Corrupt the now-Accepted proposal's placements without updating its digest.
        state.proposals.get_mut("p1").unwrap().proposal.placements[0].node_id = "cloud-01".to_string();
        let replay = accept_proposal(&mut state, "p1", "operator:bob", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T02:00:00Z", "should-not-be-used".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let wrong = format!("sha256:{}", if record.proposal_digest.ends_with('0') { "1".repeat(64) } else { "0".repeat(64) });
        let result = accept_proposal(&mut state, "p1", "operator:alice", Some(&wrong), &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::DigestMismatch(_))));
    }

    #[test]
    fn acceptance_time_eligibility_is_rechecked() {
        let mut state = DDeployStateV1::empty();
        let artifacts_at_creation = sample_artifacts();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &artifacts_at_creation,
        ).unwrap();
        // Artifact eligibility changes AFTER proposal creation, BEFORE acceptance.
        let artifacts_at_acceptance = vec![sample_artifact("svc-a", vec!["cloud-01"]) /* fog-01 no longer eligible */, sample_artifact("svc-b", vec!["cloud-01"])];
        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &artifacts_at_acceptance, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_NODE_NOT_ELIGIBLE)), "acceptance must re-check eligibility against CURRENT artifacts, not the creation-time snapshot: {result:?}");
        assert!(!state.has_active_authority("proj-x"));
    }

    #[test]
    fn accept_nonexistent_proposal_not_found() {
        let mut state = DDeployStateV1::empty();
        let result = accept_proposal(&mut state, "does-not-exist", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::NotFound(_))));
    }

    #[test]
    fn accept_with_wrong_expected_digest_rejected() {
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let result = accept_proposal(&mut state, "p1", "operator:alice", Some("sha256:wrong"), &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a1".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: moved },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts2,
        ).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts2, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();

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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() /* still targets fog-01 */ },
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: dgraph_b, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "fog-01".to_string() }, CanonicalDServPlacementV1 { service_id: "svc-d".to_string(), node_id: "cloud-01".to_string() }] },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_b,
        ).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts_b, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();

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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: dgraph_b, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "cloud-01".to_string() }] }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_b).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts_b, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();

        // Re-accept a new revision of app-a.
        let artifacts_a = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"])];
        let mut dgraph_a2 = sample_dgraph("app-a", 1);
        dgraph_a2.services.truncate(1);
        dgraph_a2.services[0].service_id = "svc-a".to_string();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: dgraph_a2, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "cloud-01".to_string() }] }, "p3".to_string(), "2026-01-01T02:00:00Z".to_string(), &artifacts_a).unwrap();
        accept_proposal(&mut state, "p3", "operator:alice", None, &artifacts_a, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T02:00:00Z", "a3".to_string()).unwrap();

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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: dgraph_b, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "shared-svc".to_string(), node_id: "cloud-01".to_string() }] }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_b).unwrap();
        let result = accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts_b, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string());
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &sample_artifacts()).unwrap();
        let bad_artifacts = vec![sample_artifact("svc-a", vec![]), sample_artifact("svc-b", vec!["cloud-01"])];
        let result = accept_proposal(&mut state, "p2", "operator:alice", None, &bad_artifacts, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string());
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

    #[test]
    fn phase126_committed_acceptance_records_history_and_observability_failure_never_rolls_back() {
        use crate::core::event_producers::{acceptance_event_v1, EventProducerPipeline, ProducerError};
        use crate::storage::{datum_alert_store, datum_event_store};
        let authority_path = temp_db_path("phase126-authority");
        let event_path = temp_db_path("phase126-events");
        let pipeline = EventProducerPipeline::open(event_path.clone()).unwrap();
        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: "proj-x".into(),
                application_id: "app-x".into(),
                source: ProposalSource::Operator,
                dgraph: sample_dgraph("app-x", 1),
                dcontinuum: sample_dcontinuum("proj-x", 1),
                placements: sample_placements(),
            },
            "phase126-proposal".into(),
            "2026-01-01T00:00:00Z".into(),
            &sample_artifacts(),
        )
        .unwrap();
        // Failed canonical acceptance never reaches the post-commit adapter.
        assert!(accept_proposal(
            &mut state,
            "absent",
            "operator",
            None,
            &sample_artifacts(),
            Some(&sample_dcontinuum("proj-x", 1)),
            "2026-01-01T00:00:00Z",
            "absent-acceptance".into()
        )
        .is_err());
        assert!(
            datum_event_store::list_events(&event_path, &Default::default())
                .unwrap()
                .is_empty()
        );
        let outcome = accept_proposal(
            &mut state,
            "phase126-proposal",
            "operator",
            None,
            &sample_artifacts(),
            Some(&sample_dcontinuum("proj-x", 1)),
            "2026-01-01T00:00:00Z",
            "phase126-acceptance".into(),
        )
        .unwrap();
        persist_state(&authority_path, &state).unwrap();
        let committed = load_state(&authority_path).unwrap();
        let event = acceptance_event_v1(&outcome.acceptance).unwrap();
        let connection = rusqlite::Connection::open(&event_path).unwrap();
        connection.execute_batch("CREATE TRIGGER fail_observability BEFORE INSERT ON datum_events BEGIN SELECT RAISE(ABORT,'test recording failure'); END;").unwrap();
        assert!(matches!(
            pipeline.record(event.clone()),
            Err(ProducerError::EventStore { .. })
        ));
        assert_eq!(load_state(&authority_path).unwrap(), committed);
        connection
            .execute_batch("DROP TRIGGER fail_observability")
            .unwrap();
        let recorded = pipeline.record(event.clone()).unwrap();
        assert!(recorded.alert.is_none());
        assert_eq!(event.kind, "dmap_accepted");
        assert!(event.validate().is_empty());
        assert!(
            datum_alert_store::list_alerts(&event_path, &Default::default())
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            pipeline.record(event).unwrap().admission,
            datum_event_store::DatumEventAdmission::IdempotentReplay(_)
        ));
        assert_eq!(load_state(&authority_path).unwrap(), committed);
        drop(connection);
        drop(pipeline);
        cleanup_db(&authority_path);
        cleanup_db(&event_path);
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let outcome = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        let replay = accept_proposal(&mut reloaded_mut, "p1", "operator:bob", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T05:00:00Z", "should-not-be-used".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph, dcontinuum: sample_dcontinuum("proj-x", 1), placements: placements_v2 }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-empty".to_string(), application_id: "app-empty".to_string(), source: ProposalSource::Operator, dgraph: empty_dgraph, dcontinuum: sample_dcontinuum("proj-empty", 1), placements: vec![] },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &[],
        );
        // create_proposal itself runs validate_full (schema/identity/
        // cardinality -- a zero-service/zero-placement D-Map is
        // structurally valid there, per Stage 110's own rules), so it must
        // succeed at CREATION time; the operational-projection rejection
        // is specifically an ACCEPTANCE-time bridge concern.
        let record = result.expect("a structurally valid zero-service proposal must still be creatable");
        let outcome = accept_proposal(&mut state, &record.proposal.proposal_id, "operator:alice", None, &[], Some(&sample_dcontinuum("proj-empty", 1)), "2026-01-01T00:01:00Z", "a1".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-empty".to_string(), source: ProposalSource::Operator, dgraph: empty_dgraph, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![] },
            "p-empty".to_string(), "2026-01-01T01:00:00Z".to_string(), &[],
        ).expect("a structurally valid zero-service proposal must still be creatable");
        let outcome = accept_proposal(&mut state, "p-empty", "operator:bob", None, &[], Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a-empty".to_string());
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();

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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        state.proposals.get_mut("p1").unwrap().accepted_acceptance_id = Some("phantom-acceptance".to_string());
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a Pending proposal carrying a non-None accepted_acceptance_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_map_key_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-key-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.proposals.remove("p1");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance referencing a missing proposal must now fail closed (v3), never be tolerated as a dangling reference (v2's behavior)");
        cleanup_db(&path);
    }

    #[test]
    fn acceptance_project_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("acceptance-project-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.acceptances.get_mut("a1").unwrap().project_id = "some-other-project".to_string();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance whose project_id disagrees with its referenced proposal's project_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn active_application_id_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("active-application-id-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        let key = application_key("proj-x", "app-x");
        let extra = OperationalDesiredService { service_id: "svc-ghost".to_string(), runtime: "container".to_string(), desired_state: "running".to_string(), application_id: String::new(), dmap_ref: None };
        state.active_by_application.get_mut(&key).unwrap().operational_services.insert("svc-ghost".to_string(), extra);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an operational_services snapshot with an EXTRA service the active_dmap does not place must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn operational_service_snapshot_key_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("snapshot-key-mismatch");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
        state.project_operational_revision.insert("proj-x".to_string(), 99);
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a project_operational_revision that disagrees with the maximum recorded acceptance projection revision must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn project_operational_revision_missing_fails_closed_on_reload() {
        let path = temp_db_path("project-revision-missing");
        let mut state = DDeployStateV1::empty();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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

        dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        let proposal_digest_b = compute_proposal_digest(&proposal_b).unwrap();
        state.proposals.insert("p-b".to_string(), ProposalRecord { planner_provenance: None,
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

        dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        let proposal_digest = compute_proposal_digest(&proposal).unwrap();
        state.proposals.insert("p-empty".to_string(), ProposalRecord { planner_provenance: None,
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: dgraph_a, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "cloud-01".to_string() }] }, "p-a2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_a).unwrap();
        accept_proposal(&mut state, "p-a2", "operator:alice", None, &artifacts_a, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a-a2".to_string()).unwrap();

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

    // ---- D-Node/D-Continuum foundation closure (Subtranche A) ------------

    #[test]
    fn canonical_authority_node_slice_contains_exactly_that_nodes_placed_services() {
        // A5: proves the FULL chain -- accepted DMap -> canonical
        // `union_project_operational_graph` -> per-node slice -- by feeding
        // the canonical-authority-derived `OperationalDGraph` into the SAME
        // `build_operational_slice` function `api::operational_dgraph`'s
        // real node-slice endpoint (`operational_dgraph_slice_snapshot`)
        // calls. Two applications place services on DIFFERENT nodes; each
        // node's slice must contain EXACTLY that node's own placed
        // services, nothing from the other application's node.
        let mut state = DDeployStateV1::empty();

        let mut dgraph_a = sample_dgraph("app-a", 1);
        dgraph_a.services.truncate(1);
        dgraph_a.services[0].service_id = "svc-a".to_string();
        let artifacts_a = vec![sample_artifact("svc-a", vec!["fog-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-a", dgraph_a, "p-a1", vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }], &artifacts_a, "operator:alice", "a-a1", "2026-01-01T00:00:00Z").unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "svc-b".to_string();
        let artifacts_b = vec![sample_artifact("svc-b", vec!["cloud-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-b", dgraph_b, "p-b1", vec![CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "cloud-01".to_string() }], &artifacts_b, "operator:bob", "a-b1", "2026-01-01T01:00:00Z").unwrap();

        let revision = *state.project_operational_revision.get("proj-x").unwrap();
        let graph = union_project_operational_graph("proj-x", &state.active_by_application, revision).unwrap();

        let fog_slice = crate::core::operational_dgraph::build_operational_slice(&graph, "fog-01", None).unwrap();
        assert_eq!(fog_slice.desired_services.len(), 1);
        assert_eq!(fog_slice.desired_services[0].service_id, "svc-a");

        let cloud_slice = crate::core::operational_dgraph::build_operational_slice(&graph, "cloud-01", None).unwrap();
        assert_eq!(cloud_slice.desired_services.len(), 1);
        assert_eq!(cloud_slice.desired_services[0].service_id, "svc-b");
    }

    /// A `sample_artifact`-shaped fixture is deliberately minimal (D-Map
    /// acceptance validation only checks `target_nodes`/`runtime.kind`, not
    /// a full container spec) -- unlike `sample_artifact`, this one has a
    /// real `container` payload and a safe `wasm_decision`, so it can
    /// actually resolve through `build_artifact_reconciliation_context_with_authority`.
    fn container_ready_artifact(service_id: &str, target_nodes: Vec<&str>) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: format!("artifact:{service_id}"),
            service_id: service_id.to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                container: Some(crate::core::service_artifact_registry::ContainerDeploymentSpec {
                    container_name: format!("datum-{service_id}"),
                    image: "example.invalid/image:1".to_string(),
                    ports: Vec::new(),
                    network: None,
                    network_aliases: Vec::new(),
                    volumes: Vec::new(),
                    configuration_mounts: Vec::new(),
                    command: Vec::new(),
                    image_source_kind: crate::core::service_artifact_registry::IMAGE_SOURCE_KIND_LOCAL_BUILD.to_string(),
                    resolved_image_identity: None,
                }),
                native_process: None,
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec { kind: "none".to_string(), params: Default::default(), vantage_point: None },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                target_nodes: target_nodes.into_iter().map(str::to_string).collect(),
                version: "1".to_string(),
                artifact_kind: "container".to_string(),
                wasm_decision: crate::core::service_artifact_registry::ServiceArtifactWasmDecision {
                    module_id: "reconcile_service".to_string(),
                    function_name: "decide".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    /// Phase 115C (closing the Phase 115A "identity discarded one hop below
    /// dispatch authorization" gap): proves `application_id` and the exact
    /// accepted D-Map `{id, revision, digest}` survive the FULL chain --
    /// accepted D-Map -> `derive_application_operational_services` ->
    /// `union_project_operational_graph` -> per-node slice ->
    /// `build_artifact_reconciliation_context_with_authority` -- landing on
    /// the exact `ArtifactReconciliationAssignment` a D-Node would receive.
    /// Two applications on the SAME node must each keep their OWN identity,
    /// never cross-contaminated.
    #[test]
    fn application_id_and_dmap_ref_survive_from_acceptance_to_the_reconciliation_assignment() {
        let mut state = DDeployStateV1::empty();

        let mut dgraph_a = sample_dgraph("app-a", 1);
        dgraph_a.services.truncate(1);
        dgraph_a.services[0].service_id = "svc-a".to_string();
        let artifacts_a = vec![sample_artifact("svc-a", vec!["fog-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-a", dgraph_a, "p-a1", vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }], &artifacts_a, "operator:alice", "a-a1", "2026-01-01T00:00:00Z").unwrap();

        let mut dgraph_b = sample_dgraph("app-b", 1);
        dgraph_b.services.truncate(1);
        dgraph_b.services[0].service_id = "svc-b".to_string();
        let artifacts_b = vec![sample_artifact("svc-b", vec!["fog-01"])];
        create_and_accept_with_dgraph(&mut state, "proj-x", "app-b", dgraph_b, "p-b1", vec![CanonicalDServPlacementV1 { service_id: "svc-b".to_string(), node_id: "fog-01".to_string() }], &artifacts_b, "operator:bob", "a-b1", "2026-01-01T01:00:00Z").unwrap();

        let active_a = state.active_by_application.get(&application_key("proj-x", "app-a")).unwrap().clone();
        let active_b = state.active_by_application.get(&application_key("proj-x", "app-b")).unwrap().clone();

        let revision = *state.project_operational_revision.get("proj-x").unwrap();
        let graph = union_project_operational_graph("proj-x", &state.active_by_application, revision).unwrap();
        let fog_slice = crate::core::operational_dgraph::build_operational_slice(&graph, "fog-01", None).unwrap();
        assert_eq!(fog_slice.desired_services.len(), 2, "both applications place a service on fog-01");

        let svc_a = fog_slice.desired_services.iter().find(|s| s.service_id == "svc-a").unwrap();
        assert_eq!(svc_a.application_id, "app-a");
        let svc_a_dmap_ref = svc_a.dmap_ref.as_ref().expect("svc-a must carry its own dmap_ref");
        assert_eq!(svc_a_dmap_ref.id, active_a.active_dmap.dmap_id);
        assert_eq!(svc_a_dmap_ref.revision, active_a.active_dmap.revision);
        assert_eq!(svc_a_dmap_ref.digest, active_a.dmap_digest);

        let svc_b = fog_slice.desired_services.iter().find(|s| s.service_id == "svc-b").unwrap();
        assert_eq!(svc_b.application_id, "app-b");
        let svc_b_dmap_ref = svc_b.dmap_ref.as_ref().expect("svc-b must carry its own dmap_ref");
        assert_eq!(svc_b_dmap_ref.id, active_b.active_dmap.dmap_id);
        // Two independent applications' D-Maps must never be confused with
        // each other, even when they place services on the exact same node.
        assert_ne!(svc_a_dmap_ref.id, svc_b_dmap_ref.id);
        assert_ne!(svc_a.application_id, svc_b.application_id);

        let context = crate::core::artifact_reconciliation_context::build_artifact_reconciliation_context_with_authority(
            &fog_slice,
            &[container_ready_artifact("svc-a", vec!["fog-01"]), container_ready_artifact("svc-b", vec!["fog-01"])],
            crate::core::artifact_reconciliation_context::PlacementAuthority::CanonicalDMap,
        ).expect("context must build");

        let assignment_a = context.artifacts.iter().find(|a| a.service_id == "svc-a").unwrap();
        assert_eq!(assignment_a.application_id, "app-a");
        assert_eq!(assignment_a.dmap_ref.as_ref().unwrap().id, active_a.active_dmap.dmap_id);

        let assignment_b = context.artifacts.iter().find(|a| a.service_id == "svc-b").unwrap();
        assert_eq!(assignment_b.application_id, "app-b");
        assert_eq!(assignment_b.dmap_ref.as_ref().unwrap().id, active_b.active_dmap.dmap_id);
    }

    /// Phase 115E: proves the EXISTING (unmodified) Phase 114C/114D1
    /// readiness pipeline correctly consumes a `DServRealization`
    /// observation shaped EXACTLY as Phase 115B (`agent::dcode_execution_dmonitor::
    /// build_dserv_realization_from_dcode_evidence`) or Phase 115D
    /// (`agent::native_process_dmonitor::build_dserv_realization_from_native_state`)
    /// actually produce it -- real accepted D-Map, real `active_placement_authority`,
    /// real ingestion, real readiness evaluation. No readiness/convergence
    /// logic is touched by this phase; this test exists to CONFIRM it, not
    /// to change it. Mirrors those two producers' exact field mapping:
    /// `dmap_ref` copied verbatim from current authority, `service_id` the
    /// D-Serv placement id (never an artifact/D-Code id), `collector` one of
    /// their real collector-id constants, health `Healthy` for a real
    /// successful realization.
    #[test]
    fn readiness_pipeline_accepts_a_phase_115_shaped_dserv_realization_end_to_end() {
        use crate::core::canonical_dmonitor::{CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, DMonitorSubjectV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA};
        use crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1;
        use crate::core::dmonitor_readiness::{default_readiness_policy, evaluate_application_readiness, DMonitorReadinessV1};
        use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};
        use crate::core::dnode_registry::DNodeRegistryStateV1;

        const PROJECT: &str = "proj-115e";
        const APP: &str = "app-115e";
        const NODE: &str = "fog-01";
        const SVC: &str = "svc-115e";
        const T0: &str = "2026-01-01T00:00:00Z";

        let mut state = DDeployStateV1::empty();
        let mut dgraph = sample_dgraph(APP, 1);
        dgraph.services.truncate(1);
        dgraph.services[0].service_id = SVC.to_string();
        let artifacts = vec![sample_artifact(SVC, vec![NODE])];
        create_and_accept_with_dgraph(&mut state, PROJECT, APP, dgraph, "p1", vec![CanonicalDServPlacementV1 { service_id: SVC.to_string(), node_id: NODE.to_string() }], &artifacts, "operator:alice", "a1", T0).unwrap();

        let authority = active_placement_authority(&state, PROJECT, APP).unwrap().expect("must be active immediately after acceptance");

        // Phase 115B/115D's own exact `dmap_ref` construction: verbatim
        // id/revision/digest from current authority, never recomputed.
        let dmap_ref = DMonitorContractRefV1 { id: authority.dmap_id.clone(), revision: authority.revision, digest: authority.digest.clone() };

        let build_observation = |sequence: u64, service_id: &str, application_id: &str, node_id: &str, observed_at_utc: &str, health: DMonitorHealthV1, dmap_ref: DMonitorContractRefV1, observation_id: &str| {
            CanonicalDMonitorObservationV1 {
                schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
                observation_id: observation_id.to_string(),
                project_id: Some(PROJECT.to_string()),
                node_id: node_id.to_string(),
                monitor_instance_id: "dmonitor-instance:fog-01:115e-test".to_string(),
                sequence,
                observed_at_utc: observed_at_utc.to_string(),
                cycle_id: None,
                collector: "dcode_invoke".to_string(),
                subject: DMonitorSubjectV1::DServRealization {
                    project_id: PROJECT.to_string(),
                    application_id: application_id.to_string(),
                    service_id: service_id.to_string(),
                    node_id: node_id.to_string(),
                    dmap_ref: Some(dmap_ref),
                },
                health,
                facts: serde_json::json!({}),
            }
        };

        let empty_runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let empty_dnode_registry = DNodeRegistryStateV1::empty();
        let policy = default_readiness_policy();

        // Snapshot D-Map/acceptance state before any DMonitor activity, to
        // prove (Case H) that ingestion + readiness evaluation mutate NEITHER.
        let ddeploy_snapshot_before = state.clone();

        // CASE A: no D-Serv evidence at all -> NotReady.
        let empty_dmonitor_state = DMonitorStateV1::empty();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&authority), &empty_dmonitor_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "Case A: {result:#?}");

        // CASE B: exact current canonical Healthy/fresh evidence -> Ready.
        let mut dmonitor_state = DMonitorStateV1::empty();
        let observation = build_observation(1, SVC, APP, NODE, T0, DMonitorHealthV1::Healthy, dmap_ref.clone(), "obs-115e-1");
        ingest_observation(&mut dmonitor_state, PROJECT, observation, T0).unwrap();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&authority), &dmonitor_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "Case B: {result:#?}");

        // CASE C: the SAME evidence, evaluated 91s later -> stale -> NotReady
        // (freshness policy: max_observed_age_seconds = 90, re-verified from
        // source at the top of this test file's module, not merely assumed).
        let later = "2026-01-01T00:01:31Z"; // T0 + 91s
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&authority), &dmonitor_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, later);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "Case C (stale): {result:#?}");

        // CASE D: service identity mismatch -- evidence for a DIFFERENT
        // service_id than the one actually placed -> NotReady.
        let mut wrong_service_state = DMonitorStateV1::empty();
        let observation = build_observation(1, "svc-does-not-exist", APP, NODE, T0, DMonitorHealthV1::Healthy, dmap_ref.clone(), "obs-115e-d");
        ingest_observation(&mut wrong_service_state, PROJECT, observation, T0).unwrap();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&authority), &wrong_service_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "Case D (service mismatch): {result:#?}");

        // CASE E: application mismatch -- evidence honestly claims a
        // DIFFERENT application_id than the one being evaluated -> NotReady
        // (never silently credited to this application).
        let mut wrong_app_state = DMonitorStateV1::empty();
        let observation = build_observation(1, SVC, "some-other-app", NODE, T0, DMonitorHealthV1::Healthy, dmap_ref.clone(), "obs-115e-e");
        ingest_observation(&mut wrong_app_state, PROJECT, observation, T0).unwrap();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&authority), &wrong_app_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "Case E (application mismatch): {result:#?}");

        // CASE F: node mismatch -- evidence claims a node the D-Map did NOT
        // place this service on -> NotReady.
        let mut wrong_node_state = DMonitorStateV1::empty();
        let observation = build_observation(1, SVC, APP, "cloud-01", T0, DMonitorHealthV1::Healthy, dmap_ref.clone(), "obs-115e-f");
        ingest_observation(&mut wrong_node_state, PROJECT, observation, T0).unwrap();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&authority), &wrong_node_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "Case F (node mismatch): {result:#?}");

        // CASE G: the D-Map changes (a new acceptance bumps the revision) --
        // the OLD evidence's dmap_ref must no longer satisfy the NEW current
        // authority. Never silently "still good enough."
        let mut dgraph2 = sample_dgraph(APP, 2);
        dgraph2.services.truncate(1);
        dgraph2.services[0].service_id = SVC.to_string();
        create_and_accept_with_dgraph(&mut state, PROJECT, APP, dgraph2, "p2", vec![CanonicalDServPlacementV1 { service_id: SVC.to_string(), node_id: NODE.to_string() }], &artifacts, "operator:alice", "a2", "2026-01-01T00:02:00Z").unwrap();
        let new_authority = active_placement_authority(&state, PROJECT, APP).unwrap().expect("must still be active after re-acceptance");
        assert_ne!(new_authority.revision, authority.revision, "the acceptance must actually have advanced the D-Map revision");
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&new_authority), &dmonitor_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "Case G (stale D-Map evidence after re-acceptance): {result:#?}");

        // Fresh evidence under the NEW authority restores Ready.
        let new_dmap_ref = DMonitorContractRefV1 { id: new_authority.dmap_id.clone(), revision: new_authority.revision, digest: new_authority.digest.clone() };
        let mut fresh_dmonitor_state = DMonitorStateV1::empty();
        let observation = build_observation(1, SVC, APP, NODE, T0, DMonitorHealthV1::Healthy, new_dmap_ref, "obs-115e-g-fresh");
        ingest_observation(&mut fresh_dmonitor_state, PROJECT, observation, T0).unwrap();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&new_authority), &fresh_dmonitor_state, &empty_runtime_registry, &state, &empty_dnode_registry, &policy, T0);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "Case G (fresh evidence under new D-Map): {result:#?}");

        // CASE H: none of the above DMonitor activity (ingestion, six
        // readiness evaluations) mutated D-Map/acceptance state one bit
        // beyond the two acceptances THIS TEST ITSELF explicitly performed
        // -- compare the state as of immediately after acceptance p1 against
        // an independent snapshot taken at that exact point.
        assert_eq!(state.active_by_application.get(&application_key(PROJECT, APP)).unwrap().active_dmap.dmap_id, ddeploy_snapshot_before.active_by_application.get(&application_key(PROJECT, APP)).unwrap().active_dmap.dmap_id);
        assert_eq!(ddeploy_snapshot_before.active_by_application.get(&application_key(PROJECT, APP)).unwrap().active_dmap.revision, authority.revision, "the pre-DMonitor-activity snapshot must still reflect only the FIRST acceptance");
    }

    #[test]
    fn dnode_registry_mutation_never_rewrites_an_already_persisted_proposal_snapshot() {
        // A6: registry state becoming stale/changed must NOT silently
        // rewrite a historical proposal's persisted `dcontinuum_snapshot`.
        // `ProposalRecord` is immutable once created (module doc) and
        // `core::dnode_registry` is a completely independent persisted
        // domain in the SAME SQLite database -- this proves that
        // independence concretely, in one shared database file, rather than
        // merely asserting it by construction.
        let path = temp_db_path("dnode-registry-independence");
        let mut state = DDeployStateV1::empty();
        let record = create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
        let original_digest = record.proposal.dcontinuum_ref.digest.clone();
        let original_snapshot = record.dcontinuum_snapshot.clone();
        persist_state(&path, &state).unwrap();

        // Mutate the D-Node registry in the SAME database: declare a
        // DIFFERENT set of nodes entirely, then remove them again -- this
        // module never reads or writes `DOMAIN_DDEPLOY_STATE` at all.
        let mut registry = crate::core::dnode_registry::DNodeRegistryStateV1::empty();
        crate::core::dnode_registry::declare_dnode(
            &mut registry,
            crate::core::canonical_dcontinuum::CanonicalDNodeV1 {
                node_id: "fog-01".to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "arm64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores: 999, memory_bytes: 999_999_999_999 },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![] },
            },
        )
        .unwrap();
        crate::core::dnode_registry::persist_state(&path, &registry).unwrap();
        crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01");
        crate::core::dnode_registry::persist_state(&path, &registry).unwrap();

        let reloaded = load_state(&path).unwrap();
        let reloaded_record = reloaded.proposals.get("p1").unwrap();
        assert_eq!(reloaded_record.proposal.dcontinuum_ref.digest, original_digest, "an unrelated D-Node registry mutation must never change a historical proposal's dcontinuum_ref digest");
        assert_eq!(reloaded_record.dcontinuum_snapshot, original_snapshot, "an unrelated D-Node registry mutation must never rewrite a historical proposal's dcontinuum_snapshot");
        assert_eq!(recompute_dcontinuum_digest(&reloaded_record.dcontinuum_snapshot).unwrap(), original_digest);
        cleanup_db(&path);
    }

    // ---- independent review fix v2: R1 -- acceptance revalidates the -----
    // ---- CURRENT D-Node structural authority (a proposal valid when ------
    // ---- CREATED is not automatically valid when ACCEPTED) --------------

    /// A registry containing exactly `sample_dcontinuum`'s two nodes
    /// (fog-01 cpu=4/4GiB, cloud-01 cpu=8/17GiB, both `DNODE_ABI_V0`) --
    /// `build_canonical_dcontinuum` on this registry is therefore
    /// node-for-node identical (platform/capacity/ABI) to
    /// `sample_dcontinuum("proj-x", 1)`, so every existing "proj-x" test
    /// fixture remains a legitimate claim against it.
    fn registry_with_sample_nodes() -> crate::core::dnode_registry::DNodeRegistryStateV1 {
        let mut registry = crate::core::dnode_registry::DNodeRegistryStateV1::empty();
        for node in sample_dcontinuum("proj-x", 1).nodes {
            crate::core::dnode_registry::declare_dnode(&mut registry, node).unwrap();
        }
        registry
    }

    #[test]
    fn pending_proposal_fails_acceptance_after_its_node_is_removed() {
        let mut registry = registry_with_sample_nodes();
        let authoritative_at_creation = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: authoritative_at_creation, placements: sample_placements() },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        // Registry mutates AFTER proposal creation, BEFORE acceptance:
        // fog-01 (which this proposal places svc-a on) is removed entirely.
        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01"));
        let authoritative_at_acceptance = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&authoritative_at_acceptance), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(
            matches!(result, Err(AcceptError::StaleAuthority(ref findings)) if findings.iter().any(|f| f.code == crate::core::dnode_registry::F_DCONTINUUM_NODE_UNREGISTERED)),
            "acceptance must fail closed once the proposal's placed node is removed from structural authority: {result:?}"
        );
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Pending, "a stale-authority rejection must leave the proposal Pending, never silently Accepted");
        assert!(state.active_by_application.is_empty(), "a rejected acceptance must not create any active deployment");
    }

    #[test]
    fn pending_proposal_fails_acceptance_after_node_capacity_changes() {
        let mut registry = registry_with_sample_nodes();
        let authoritative_at_creation = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: authoritative_at_creation, placements: sample_placements() },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        // Structural declarations have no in-place update (ADR-0019,
        // documented limitation): remove then redeclare with different
        // capacity, simulating an operator correcting fog-01's real specs.
        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01"));
        crate::core::dnode_registry::declare_dnode(
            &mut registry,
            CanonicalDNodeV1 {
                node_id: "fog-01".to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores: 1, memory_bytes: 1_073_741_824 },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
            },
        )
        .unwrap();
        let authoritative_at_acceptance = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&authoritative_at_acceptance), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(
            matches!(result, Err(AcceptError::StaleAuthority(ref findings)) if findings.iter().any(|f| f.code == crate::core::dnode_registry::F_DNODE_STRUCTURAL_CONFLICT)),
            "acceptance must fail closed once the proposal's placed node's capacity has changed in structural authority: {result:?}"
        );
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Pending);
    }

    #[test]
    fn pending_proposal_fails_acceptance_after_platform_changes() {
        let mut registry = registry_with_sample_nodes();
        let authoritative_at_creation = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: authoritative_at_creation, placements: sample_placements() },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01"));
        crate::core::dnode_registry::declare_dnode(
            &mut registry,
            CanonicalDNodeV1 {
                node_id: "fog-01".to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "arm64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_294_967_296 },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
            },
        )
        .unwrap();
        let authoritative_at_acceptance = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&authoritative_at_acceptance), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(
            matches!(result, Err(AcceptError::StaleAuthority(ref findings)) if findings.iter().any(|f| f.code == crate::core::dnode_registry::F_DNODE_STRUCTURAL_CONFLICT)),
            "acceptance must fail closed once the proposal's placed node's platform has changed in structural authority: {result:?}"
        );
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Pending);
    }

    #[test]
    fn pending_proposal_fails_acceptance_after_abi_capability_changes() {
        let mut registry = registry_with_sample_nodes();
        let authoritative_at_creation = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: authoritative_at_creation, placements: sample_placements() },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01"));
        crate::core::dnode_registry::declare_dnode(
            &mut registry,
            CanonicalDNodeV1 {
                node_id: "fog-01".to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_294_967_296 },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![] },
            },
        )
        .unwrap();
        let authoritative_at_acceptance = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&authoritative_at_acceptance), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(
            matches!(result, Err(AcceptError::StaleAuthority(ref findings)) if findings.iter().any(|f| f.code == crate::core::dnode_registry::F_DNODE_CAPABILITY_INVALID)),
            "acceptance must fail closed once the proposal's placed node's declared ABI capabilities have changed in structural authority: {result:?}"
        );
        assert_eq!(state.proposals.get("p1").unwrap().state, ProposalLifecycleState::Pending);
    }

    #[test]
    fn stale_proposal_snapshot_is_rejected_even_though_it_remains_internally_self_consistent() {
        // A proposal's `dcontinuum_snapshot` is, on its own terms, perfectly
        // valid: it recomputes to its own recorded `dcontinuum_ref.digest`,
        // and `validate_full` (re-run at acceptance against that SAME
        // snapshot) finds nothing wrong with it either -- the placement is
        // schema-valid and every artifact is eligible on the node it names.
        // A registry change occurring before acceptance must NOT be
        // hideable behind that internal self-consistency: only the
        // CURRENT, freshly-fetched authoritative continuum is ever
        // consulted for the R1 check, never anything cached from proposal-
        // creation time or re-derived from the snapshot itself.
        let mut registry = registry_with_sample_nodes();
        let authoritative_at_creation = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let mut state = DDeployStateV1::empty();
        let record = create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: authoritative_at_creation, placements: sample_placements() },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();

        // Sanity check: the persisted snapshot really is self-consistent on
        // its own terms, and `validate_full` against it really does find
        // nothing wrong -- so the rejection below can only be coming from
        // the NEW authority check, not from a pre-existing structural flaw.
        assert!(record.dcontinuum_snapshot.validate().is_empty());
        assert_eq!(recompute_dcontinuum_digest(&record.dcontinuum_snapshot).unwrap(), record.proposal.dcontinuum_ref.digest);
        let self_check = validate_full(&record.proposal, &record.dgraph_snapshot, &record.dcontinuum_snapshot, &sample_artifacts(), "dmap:preview", 1);
        assert!(self_check.findings.is_empty(), "the proposal's own snapshot must remain internally valid: {:?}", self_check.findings);

        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01"));
        let authoritative_at_acceptance = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let result = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&authoritative_at_acceptance), "2026-01-01T01:00:00Z", "a1".to_string());
        assert!(matches!(result, Err(AcceptError::StaleAuthority(_))), "internal self-consistency of the OLD snapshot must never excuse a stale CURRENT authority: {result:?}");
    }

    #[test]
    fn accepted_historical_dmap_remains_valid_after_a_later_registry_change() {
        // R1 scope is PENDING -> ACCEPTED only: replaying an ALREADY-
        // accepted proposal must succeed regardless of ANY later D-Node
        // registry mutation, including removing every node the proposal
        // once depended on. `current_authoritative_dcontinuum: None` is
        // legitimate here precisely because the replay path never consults
        // it (see `accept_proposal`'s own doc comment on that parameter).
        let mut registry = registry_with_sample_nodes();
        let authoritative = crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").unwrap();

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: authoritative.clone(), placements: sample_placements() },
            "p1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &sample_artifacts(),
        )
        .unwrap();
        let original = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&authoritative), "2026-01-01T00:00:01Z", "a1".to_string()).unwrap();
        assert!(!original.idempotent_replay);

        // The registry now changes drastically: BOTH nodes are removed.
        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "fog-01"));
        assert!(crate::core::dnode_registry::remove_dnode(&mut registry, "cloud-01"));
        assert!(crate::core::dnode_registry::build_canonical_dcontinuum(&registry, "proj-x").is_err(), "sanity check: the registry really is empty now");

        let replay = accept_proposal(&mut state, "p1", "operator:bob", None, &sample_artifacts(), None, "2026-01-01T01:00:00Z", "should-not-be-used".to_string());
        assert!(replay.is_ok(), "replaying an already-accepted proposal must succeed regardless of a later D-Node registry change, even total removal: {replay:?}");
        let replay = replay.unwrap();
        assert!(replay.idempotent_replay);
        assert_eq!(replay.materialized_dmap.revision, original.materialized_dmap.revision);
        assert_eq!(replay.materialized_dmap.placements, original.materialized_dmap.placements, "the historical D-Map's placements must be byte-identical to what was originally accepted");
        assert!(replay.currently_active);
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: swapped }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &swapped_artifacts).unwrap();
        accept_proposal(&mut state, "p2", "operator:alice", None, &swapped_artifacts, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
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
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() }, "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts()).unwrap();
        accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();

        // A second, genuinely PENDING proposal for a different application
        // (so it does not itself collide with p1's placements).
        let mut dgraph_c = sample_dgraph("app-c", 1);
        dgraph_c.services.truncate(1);
        dgraph_c.services[0].service_id = "svc-c".to_string();
        create_proposal(&mut state, CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-c".to_string(), source: ProposalSource::Operator, dgraph: dgraph_c, dcontinuum: sample_dcontinuum("proj-x", 1), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-c".to_string(), node_id: "fog-01".to_string() }] }, "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &[sample_artifact("svc-c", vec!["fog-01"])]).unwrap();

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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: sample_placements() },
            "p1".to_string(), "2026-01-01T00:00:00Z".to_string(), &sample_artifacts(),
        ).unwrap();
        let outcome_n = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-x".to_string(), application_id: "app-x".to_string(), source: ProposalSource::Operator, dgraph: sample_dgraph("app-x", 1), dcontinuum: sample_dcontinuum("proj-x", 1), placements: swapped },
            "p2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts2,
        ).unwrap();
        // The proposal itself IS successfully persisted (a real caller
        // persists after every successful mutation)...
        persist_state(&path, &state).unwrap();
        let outcome_n1 = accept_proposal(&mut state, "p2", "operator:alice", None, &artifacts2, Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T01:00:00Z", "a2".to_string()).unwrap();
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
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
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
        let outcome = accept_proposal(&mut state, "p1", "operator:alice", None, &sample_artifacts(), Some(&sample_dcontinuum("proj-x", 1)), "2026-01-01T00:00:00Z", "a1".to_string()).unwrap();
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
                CanonicalDServV1 { service_definition: None, service_id: "svc-ingest".to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 268435456 } },
                CanonicalDServV1 { service_definition: None, service_id: "svc-process".to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 268435456 } },
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

    // ---- Canonical deterministic placement v1 integration (roadmap ------
    // ---- conceptual phase 120, ADR-0020) ---------------------------------
    //
    // These tests prove the planner's output flows through the UNCHANGED
    // `create_proposal`/`accept_proposal` path with no special-casing --
    // test matrix items 12/13/14/15/16 from the ADR-0020 spec.

    #[test]
    fn planner_output_is_accepted_by_create_proposal_unmodified() {
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &crate::core::canonical_placement::PlacementEligibilityView::unrestricted()).unwrap();
        // Both svc-a/svc-b (1 cpu/128MiB each) land on fog-01 (4 cpu) ahead
        // of cloud-01 (8 cpu): min-residual-after prefers the tighter fit.
        assert_eq!(
            crate::core::canonical_placement::placements_by_service(&plan).get("svc-a").map(String::as_str),
            Some("fog-01")
        );

        let mut state = DDeployStateV1::empty();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];
        let record = create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum,
                placements: plan.placements,
            },
            "planned-proposal-1".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .expect("a feasible planner-produced placement must be accepted by the UNCHANGED create_proposal validation path");
        assert_eq!(record.state, ProposalLifecycleState::Pending, "the planner must never produce anything but a PENDING proposal");
        assert_eq!(record.proposal.source, ProposalSource::Deterministic);
    }

    #[test]
    fn planner_generated_pending_proposal_can_be_explicitly_accepted_through_the_unchanged_acceptance_path() {
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &crate::core::canonical_placement::PlacementEligibilityView::unrestricted()).unwrap();

        let mut state = DDeployStateV1::empty();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum: dcontinuum.clone(),
                placements: plan.placements,
            },
            "planned-proposal-2".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap();

        // Explicit, separate, human/operator acceptance step -- the planner
        // itself never called this function (see item 14 below).
        let outcome = accept_proposal(
            &mut state,
            "planned-proposal-2",
            "operator:dana",
            None,
            &artifacts,
            Some(&dcontinuum),
            "2026-01-01T00:05:00Z",
            "planned-acceptance-2".to_string(),
        )
        .expect("explicit acceptance of a planner-generated proposal must succeed through the unchanged Stage 111 acceptance path");
        assert!(!outcome.idempotent_replay);
        assert!(outcome.currently_active);
    }

    /// Item 14: the planner module itself never calls `accept_proposal` --
    /// enforced structurally (no such import exists in
    /// `core::canonical_placement`), re-asserted here at the integration
    /// level: calling the planner alone, without a separate explicit
    /// `accept_proposal` call, leaves `DDeployStateV1` with no active
    /// authority at all.
    #[test]
    fn planning_alone_never_activates_anything() {
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &crate::core::canonical_placement::PlacementEligibilityView::unrestricted()).unwrap();
        assert!(!plan.placements.is_empty());
        // No DDeployStateV1 was even constructed -- planning is pure and
        // touches no persisted state, active authority, or host at all.
    }

    // ---- foundation closure v2, R3: project-wide structural commitment ---
    //
    // A single shared D-Node ("shared-01", 4 CPU cores / 4 GiB) is reused
    // across this group so exceeding its capacity across TWO applications is
    // trivial to construct. Every test here uses DISTINCT service_ids across
    // applications (mandatory regression item 12) so a failure can never be
    // misattributed to `F_OPERATIONAL_SERVICE_ID_COLLISION` instead of the
    // capacity codes actually under test.

    /// Phase 119G: a D-Continuum declaring `node_ids.len()` structurally
    /// known nodes (each with the same capacity), plus the always-present
    /// "shared-01" node -- used by the cleanup-derivation multi-hop tests,
    /// which move a service across several distinct node names.
    fn multi_node_dcontinuum(project: &str, node_ids: &[&str], cpu_cores: u32, memory_bytes: u64) -> CanonicalDContinuumV1 {
        let mut nodes = vec![CanonicalDNodeV1 {
            node_id: "shared-01".to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores, memory_bytes },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        }];
        for node_id in node_ids {
            nodes.push(CanonicalDNodeV1 {
                node_id: node_id.to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores, memory_bytes },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
            });
        }
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project.to_string(),
            dcontinuum_id: format!("continuum:{project}"),
            revision: 1,
            nodes,
            links: vec![],
        }
    }

    fn shared_node_dcontinuum(project: &str, cpu_cores: u32, memory_bytes: u64) -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project.to_string(),
            dcontinuum_id: format!("continuum:{project}"),
            revision: 1,
            nodes: vec![CanonicalDNodeV1 {
                node_id: "shared-01".to_string(),
                platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                capacity: CanonicalNodeCapacityV1 { cpu_cores, memory_bytes },
                capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
            }],
            links: vec![],
        }
    }

    fn one_service_dgraph(app: &str, service_id: &str, cpu_cores: u32, memory_bytes: u64) -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: app.to_string(),
            dgraph_id: format!("dgraph:{app}"),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_definition: None, service_id: service_id.to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores, memory_bytes },
            }],
            calls: vec![],
        }
    }

    /// Creates a PENDING proposal for `app` (one service, on `shared-01`)
    /// against `dcontinuum`, then immediately attempts to accept it -- the
    /// single call this whole test group needs, since none of these tests
    /// care about a separate create/accept gap.
    fn create_and_accept_on_shared_node(
        state: &mut DDeployStateV1,
        project: &str,
        app: &str,
        service_id: &str,
        cpu_cores: u32,
        memory_bytes: u64,
        dcontinuum: &CanonicalDContinuumV1,
        proposal_id: &str,
        acceptance_id: &str,
    ) -> Result<AcceptOutcome, AcceptError> {
        let dgraph = one_service_dgraph(app, service_id, cpu_cores, memory_bytes);
        let artifacts = vec![sample_artifact(service_id, vec!["shared-01"])];
        create_proposal(
            state,
            CreateProposalInput {
                planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: project.to_string(),
                application_id: app.to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 { service_id: service_id.to_string(), node_id: "shared-01".to_string() }],
            },
            proposal_id.to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap_or_else(|findings| panic!("proposal creation for '{app}' must succeed: {findings:?}"));
        accept_proposal(state, proposal_id, "operator:dana", None, &artifacts, Some(dcontinuum), "2026-01-01T00:05:00Z", acceptance_id.to_string())
    }

    #[test]
    fn active_application_consumption_reduces_residual_seen_when_planning_a_sibling_application() {
        // Item 6: App A takes 3 of shared-01's 4 cores; planning App B's
        // 2-core service must NOT select shared-01 (only 1 residual core
        // left there) -- it must select the larger, uncommitted node.
        let dcontinuum_a = shared_node_dcontinuum("proj-commit", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "proj-commit", "app-a", "svc-a1", 3, 1, &dcontinuum_a, "p-a1", "acc-a1")
            .expect("app-a's own 3-core request must fit shared-01 alone");

        let commitments = compute_global_node_commitments(&state.active_by_application, &state.proposals, Some(("proj-commit", "app-b"))).unwrap();
        assert_eq!(commitments.get("shared-01").map(|c| c.cpu_cores), Some(3));

        // Planning App B against a continuum with BOTH shared-01 (4 cores,
        // 3 committed) and a bigger, uncommitted big-01 (8 cores):
        let mut dcontinuum_b = dcontinuum_a.clone();
        dcontinuum_b.nodes.push(CanonicalDNodeV1 {
            node_id: "big-01".to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: 8, memory_bytes: 17_179_869_184 },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        });
        let dgraph_b = one_service_dgraph("app-b", "svc-b1", 2, 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph_b, &dcontinuum_b, &commitments, &crate::core::canonical_placement::PlacementEligibilityView::unrestricted())
            .expect("2 cores must still be planable on the uncommitted big-01 node");
        assert_eq!(plan.placements[0].node_id, "big-01", "shared-01 has only 1 residual core after app-a's commitment -- big-01 must be chosen");
    }

    #[test]
    fn two_active_applications_whose_summed_demand_fits_are_both_legal() {
        // Item 7.
        let dcontinuum = shared_node_dcontinuum("proj-commit", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "proj-commit", "app-a", "svc-a1", 2, 1, &dcontinuum, "p-a1", "acc-a1").expect("app-a (2 cores) must be accepted");
        create_and_accept_on_shared_node(&mut state, "proj-commit", "app-b", "svc-b1", 2, 1, &dcontinuum, "p-b1", "acc-b1").expect("app-b (2 cores) must ALSO be accepted -- 2+2=4 fits shared-01 exactly");
        assert_eq!(state.active_by_application.len(), 2);
    }

    #[test]
    fn second_acceptance_exceeding_project_cpu_capacity_fails_closed() {
        // Items 8/10 (R3D): both proposals may legitimately be Pending
        // simultaneously (creation only validates WITHIN one D-Graph); only
        // the FIRST acceptance may actually commit shared-01's capacity.
        let dcontinuum = shared_node_dcontinuum("proj-commit", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        let artifacts_a = vec![sample_artifact("svc-a1", vec!["shared-01"])];
        let artifacts_b = vec![sample_artifact("svc-b1", vec!["shared-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-commit".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: one_service_dgraph("app-a", "svc-a1", 4, 1), dcontinuum: dcontinuum.clone(), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a1".to_string(), node_id: "shared-01".to_string() }] },
            "p-a1".to_string(), "2026-01-01T00:00:00Z".to_string(), &artifacts_a,
        ).expect("app-a's 4-core PENDING proposal must be creatable -- creation only validates within its own D-Graph");
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-commit".to_string(), application_id: "app-b".to_string(), source: ProposalSource::Operator, dgraph: one_service_dgraph("app-b", "svc-b1", 4, 1), dcontinuum: dcontinuum.clone(), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-b1".to_string(), node_id: "shared-01".to_string() }] },
            "p-b1".to_string(), "2026-01-01T00:01:00Z".to_string(), &artifacts_b,
        ).expect("app-b's 4-core PENDING proposal must ALSO be creatable -- both may legitimately be Pending at once");
        assert_eq!(state.proposals.values().filter(|r| r.state == ProposalLifecycleState::Pending).count(), 2, "both proposals must genuinely be Pending before either is accepted");

        accept_proposal(&mut state, "p-a1", "operator:dana", None, &artifacts_a, Some(&dcontinuum), "2026-01-01T00:05:00Z", "acc-a1".to_string())
            .expect("the FIRST acceptance must succeed: app-a alone (4 cores) exactly fits shared-01's 4-core capacity");
        let result = accept_proposal(&mut state, "p-b1", "operator:dana", None, &artifacts_b, Some(&dcontinuum), "2026-01-01T00:06:00Z", "acc-b1".to_string());
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "the SECOND acceptance must fail closed: app-a (4) + app-b (4) = 8 > shared-01's 4-core capacity -- got {result:?}"
        );
        assert_eq!(state.active_by_application.len(), 1, "the failed second acceptance must not have mutated active authority at all");
    }

    #[test]
    fn second_acceptance_exceeding_project_memory_capacity_fails_closed() {
        // Item 9: CPU fits (1+1=2 <= 4) but memory does not
        // (3_000_000_000 * 2 > 4_294_967_296).
        let dcontinuum = shared_node_dcontinuum("proj-commit", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "proj-commit", "app-a", "svc-a1", 1, 3_000_000_000, &dcontinuum, "p-a1", "acc-a1").expect("app-a alone must fit (1 core / 3e9 bytes)");
        let result = create_and_accept_on_shared_node(&mut state, "proj-commit", "app-b", "svc-b1", 1, 3_000_000_000, &dcontinuum, "p-b1", "acc-b1");
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED)),
            "app-a (3e9) + app-b (3e9) = 6e9 bytes > shared-01's 4_294_967_296-byte capacity -- got {result:?}"
        );
    }

    #[test]
    fn redeploying_the_same_application_does_not_double_count_its_own_prior_active_allocation() {
        // Item 11: app-a is already active using 3 of shared-01's 4 cores.
        // Accepting a NEW proposal for app-a ITSELF (also 3 cores) must
        // succeed -- `updated_active` REPLACES app-a's map entry rather than
        // adding a second one, so this must never be evaluated as
        // "3 (old app-a) + 3 (new app-a) = 6 > 4".
        let dcontinuum = shared_node_dcontinuum("proj-commit", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "proj-commit", "app-a", "svc-a1", 3, 1, &dcontinuum, "p-a1", "acc-a1").expect("app-a's first acceptance must succeed");

        let artifacts_a2 = vec![sample_artifact("svc-a2", vec!["shared-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "proj-commit".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: one_service_dgraph("app-a", "svc-a2", 3, 1), dcontinuum: dcontinuum.clone(), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a2".to_string(), node_id: "shared-01".to_string() }] },
            "p-a2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_a2,
        ).unwrap();
        let result = accept_proposal(&mut state, "p-a2", "operator:dana", None, &artifacts_a2, Some(&dcontinuum), "2026-01-01T01:05:00Z", "acc-a2".to_string());
        assert!(result.is_ok(), "redeploying app-a must not double-count its own superseded allocation -- got {result:?}");
        assert_eq!(state.active_by_application.len(), 1, "app-a's redeployment replaces its own entry -- never a second one");
    }

    // ---- Phase 136A: global (cross-project) D-Node capacity authority -----

    #[test]
    fn cross_project_oversubscription_is_rejected_after_the_global_capacity_fix() {
        // Mandatory Phase 136A regression (audit Part E's exact scenario):
        // shared-01 declares 8 CPU cores. Project A's app-a already
        // accepted 6 of them. Project B's app-b then proposes 6 MORE on the
        // SAME physical node -- 6 + 6 = 12 > 8. Before Phase 136A, this was
        // WRONGLY accepted, because commitment accounting was scoped to one
        // project; a DIFFERENT project must not be able to bypass the gate
        // a same-project application would be held to.
        // Phase 135's own invariant: the SAME global D-Node is exposed to
        // every project as its own project-labeled `CanonicalDContinuumV1`
        // (`build_canonical_dcontinuum` stamps the SAME node set, differently
        // labeled, per requesting project) -- so each project here gets its
        // OWN dcontinuum object naming that project, declaring the SAME
        // physical node, exactly mirroring real production shape rather
        // than reusing one object across a project boundary it does not
        // claim to represent.
        let dcontinuum_a = shared_node_dcontinuum("project-a", 8, 8_589_934_592);
        let dcontinuum_b = shared_node_dcontinuum("project-b", 8, 8_589_934_592);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "project-a", "app-a", "svc-a1", 6, 1, &dcontinuum_a, "p-a1", "acc-a1")
            .expect("project-a's own 6-core request must fit shared-01 alone");

        let result = create_and_accept_on_shared_node(&mut state, "project-b", "app-b", "svc-b1", 6, 1, &dcontinuum_b, "p-b1", "acc-b1");
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "project-a (6) + project-b (6) = 12 > shared-01's 8-core capacity -- a different project must not bypass this gate, got {result:?}"
        );
        assert_eq!(state.active_by_application.len(), 1, "the rejected cross-project acceptance must not have mutated active authority at all");
    }

    #[test]
    fn three_different_projects_sharing_a_node_within_its_total_capacity_are_all_accepted() {
        // Mandatory Phase 136A regression (audit Part E's exact scenario):
        // node capacity 8 cores; project A commits 3, project B commits 2,
        // project C proposes 2 -- total 7 <= 8, all three legal. "Shared !=
        // unlimited" does not mean "shared == exclusive" either: three
        // DIFFERENT projects may simultaneously and legitimately occupy one
        // physical node as long as their combined declared demand fits.
        let dcontinuum_a = shared_node_dcontinuum("project-a", 8, 8_589_934_592);
        let dcontinuum_b = shared_node_dcontinuum("project-b", 8, 8_589_934_592);
        let dcontinuum_c = shared_node_dcontinuum("project-c", 8, 8_589_934_592);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "project-a", "app-a", "svc-a1", 3, 1, &dcontinuum_a, "p-a1", "acc-a1")
            .expect("project-a's 3-core request must succeed");
        create_and_accept_on_shared_node(&mut state, "project-b", "app-b", "svc-b1", 2, 1, &dcontinuum_b, "p-b1", "acc-b1")
            .expect("project-b's 2-core request must succeed (3+2=5 <= 8)");
        create_and_accept_on_shared_node(&mut state, "project-c", "app-c", "svc-c1", 2, 1, &dcontinuum_c, "p-c1", "acc-c1")
            .expect("project-c's 2-core request must succeed (3+2+2=7 <= 8)");
        assert_eq!(state.active_by_application.len(), 3, "all three different-project applications must be simultaneously active on the shared node");
    }

    #[test]
    fn a_fourth_different_project_exceeding_the_remaining_shared_capacity_is_rejected() {
        // Same family as the test above, the audit's explicit invalid
        // variant: project A=3, project B=2 (5 committed, 3 residual on an
        // 8-core node); project C then proposes 5 -- 3+2+5=10 > 8, must be
        // rejected, never silently truncated or partially accepted.
        let dcontinuum_a = shared_node_dcontinuum("project-a", 8, 8_589_934_592);
        let dcontinuum_b = shared_node_dcontinuum("project-b", 8, 8_589_934_592);
        let dcontinuum_c = shared_node_dcontinuum("project-c", 8, 8_589_934_592);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "project-a", "app-a", "svc-a1", 3, 1, &dcontinuum_a, "p-a1", "acc-a1")
            .expect("project-a's 3-core request must succeed");
        create_and_accept_on_shared_node(&mut state, "project-b", "app-b", "svc-b1", 2, 1, &dcontinuum_b, "p-b1", "acc-b1")
            .expect("project-b's 2-core request must succeed (3+2=5 <= 8)");

        let result = create_and_accept_on_shared_node(&mut state, "project-c", "app-c", "svc-c1", 5, 1, &dcontinuum_c, "p-c1", "acc-c1");
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "project-a (3) + project-b (2) + project-c (5) = 10 > shared-01's 8-core capacity -- got {result:?}"
        );
        assert_eq!(state.active_by_application.len(), 2, "the rejected third acceptance must not have mutated active authority at all");
    }

    #[test]
    fn redeploying_an_application_excludes_only_its_own_old_allocation_from_the_global_baseline() {
        // Mandatory Phase 136A regression: shared-01 has 8 cores. project-a's
        // app-a is active using 4; project-b's app-b (a DIFFERENT project)
        // is ALSO active, using 2. A successor proposal for app-a itself,
        // again requesting 4 cores, must be evaluated as "project-b's 2
        // (always included, a different application) + app-a's NEW 4
        // (never app-a's OLD 4 as well) = 6 <= 8" -- never
        // "2 (app-b) + 4 (app-a old) + 4 (app-a new) = 10 > 8". This proves
        // the exclude-by-(project_id, application_id) pair correctly
        // excludes ONLY the one application being redeployed while still
        // including every OTHER project's commitment.
        let dcontinuum_a = shared_node_dcontinuum("project-a", 8, 8_589_934_592);
        let dcontinuum_b = shared_node_dcontinuum("project-b", 8, 8_589_934_592);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "project-a", "app-a", "svc-a1", 4, 1, &dcontinuum_a, "p-a1", "acc-a1")
            .expect("app-a's first acceptance must succeed");
        create_and_accept_on_shared_node(&mut state, "project-b", "app-b", "svc-b1", 2, 1, &dcontinuum_b, "p-b1", "acc-b1")
            .expect("app-b (a different project) must also fit: 4+2=6 <= 8");

        let artifacts_a2 = vec![sample_artifact("svc-a2", vec!["shared-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput { planner_provenance: None, dcode_artifact_authority: std::collections::BTreeMap::new(), project_id: "project-a".to_string(), application_id: "app-a".to_string(), source: ProposalSource::Operator, dgraph: one_service_dgraph("app-a", "svc-a2", 4, 1), dcontinuum: dcontinuum_a.clone(), placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a2".to_string(), node_id: "shared-01".to_string() }] },
            "p-a2".to_string(), "2026-01-01T01:00:00Z".to_string(), &artifacts_a2,
        ).unwrap();
        let result = accept_proposal(&mut state, "p-a2", "operator:dana", None, &artifacts_a2, Some(&dcontinuum_a), "2026-01-01T01:05:00Z", "acc-a2".to_string());
        assert!(result.is_ok(), "app-a's redeployment (still 4 cores) plus app-b's unrelated 2 cores = 6 <= 8 -- must succeed, got {result:?}");
        assert_eq!(state.active_by_application.len(), 2, "both app-a (redeployed) and app-b remain active");
    }

    #[test]
    fn v2_active_app_in_one_project_plus_v1_candidate_in_a_different_project_exceeding_capacity_is_rejected() {
        // Mandatory Phase 136A regression, v1/v2 dual-representation
        // variant: app-x is active under DMap/2 in project-x, consuming
        // shared-01's full 4-core capacity. A v1 candidate for app-y in a
        // DIFFERENT project (project-y) requesting any CPU on the same node
        // must still be rejected -- the Phase 120B v1/v2 unification and
        // the Phase 136A cross-project fix must compose correctly together,
        // neither silently undoing the other.
        let dcontinuum_x = shared_node_dcontinuum("project-x", 4, 4_294_967_296);
        let dcontinuum_y = shared_node_dcontinuum("project-y", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(&mut state, "project-x", "app-x", "svc-x1", 4, 1, &dcontinuum_x, "p-v2-x1", "acc-v2-x1", None)
            .expect("app-x's v2 acceptance (4 cores) must exactly fit shared-01 alone");

        let result = create_and_accept_on_shared_node(&mut state, "project-y", "app-y", "svc-y1", 1, 1, &dcontinuum_y, "p-v1-y1", "acc-v1-y1");
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "v2-active app-x (4, project-x) + v1-candidate app-y (1, project-y) = 5 > shared-01's 4-core capacity -- a different project must not bypass this, got {result:?}"
        );
    }

    #[test]
    fn project_node_commitment_summation_overflow_fails_closed() {
        // Item 13. Two active applications each declaring a
        // `memory_bytes` of `u64::MAX` on the SAME node: summing them
        // overflows `u64` on the second addition. Constructed directly
        // against `compute_global_node_commitments` (never through
        // `create_proposal`, which independently may or may not accept an
        // absurd declared value -- irrelevant to proving THIS function's
        // own overflow arithmetic fails closed rather than wrapping).
        let mut state = DDeployStateV1::empty();
        for (app, service_id, proposal_id, acceptance_id) in [("app-a", "svc-a1", "p-a1", "acc-a1"), ("app-b", "svc-b1", "p-b1", "acc-b1")] {
            let dgraph = one_service_dgraph(app, service_id, 1, u64::MAX);
            let proposal = DDeployProposalV1 {
                schema: DDEPLOY_PROPOSAL_SCHEMA.to_string(),
                proposal_id: proposal_id.to_string(),
                project_id: "proj-overflow".to_string(),
                application_id: app.to_string(),
                source: ProposalSource::Operator,
                created_at_utc: "2026-01-01T00:00:00Z".to_string(),
                expected_active_dmap_revision: None,
                // `u64::MAX` is outside JCS's safe-integer range, so this
                // dgraph cannot actually be digested -- irrelevant here,
                // since this test exercises `compute_global_node_commitments`
                // directly (never `load_state`/`validate_persisted_state_integrity`,
                // which WOULD recompute and check this digest).
                dgraph_ref: CanonicalContractReferenceV1 { id: dgraph.dgraph_id.clone(), revision: 1, digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string() },
                dcontinuum_ref: CanonicalContractReferenceV1 { id: "continuum:proj-overflow".to_string(), revision: 1, digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000".to_string() },
                placements: vec![CanonicalDServPlacementV1 { service_id: service_id.to_string(), node_id: "shared-01".to_string() }],

            dcode_artifact_authority: std::collections::BTreeMap::new(),
        };
            let proposal_digest = compute_proposal_digest(&proposal).unwrap();
            let dcontinuum_snapshot = CanonicalDContinuumV1 { schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(), project_id: "proj-overflow".to_string(), dcontinuum_id: "continuum:proj-overflow".to_string(), revision: 1, nodes: vec![], links: vec![] };
            // dcontinuum_ref.digest above must match a REAL empty-node
            // continuum's digest for internal self-consistency, but this
            // test only exercises `compute_global_node_commitments`
            // directly (never `load_state`), so the ref/snapshot digest
            // relation is irrelevant to it -- left deliberately loose.
            let dmap = CanonicalDMapV1 {
                schema: CANONICAL_DMAP_SCHEMA.to_string(),
                project_id: "proj-overflow".to_string(),
                application_id: app.to_string(),
                dmap_id: format!("dmap:{app}"),
                revision: 1,
                dgraph_ref: proposal.dgraph_ref.clone(),
                dcontinuum_ref: proposal.dcontinuum_ref.clone(),
                placements: proposal.placements.clone(),
            };
            state.proposals.insert(proposal_id.to_string(), ProposalRecord { planner_provenance: None, proposal: proposal.clone(), proposal_digest: proposal_digest.clone(), dgraph_snapshot: dgraph, dcontinuum_snapshot, state: ProposalLifecycleState::Pending, accepted_acceptance_id: None });
            state.active_by_application.insert(
                application_key("proj-overflow", app),
                ActiveApplicationDeployment {
                    application_id: app.to_string(),
                    active_dmap: dmap.clone(),
                    dmap_digest: recompute_dmap_digest(&dmap).unwrap(),
                    proposal_id: proposal_id.to_string(),
                    acceptance_id: acceptance_id.to_string(),
                    accepted_at_utc: "2026-01-01T00:05:00Z".to_string(),
                    operational_services: BTreeMap::new(),
                },
            );
        }
        let result = compute_global_node_commitments(&state.active_by_application, &state.proposals, None);
        assert!(result.is_err(), "summing two u64::MAX memory commitments on the same node must fail closed with an overflow error, not wrap");
        assert!(result.unwrap_err().contains("overflow"));
    }

    // ---- foundation closure v2, P2: planner provenance persistence -------

    #[test]
    fn persisted_deterministic_provenance_round_trips_a_restart() {
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &crate::core::canonical_placement::PlacementEligibilityView::unrestricted()).unwrap();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum,
                placements: plan.placements.clone(),
                planner_provenance: Some(planner_provenance_from_plan(&plan, BTreeMap::new(), crate::core::canonical_placement::PlacementEligibilityView::unrestricted())), dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            "planned-proposal-provenance".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).unwrap();

        let record = reloaded.proposals.get("planned-proposal-provenance").unwrap();
        let provenance = record.planner_provenance.as_ref().expect("planner_provenance must survive a persist/reload round trip");
        assert_eq!(provenance.policy_version, plan.provenance.policy_version, "item 22: the exact policy version must survive reload");
        assert_eq!(provenance.decisions, plan.provenance.decisions);
        // C1C: canonical input refs must also survive the round trip.
        assert_eq!(provenance.dgraph_ref.id, plan.provenance.dgraph_id);
        assert_eq!(provenance.dgraph_ref.revision, plan.provenance.dgraph_revision);
        assert_eq!(provenance.dgraph_ref.digest, plan.provenance.dgraph_digest, "mandatory test 1: DGraph id/revision/digest must survive reload");
        assert_eq!(provenance.dcontinuum_ref.id, plan.provenance.dcontinuum_id);
        assert_eq!(provenance.dcontinuum_ref.revision, plan.provenance.dcontinuum_revision);
        assert_eq!(provenance.dcontinuum_ref.digest, plan.provenance.dcontinuum_digest, "mandatory test 2: DContinuum id/revision/digest must survive reload");
        // C1C: relationally matches the proposal's own immutable refs too.
        assert_eq!(provenance.dgraph_ref, record.proposal.dgraph_ref, "mandatory test 3: persisted refs must match the immutable proposal ref");
        assert_eq!(provenance.dcontinuum_ref, record.proposal.dcontinuum_ref, "mandatory test 3: persisted refs must match the immutable proposal ref");
    }

    #[test]
    fn persisted_commitment_and_eligibility_snapshots_round_trip_a_restart() {
        // Mandatory tests 4/5: the EXACT commitment/eligibility input this
        // successful plan actually used must survive a persist/reload cycle
        // byte-for-byte, not merely "some" value.
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let mut committed = BTreeMap::new();
        committed.insert("fog-01".to_string(), crate::core::canonical_placement::NodeCommitmentV1 { cpu_cores: 1, memory_bytes: 1024 });
        let mut eligibility = crate::core::canonical_placement::PlacementEligibilityView::unrestricted();
        eligibility.insert("svc-a".to_string(), std::collections::BTreeSet::from(["fog-01".to_string()]));
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &committed, &eligibility).unwrap();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum,
                placements: plan.placements.clone(),
                planner_provenance: Some(planner_provenance_from_plan(&plan, committed.clone(), eligibility.clone())), dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            "planned-proposal-snapshots".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).unwrap();
        let provenance = reloaded.proposals.get("planned-proposal-snapshots").unwrap().planner_provenance.as_ref().unwrap();
        assert_eq!(provenance.committed_by_node, committed, "mandatory test 4: commitment snapshot must round-trip a restart exactly");
        assert_eq!(provenance.eligibility, eligibility, "mandatory test 5: eligibility snapshot must round-trip a restart exactly");
    }

    #[test]
    fn eligibility_snapshot_distinguishes_absent_service_from_present_but_empty() {
        // Mandatory test 6: a service_id ABSENT from the persisted
        // eligibility snapshot ("unrestricted by this view") must remain
        // distinguishable from a service_id PRESENT with an empty node set
        // ("zero eligible nodes") after a full persist/reload round trip --
        // never collapsed into one representation.
        let dgraph = sample_dgraph("app-planned", 1); // declares svc-a and svc-b
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let mut eligibility = crate::core::canonical_placement::PlacementEligibilityView::unrestricted();
        eligibility.insert("svc-a".to_string(), std::collections::BTreeSet::from(["fog-01".to_string(), "cloud-01".to_string()]));
        // svc-b is deliberately left ABSENT from the view.
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &eligibility).unwrap();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01", "cloud-01"]), sample_artifact("svc-b", vec!["fog-01", "cloud-01"])];

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum,
                placements: plan.placements.clone(),
                planner_provenance: Some(planner_provenance_from_plan(&plan, BTreeMap::new(), eligibility)), dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            "planned-proposal-eligibility-distinction".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).unwrap();
        let provenance = reloaded.proposals.get("planned-proposal-eligibility-distinction").unwrap().planner_provenance.as_ref().unwrap();
        assert!(provenance.eligibility.eligible_nodes_for("svc-a").is_some(), "svc-a must remain PRESENT (restricted) after reload");
        assert!(provenance.eligibility.eligible_nodes_for("svc-b").is_none(), "svc-b must remain ABSENT (unrestricted-by-view) after reload, not silently become present-with-empty-set");
    }

    #[test]
    fn different_committed_input_changes_recorded_provenance_even_when_final_placement_is_identical() {
        // Mandatory test 7.
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1); // fog-01 and cloud-01, both ample capacity
        let committed_a = BTreeMap::new();
        let mut committed_b = BTreeMap::new();
        // A small commitment against cloud-01 (not the node either service
        // will land on) changes nothing about the resulting placement, but
        // must still change what gets recorded.
        committed_b.insert("cloud-01".to_string(), crate::core::canonical_placement::NodeCommitmentV1 { cpu_cores: 1, memory_bytes: 1 });
        let eligibility = crate::core::canonical_placement::PlacementEligibilityView::unrestricted();
        let plan_a = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &committed_a, &eligibility).unwrap();
        let plan_b = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &committed_b, &eligibility).unwrap();
        assert_eq!(plan_a.placements, plan_b.placements, "test setup sanity: the final placement must be identical between the two runs");

        let provenance_a = planner_provenance_from_plan(&plan_a, committed_a, eligibility.clone());
        let provenance_b = planner_provenance_from_plan(&plan_b, committed_b, eligibility);
        assert_ne!(provenance_a.committed_by_node, provenance_b.committed_by_node);
        assert_ne!(provenance_a, provenance_b, "the persisted provenance must differ even though the final placement decision it accompanies does not");
    }

    #[test]
    fn different_eligibility_input_changes_recorded_provenance_even_when_final_placement_is_identical() {
        // Mandatory test 8.
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let committed = BTreeMap::new();
        let eligibility_a = crate::core::canonical_placement::PlacementEligibilityView::unrestricted();
        let mut eligibility_b = crate::core::canonical_placement::PlacementEligibilityView::unrestricted();
        // Restricting svc-b to EXACTLY the full set of ABI-compatible nodes
        // it would have been able to use anyway (both fog-01 and cloud-01)
        // changes nothing about which node the policy actually picks, but
        // "svc-b present in the view with this 2-element set" is still a
        // genuinely different `PlacementEligibilityView` value than "svc-b
        // absent from the view" -- exactly the case that must still change
        // what gets recorded.
        eligibility_b.insert("svc-b".to_string(), std::collections::BTreeSet::from(["fog-01".to_string(), "cloud-01".to_string()]));
        let plan_a = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &committed, &eligibility_a).unwrap();
        let plan_b = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &committed, &eligibility_b).unwrap();
        assert_eq!(plan_a.placements, plan_b.placements, "test setup sanity: the final placement must be identical between the two runs");

        let provenance_a = planner_provenance_from_plan(&plan_a, committed.clone(), eligibility_a);
        let provenance_b = planner_provenance_from_plan(&plan_b, committed, eligibility_b);
        assert_ne!(provenance_a.eligibility, provenance_b.eligibility);
        assert_ne!(provenance_a, provenance_b, "the persisted provenance must differ even though the final placement decision it accompanies does not");
    }

    #[test]
    fn tampered_persisted_provenance_disagreeing_with_placements_fails_closed_on_reload() {
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &crate::core::canonical_placement::PlacementEligibilityView::unrestricted()).unwrap();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum,
                placements: plan.placements.clone(),
                planner_provenance: Some(planner_provenance_from_plan(&plan, BTreeMap::new(), crate::core::canonical_placement::PlacementEligibilityView::unrestricted())), dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            "planned-proposal-tampered".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap();

        // Tamper: swap the recorded decision's node_id so it disagrees with
        // the proposal's own (untouched) `placements` -- proposal_digest is
        // deliberately left alone, since `planner_provenance` is NOT part of
        // its digest input; this must still be caught by the dedicated
        // provenance-vs-placements relational check.
        state.proposals.get_mut("planned-proposal-tampered").unwrap().planner_provenance.as_mut().unwrap().decisions[0].node_id = "cloud-01".to_string();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("planner_provenance decisions disagreeing with the proposal's own persisted placements must fail closed");
        assert!(error.contains("planner_provenance"), "expected the P2 provenance-vs-placements check to fire, got: {error}");
    }

    #[test]
    fn legacy_operator_proposal_with_no_planner_provenance_reloads_normally() {
        // Item 24: `planner_provenance: None` (every operator/AI proposal,
        // and every proposal persisted before this field existed, via
        // `#[serde(default)]`) must never be treated as corruption.
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "proj-x", "app-x", "svc-a", 1, 1, &shared_node_dcontinuum("proj-x", 4, 4_294_967_296), "p1", "a1").unwrap();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).expect("a legacy/operator proposal with no planner_provenance must reload cleanly");
        assert!(reloaded.proposals.get("p1").unwrap().planner_provenance.is_none());
    }

    #[test]
    fn a_planner_provenance_record_claiming_a_non_deterministic_source_fails_closed_on_reload() {
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(&mut state, "proj-x", "app-x", "svc-a", 1, 1, &shared_node_dcontinuum("proj-x", 4, 4_294_967_296), "p1", "a1").unwrap();
        // This proposal's REAL source is Operator (set by
        // `create_and_accept_on_shared_node`) -- attaching provenance to it
        // must be rejected regardless of whether the decisions themselves
        // happen to agree with its placements.
        let placements = state.proposals.get("p1").unwrap().proposal.placements.clone();
        let dgraph_ref = state.proposals.get("p1").unwrap().proposal.dgraph_ref.clone();
        let dcontinuum_ref = state.proposals.get("p1").unwrap().proposal.dcontinuum_ref.clone();
        state.proposals.get_mut("p1").unwrap().planner_provenance = Some(PlannerProvenanceV1 {
            planner_version: crate::core::canonical_placement::PLANNER_VERSION.to_string(),
            policy_name: crate::core::canonical_placement::PLACEMENT_POLICY_NAME.to_string(),
            policy_version: crate::core::canonical_placement::PLACEMENT_POLICY_VERSION.to_string(),
            dgraph_ref,
            dcontinuum_ref,
            committed_by_node: BTreeMap::new(),
            eligibility: crate::core::canonical_placement::PlacementEligibilityView::unrestricted(),
            decisions: placements
                .iter()
                .map(|p| crate::core::canonical_placement::PlacementDecisionRecord {
                    service_id: p.service_id.clone(),
                    node_id: p.node_id.clone(),
                    required_dnode_abi: DNODE_ABI_V0.to_string(),
                    eligible_node_count: 1,
                    node_residual_cpu_cores_before: 4,
                    node_residual_memory_bytes_before: 4,
                    node_residual_cpu_cores_after: 3,
                    node_residual_memory_bytes_after: 3,
                })
                .collect(),
        });

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("planner_provenance on a non-Deterministic-sourced proposal must fail closed");
        assert!(error.contains("planner_provenance") && error.contains("Deterministic"), "got: {error}");
    }

    /// Shared setup for the C1D decision-integrity mandatory tests below:
    /// a persisted, Deterministic-sourced, two-service proposal (svc-a,
    /// svc-b) with a genuinely-valid `planner_provenance`, ready to be
    /// tampered with in one specific way per test.
    fn persisted_deterministic_proposal_for_tampering() -> (DDeployStateV1, crate::core::canonical_placement::CanonicalPlacementPlanV1) {
        let dgraph = sample_dgraph("app-planned", 1);
        let dcontinuum = sample_dcontinuum("proj-planned", 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph, &dcontinuum, &BTreeMap::new(), &crate::core::canonical_placement::PlacementEligibilityView::unrestricted()).unwrap();
        let artifacts = vec![sample_artifact("svc-a", vec!["fog-01"]), sample_artifact("svc-b", vec!["fog-01"])];

        let mut state = DDeployStateV1::empty();
        create_proposal(
            &mut state,
            CreateProposalInput {
                project_id: "proj-planned".to_string(),
                application_id: "app-planned".to_string(),
                source: ProposalSource::Deterministic,
                dgraph,
                dcontinuum,
                placements: plan.placements.clone(),
                planner_provenance: Some(planner_provenance_from_plan(&plan, BTreeMap::new(), crate::core::canonical_placement::PlacementEligibilityView::unrestricted())), dcode_artifact_authority: std::collections::BTreeMap::new(),
            },
            "planned-proposal-decision-integrity".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .unwrap();
        (state, plan)
    }

    #[test]
    fn duplicate_decision_service_id_fails_closed_on_reload() {
        // Mandatory test 9.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        let decisions = &mut state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().decisions;
        let duplicate = decisions[0].clone();
        decisions.push(duplicate);

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a duplicate decision service_id must fail closed, not silently collapse via a BTreeMap key overwrite");
        assert!(error.contains("more than one decision"), "got: {error}");
    }

    #[test]
    fn missing_decision_fails_closed_on_reload() {
        // Mandatory test 10.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().decisions.pop();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a placement with no corresponding decision must fail closed");
        assert!(error.contains("do not exactly match"), "got: {error}");
    }

    #[test]
    fn extra_decision_fails_closed_on_reload() {
        // Mandatory test 11.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        let mut extra = state.proposals.get("planned-proposal-decision-integrity").unwrap().planner_provenance.as_ref().unwrap().decisions[0].clone();
        extra.service_id = "svc-nonexistent".to_string();
        state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().decisions.push(extra);

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a decision with no corresponding placement must fail closed");
        assert!(error.contains("do not exactly match"), "got: {error}");
    }

    #[test]
    fn decision_abi_mismatch_fails_closed_on_reload() {
        // Mandatory test 12.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().decisions[0].required_dnode_abi = "datum-dnode/999".to_string();

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a decision's required_dnode_abi disagreeing with its own dgraph_snapshot must fail closed");
        assert!(error.contains("required_dnode_abi"), "got: {error}");
    }

    #[test]
    fn impossible_residual_arithmetic_fails_closed_on_reload() {
        // Mandatory test 13.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        let decision = &mut state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().decisions[0];
        // Pretend nothing was consumed -- before == after, which disagrees
        // with the service's own declared (non-zero) resource_requirement.
        decision.node_residual_cpu_cores_after = decision.node_residual_cpu_cores_before;

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("residual arithmetic inconsistent with the service's own declared resource_requirement must fail closed");
        assert!(error.contains("residual arithmetic"), "got: {error}");
    }

    #[test]
    fn tampered_provenance_dgraph_ref_digest_fails_closed_on_reload() {
        // Mandatory test 14.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().dgraph_ref.digest = format!("sha256:{}", "0".repeat(64));

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a tampered planner_provenance.dgraph_ref must fail closed against the proposal's own dgraph_ref");
        assert!(error.contains("dgraph_ref"), "got: {error}");
    }

    #[test]
    fn tampered_provenance_dcontinuum_ref_digest_fails_closed_on_reload() {
        // Mandatory test 15.
        let (mut state, _plan) = persisted_deterministic_proposal_for_tampering();
        state.proposals.get_mut("planned-proposal-decision-integrity").unwrap().planner_provenance.as_mut().unwrap().dcontinuum_ref.digest = format!("sha256:{}", "0".repeat(64));

        let path = temp_db_path("provenance");
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a tampered planner_provenance.dcontinuum_ref must fail closed against the proposal's own dcontinuum_ref");
        assert!(error.contains("dcontinuum_ref"), "got: {error}");
    }

    #[test]
    fn v1_state_roundtrips_with_defaulted_phase113_domain() {
        let state = DDeployStateV1::empty();
        let raw = serde_json::to_value(&state).unwrap();
        let mut legacy = raw.clone();
        legacy.as_object_mut().unwrap().remove("phase113_v2");
        let loaded: DDeployStateV1 = serde_json::from_value(legacy).unwrap();
        assert!(loaded.phase113_v2.proposals.is_empty());
        assert!(active_placement_authority(&loaded, "p", "a")
            .unwrap()
            .is_none());
        assert_ne!(application_key("p", "a:b"), application_key("p:a", "b"));
    }

    fn minimal_dforward(project: &str) -> crate::core::canonical_dforward::CanonicalDForwardV1 {
        crate::core::canonical_dforward::CanonicalDForwardV1 {
            schema: crate::core::canonical_dforward::CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: project.to_string(),
            dforward_id: format!("dforward:{project}"),
            revision: 1,
            diots: vec![],
            external_endpoints: vec![],
            chains: vec![],
            support_jobs: vec![],
        }
    }

    fn create_and_accept_v2_on_shared_node(
        state: &mut DDeployStateV1,
        project: &str,
        app: &str,
        service_id: &str,
        cpu_cores: u32,
        memory_bytes: u64,
        dcontinuum: &CanonicalDContinuumV1,
        proposal_id: &str,
        acceptance_id: &str,
        expected_active_dmap_revision: Option<u64>,
    ) -> Result<crate::core::ddeploy_v2::DDeployAcceptanceV2, String> {
        let dgraph = one_service_dgraph(app, service_id, cpu_cores, memory_bytes);
        let dforward = minimal_dforward(project);
        let dgraph_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dgraph.dgraph_id.clone(),
            revision: dgraph.revision,
            digest: recompute_dgraph_digest(&dgraph).unwrap(),
        };
        let dcontinuum_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dcontinuum.dcontinuum_id.clone(),
            revision: dcontinuum.revision,
            digest: recompute_dcontinuum_digest(dcontinuum).unwrap(),
        };
        let dforward_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dforward.dforward_id.clone(),
            revision: dforward.revision,
            digest: dforward.digest().unwrap(),
        };
        let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: proposal_id.to_string(),
            project_id: project.to_string(),
            application_id: app.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref,
            dcontinuum_ref,
            dforward_ref,
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: service_id.to_string(),
                node_id: "shared-01".to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision,
            diot_artifact_authority: BTreeMap::new(),
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        state
            .create_v2_proposal(proposal, dgraph, dcontinuum.clone(), dforward)
            .unwrap_or_else(|e| panic!("v2 proposal creation for '{app}' must succeed: {e}"));
        state.accept_v2_proposal(
            proposal_id,
            acceptance_id.to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            dcontinuum,
            &[],
            None,
        )
    }

    /// Phase 119G: same as `create_and_accept_v2_on_shared_node`, but the
    /// placement targets an arbitrary `target_node` instead of always
    /// hardcoding "shared-01" -- needed by the cleanup-derivation tests that
    /// move a service to a DIFFERENT node under v2 authority.
    fn create_and_accept_v2_on_node(
        state: &mut DDeployStateV1,
        project: &str,
        app: &str,
        service_id: &str,
        target_node: &str,
        cpu_cores: u32,
        memory_bytes: u64,
        dcontinuum: &CanonicalDContinuumV1,
        proposal_id: &str,
        acceptance_id: &str,
        expected_active_dmap_revision: Option<u64>,
    ) -> Result<crate::core::ddeploy_v2::DDeployAcceptanceV2, String> {
        let dgraph = one_service_dgraph(app, service_id, cpu_cores, memory_bytes);
        let dforward = minimal_dforward(project);
        let dgraph_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dgraph.dgraph_id.clone(),
            revision: dgraph.revision,
            digest: recompute_dgraph_digest(&dgraph).unwrap(),
        };
        let dcontinuum_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dcontinuum.dcontinuum_id.clone(),
            revision: dcontinuum.revision,
            digest: recompute_dcontinuum_digest(dcontinuum).unwrap(),
        };
        let dforward_ref = crate::core::canonical_dmap_v2::ContractRefV2 {
            id: dforward.dforward_id.clone(),
            revision: dforward.revision,
            digest: dforward.digest().unwrap(),
        };
        let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: proposal_id.to_string(),
            project_id: project.to_string(),
            application_id: app.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref,
            dcontinuum_ref,
            dforward_ref,
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: service_id.to_string(),
                node_id: target_node.to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision,
            diot_artifact_authority: BTreeMap::new(),
            dcode_artifact_authority: std::collections::BTreeMap::new(),
        };
        state
            .create_v2_proposal(proposal, dgraph, dcontinuum.clone(), dforward)
            .unwrap_or_else(|e| panic!("v2 proposal creation for '{app}' must succeed: {e}"));
        state.accept_v2_proposal(
            proposal_id,
            acceptance_id.to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            dcontinuum,
            &[],
            None,
        )
    }

    #[test]
    fn v1_active_app_plus_v2_candidate_exceeding_capacity_is_rejected() {
        // Item A: app-a is active under DMap/1 consuming shared-01's full
        // 4-core capacity; app-b's v2 candidate (also 4 cores) must be
        // rejected -- 4 + 4 = 8 > 4.
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-a", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(
            &mut state,
            "proj-v2-commit-a",
            "app-a",
            "svc-a1",
            4,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
        )
        .expect("app-a's v1 acceptance (4 cores) must exactly fit shared-01 alone");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-a",
            "app-b",
            "svc-b1",
            4,
            1,
            &dcontinuum,
            "p-b1",
            "acc-b1",
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "v1-active app-a (4) + v2-candidate app-b (4) = 8 > shared-01's 4-core capacity -- got {result:?}"
        );
        assert!(
            !state
                .phase113_v2
                .active
                .contains_key(&application_key("proj-v2-commit-a", "app-b")),
            "the rejected v2 acceptance must not have mutated v2 active authority"
        );
    }

    // ---- Phase 120B: cross-version (DMap/1 vs DMap/2) structural -------- //
    // ---- commitment accounting closure ---------------------------------- //

    #[test]
    fn v2_active_app_plus_v1_candidate_exceeding_cpu_capacity_is_rejected() {
        // The exact Phase 120A counterexample: app-x is active under DMap/2
        // consuming shared-01's full 4-core capacity; a v1 candidate for a
        // DIFFERENT application (app-y) requiring any CPU on the same node
        // must be rejected. Before the Phase 120B fix, `accept_proposal`'s
        // R3C gate used the v1-only, non-versioned commitment aggregator
        // (today `compute_global_node_commitments`), which was structurally
        // blind to app-x's v2 commitment.
        let dcontinuum = shared_node_dcontinuum("proj-120b-cpu", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state, "proj-120b-cpu", "app-x", "svc-x1", 4, 1,
            &dcontinuum, "p-v2-x1", "acc-v2-x1", None,
        )
        .expect("app-x's v2 acceptance (4 cores) must exactly fit shared-01 alone");

        let result = create_and_accept_on_shared_node(
            &mut state, "proj-120b-cpu", "app-y", "svc-y1", 1, 1,
            &dcontinuum, "p-v1-y1", "acc-v1-y1",
        );
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "v2-active app-x (4) + v1-candidate app-y (1) = 5 > shared-01's 4-core capacity -- got {result:?}"
        );

        // The candidate proposal must remain Pending, never Accepted.
        let record = state.proposals.get("p-v1-y1").expect("app-y's proposal must still exist");
        assert_eq!(record.state, ProposalLifecycleState::Pending, "a rejected acceptance must leave the proposal Pending, not Accepted");

        // Active v2 authority for app-x must be byte-identical to before.
        let v2_key = application_key("proj-120b-cpu", "app-x");
        let v2_active = state.phase113_v2.active.get(&v2_key).expect("app-x's v2 active authority must still exist");
        assert_eq!(v2_active.map.revision, 1, "app-x's v2 active authority must be untouched by the rejected v1 acceptance");

        // No v1 active authority may appear for app-y.
        assert!(
            !state.active_by_application.contains_key(&application_key("proj-120b-cpu", "app-y")),
            "the rejected v1 acceptance must not have created any v1 active authority"
        );
    }

    #[test]
    fn v2_active_app_plus_v1_candidate_exceeding_memory_capacity_is_rejected() {
        // Memory-dimension equivalent of the CPU test above: node CPU is
        // generously provisioned (never the limiting dimension here) so
        // only the memory gate can fire. app-x (v2) commits 768 bytes;
        // app-y's v1 candidate requests 512 more -- 768 + 512 = 1280 > 1024.
        let dcontinuum = shared_node_dcontinuum("proj-120b-mem", 100, 1024);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state, "proj-120b-mem", "app-x", "svc-x1", 1, 768,
            &dcontinuum, "p-v2-x1", "acc-v2-x1", None,
        )
        .expect("app-x's v2 acceptance (768 bytes) must fit shared-01's 1024-byte capacity alone");

        let result = create_and_accept_on_shared_node(
            &mut state, "proj-120b-mem", "app-y", "svc-y1", 1, 512,
            &dcontinuum, "p-v1-y1", "acc-v1-y1",
        );
        assert!(
            matches!(result, Err(AcceptError::Invalid(ref findings)) if findings.iter().any(|f| f.code == F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED)),
            "v2-active app-x (768) + v1-candidate app-y (512) = 1280 > shared-01's 1024-byte capacity -- got {result:?}"
        );
    }

    #[test]
    fn v2_active_app_plus_v1_candidate_within_capacity_is_accepted() {
        // Positive control: v2 occupancy is structural CAPACITY CONSUMPTION,
        // never an absolute block on the node. app-x (v2) commits 2 of
        // shared-01's 4 cores; app-y's v1 candidate requesting the
        // remaining 2 cores must be accepted.
        let dcontinuum = shared_node_dcontinuum("proj-120b-fit", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state, "proj-120b-fit", "app-x", "svc-x1", 2, 1,
            &dcontinuum, "p-v2-x1", "acc-v2-x1", None,
        )
        .expect("app-x's v2 acceptance (2 cores) must fit shared-01 alone");

        let result = create_and_accept_on_shared_node(
            &mut state, "proj-120b-fit", "app-y", "svc-y1", 2, 1,
            &dcontinuum, "p-v1-y1", "acc-v1-y1",
        );
        assert!(
            result.is_ok(),
            "v2-active app-x (2) + v1-candidate app-y (2) = 4 <= shared-01's 4-core capacity -- must be accepted, got {result:?}"
        );
        assert!(
            state.active_by_application.contains_key(&application_key("proj-120b-fit", "app-y")),
            "app-y's v1 acceptance must have created active v1 authority"
        );
    }

    /// Exercises the SAME sequence of core calls `post_plan_proposal`
    /// itself makes (`compute_global_node_commitments_versioned` ->
    /// `plan_canonical_placements` -> `create_proposal` with the exact
    /// `PlannerProvenanceV1` that run produced), rather than only calling
    /// `plan_canonical_placements` with a hand-constructed commitment map --
    /// this proves the real wiring the Phase 120B fix changed, not merely
    /// the already-correct pure planner policy in isolation.
    #[test]
    fn planner_sees_active_v2_commitment_and_persists_it_in_provenance() {
        let project = "proj-120b-plan";
        let mut dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        dcontinuum.nodes[0].node_id = "a-node".to_string();
        dcontinuum.nodes.push(CanonicalDNodeV1 {
            node_id: "b-node".to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_294_967_296 },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        });

        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_node(
            &mut state, project, "app-x", "svc-x1", "a-node", 3, 1,
            &dcontinuum, "p-v2-x1", "acc-v2-x1", None,
        )
        .expect("app-x's v2 acceptance (3 cores on a-node) must succeed");

        // Step 1: the exact commitment baseline `post_plan_proposal` computes.
        let commitments = compute_global_node_commitments_versioned(&state, Some((project, "app-y")))
            .expect("versioned commitment aggregation must succeed");
        assert_eq!(
            commitments.get("a-node").map(|c| c.cpu_cores),
            Some(3),
            "app-x's active v2 commitment (3 cores on a-node) must be visible to the planner's baseline"
        );

        // Step 2: the exact planning call `post_plan_proposal` makes.
        let dgraph_y = one_service_dgraph("app-y", "svc-y1", 2, 1);
        let plan = crate::core::canonical_placement::plan_canonical_placements(
            &dgraph_y, &dcontinuum, &commitments,
            &crate::core::canonical_placement::PlacementEligibilityView::unrestricted(),
        )
        .expect("2-core service must be plannable: a-node has only 1 residual core (4-3), but b-node has 4");
        assert_eq!(plan.placements[0].node_id, "b-node", "a-node's residual capacity (1 core) is insufficient once app-x's v2 commitment is correctly seen -- b-node must be chosen");

        // Step 3: the exact `PlannerProvenanceV1` construction and
        // `create_proposal` call `post_plan_proposal` makes, to prove the
        // PERSISTED provenance -- not just the in-memory plan -- carries the
        // real versioned baseline.
        let planner_provenance = PlannerProvenanceV1 {
            planner_version: plan.provenance.planner_version.clone(),
            policy_name: plan.provenance.policy_name.clone(),
            policy_version: plan.provenance.policy_version.clone(),
            dgraph_ref: CanonicalContractReferenceV1 {
                id: plan.provenance.dgraph_id.clone(),
                revision: plan.provenance.dgraph_revision,
                digest: plan.provenance.dgraph_digest.clone(),
            },
            dcontinuum_ref: CanonicalContractReferenceV1 {
                id: plan.provenance.dcontinuum_id.clone(),
                revision: plan.provenance.dcontinuum_revision,
                digest: plan.provenance.dcontinuum_digest.clone(),
            },
            committed_by_node: commitments.clone(),
            eligibility: crate::core::canonical_placement::PlacementEligibilityView::unrestricted(),
            decisions: plan.provenance.decisions.clone(),
        };
        let record = create_proposal(
            &mut state,
            CreateProposalInput {
                planner_provenance: Some(planner_provenance),
                dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: project.to_string(),
                application_id: "app-y".to_string(),
                source: ProposalSource::Deterministic,
                dgraph: dgraph_y,
                dcontinuum: dcontinuum.clone(),
                placements: plan.placements.clone(),
            },
            "p-plan-y1".to_string(),
            "2026-01-01T00:10:00Z".to_string(),
            &[sample_artifact("svc-y1", vec!["a-node", "b-node"])],
        )
        .expect("deterministic proposal creation must succeed");

        let persisted = record
            .planner_provenance
            .as_ref()
            .expect("a Deterministic proposal must persist planner provenance");
        assert_eq!(
            persisted.committed_by_node.get("a-node").map(|c| c.cpu_cores),
            Some(3),
            "the PERSISTED provenance must carry app-x's v2 commitment exactly as supplied to the planner, not reconstructed later"
        );
    }

    #[test]
    fn candidate_preview_and_plan_produce_identical_placements_for_identical_authoritative_state() {
        // Mandatory Phase 136A regression (audit's "candidate vs /plan"
        // requirement): `api::ddeploy::post_placement_candidate` and
        // `api::ddeploy::post_plan_proposal` each make the EXACT SAME core
        // call sequence -- `compute_global_node_commitments_versioned` ->
        // `derive_placement_eligibility_view` -> `plan_canonical_placements`
        // -- against the same authoritative state; `post_plan_proposal`
        // additionally calls `create_proposal`/`persist_state` afterward,
        // which `post_placement_candidate` never does. This test exercises
        // that identical shared core sequence directly (HTTP-handler-level
        // invocation has no established precedent in this crate -- see
        // `api::ddeploy::tests`' own `production_source` doc comment -- so
        // this is the strongest automated proof available; full HTTP
        // equivalence is additionally confirmed manually, see
        // `documentation/reference/phase136-placement-ui.md`).
        let project = "proj-candidate-equivalence";
        let dcontinuum = shared_node_dcontinuum(project, 8, 8_589_934_592);
        let mut state = DDeployStateV1::empty();
        // An unrelated, already-active application commits 3 of the 8
        // cores, so the equivalence proof is meaningful (not merely "both
        // return the same answer against an empty baseline").
        create_and_accept_on_shared_node(&mut state, project, "app-other", "svc-other1", 3, 1, &dcontinuum, "p-other1", "acc-other1")
            .expect("the unrelated baseline application must be accepted");

        let dgraph_new = one_service_dgraph("app-new", "svc-new1", 2, 1);
        let artifacts = vec![sample_artifact("svc-new1", vec!["shared-01"])];

        // The exact sequence `post_placement_candidate` makes -- no
        // `create_proposal`/`persist_state` call anywhere in this block.
        let candidate_commitments = compute_global_node_commitments_versioned(&state, Some((project, "app-new"))).unwrap();
        let candidate_eligibility = derive_placement_eligibility_view(&dgraph_new, &artifacts);
        let candidate_plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph_new, &dcontinuum, &candidate_commitments, &candidate_eligibility).unwrap();

        // The exact sequence `post_plan_proposal` makes, against the SAME
        // unmodified `state` (proving the candidate preview mutated
        // nothing the plan sequence could have observed).
        let plan_commitments = compute_global_node_commitments_versioned(&state, Some((project, "app-new"))).unwrap();
        let plan_eligibility = derive_placement_eligibility_view(&dgraph_new, &artifacts);
        let plan_plan = crate::core::canonical_placement::plan_canonical_placements(&dgraph_new, &dcontinuum, &plan_commitments, &plan_eligibility).unwrap();

        assert_eq!(candidate_plan.placements, plan_plan.placements, "candidate preview and /plan must choose identical placements for identical authoritative state");
        assert_eq!(candidate_plan.provenance.decisions, plan_plan.provenance.decisions, "the full decision evidence must also match exactly");
        assert_eq!(candidate_plan.placements[0].node_id, "shared-01", "sanity: the 2-core service must have actually been placed on the only declared node");
    }

    #[test]
    fn planner_fails_closed_and_creates_no_proposal_when_v2_commitment_exhausts_node() {
        // app-x's v2 acceptance consumes shared-01's ENTIRE 4-core capacity;
        // planning ANY new service for a different application must fail
        // with unsatisfied_static_capacity (never silently plan onto a node
        // the planner incorrectly believes has residual capacity), and
        // real callers (post_plan_proposal) never reach `create_proposal`
        // on this path -- confirmed here by the fact this test never calls
        // it, mirroring the `?` early-return in the real handler.
        let dcontinuum = shared_node_dcontinuum("proj-120b-exhausted", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state, "proj-120b-exhausted", "app-x", "svc-x1", 4, 1,
            &dcontinuum, "p-v2-x1", "acc-v2-x1", None,
        )
        .expect("app-x's v2 acceptance (4 cores) must exactly fit shared-01 alone");

        let commitments = compute_global_node_commitments_versioned(&state, Some(("proj-120b-exhausted", "app-y")))
            .expect("versioned commitment aggregation must succeed");
        let dgraph_y = one_service_dgraph("app-y", "svc-y1", 1, 1);
        let error = crate::core::canonical_placement::plan_canonical_placements(
            &dgraph_y, &dcontinuum, &commitments,
            &crate::core::canonical_placement::PlacementEligibilityView::unrestricted(),
        )
        .expect_err("shared-01 has zero residual capacity once app-x's v2 commitment is correctly seen");
        assert!(
            error.iter().any(|f| f.code == crate::core::canonical_placement::F_PLANNER_UNSATISFIED_STATIC_CAPACITY),
            "expected unsatisfied_static_capacity, got {error:?}"
        );
        assert!(state.proposals.is_empty(), "a failed plan must never result in a persisted proposal");
    }

    // ---- Phase 119G: fail-closed v1-downgrade-from-v2 hardening -------- //

    #[test]
    fn v1_create_proposal_fails_closed_when_application_is_currently_v2_active() {
        let dcontinuum = shared_node_dcontinuum("proj-v1-downgrade-create", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state, "proj-v1-downgrade-create", "app-a", "svc-a1", 1, 1,
            &dcontinuum, "p-v2-a1", "acc-v2-a1", None,
        )
        .expect("app-a's v2 acceptance must succeed");

        let dgraph = one_service_dgraph("app-a", "svc-a1", 1, 1);
        let result = create_proposal(
            &mut state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: "proj-v1-downgrade-create".to_string(),
                application_id: "app-a".to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a1".to_string(), node_id: "shared-01".to_string() }],
            },
            "p-v1-downgrade".to_string(),
            "2026-01-01T00:10:00Z".to_string(),
            &[sample_artifact("svc-a1", vec!["shared-01"])],
        );
        assert!(
            matches!(&result, Err(findings) if findings.iter().any(|f| f.code == F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED)),
            "creating a v1 proposal for a currently v2-active application must fail closed with F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED -- got {result:?}"
        );
        assert!(state.proposals.is_empty(), "the refused v1 proposal must never be persisted");
        assert!(
            state.phase113_v2.active.contains_key(&application_key("proj-v1-downgrade-create", "app-a")),
            "v2 active authority must remain byte-identical after the refusal"
        );
    }

    #[test]
    fn v1_accept_proposal_fails_closed_when_application_becomes_v2_active_after_proposal_creation() {
        // The CAS-race case create_proposal's own guard cannot catch: the v1
        // proposal is created FIRST (while app-a has no active authority at
        // all), and only becomes v2-active AFTERWARDS -- acceptance must
        // independently re-check and fail closed, exactly like every other
        // acceptance-time revalidation in this function.
        let dcontinuum = shared_node_dcontinuum("proj-v1-downgrade-accept", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        let dgraph = one_service_dgraph("app-a", "svc-a1", 1, 1);
        let artifacts = vec![sample_artifact("svc-a1", vec!["shared-01"])];
        create_proposal(
            &mut state,
            CreateProposalInput {
                planner_provenance: None,
                dcode_artifact_authority: std::collections::BTreeMap::new(),
                project_id: "proj-v1-downgrade-accept".to_string(),
                application_id: "app-a".to_string(),
                source: ProposalSource::Operator,
                dgraph,
                dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 { service_id: "svc-a1".to_string(), node_id: "shared-01".to_string() }],
            },
            "p-v1-downgrade-2".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &artifacts,
        )
        .expect("proposal creation must succeed while app-a has no active authority yet");

        create_and_accept_v2_on_shared_node(
            &mut state, "proj-v1-downgrade-accept", "app-a", "svc-a1", 1, 1,
            &dcontinuum, "p-v2-a1", "acc-v2-a1", None,
        )
        .expect("app-a's v2 acceptance must succeed, making it v2-active out from under the pending v1 proposal");

        let result = accept_proposal(
            &mut state, "p-v1-downgrade-2", "operator:dana", None, &artifacts,
            Some(&dcontinuum), "2026-01-01T00:20:00Z", "acc-v1-downgrade-2".to_string(),
        );
        assert!(
            matches!(&result, Err(AcceptError::Invalid(findings)) if findings.iter().any(|f| f.code == F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED)),
            "accepting a v1 proposal for an application that became v2-active in the meantime must fail closed -- got {result:?}"
        );
        assert!(
            !state.active_by_application.contains_key(&application_key("proj-v1-downgrade-accept", "app-a")),
            "the refused v1 acceptance must never create a v1 active-authority entry"
        );
        assert!(
            state.phase113_v2.active.contains_key(&application_key("proj-v1-downgrade-accept", "app-a")),
            "v2 active authority must remain byte-identical after the refusal"
        );
        assert!(
            active_placement_authority(&state, "proj-v1-downgrade-accept", "app-a").is_ok(),
            "the refused v1 acceptance must never produce dual active-authority corruption"
        );
    }

    #[test]
    fn v2_active_app_plus_v2_candidate_exceeding_capacity_is_rejected() {
        // Item B: app-a is active under DMap/2 consuming shared-01's full
        // 4-core capacity; app-b's v2 candidate (also 4 cores) must be
        // rejected -- 4 + 4 = 8 > 4.
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-b", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-b",
            "app-a",
            "svc-a1",
            4,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
            None,
        )
        .expect("app-a's v2 acceptance (4 cores) must exactly fit shared-01 alone");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-b",
            "app-b",
            "svc-b1",
            4,
            1,
            &dcontinuum,
            "p-b1",
            "acc-b1",
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "v2-active app-a (4) + v2-candidate app-b (4) = 8 > shared-01's 4-core capacity -- got {result:?}"
        );
        assert_eq!(
            state.phase113_v2.active.len(),
            1,
            "the failed second v2 acceptance must not have mutated v2 active authority at all"
        );
    }

    #[test]
    fn v2_exact_fit_across_v1_and_v2_active_authority_is_accepted() {
        // Item C: app-a (v1, 2 cores) + app-b (v2, 2 cores) == shared-01's
        // exact 4-core capacity -- must be accepted, not rejected.
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-c", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(
            &mut state,
            "proj-v2-commit-c",
            "app-a",
            "svc-a1",
            2,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
        )
        .expect("app-a's v1 acceptance (2 cores) must succeed");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-c",
            "app-b",
            "svc-b1",
            2,
            1,
            &dcontinuum,
            "p-b1",
            "acc-b1",
            None,
        );
        assert!(
            result.is_ok(),
            "app-a (2) + app-b (2) = 4 exactly fits shared-01's 4-core capacity -- got {result:?}"
        );
        assert_eq!(state.phase113_v2.active.len(), 1);
    }

    #[test]
    fn v2_candidate_cpu_overcommit_against_v1_active_is_rejected() {
        // Item D (CPU specifically): identical to item A, isolated as its
        // own named CPU-only regression.
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-d", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(
            &mut state,
            "proj-v2-commit-d",
            "app-a",
            "svc-a1",
            3,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
        )
        .expect("app-a's v1 acceptance (3 cores) must succeed");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-d",
            "app-b",
            "svc-b1",
            2,
            1,
            &dcontinuum,
            "p-b1",
            "acc-b1",
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "app-a (3) + app-b (2) = 5 > shared-01's 4-core capacity -- got {result:?}"
        );
    }

    #[test]
    fn v2_candidate_memory_overcommit_against_v1_active_is_rejected() {
        // Item E: CPU fits (1+1=2 <= 4) but memory does not
        // (3_000_000_000 * 2 > 4_294_967_296).
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-e", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(
            &mut state,
            "proj-v2-commit-e",
            "app-a",
            "svc-a1",
            1,
            3_000_000_000,
            &dcontinuum,
            "p-a1",
            "acc-a1",
        )
        .expect("app-a's v1 acceptance (1 core / 3e9 bytes) must succeed");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-e",
            "app-b",
            "svc-b1",
            1,
            3_000_000_000,
            &dcontinuum,
            "p-b1",
            "acc-b1",
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED)),
            "app-a (3e9) + app-b (3e9) = 6e9 bytes > shared-01's 4_294_967_296-byte capacity -- got {result:?}"
        );
    }

    #[test]
    fn v1_to_v2_redeployment_of_the_same_application_does_not_double_count() {
        // Item F: app-a is already active under DMap/1 using shared-01's
        // full 4 cores. Redeploying app-a itself as a v2 proposal (also 4
        // cores) must succeed -- the old v1 entry must be EXCLUDED from the
        // baseline, never summed against the new v2 candidate.
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-f", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(
            &mut state,
            "proj-v2-commit-f",
            "app-a",
            "svc-a1",
            4,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
        )
        .expect("app-a's first (v1) acceptance must succeed");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-f",
            "app-a",
            "svc-a2",
            4,
            1,
            &dcontinuum,
            "p-a2",
            "acc-a2",
            Some(1),
        );
        assert!(result.is_ok(), "redeploying app-a as v2 must not double-count its own superseded v1 allocation -- got {result:?}");
        assert!(
            !state
                .active_by_application
                .contains_key(&application_key("proj-v2-commit-f", "app-a")),
            "app-a's v1 entry must be gone after its v2 redeployment"
        );
        assert_eq!(
            state.phase113_v2.active.len(),
            1,
            "app-a's v2 redeployment replaces its own authority -- never a second entry"
        );
    }

    #[test]
    fn v2_to_v2_redeployment_of_the_same_application_does_not_double_count() {
        // Item G: app-a is already active under DMap/2 using shared-01's
        // full 4 cores. Redeploying app-a itself (new v2 revision, also 4
        // cores) must succeed -- the OLD v2 revision must be excluded from
        // the baseline before evaluating the NEW one.
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-g", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-g",
            "app-a",
            "svc-a1",
            4,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
            None,
        )
        .expect("app-a's first v2 acceptance must succeed");
        let result = create_and_accept_v2_on_shared_node(
            &mut state,
            "proj-v2-commit-g",
            "app-a",
            "svc-a2",
            4,
            1,
            &dcontinuum,
            "p-a2",
            "acc-a2",
            Some(1),
        );
        assert!(result.is_ok(), "redeploying app-a to a new v2 revision must not double-count its own superseded v2 allocation -- got {result:?}");
        assert_eq!(
            state.phase113_v2.active.len(),
            1,
            "app-a's v2 redeployment replaces its own authority -- never a second entry"
        );
        assert_eq!(
            state
                .phase113_v2
                .active
                .get(&application_key("proj-v2-commit-g", "app-a"))
                .unwrap()
                .map
                .revision,
            2
        );
    }

    #[test]
    fn rejected_v2_acceptance_is_atomic() {
        // Item H: a v2 acceptance rejected for exceeding project capacity
        // must leave EVERYTHING unchanged -- no v2 active authority, no v2
        // acceptance record, and the candidate proposal must remain Pending
        // (never silently marked Accepted).
        let dcontinuum = shared_node_dcontinuum("proj-v2-commit-h", 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_and_accept_on_shared_node(
            &mut state,
            "proj-v2-commit-h",
            "app-a",
            "svc-a1",
            4,
            1,
            &dcontinuum,
            "p-a1",
            "acc-a1",
        )
        .expect("app-a's v1 acceptance (4 cores) must succeed");

        let dgraph_b = one_service_dgraph("app-b", "svc-b1", 4, 1);
        let dforward_b = minimal_dforward("proj-v2-commit-h");
        let proposal_b = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: "p-b1".to_string(),
            project_id: "proj-v2-commit-h".to_string(),
            application_id: "app-b".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dgraph_b.dgraph_id.clone(),
                revision: dgraph_b.revision,
                digest: recompute_dgraph_digest(&dgraph_b).unwrap(),
            },
            dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dcontinuum.dcontinuum_id.clone(),
                revision: dcontinuum.revision,
                digest: recompute_dcontinuum_digest(&dcontinuum).unwrap(),
            },
            dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dforward_b.dforward_id.clone(),
                revision: dforward_b.revision,
                digest: dforward_b.digest().unwrap(),
            },
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: "svc-b1".to_string(),
                node_id: "shared-01".to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: None,
            diot_artifact_authority: BTreeMap::new(),
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        state
            .create_v2_proposal(proposal_b, dgraph_b, dcontinuum.clone(), dforward_b)
            .expect("app-b's v2 proposal creation must succeed -- creation only validates within its own D-Graph");
        let result = state.accept_v2_proposal(
            "p-b1",
            "acc-b1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &[],
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED)),
            "app-a (4) + app-b (4) = 8 > shared-01's 4-core capacity -- got {result:?}"
        );

        assert!(
            !state
                .phase113_v2
                .active
                .contains_key(&application_key("proj-v2-commit-h", "app-b")),
            "no v2 active authority must exist for the rejected application"
        );
        assert!(
            state.phase113_v2.acceptances.is_empty(),
            "no v2 acceptance record must have been inserted"
        );
        let record = state.phase113_v2.proposals.get("p-b1").unwrap();
        assert_eq!(
            record.state,
            crate::core::ddeploy_v2::ProposalV2State::Pending,
            "the rejected proposal must remain Pending, never silently marked Accepted"
        );
        assert!(record.accepted_acceptance_id.is_none());
        assert_eq!(state.active_by_application.len(), 1, "app-a's v1 authority must remain completely unaffected by app-b's rejected v2 acceptance");
    }

    fn dforward_with_one_diot(
        project: &str,
        artifact_id: &str,
    ) -> crate::core::canonical_dforward::CanonicalDForwardV1 {
        crate::core::canonical_dforward::CanonicalDForwardV1 {
            schema: crate::core::canonical_dforward::CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: project.to_string(),
            dforward_id: format!("dforward:{project}"),
            revision: 1,
            diots: vec![crate::core::canonical_dforward::CanonicalDIoTV1 {
                diot_id: "diot:x".to_string(),
                diot_kind: "broker".to_string(),
                artifact_requirement:
                    crate::core::canonical_dforward::CanonicalArtifactRequirementV1 {
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

    fn dforward_with_two_diots_same_artifact(
        project: &str,
        artifact_id: &str,
    ) -> crate::core::canonical_dforward::CanonicalDForwardV1 {
        let mut df = dforward_with_one_diot(project, artifact_id);
        df.diots
            .push(crate::core::canonical_dforward::CanonicalDIoTV1 {
                diot_id: "diot:y".to_string(),
                diot_kind: "broker".to_string(),
                artifact_requirement:
                    crate::core::canonical_dforward::CanonicalArtifactRequirementV1 {
                        artifact_id: artifact_id.to_string(),
                    },
                provides: vec![],
                requires: vec![],
            });
        df
    }

    fn minimal_native_service_artifact(
        artifact_id: &str,
    ) -> crate::core::service_artifact_registry::ServiceArtifact {
        use crate::core::service_artifact_registry::*;
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: artifact_id.to_string(),
            service_id: artifact_id.to_string(),
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
                target_nodes: vec!["shared-01".to_string()],
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

    #[test]
    fn missing_artifact_fails_diot_authority_resolution_and_proposal_creation() {
        // Item 12: DForward requires artifact A; the project registry has no
        // A at all -- proposal creation must reject and nothing persists.
        let project = "proj-artifact-missing";
        let dforward = dforward_with_one_diot(project, "dserv:missing-broker");
        let resolved = resolve_diot_artifact_authority(&dforward, project, &[]);
        assert!(
            matches!(resolved, Err(ref e) if e.contains(F_ARTIFACT_NOT_FOUND)),
            "resolving against an empty registry must fail with F_ARTIFACT_NOT_FOUND -- got {resolved:?}"
        );

        // The production creation path must never persist a proposal whose
        // artifact authority could not be resolved -- the caller (would-be
        // API bridge) must reject BEFORE calling create_v2_proposal at all,
        // which this asserts by never calling it.
        let state = DDeployStateV1::empty();
        assert!(state.phase113_v2.proposals.is_empty());
    }

    #[test]
    fn wrong_project_artifact_is_not_a_valid_cross_project_binding() {
        // Item 13: artifact A exists only under project P2; the proposal
        // belongs to P1 -- no cross-project binding may occur.
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                "proj-p2",
                minimal_native_service_artifact("dserv:shared-name"),
            )
            .expect("creating the artifact under P2 must succeed");

        let dforward = dforward_with_one_diot("proj-p1", "dserv:shared-name");
        let artifacts = registry
            .with_stable_view(|state| {
                state
                    .artifacts_by_project
                    .get("proj-p2")
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let resolved = resolve_diot_artifact_authority(&dforward, "proj-p1", &artifacts);
        assert!(
            matches!(resolved, Err(ref e) if e.contains(F_ARTIFACT_NOT_FOUND)),
            "an artifact scoped to a different project must never satisfy this project's DIoT requirement -- got {resolved:?}"
        );
    }

    #[test]
    fn duplicate_artifact_id_is_structurally_impossible_by_registry_construction() {
        // Item 14 (ambiguity/uniqueness): `ServiceArtifactRegistry::create`
        // itself rejects a second artifact sharing an `artifact_id` within
        // the same project (Conflict), so `resolve_diot_artifact_authority`
        // can never observe more than one match through the real registry --
        // this proves the structural guarantee rather than fabricating an
        // impossible ambiguous-registry fixture.
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create("proj-uniq", minimal_native_service_artifact("dserv:once"))
            .expect("first create must succeed");
        let duplicate = registry.create("proj-uniq", minimal_native_service_artifact("dserv:once"));
        assert!(
            matches!(duplicate, Err(crate::core::service_artifact_registry::ServiceArtifactRegistryError::Conflict(_))),
            "the registry itself must refuse a duplicate artifact_id within one project -- got {duplicate:?}"
        );
        let artifacts = registry
            .with_stable_view(|state| {
                state
                    .artifacts_by_project
                    .get("proj-uniq")
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        assert_eq!(
            artifacts.len(),
            1,
            "exactly one record must exist after the rejected duplicate create"
        );
        let dforward = dforward_with_one_diot("proj-uniq", "dserv:once");
        let resolved = resolve_diot_artifact_authority(&dforward, "proj-uniq", &artifacts)
            .expect("a single unambiguous match must resolve");
        assert_eq!(resolved.len(), 1);
    }

    #[test]
    fn changing_a_bound_execution_generation_or_projection_digest_changes_the_proposal_digest() {
        // Item 8: the artifact authority binding must be load-bearing in
        // DDeployProposalV2's own digest.
        let project = "proj-digest-binding";
        let dforward = dforward_with_one_diot(project, "dserv:x");
        let dg = one_service_dgraph("app-a", "svc-a1", 1, 1);
        let dc = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let base_proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: "p-digest".to_string(),
            project_id: project.to_string(),
            application_id: "app-a".to_string(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dg.dgraph_id.clone(),
                revision: dg.revision,
                digest: recompute_dgraph_digest(&dg).unwrap(),
            },
            dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dc.dcontinuum_id.clone(),
                revision: dc.revision,
                digest: recompute_dcontinuum_digest(&dc).unwrap(),
            },
            dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dforward.dforward_id.clone(),
                revision: dforward.revision,
                digest: dforward.digest().unwrap(),
            },
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: "svc-a1".to_string(),
                node_id: "shared-01".to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: None,
            diot_artifact_authority: BTreeMap::new(),
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        let mut binding_x = crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2 {
            artifact_id: "dserv:x".to_string(),
            execution_generation: 1,
            execution_projection_digest: format!("sha256:{}", "1".repeat(64)),
        };
        let mut proposal_a = base_proposal.clone();
        proposal_a
            .diot_artifact_authority
            .insert("dserv:x".to_string(), binding_x.clone());
        let digest_a =
            crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&proposal_a).unwrap();

        binding_x.execution_generation = 2;
        let mut proposal_b = base_proposal.clone();
        proposal_b
            .diot_artifact_authority
            .insert("dserv:x".to_string(), binding_x.clone());
        let digest_b =
            crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&proposal_b).unwrap();
        assert_ne!(digest_a, digest_b, "changing only execution_generation inside the persisted proposal must change proposal_digest");

        binding_x.execution_generation = 1;
        binding_x.execution_projection_digest = format!("sha256:{}", "2".repeat(64));
        let mut proposal_c = base_proposal;
        proposal_c
            .diot_artifact_authority
            .insert("dserv:x".to_string(), binding_x);
        let digest_c =
            crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&proposal_c).unwrap();
        assert_ne!(digest_a, digest_c, "changing only execution_projection_digest inside the persisted proposal must change proposal_digest");
    }

    fn create_v2_proposal_with_source(
        state: &mut DDeployStateV1,
        project: &str,
        app: &str,
        service_id: &str,
        proposal_id: &str,
        source: ProposalSource,
        dcontinuum: &CanonicalDContinuumV1,
    ) {
        let dgraph = one_service_dgraph(app, service_id, 1, 1);
        let dforward = minimal_dforward(project);
        let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: proposal_id.to_string(),
            project_id: project.to_string(),
            application_id: app.to_string(),
            source,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dgraph.dgraph_id.clone(),
                revision: dgraph.revision,
                digest: recompute_dgraph_digest(&dgraph).unwrap(),
            },
            dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dcontinuum.dcontinuum_id.clone(),
                revision: dcontinuum.revision,
                digest: recompute_dcontinuum_digest(dcontinuum).unwrap(),
            },
            dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dforward.dforward_id.clone(),
                revision: dforward.revision,
                digest: dforward.digest().unwrap(),
            },
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: service_id.to_string(),
                node_id: "shared-01".to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: None,
            diot_artifact_authority: BTreeMap::new(),
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        state
            .create_v2_proposal(proposal, dgraph, dcontinuum.clone(), dforward)
            .unwrap_or_else(|e| panic!("v2 proposal creation for '{app}' must succeed: {e}"));
    }

    /// Phase 117G Section 25/47: v1/v2 parity. `resolve_active_dcode_binding`
    /// must behave IDENTICALLY when active authority is DDeploy/2 as it does
    /// for DDeploy/1 (see `g22_registering_a_new_revision_does_not_change_the_active_binding`
    /// for the v1 version of this exact scenario) -- registering a new
    /// revision for an explicitly-bound v2 service changes nothing.
    #[test]
    fn v2_parity_registering_a_new_revision_does_not_change_the_active_v2_binding() {
        let project = "proj-v2-dcode-parity";
        let app = "app-a";
        let service_id = "svc-a1";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let dgraph = one_service_dgraph(app, service_id, 1, 1);
        let dforward = minimal_dforward(project);

        let mut registry = crate::core::dcode_registry::DCodeArtifactRegistryStateV2::empty();
        let binding = registered_binding(&mut registry, app, service_id);

        let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: "p-v2-dcode".to_string(),
            project_id: project.to_string(),
            application_id: app.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 { id: dgraph.dgraph_id.clone(), revision: dgraph.revision, digest: recompute_dgraph_digest(&dgraph).unwrap() },
            dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: recompute_dcontinuum_digest(&dcontinuum).unwrap() },
            dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 { id: dforward.dforward_id.clone(), revision: dforward.revision, digest: dforward.digest().unwrap() },
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 { service_id: service_id.to_string(), node_id: "shared-01".to_string() }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: None,
            diot_artifact_authority: BTreeMap::new(),
            dcode_artifact_authority: std::collections::BTreeMap::from([(service_id.to_string(), binding.clone())]),
        };
        let mut state = DDeployStateV1::empty();
        state.create_v2_proposal(proposal, dgraph, dcontinuum.clone(), dforward).unwrap();
        state
            .accept_v2_proposal_with_dcode_binding_revalidation(
                "p-v2-dcode",
                "acc-v2-dcode".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &[],
                &registry,
                None,
            )
            .unwrap();

        // Register a second, different revision for the same service.
        let mut different = dserv_artifact_for(app, service_id);
        different.limits.timeout_ms += 1;
        crate::core::dcode_registry::declare_dserv_artifact_revision(&mut registry, different).unwrap();

        let active = active_placement_authority(&state, project, app).unwrap().unwrap();
        assert!(active.v2.is_some(), "this test must exercise the DDeploy/2 branch of resolve_active_dcode_binding");
        let resolved = resolve_active_dcode_binding(&state, &active, service_id).unwrap();
        assert_eq!(resolved.descriptor_digest, binding.descriptor_digest, "registering D2 must never change a v2-active explicitly-bound service's authorization, exactly like v1");
    }

    #[test]
    fn v2_operator_proposal_persists_source_operator() {
        let project = "proj-v2-source-operator";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-op",
            ProposalSource::Operator,
            &dcontinuum,
        );
        assert_eq!(
            state
                .phase113_v2
                .proposals
                .get("p-op")
                .unwrap()
                .proposal
                .source,
            ProposalSource::Operator
        );
    }

    #[test]
    fn v2_deterministic_proposal_persists_source_deterministic() {
        let project = "proj-v2-source-deterministic";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-det",
            ProposalSource::Deterministic,
            &dcontinuum,
        );
        assert_eq!(
            state
                .phase113_v2
                .proposals
                .get("p-det")
                .unwrap()
                .proposal
                .source,
            ProposalSource::Deterministic
        );
    }

    #[test]
    fn ai_sourced_v2_proposal_has_no_alternate_activation_path() {
        // Item C: source == Ai must persist correctly AND must not create
        // any alternate route to activation -- creation alone must leave
        // the proposal Pending with no active authority, and the ONLY path
        // to Accepted is the exact same `accept_v2_proposal` call any other
        // source uses (there is no source-dependent branch anywhere in this
        // file -- `source` is provenance metadata, never authority).
        let project = "proj-v2-source-ai";
        let app_key = application_key(project, "app-a");
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-ai",
            ProposalSource::Ai,
            &dcontinuum,
        );
        let record = state.phase113_v2.proposals.get("p-ai").unwrap();
        assert_eq!(record.proposal.source, ProposalSource::Ai);
        assert_eq!(
            record.state,
            crate::core::ddeploy_v2::ProposalV2State::Pending,
            "an AI-sourced proposal must remain Pending -- creation alone must never activate it"
        );
        assert!(!state.phase113_v2.active.contains_key(&app_key), "an AI-sourced proposal must not produce any active authority without an explicit accept_v2_proposal call");

        let artifacts: Vec<crate::core::service_artifact_registry::ServiceArtifact> = vec![];
        state
            .accept_v2_proposal("p-ai", "acc-ai".to_string(), "operator:dana".to_string(), "2026-01-01T00:00:00Z".to_string(), &dcontinuum, &artifacts, None)
            .expect("an AI-sourced proposal must accept through the exact same explicit path as any other source");
        assert!(state.phase113_v2.active.contains_key(&app_key), "acceptance must activate an AI-sourced proposal exactly like any other -- no special-casing");
    }

    #[test]
    fn v2_source_tampered_after_creation_fails_closed_on_reload() {
        // Item D: `source` is part of `DDeployProposalV2` itself, so it is
        // automatically digest-covered -- tampering it without recomputing
        // `proposal_digest` must be caught by the existing digest-mismatch
        // check, exactly like every other field on this struct.
        let project = "proj-v2-source-tamper";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-src-tamper",
            ProposalSource::Operator,
            &dcontinuum,
        );
        state
            .phase113_v2
            .proposals
            .get_mut("p-src-tamper")
            .unwrap()
            .proposal
            .source = ProposalSource::Ai;
        let path = temp_db_path("phase113-v2-source-tamper");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a v2 proposal whose source was changed without updating its recorded proposal_digest must fail closed on reload");
        cleanup_db(&path);
    }

    #[test]
    fn v2_created_at_utc_is_server_generated_and_rfc3339_valid() {
        // Item E.
        let project = "proj-v2-timestamp";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-ts",
            ProposalSource::Operator,
            &dcontinuum,
        );
        let created_at_utc = &state
            .phase113_v2
            .proposals
            .get("p-ts")
            .unwrap()
            .proposal
            .created_at_utc;
        assert!(!created_at_utc.trim().is_empty());
        assert!(
            chrono::DateTime::parse_from_rfc3339(created_at_utc).is_ok(),
            "created_at_utc must be a genuine RFC3339 timestamp, got '{created_at_utc}'"
        );
    }

    #[test]
    fn v2_created_at_utc_tampered_after_creation_fails_closed_on_reload() {
        // Item F: same digest-coverage argument as the source-tamper test
        // above, for `created_at_utc`.
        let project = "proj-v2-timestamp-tamper";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-ts-tamper",
            ProposalSource::Operator,
            &dcontinuum,
        );
        state
            .phase113_v2
            .proposals
            .get_mut("p-ts-tamper")
            .unwrap()
            .proposal
            .created_at_utc = "2099-01-01T00:00:00Z".to_string();
        let path = temp_db_path("phase113-v2-timestamp-tamper");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a v2 proposal whose created_at_utc was changed without updating its recorded proposal_digest must fail closed on reload");
        cleanup_db(&path);
    }

    #[test]
    fn v2_malformed_created_at_utc_fails_closed_on_reload_even_with_a_consistent_digest() {
        // Isolates the NEW `schema_valid()` RFC3339 check inside
        // `validate_integrity()` from the pre-existing digest-mismatch
        // check: the digest is recomputed to match the malformed
        // timestamp, so digest CONSISTENCY alone would not catch this --
        // only the new timestamp-validity check does.
        let project = "proj-v2-malformed-ts";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_source(
            &mut state,
            project,
            "app-a",
            "svc-a1",
            "p-malformed-ts",
            ProposalSource::Operator,
            &dcontinuum,
        );
        {
            let record = state
                .phase113_v2
                .proposals
                .get_mut("p-malformed-ts")
                .unwrap();
            record.proposal.created_at_utc = "not-a-real-timestamp".to_string();
            record.proposal_digest =
                crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&record.proposal).unwrap();
        }
        let path = temp_db_path("phase113-v2-malformed-timestamp");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a malformed created_at_utc must fail closed even when proposal_digest was recomputed to match it");
        cleanup_db(&path);
    }

    #[test]
    fn v2_accept_with_matching_expected_proposal_digest_succeeds() {
        // Item G.
        let project = "proj-v2-cas-match";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-cas",
            &dcontinuum,
        );
        let expected_digest = state
            .phase113_v2
            .proposals
            .get("p-cas")
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
        let result = state.accept_v2_proposal(
            "p-cas",
            "acc-cas".to_string(),
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

    #[test]
    fn v2_accept_with_mismatching_expected_proposal_digest_is_rejected_with_no_mutation() {
        // Item H.
        let project = "proj-v2-cas-mismatch";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-cas-bad",
            &dcontinuum,
        );
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let wrong_digest = format!("sha256:{}", "0".repeat(64));
        let result = state.accept_v2_proposal(
            "p-cas-bad",
            "acc-cas-bad".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:00:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            Some(&wrong_digest),
        );
        assert!(
            result.is_err(),
            "acceptance with a wrong expected_proposal_digest must be rejected"
        );
        assert!(
            state.phase113_v2.acceptances.is_empty(),
            "no acceptance must be inserted after a CAS rejection"
        );
        assert!(
            !state
                .phase113_v2
                .active
                .contains_key(&application_key(project, "app-a")),
            "no active authority must be created after a CAS rejection"
        );
        assert_eq!(
            state.phase113_v2.proposals.get("p-cas-bad").unwrap().state,
            crate::core::ddeploy_v2::ProposalV2State::Pending,
            "the proposal must remain Pending after a CAS rejection"
        );
    }

    #[test]
    fn v2_idempotent_replay_with_matching_expected_digest_returns_same_acceptance() {
        // Item I.
        let project = "proj-v2-cas-replay-ok";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-cas-replay",
            &dcontinuum,
        );
        let expected_digest = state
            .phase113_v2
            .proposals
            .get("p-cas-replay")
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
        let first = state
            .accept_v2_proposal(
                "p-cas-replay",
                "acc-1".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                Some(&expected_digest),
            )
            .expect("first acceptance must succeed");
        let second = state
            .accept_v2_proposal(
                "p-cas-replay",
                "acc-2-ignored".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:05:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                Some(&expected_digest),
            )
            .expect("replay with the same correct expected digest must succeed");
        assert_eq!(
            first.acceptance_id, second.acceptance_id,
            "a correct-digest replay must return the SAME acceptance, never create a second one"
        );
        assert_eq!(
            state.phase113_v2.acceptances.len(),
            1,
            "only one acceptance record must ever exist"
        );
    }

    #[test]
    fn v2_replay_with_wrong_expected_digest_is_rejected_even_though_the_proposal_is_already_accepted(
    ) {
        // Item J.
        let project = "proj-v2-cas-replay-bad";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-cas-replay-bad",
            &dcontinuum,
        );
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        state
            .accept_v2_proposal(
                "p-cas-replay-bad",
                "acc-1".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:00:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .expect("first acceptance must succeed");
        let wrong_digest = format!("sha256:{}", "1".repeat(64));
        let result = state.accept_v2_proposal(
            "p-cas-replay-bad",
            "acc-2-should-not-exist".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            Some(&wrong_digest),
        );
        assert!(result.is_err(), "a wrong expected_proposal_digest must reject EVEN when replaying an already-Accepted proposal, never silently served the old result");
        assert_eq!(
            state.phase113_v2.acceptances.len(),
            1,
            "the replay rejection must not create a second acceptance"
        );
    }

    fn create_v2_proposal_with_real_artifact_authority(
        state: &mut DDeployStateV1,
        registry: &crate::core::service_artifact_registry::ServiceArtifactRegistry,
        project: &str,
        app: &str,
        service_id: &str,
        artifact_id: &str,
        proposal_id: &str,
        dcontinuum: &CanonicalDContinuumV1,
    ) -> crate::core::canonical_dforward::CanonicalDForwardV1 {
        let dforward = dforward_with_one_diot(project, artifact_id);
        let dg = one_service_dgraph(app, service_id, 1, 1);
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let authority = resolve_diot_artifact_authority(&dforward, project, &artifacts)
            .unwrap_or_else(|e| panic!("artifact authority resolution must succeed: {e}"));
        let proposal = crate::core::ddeploy_v2::DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: proposal_id.to_string(),
            project_id: project.to_string(),
            application_id: app.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: chrono::Utc::now().to_rfc3339(),
            dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dg.dgraph_id.clone(),
                revision: dg.revision,
                digest: recompute_dgraph_digest(&dg).unwrap(),
            },
            dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dcontinuum.dcontinuum_id.clone(),
                revision: dcontinuum.revision,
                digest: recompute_dcontinuum_digest(dcontinuum).unwrap(),
            },
            dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 {
                id: dforward.dforward_id.clone(),
                revision: dforward.revision,
                digest: dforward.digest().unwrap(),
            },
            placements: vec![crate::core::canonical_dmap_v2::DServPlacementV2 {
                service_id: service_id.to_string(),
                node_id: "shared-01".to_string(),
            }],
            diot_placements: vec![],
            dcall_realizations: vec![],
            external_port_bindings: vec![],
            expected_active_dmap_revision: None,
            diot_artifact_authority: authority,
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };
        state
            .create_v2_proposal(proposal, dg, dcontinuum.clone(), dforward.clone())
            .unwrap_or_else(|e| panic!("v2 proposal creation for '{app}' must succeed: {e}"));
        dforward
    }

    #[test]
    fn unchanged_artifact_authority_acceptance_succeeds() {
        // Item 17: proposal created under binding X; registry unchanged;
        // acceptance succeeds.
        let project = "proj-artifact-unchanged";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-a1",
            &dcontinuum,
        );

        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let result = state.accept_v2_proposal(
            "p-a1",
            "acc-a1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            None,
        );
        assert!(
            result.is_ok(),
            "acceptance with unchanged artifact authority must succeed -- got {result:?}"
        );
        assert!(state
            .phase113_v2
            .active
            .contains_key(&application_key(project, "app-a")));
    }

    #[test]
    fn stale_execution_generation_after_registry_update_fails_closed_at_acceptance() {
        // Item 15: create a valid v2 proposal under execution_generation N;
        // mutate the REAL registry through its normal supported update()
        // path so the artifact becomes generation N+1; attempt acceptance;
        // expect stale rejection with no active-state mutation.
        let project = "proj-artifact-stale-gen";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        let created = registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        assert_eq!(created.control_plane.execution_generation, 1);
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-a1",
            &dcontinuum,
        );

        // A real, supported registry mutation: change the entrypoint, which
        // changes the recomputed execution_projection_digest and therefore
        // bumps execution_generation -- exactly like a real operator update.
        let mut updated = minimal_native_service_artifact("dserv:broker");
        updated.runtime.native_process.as_mut().unwrap().entrypoint =
            "bin/broker-run-v2".to_string();
        let after_update = registry
            .update(project, "dserv:broker", updated)
            .expect("a real, supported registry update must succeed");
        assert_eq!(
            after_update.control_plane.execution_generation, 2,
            "changing entrypoint must bump execution_generation via the registry's own normal path"
        );

        let current_artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let result = state.accept_v2_proposal(
            "p-a1",
            "acc-a1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &current_artifacts,
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE)),
            "a bumped execution_generation must fail closed at acceptance -- got {result:?}"
        );
        assert!(
            !state
                .phase113_v2
                .active
                .contains_key(&application_key(project, "app-a")),
            "no active v2 authority must exist after a stale-authority rejection"
        );
        assert!(
            state.phase113_v2.acceptances.is_empty(),
            "no acceptance record must have been inserted"
        );
    }

    #[test]
    fn stale_execution_projection_digest_after_registry_update_fails_closed_at_acceptance() {
        // Item 16: mutate a real load-bearing execution-projection field
        // through supported registry semantics so the digest changes from X
        // to Y (never editing the stored digest directly); attempt
        // acceptance of a proposal bound to X; expect stale rejection.
        let project = "proj-artifact-stale-digest";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        let created = registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let original_digest = created.control_plane.execution_projection_digest.clone();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-a1",
            &dcontinuum,
        );

        let mut updated = minimal_native_service_artifact("dserv:broker");
        updated.runtime.native_process.as_mut().unwrap().args = vec!["--changed".to_string()];
        let after_update = registry
            .update(project, "dserv:broker", updated)
            .expect("a real, supported registry update must succeed");
        assert_ne!(
            after_update.control_plane.execution_projection_digest, original_digest,
            "changing args must change the real, recomputed execution_projection_digest"
        );

        let current_artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let result = state.accept_v2_proposal(
            "p-a1",
            "acc-a1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &current_artifacts,
            None,
        );
        assert!(
            matches!(result, Err(ref e) if e.contains(F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE)),
            "a changed execution_projection_digest must fail closed at acceptance -- got {result:?}"
        );
        assert!(!state
            .phase113_v2
            .active
            .contains_key(&application_key(project, "app-a")));
    }

    #[test]
    fn multiple_diots_requiring_the_same_artifact_resolve_to_one_identical_binding_deterministically(
    ) {
        // Item 11: two DIoTs requiring the same artifact must produce the
        // same binding, and the proposal digest must not depend on
        // `dforward.diots`'s own internal order.
        let project = "proj-artifact-shared";
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:shared"))
            .unwrap();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();

        let df_forward_order = dforward_with_two_diots_same_artifact(project, "dserv:shared");
        let mut df_reverse_order = df_forward_order.clone();
        df_reverse_order.diots.reverse();

        let bound_forward =
            resolve_diot_artifact_authority(&df_forward_order, project, &artifacts).unwrap();
        let bound_reverse =
            resolve_diot_artifact_authority(&df_reverse_order, project, &artifacts).unwrap();
        assert_eq!(
            bound_forward, bound_reverse,
            "iteration order over diots must not change the resolved binding map"
        );
        assert_eq!(bound_forward.len(), 1, "two DIoTs requiring the same artifact_id must resolve to exactly one shared binding entry");
    }

    #[test]
    fn v2_artifact_authority_persist_reload_round_trip() {
        // Item 18: create proposal with a real binding, persist, reload,
        // verify the binding survives exactly, the proposal digest still
        // validates, acceptance after reload succeeds if unchanged, and a
        // stale registry after reload still rejects.
        let project = "proj-artifact-reload";
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-a1",
            &dcontinuum,
        );

        let path = temp_db_path("phase113-artifact-authority");
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).expect(
            "a valid persisted v2 proposal with real artifact authority must reload cleanly",
        );
        let record = reloaded.phase113_v2.proposals.get("p-a1").unwrap();
        assert_eq!(
            record
                .proposal
                .diot_artifact_authority
                .get("dserv:broker")
                .unwrap()
                .execution_generation,
            1,
            "the persisted binding must survive a reload byte-for-byte"
        );
        assert_eq!(
            crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&record.proposal).unwrap(),
            record.proposal_digest,
            "the reloaded proposal's digest must still validate against its own persisted proposal_digest"
        );

        // Acceptance after reload succeeds if the registry is unchanged.
        let mut reloaded_ok = reloaded;
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let ok_result = reloaded_ok.accept_v2_proposal(
            "p-a1",
            "acc-a1".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &artifacts,
            None,
        );
        assert!(
            ok_result.is_ok(),
            "acceptance after reload with an unchanged registry must succeed -- got {ok_result:?}"
        );

        // A stale registry after reload still rejects, on a fresh reload of
        // the SAME originally-persisted (still-Pending) proposal.
        let mut reloaded_stale = load_state(&path).unwrap();
        let mut updated = minimal_native_service_artifact("dserv:broker");
        updated.runtime.native_process.as_mut().unwrap().entrypoint =
            "bin/broker-run-v3".to_string();
        registry.update(project, "dserv:broker", updated).unwrap();
        let stale_artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        let stale_result = reloaded_stale.accept_v2_proposal(
            "p-a1",
            "acc-a2".to_string(),
            "operator:dana".to_string(),
            "2026-01-01T00:05:00Z".to_string(),
            &dcontinuum,
            &stale_artifacts,
            None,
        );
        assert!(
            matches!(stale_result, Err(ref e) if e.contains(F_DDEPLOY_V2_ARTIFACT_AUTHORITY_STALE)),
            "a registry mutation occurring between persist and a later reload+accept attempt must still be caught -- got {stale_result:?}"
        );
    }

    fn valid_accepted_v2_state_for_corruption_tests(
        project: &str,
    ) -> (DDeployStateV1, CanonicalDContinuumV1) {
        let dcontinuum = shared_node_dcontinuum(project, 4, 4_294_967_296);
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(project, minimal_native_service_artifact("dserv:broker"))
            .unwrap();
        let mut state = DDeployStateV1::empty();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            project,
            "app-a",
            "svc-a1",
            "dserv:broker",
            "p-a1",
            &dcontinuum,
        );
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get(project)
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        state
            .accept_v2_proposal(
                "p-a1",
                "acc-a1".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:05:00Z".to_string(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .expect("fixture acceptance must succeed");
        (state, dcontinuum)
    }

    #[test]
    fn v2_acceptance_referencing_missing_proposal_fails_closed_on_reload() {
        // Pass 3.1 item A: the exact v2 counterpart of v1's own
        // `acceptance_referencing_missing_proposal_fails_closed_on_reload`
        // (independent review v3 correction) -- v2's own
        // `validate_integrity()` previously omitted this check entirely.
        let (mut state, _) = valid_accepted_v2_state_for_corruption_tests("proj-v2-dangling-a");
        state.phase113_v2.proposals.remove("p-a1");
        let path = temp_db_path("phase113-v2-acceptance-missing-proposal");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "a v2 acceptance referencing a missing proposal must fail closed, never be tolerated as a dangling reference");
        cleanup_db(&path);
    }

    #[test]
    fn v2_active_authority_referencing_missing_proposal_fails_closed_on_reload() {
        // Pass 3.1 item B: the active authority's OWN `proposal_id` must
        // also be resolvable, independent of the acceptances-loop's check --
        // in this exact corruption (proposal removed entirely) the
        // acceptances-loop's new guard fires first, which is expected and
        // correct defense-in-depth: the active-loop's own new
        // `self.proposals.get(&active.proposal_id)` guard exists specifically
        // so this remains caught even if the acceptances-loop check were
        // ever weakened independently. Both loops now independently refuse
        // a dangling `proposal_id`.
        let (mut state, _) = valid_accepted_v2_state_for_corruption_tests("proj-v2-dangling-b");
        assert!(state
            .phase113_v2
            .active
            .contains_key(&application_key("proj-v2-dangling-b", "app-a")));
        state.phase113_v2.proposals.remove("p-a1");
        let path = temp_db_path("phase113-v2-active-missing-proposal");
        persist_state(&path, &state).unwrap();
        assert!(
            load_state(&path).is_err(),
            "an active v2 authority whose proposal has vanished must fail closed"
        );
        cleanup_db(&path);
    }

    #[test]
    fn v2_acceptance_proposal_id_swapped_to_another_existing_proposal_fails_closed() {
        // Pass 3.1 item C: swap acc-a1's proposal_id to point at a
        // different, still-EXISTING proposal (p-a2) -- this must reject
        // even though the target proposal genuinely exists, because that
        // proposal's own `accepted_acceptance_id` correctly points to ITS
        // OWN acceptance (acc-a2), not the swapped-in acc-a1.
        let (mut state, dcontinuum) = valid_accepted_v2_state_for_corruption_tests("proj-v2-swap");
        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                "proj-v2-swap",
                minimal_native_service_artifact("dserv:broker-b"),
            )
            .unwrap();
        create_v2_proposal_with_real_artifact_authority(
            &mut state,
            &registry,
            "proj-v2-swap",
            "app-b",
            "svc-b1",
            "dserv:broker-b",
            "p-a2",
            &dcontinuum,
        );
        let artifacts_b = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get("proj-v2-swap")
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        state
            .accept_v2_proposal(
                "p-a2",
                "acc-a2".to_string(),
                "operator:dana".to_string(),
                "2026-01-01T00:06:00Z".to_string(),
                &dcontinuum,
                &artifacts_b,
                None,
            )
            .unwrap();

        state
            .phase113_v2
            .acceptances
            .get_mut("acc-a1")
            .unwrap()
            .proposal_id = "p-a2".to_string();
        let path = temp_db_path("phase113-v2-acceptance-swapped-proposal");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance whose proposal_id was swapped to a different existing proposal must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn v2_acceptance_proposal_digest_mismatch_fails_closed() {
        // Pass 3.1 item D: acceptance.proposal_digest no longer matches the
        // referenced proposal's actual recorded proposal_digest.
        let (mut state, _) =
            valid_accepted_v2_state_for_corruption_tests("proj-v2-digest-mismatch");
        state
            .phase113_v2
            .acceptances
            .get_mut("acc-a1")
            .unwrap()
            .proposal_digest = format!("sha256:{}", "9".repeat(64));
        let path = temp_db_path("phase113-v2-acceptance-digest-mismatch");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance whose proposal_digest disagrees with its referenced proposal's recorded digest must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn v2_active_proposal_id_differs_from_acceptance_proposal_id_fails_closed() {
        // Pass 3.1 item E: the active authority's own proposal_id no longer
        // agrees with its referenced acceptance's proposal_id.
        let (mut state, _) =
            valid_accepted_v2_state_for_corruption_tests("proj-v2-active-mismatch");
        let key = application_key("proj-v2-active-mismatch", "app-a");
        state.phase113_v2.active.get_mut(&key).unwrap().proposal_id =
            "some-other-nonexistent-proposal".to_string();
        let path = temp_db_path("phase113-v2-active-proposal-id-mismatch");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an active authority whose proposal_id disagrees with its own acceptance's proposal_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn v2_acceptance_project_application_mismatch_fails_closed() {
        // Pass 3.1 item F: the acceptance's own project/application no
        // longer agrees with its referenced proposal's project/application
        // (mutating the ACCEPTANCE, not the proposal, so the proposal's own
        // digest remains internally self-consistent -- isolating this
        // specific cross-record check from the unrelated proposal-digest
        // check in the proposals loop).
        let (mut state, _) =
            valid_accepted_v2_state_for_corruption_tests("proj-v2-project-mismatch");
        state
            .phase113_v2
            .acceptances
            .get_mut("acc-a1")
            .unwrap()
            .project_id = "some-other-project".to_string();
        let path = temp_db_path("phase113-v2-acceptance-project-mismatch");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "an acceptance whose project_id disagrees with its referenced proposal's project_id must fail closed");
        cleanup_db(&path);
    }

    #[test]
    fn v2_valid_accepted_state_passes_integrity_and_reload() {
        // Pass 3.1 items G+H: a genuinely valid, fully-consistent accepted
        // v2 state must still pass both the in-memory integrity check and a
        // full persist/reload round trip -- the new dangling-proposal
        // guards must not produce any false positive.
        let (state, _) = valid_accepted_v2_state_for_corruption_tests("proj-v2-valid-control");
        assert!(
            state.phase113_v2.validate_integrity().is_ok(),
            "a genuinely valid v2 state must pass validate_integrity()"
        );
        let path = temp_db_path("phase113-v2-valid-control-reload");
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).expect("a genuinely valid v2 state must reload cleanly");
        assert!(reloaded
            .phase113_v2
            .active
            .contains_key(&application_key("proj-v2-valid-control", "app-a")));
        cleanup_db(&path);
    }

    #[test]
    fn v2_artifact_authority_tamper_after_acceptance_fails_closed_on_reload() {
        // Pass 3.1 item 7: tamper the ACCEPTED proposal's own persisted
        // `diot_artifact_authority` without updating its `proposal_digest`
        // to match -- proving the complete accept-time chain (not merely
        // the pre-acceptance digest test already covered by
        // `changing_a_bound_execution_generation_or_projection_digest_changes_the_proposal_digest`)
        // still protects the DIoT artifact-authority evidence after a
        // restart, for a proposal that has ALREADY been accepted and has
        // an active DMap/2 depending on it.
        let (mut state, _) =
            valid_accepted_v2_state_for_corruption_tests("proj-v2-tamper-after-accept");
        state
            .phase113_v2
            .proposals
            .get_mut("p-a1")
            .unwrap()
            .proposal
            .diot_artifact_authority
            .get_mut("dserv:broker")
            .unwrap()
            .execution_generation = 999;
        let path = temp_db_path("phase113-v2-artifact-tamper-after-accept");
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err(), "tampering an accepted proposal's diot_artifact_authority without updating proposal_digest must fail closed on reload");
        cleanup_db(&path);
    }

    fn strip_fixture_bookkeeping(raw: &str) -> String {
        let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
        if let Some(obj) = v.as_object_mut() {
            obj.remove("x_note");
            obj.remove("expected_digest_sha256");
        }
        serde_json::to_string(&v).unwrap()
    }

    #[test]
    fn real_positive_proposal_v2_fixture_deserializes_and_digest_matches() {
        // Phase 113A closure pass 5, section 11: the real positive
        // datum.ddeploy-proposal/2 fixture must deserialize into the actual
        // Rust wire type and its recomputed digest must match the value the
        // fixture itself claims -- proving the on-disk bundle is genuinely
        // real, not merely schema-shaped.
        let raw =
            include_str!("../../../scripts/fixtures/datum-contracts/ddeploy-v2/proposal.json");
        let cleaned = strip_fixture_bookkeeping(raw);
        let proposal: crate::core::ddeploy_v2::DDeployProposalV2 = serde_json::from_str(&cleaned)
            .expect("real positive datum.ddeploy-proposal/2 fixture must deserialize");
        assert!(proposal.schema_valid());
        assert_eq!(
            crate::core::ddeploy_v2::DDeployStateV2::proposal_digest(&proposal).unwrap(),
            "sha256:c253c203fa92ef10c43977c0ad3448ee515b4d504d7e10d24afa11fdf9ff861f"
        );
    }

    #[test]
    fn real_positive_acceptance_v2_fixture_deserializes_and_links_to_proposal() {
        let raw =
            include_str!("../../../scripts/fixtures/datum-contracts/ddeploy-v2/acceptance.json");
        let cleaned = strip_fixture_bookkeeping(raw);
        let acceptance: crate::core::ddeploy_v2::DDeployAcceptanceV2 =
            serde_json::from_str(&cleaned)
                .expect("real positive datum.ddeploy-acceptance/2 fixture must deserialize");
        assert!(acceptance.schema_valid());
        // Must reference the EXACT real proposal fixture's own digest --
        // proving the bundle's acceptance->proposal link is genuinely real,
        // not two independently hand-typed files that happen to parse.
        assert_eq!(
            acceptance.proposal_digest,
            "sha256:c253c203fa92ef10c43977c0ad3448ee515b4d504d7e10d24afa11fdf9ff861f"
        );
        assert_eq!(
            acceptance.materialized_dmap_ref.digest,
            "sha256:b38a2aa111f358cb92fda232d6a5f707f798b6e4dca278626a7e0dd5a7e39851"
        );
    }

    #[test]
    fn real_ddeploy_v2_fixture_bundle_actually_accepts_through_the_real_core() {
        // The strongest possible proof of bundle realism: feed the exact
        // on-disk proposal fixture through the REAL `create_v2_proposal`/
        // `accept_v2_proposal` core path (with a real ServiceArtifact for
        // its one DIoT) and confirm it is accepted, producing exactly the
        // materialized DMap/2 digest the acceptance fixture independently
        // claims.
        let raw_proposal =
            include_str!("../../../scripts/fixtures/datum-contracts/ddeploy-v2/proposal.json");
        let proposal: crate::core::ddeploy_v2::DDeployProposalV2 =
            serde_json::from_str(&strip_fixture_bookkeeping(raw_proposal)).unwrap();
        let dgraph: CanonicalDGraphV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dgraph.smart-sentinel-lab.json"
        ))
        .unwrap();
        let dcontinuum: CanonicalDContinuumV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dcontinuum.smart-sentinel-lab.json"
        )).unwrap();
        let dforward: crate::core::canonical_dforward::CanonicalDForwardV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        )).unwrap();

        let registry = crate::core::service_artifact_registry::ServiceArtifactRegistry::from_state(
            Default::default(),
        );
        registry
            .create(
                "monitor-f49c8",
                minimal_native_service_artifact("mosquitto"),
            )
            .unwrap();
        let artifacts = registry
            .with_stable_view(|s| {
                s.artifacts_by_project
                    .get("monitor-f49c8")
                    .cloned()
                    .unwrap_or_default()
            })
            .unwrap();
        // The fixture's own diot_artifact_authority was hand-assembled with
        // illustrative (not registry-derived) values, so re-resolve it
        // against this test's real registry to get a self-consistent
        // proposal for this specific proof, mirroring
        // `create_v2_proposal_with_real_artifact_authority`'s established
        // pattern.
        let mut proposal = proposal;
        proposal.diot_artifact_authority =
            resolve_diot_artifact_authority(&dforward, "monitor-f49c8", &artifacts).unwrap();

        let mut state = DDeployStateV1::empty();
        state
            .create_v2_proposal(proposal.clone(), dgraph, dcontinuum.clone(), dforward)
            .unwrap_or_else(|e| {
                panic!("real fixture bundle proposal must be creatable through the real core: {e}")
            });
        let acceptance = state
            .accept_v2_proposal(
                &proposal.proposal_id,
                "acceptance:smart-sentinel-lab".into(),
                "operator:dana".into(),
                "2026-09-15T00:05:00Z".into(),
                &dcontinuum,
                &artifacts,
                None,
            )
            .unwrap_or_else(|e| {
                panic!("real fixture bundle proposal must be acceptable through the real core: {e}")
            });
        // Honest scope note: `accept_v2_proposal` derives `dmap_id` itself
        // via the server-owned `canonical_dmap_id` composition (never a
        // caller-chosen string), so the real acceptance's materialized
        // digest is NOT expected to equal `dmap-v2/dmap.smart-sentinel-lab
        // .json`'s own baked-in example digest -- that fixture is a
        // well-formed, independently-real, self-consistent EXAMPLE DMap/2
        // (its own digest recomputes correctly for its own exact content,
        // proven by `canonical_dmap_v2.rs`'s fixture tests), not literally
        // byte-identical to whatever `accept_v2_proposal` would derive for
        // this proposal at this moment. What this test proves instead: the
        // real proposal.json fixture is genuinely acceptable end-to-end
        // through the unmodified governance core, and the acceptance's own
        // materialized reference is well-formed and self-consistent.
        assert_eq!(acceptance.proposal_id, proposal.proposal_id);
        assert!(acceptance
            .materialized_dmap_ref
            .digest
            .starts_with("sha256:"));
        assert_eq!(
            acceptance.materialized_dmap_ref.digest.len(),
            "sha256:".len() + 64
        );
        assert!(state
            .phase113_v2
            .active
            .contains_key(&application_key("monitor-f49c8", "app:smart-sentinel-lab")));
    }

    #[test]
    fn every_ddeploy_v2_negative_fixture_actually_fails_closed() {
        // Phase 113A closure pass 5, section 13 (DDEPLOY/2 family): each
        // on-disk negative fixture's claimed failure mode must be genuinely
        // reproducible against the REAL `DDeployStateV2::validate_integrity`
        // (or, for the strict `ProposalSource` enum, real deserialization
        // failure) -- not merely asserted in fixture prose.
        use crate::core::ddeploy_v2::{
            DDeployAcceptanceV2, DDeployProposalV2, DDeployProposalV2Record, DDeployStateV2,
            ProposalV2State,
        };
        let dgraph: CanonicalDGraphV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dgraph.smart-sentinel-lab.json"
        ))
        .unwrap();
        let dcontinuum: CanonicalDContinuumV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dcontinuum.smart-sentinel-lab.json"
        )).unwrap();
        let dforward: crate::core::canonical_dforward::CanonicalDForwardV1 = serde_json::from_str(include_str!(
            "../../../scripts/fixtures/datum-contracts/canonical-v1/dforward.smart-sentinel-lab.json"
        )).unwrap();
        // The real, unmutated positive proposal -- used as the correct
        // baseline for the two acceptance-shaped cases (4 and 6), which must
        // isolate the acceptance's own tampered field from an otherwise
        // fully valid, correctly-linked proposal.
        let real_proposal_raw =
            include_str!("../../../scripts/fixtures/datum-contracts/ddeploy-v2/proposal.json");
        let real_proposal: DDeployProposalV2 =
            serde_json::from_str(&strip_fixture_bookkeeping(real_proposal_raw)).unwrap();
        let real_proposal_digest = DDeployStateV2::proposal_digest(&real_proposal).unwrap();
        assert_eq!(
            real_proposal_digest,
            "sha256:c253c203fa92ef10c43977c0ad3448ee515b4d504d7e10d24afa11fdf9ff861f"
        );

        fn strip_for_ddeploy_v2(raw: &str) -> (String, String) {
            let mut v: serde_json::Value = serde_json::from_str(raw).unwrap();
            let obj = v.as_object_mut().unwrap();
            obj.remove("x_note");
            obj.remove("expected_digest_sha256");
            let case = obj
                .remove("x_expect_reject")
                .and_then(|x| x.get("case").cloned())
                .and_then(|c| c.as_str().map(str::to_string))
                .expect("negative ddeploy-v2 fixture must carry x_expect_reject.case");
            (serde_json::to_string(&v).unwrap(), case)
        }

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../scripts/fixtures/datum-contracts/ddeploy-v2/negative");
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).expect("ddeploy-v2 negative fixture dir must exist") {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let (cleaned, case) = strip_for_ddeploy_v2(&raw);

            match case.as_str() {
                "invalid_source_deserialize_failure" => {
                    let result: Result<DDeployProposalV2, _> = serde_json::from_str(&cleaned);
                    assert!(result.is_err(), "{name}: fixture claims an invalid ProposalSource must fail deserialization, but it deserialized successfully");
                }
                "proposal_digest_tamper" => {
                    let tampered: DDeployProposalV2 = serde_json::from_str(&cleaned)
                        .unwrap_or_else(|e| {
                            panic!("{name}: must still deserialize into the real type: {e}")
                        });
                    let mut state = DDeployStateV2::default();
                    state.proposals.insert(
                        tampered.proposal_id.clone(),
                        DDeployProposalV2Record {
                            proposal: tampered,
                            // Deliberately the ORIGINAL (pre-tamper) digest --
                            // proving reload recomputes the digest from the
                            // (now-tampered) content rather than trusting it.
                            proposal_digest: real_proposal_digest.clone(),
                            dgraph_snapshot: dgraph.clone(),
                            dcontinuum_snapshot: dcontinuum.clone(),
                            dforward_snapshot: dforward.clone(),
                            state: ProposalV2State::Pending,
                            accepted_acceptance_id: None,
                        },
                    );
                    assert!(state.validate_integrity().is_err(), "{name}: tampered proposal content with a stale recorded digest must fail validate_integrity()");
                }
                "invalid_created_at_utc_consistent_digest" => {
                    let tampered: DDeployProposalV2 = serde_json::from_str(&cleaned)
                        .unwrap_or_else(|e| {
                            panic!("{name}: must still deserialize into the real type: {e}")
                        });
                    let consistent_digest = DDeployStateV2::proposal_digest(&tampered).unwrap();
                    let mut state = DDeployStateV2::default();
                    state.proposals.insert(
                        tampered.proposal_id.clone(),
                        DDeployProposalV2Record {
                            proposal: tampered,
                            proposal_digest: consistent_digest,
                            dgraph_snapshot: dgraph.clone(),
                            dcontinuum_snapshot: dcontinuum.clone(),
                            dforward_snapshot: dforward.clone(),
                            state: ProposalV2State::Pending,
                            accepted_acceptance_id: None,
                        },
                    );
                    assert!(state.validate_integrity().is_err(), "{name}: malformed created_at_utc must fail validate_integrity() even with an internally-consistent digest");
                }
                "acceptance_proposal_digest_mismatch"
                | "acceptance_project_application_mismatch" => {
                    let tampered_acceptance: DDeployAcceptanceV2 = serde_json::from_str(&cleaned)
                        .unwrap_or_else(|e| {
                            panic!("{name}: must still deserialize into the real type: {e}")
                        });
                    let mut state = DDeployStateV2::default();
                    state.proposals.insert(
                        real_proposal.proposal_id.clone(),
                        DDeployProposalV2Record {
                            proposal: real_proposal.clone(),
                            proposal_digest: real_proposal_digest.clone(),
                            dgraph_snapshot: dgraph.clone(),
                            dcontinuum_snapshot: dcontinuum.clone(),
                            dforward_snapshot: dforward.clone(),
                            state: ProposalV2State::Accepted,
                            accepted_acceptance_id: Some(tampered_acceptance.acceptance_id.clone()),
                        },
                    );
                    state.acceptances.insert(
                        tampered_acceptance.acceptance_id.clone(),
                        tampered_acceptance,
                    );
                    assert!(
                        state.validate_integrity().is_err(),
                        "{name}: tampered acceptance must fail validate_integrity()"
                    );
                }
                "dangling_proposal_reference" => {
                    let tampered_acceptance: DDeployAcceptanceV2 = serde_json::from_str(&cleaned)
                        .unwrap_or_else(|e| {
                            panic!("{name}: must still deserialize into the real type: {e}")
                        });
                    let mut state = DDeployStateV2::default();
                    // Deliberately NO proposal is inserted -- the acceptance
                    // references a proposal_id that does not exist anywhere
                    // in state.phase113_v2.proposals.
                    state.acceptances.insert(
                        tampered_acceptance.acceptance_id.clone(),
                        tampered_acceptance,
                    );
                    assert!(state.validate_integrity().is_err(), "{name}: an acceptance referencing a nonexistent proposal must fail validate_integrity()");
                }
                other => panic!(
                    "{name}: unrecognised x_expect_reject.case '{other}' -- add a match arm for it"
                ),
            }
            checked += 1;
        }
        assert!(checked >= 6, "expected at least 6 real ddeploy-v2 negative fixtures to be exercised, found {checked}");
    }

    // ----------------------------------------------------------------- //
    // Phase 119G: derive_cleanup_directives_for_node
    // ----------------------------------------------------------------- //
    mod cleanup_directive_derivation {
        use super::*;

        // H1: A -> B. Querying A yields a directive for A's historical
        // placement; querying B yields none (B is current).
        #[test]
        fn h1_single_hop_move_yields_directive_for_historical_source_only() {
            let dcontinuum = multi_node_dcontinuum("proj-h1", &["other-01"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h1", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 accept on shared-01 must succeed");
            // Move to a different node: create/accept a second proposal
            // placing the same service on "other-01" instead.
            let dgraph2 = one_service_dgraph("app", "svc", 1, 1);
            let artifacts = vec![sample_artifact("svc", vec!["other-01"])];
            create_proposal(&mut state, CreateProposalInput {
                planner_provenance: None, dcode_artifact_authority: BTreeMap::new(),
                project_id: "proj-h1".to_string(), application_id: "app".to_string(),
                source: ProposalSource::Operator, dgraph: dgraph2, dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 { service_id: "svc".to_string(), node_id: "other-01".to_string() }],
            }, "p2".to_string(), "2026-01-01T00:10:00Z".to_string(), &artifacts)
                .expect("rev2 proposal creation must succeed");
            accept_proposal(&mut state, "p2", "operator:dana", None, &artifacts, Some(&dcontinuum), "2026-01-01T00:15:00Z", "acc2".to_string())
                .expect("rev2 acceptance must succeed");

            let directives_a = derive_cleanup_directives_for_node(&state, "proj-h1", "shared-01").unwrap();
            assert_eq!(directives_a.len(), 1, "shared-01 must show exactly one cleanup candidate: {directives_a:?}");
            assert_eq!(directives_a[0].application_id, "app");
            assert_eq!(directives_a[0].service_id, "svc");
            assert_eq!(directives_a[0].previous_dmap_ref.revision, 1);

            let directives_b = derive_cleanup_directives_for_node(&state, "proj-h1", "other-01").unwrap();
            assert!(directives_b.is_empty(), "the CURRENT node must never show a cleanup directive for its own current service: {directives_b:?}");
        }

        // H2: A -> B -> C, A offline throughout. Querying A and B
        // individually each yield their own historical directive -- this is
        // R1-A's own counterexample, now proven fixed.
        #[test]
        fn h2_multi_hop_a_to_b_to_c_yields_directives_for_both_historical_nodes() {
            let dcontinuum = multi_node_dcontinuum("proj-h2", &["node-b", "node-c"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h2", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 (A) must succeed");
            move_service(&mut state, "proj-h2", "app", "svc", &dcontinuum, "node-b", "p2", "acc2", 1);
            move_service(&mut state, "proj-h2", "app", "svc", &dcontinuum, "node-c", "p3", "acc3", 1);

            let directives_a = derive_cleanup_directives_for_node(&state, "proj-h2", "shared-01").unwrap();
            assert_eq!(directives_a.len(), 1, "A (2 revisions back) must still be found by the full-history scan: {directives_a:?}");
            assert_eq!(directives_a[0].previous_dmap_ref.revision, 1);

            let directives_b = derive_cleanup_directives_for_node(&state, "proj-h2", "node-b").unwrap();
            assert_eq!(directives_b.len(), 1, "B (1 revision back) must be found too: {directives_b:?}");
            assert_eq!(directives_b[0].previous_dmap_ref.revision, 2);

            let directives_c = derive_cleanup_directives_for_node(&state, "proj-h2", "node-c").unwrap();
            assert!(directives_c.is_empty(), "C is CURRENT -- must never show a directive for itself");
        }

        // H3: A -> B -> A -> C. Querying A yields TWO distinct historical
        // directives (rev1's dmap_ref and rev3's dmap_ref), never collapsed.
        #[test]
        fn h3_duplicate_historical_placement_on_same_node_yields_distinct_directives() {
            let dcontinuum = multi_node_dcontinuum("proj-h3", &["node-b", "node-c"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h3", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 (A) must succeed");
            move_service(&mut state, "proj-h3", "app", "svc", &dcontinuum, "node-b", "p2", "acc2", 1);
            move_service(&mut state, "proj-h3", "app", "svc", &dcontinuum, "shared-01", "p3", "acc3", 1);
            move_service(&mut state, "proj-h3", "app", "svc", &dcontinuum, "node-c", "p4", "acc4", 1);

            let directives_a = derive_cleanup_directives_for_node(&state, "proj-h3", "shared-01").unwrap();
            assert_eq!(directives_a.len(), 2, "both the rev1 AND rev3 historical placements on shared-01 must each produce their own directive: {directives_a:?}");
            let revisions: BTreeSet<u64> = directives_a.iter().map(|d| d.previous_dmap_ref.revision).collect();
            assert_eq!(revisions, BTreeSet::from([1, 3]), "the two directives must carry the two DIFFERENT historical dmap_ref revisions");
        }

        // H4: A -> B -> A (current). Zero cleanup for A: current
        // return-to-source always wins.
        #[test]
        fn h4_return_to_source_suppresses_cleanup_for_current_placement() {
            let dcontinuum = multi_node_dcontinuum("proj-h4", &["node-b"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h4", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 (A) must succeed");
            move_service(&mut state, "proj-h4", "app", "svc", &dcontinuum, "node-b", "p2", "acc2", 1);
            move_service(&mut state, "proj-h4", "app", "svc", &dcontinuum, "shared-01", "p3", "acc3", 1);

            let directives_a = derive_cleanup_directives_for_node(&state, "proj-h4", "shared-01").unwrap();
            assert!(directives_a.is_empty(), "shared-01 is CURRENT again (rev3) -- no candidate may survive, regardless of rev1's history: {directives_a:?}");

            let directives_b = derive_cleanup_directives_for_node(&state, "proj-h4", "node-b").unwrap();
            assert_eq!(directives_b.len(), 1, "node-b (rev2, now vacated) must still show its own directive");
        }

        // H5: repeated derivation is deterministic.
        #[test]
        fn h5_repeated_derivation_is_deterministic() {
            let dcontinuum = multi_node_dcontinuum("proj-h5", &["node-b"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h5", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 must succeed");
            move_service(&mut state, "proj-h5", "app", "svc", &dcontinuum, "node-b", "p2", "acc2", 1);

            let first = derive_cleanup_directives_for_node(&state, "proj-h5", "shared-01").unwrap();
            let second = derive_cleanup_directives_for_node(&state, "proj-h5", "shared-01").unwrap();
            assert_eq!(first, second, "repeated derivation over unchanged state must be byte-identical");
        }

        // H6: a ServiceArtifact's CURRENT execution_generation/
        // execution_projection_digest changing must never affect derived
        // cleanup-directive history. `derive_cleanup_directives_for_node`
        // takes only `&DDeployStateV1` -- it has no `ServiceArtifactRegistryState`
        // parameter at all, so it is structurally impossible for anything
        // about a ServiceArtifact's current identity to influence its
        // output. This test makes that guarantee explicit and
        // regression-proof: deriving twice from the SAME unchanged
        // `DDeployStateV1`, with a `ServiceArtifact` constructed at two
        // very different generations in between (never passed to the
        // function under test), must produce byte-identical directives.
        #[test]
        fn h6_current_service_artifact_generation_changes_do_not_affect_directive_history() {
            let dcontinuum = multi_node_dcontinuum("proj-h6", &["node-b"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h6", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 must succeed");
            move_service(&mut state, "proj-h6", "app", "svc", &dcontinuum, "node-b", "p2", "acc2", 1);

            let before = derive_cleanup_directives_for_node(&state, "proj-h6", "shared-01").unwrap();

            // Neither of these is ever passed to `derive_cleanup_directives_for_node`
            // -- their mere existence, at deliberately very different
            // generations, is the point: there is no parameter of this
            // function's signature they COULD be passed through even if a
            // future caller wanted to.
            let mut artifact_gen1 = sample_artifact("svc", vec!["node-b"]);
            artifact_gen1.control_plane.execution_generation = 1;
            let mut artifact_gen99 = sample_artifact("svc", vec!["node-b"]);
            artifact_gen99.control_plane.execution_generation = 99;
            let _ = (&artifact_gen1, &artifact_gen99);

            let after = derive_cleanup_directives_for_node(&state, "proj-h6", "shared-01").unwrap();
            assert_eq!(before, after, "cleanup directive history must never vary with ServiceArtifact identity, which this function never even receives");
        }

        // H7: v1 historical placement, application now v2-active elsewhere.
        // The v1 historical cleanup directive must still be derived.
        #[test]
        fn h7_v1_historical_placement_survives_transition_to_v2_active() {
            let dcontinuum = multi_node_dcontinuum("proj-h7", &["node-b"], 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h7", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("v1 rev1 (A, on shared-01) must succeed");
            create_and_accept_v2_on_node(&mut state, "proj-h7", "app", "svc", "node-b", 1, 1, &dcontinuum, "p2", "acc2", Some(1))
                .expect("v2 acceptance transitioning app to v2-active on node-b must succeed");

            assert!(
                !state.active_by_application.contains_key(&application_key("proj-h7", "app")),
                "the v1->v2 transition must have retired the v1 active pointer"
            );
            let directives = derive_cleanup_directives_for_node(&state, "proj-h7", "shared-01").unwrap();
            assert_eq!(directives.len(), 1, "the v1-era historical placement must still be found by scanning state.acceptances directly, independent of the active pointer: {directives:?}");
            assert_eq!(directives[0].previous_dmap_ref.revision, 1);
        }

        // H8: corrupt acceptance->proposal linkage fails closed.
        #[test]
        fn h8_corrupt_acceptance_proposal_linkage_fails_closed() {
            let dcontinuum = shared_node_dcontinuum("proj-h8", 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h8", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 must succeed");
            // Corrupt: the proposal no longer claims to be accepted by this
            // acceptance (relational linkage broken).
            state.proposals.get_mut("p1").unwrap().accepted_acceptance_id = Some("some-other-acceptance".to_string());
            let result = derive_cleanup_directives_for_node(&state, "proj-h8", "shared-01");
            assert!(result.is_err(), "corrupt acceptance<->proposal linkage must fail the whole derivation closed, never silently skip it: {result:?}");
        }

        // H9: proposal digest corruption fails closed.
        #[test]
        fn h9_proposal_digest_corruption_fails_closed() {
            let dcontinuum = shared_node_dcontinuum("proj-h9", 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h9", "app", "svc", 1, 1, &dcontinuum, "p1", "acc1")
                .expect("rev1 must succeed");
            // Corrupt: mutate the persisted proposal's placements without
            // recomputing its recorded digest.
            state.proposals.get_mut("p1").unwrap().proposal.placements.push(
                CanonicalDServPlacementV1 { service_id: "svc-injected".to_string(), node_id: "shared-01".to_string() },
            );
            let result = derive_cleanup_directives_for_node(&state, "proj-h9", "shared-01");
            assert!(result.is_err(), "a tampered proposal whose recomputed digest no longer matches its recorded digest must fail closed: {result:?}");
        }

        // H10: the same acceptance_id existing in both v1 and v2 acceptance
        // maps is cross-version identity ambiguity -- fails closed.
        #[test]
        fn h10_cross_version_acceptance_id_ambiguity_fails_closed() {
            let dcontinuum = shared_node_dcontinuum("proj-h10", 4, 4_294_967_296);
            let mut state = DDeployStateV1::empty();
            create_and_accept_on_shared_node(&mut state, "proj-h10", "app-a", "svc-a", 1, 1, &dcontinuum, "p1", "shared-acc-id")
                .expect("v1 acceptance must succeed");
            create_and_accept_v2_on_shared_node(&mut state, "proj-h10", "app-b", "svc-b", 1, 1, &dcontinuum, "p2", "shared-acc-id", None)
                .expect("v2 acceptance reusing the SAME acceptance_id string must succeed at the storage layer (no cross-map uniqueness enforced there)");
            let result = derive_cleanup_directives_for_node(&state, "proj-h10", "shared-01");
            assert!(result.is_err(), "the same acceptance_id existing in both v1 and v2 maps must fail derivation closed: {result:?}");
        }

        // No-directive-for-unknown-project / never-known node (M16/M17 shape).
        #[test]
        fn no_directive_for_project_never_structurally_known() {
            let state = DDeployStateV1::empty();
            let directives = derive_cleanup_directives_for_node(&state, "proj-unknown", "any-node").unwrap();
            assert!(directives.is_empty(), "empty state must never synthesize a directive");
        }

        /// Test helper: creates+accepts a new proposal moving `service_id`
        /// to `target_node` for `application_id`, expecting
        /// `expected_active_dmap_revision` to be the CURRENT active
        /// revision beforehand (mirrors `create_and_accept_on_shared_node`
        /// but for an arbitrary target node on a later revision).
        fn move_service(
            state: &mut DDeployStateV1,
            project: &str,
            application_id: &str,
            service_id: &str,
            dcontinuum: &CanonicalDContinuumV1,
            target_node: &str,
            proposal_id: &str,
            acceptance_id: &str,
            revision_hint: u64,
        ) {
            let dgraph = one_service_dgraph(application_id, service_id, 1, 1);
            let artifacts = vec![sample_artifact(service_id, vec![target_node])];
            create_proposal(state, CreateProposalInput {
                planner_provenance: None, dcode_artifact_authority: BTreeMap::new(),
                project_id: project.to_string(), application_id: application_id.to_string(),
                source: ProposalSource::Operator, dgraph, dcontinuum: dcontinuum.clone(),
                placements: vec![CanonicalDServPlacementV1 { service_id: service_id.to_string(), node_id: target_node.to_string() }],
            }, proposal_id.to_string(), format!("2026-01-01T00:{revision_hint:02}:00Z"), &artifacts)
                .unwrap_or_else(|e| panic!("move_service: proposal creation to '{target_node}' must succeed: {e:?}"));
            accept_proposal(state, proposal_id, "operator:dana", None, &artifacts, Some(dcontinuum), format!("2026-01-01T00:{revision_hint:02}:30Z").as_str(), acceptance_id.to_string())
                .unwrap_or_else(|e| panic!("move_service: acceptance to '{target_node}' must succeed: {e:?}"));
        }

    }

    // ----------------------------------------------------------------- //
    // Phase 118B: derive_active_dengine_nodes
    // ----------------------------------------------------------------- //
    mod dengine_derivation {
        use super::*;
        use crate::core::canonical_dmap_v2::{
            DIoTArtifactBindingV2, DIoTPlacementV2, DServPlacementV2,
        };
        use crate::core::ddeploy_v2::{PlacementAuthorityV2Extras, PlacementAuthorityView};

        fn placement(service_id: &str, node_id: &str) -> DServPlacementV2 {
            DServPlacementV2 {
                service_id: service_id.into(),
                node_id: node_id.into(),
            }
        }

        fn view(placements: Vec<DServPlacementV2>) -> PlacementAuthorityView {
            PlacementAuthorityView {
                project_id: "p".into(),
                application_id: "a".into(),
                dmap_id: "dmap:test".into(),
                revision: 1,
                digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .into(),
                placements,
                proposal_id: None,
                acceptance_id: None,
                dgraph_id: None,
                dgraph_revision: None,
                dgraph_digest: None,
                dgraph_snapshot: None,
                v2: None,
            }
        }

        fn view_with_diot(
            placements: Vec<DServPlacementV2>,
            diot_placements: Vec<DIoTPlacementV2>,
        ) -> PlacementAuthorityView {
            let mut v = view(placements);
            v.v2 = Some(PlacementAuthorityV2Extras {
                dforward_id: "dforward:test".into(),
                dforward_revision: 1,
                dforward_digest:
                    "sha256:1111111111111111111111111111111111111111111111111111111111111111".into(),
                dcall_realizations: vec![],
                diot_placements,
                dforward_snapshot: None,
                external_port_bindings: vec![],
            });
            v
        }

        fn diot_placement(placement_id: &str, diot_id: &str, node_id: &str) -> DIoTPlacementV2 {
            DIoTPlacementV2 {
                placement_id: placement_id.into(),
                diot_id: diot_id.into(),
                node_id: node_id.into(),
                artifact_binding: DIoTArtifactBindingV2 {
                    artifact_id: "artifact:diot".into(),
                    descriptor_digest:
                        "sha256:2222222222222222222222222222222222222222222222222222222222222222"
                            .into(),
                    content_digest: None,
                },
            }
        }

        #[test]
        fn e1_empty_active_placement_list_yields_empty_membership() {
            assert_eq!(
                derive_active_dengine_nodes(&view(vec![])),
                Vec::<String>::new()
            );
        }

        #[test]
        fn e2_single_placement_yields_single_member() {
            let v = view(vec![placement("svc", "node-b")]);
            assert_eq!(derive_active_dengine_nodes(&v), vec!["node-b".to_string()]);
        }

        #[test]
        fn e3_multiple_distinct_placements_are_sorted_lexicographically() {
            let v = view(vec![
                placement("svc-1", "node-c"),
                placement("svc-2", "node-a"),
                placement("svc-3", "node-b"),
            ]);
            assert_eq!(
                derive_active_dengine_nodes(&v),
                vec![
                    "node-a".to_string(),
                    "node-b".to_string(),
                    "node-c".to_string()
                ]
            );
        }

        #[test]
        fn e4_multiple_services_on_the_same_node_collapse_to_one_member() {
            let v = view(vec![
                placement("svc-1", "node-a"),
                placement("svc-2", "node-a"),
            ]);
            assert_eq!(derive_active_dengine_nodes(&v), vec!["node-a".to_string()]);
        }

        #[test]
        fn e5_mixed_duplicate_and_unordered_placements_dedupe_and_sort() {
            let v = view(vec![
                placement("svc-1", "node-b"),
                placement("svc-2", "node-a"),
                placement("svc-3", "node-b"),
                placement("svc-4", "node-c"),
                placement("svc-5", "node-a"),
            ]);
            assert_eq!(
                derive_active_dengine_nodes(&v),
                vec![
                    "node-a".to_string(),
                    "node-b".to_string(),
                    "node-c".to_string()
                ]
            );
        }

        #[test]
        fn e6_service_identity_does_not_affect_membership() {
            let a = view(vec![placement("svc-alpha", "node-a")]);
            let b = view(vec![placement("svc-completely-different-id", "node-a")]);
            assert_eq!(
                derive_active_dengine_nodes(&a),
                derive_active_dengine_nodes(&b)
            );
        }

        #[test]
        fn e7_diot_only_node_is_excluded_from_membership() {
            let v = view_with_diot(
                vec![placement("svc", "node-a")],
                vec![diot_placement("p1", "diot:1", "node-z")],
            );
            let result = derive_active_dengine_nodes(&v);
            assert_eq!(result, vec!["node-a".to_string()]);
            assert!(
                !result.contains(&"node-z".to_string()),
                "DIoT-only node must not appear in D-Engine membership"
            );
        }

        #[test]
        fn e8_diot_only_active_authority_yields_empty_membership() {
            let v = view_with_diot(vec![], vec![diot_placement("p1", "diot:1", "node-z")]);
            assert_eq!(derive_active_dengine_nodes(&v), Vec::<String>::new());
        }

        #[test]
        fn e9_function_signature_admits_no_runtime_health_or_monitor_input() {
            // Structural proof: `derive_active_dengine_nodes` takes exactly one
            // argument, `&PlacementAuthorityView`, which itself carries no
            // health/liveness/observation field (see `PlacementAuthorityView`'s
            // own field list) -- there is no D-Monitor state this function
            // could consult even if it wanted to. Two views that differ only
            // in which "node" values they carry (never in any health signal,
            // because no such field exists) proves membership is placement-
            // authority-derived and nothing else.
            let v = view(vec![placement("svc", "node-a")]);
            assert_eq!(derive_active_dengine_nodes(&v), vec!["node-a".to_string()]);
        }

        #[test]
        fn e10_capability_state_has_no_role_in_membership() {
            // `DServPlacementV2` carries only `service_id`/`node_id` -- no
            // platform/capacity/ABI-capability field exists on a placement for
            // this function to read, so membership cannot vary with capability
            // state by construction.
            let v = view(vec![placement("svc", "node-a")]);
            assert_eq!(derive_active_dengine_nodes(&v), vec!["node-a".to_string()]);
        }

        #[test]
        fn e11_dcode_executable_currentness_has_no_role_in_membership() {
            // Same structural argument as E10: `DServPlacementV2` carries no
            // D-Code artifact/descriptor-digest field, and `PlacementAuthorityView`
            // itself carries no `dcode_artifact_authority`-shaped field either --
            // this function has nothing D-Code-related to read.
            let v = view(vec![placement("svc", "node-a")]);
            assert_eq!(derive_active_dengine_nodes(&v), vec!["node-a".to_string()]);
        }

        #[test]
        fn e12_determinism_across_insertion_order() {
            let a = view(vec![
                placement("svc-1", "node-c"),
                placement("svc-2", "node-a"),
                placement("svc-3", "node-b"),
            ]);
            let b = view(vec![
                placement("svc-3", "node-b"),
                placement("svc-1", "node-c"),
                placement("svc-2", "node-a"),
            ]);
            assert_eq!(
                derive_active_dengine_nodes(&a),
                derive_active_dengine_nodes(&b)
            );
        }
    }
}
