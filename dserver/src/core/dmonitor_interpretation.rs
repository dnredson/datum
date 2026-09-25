//! DMonitor derived desired-vs-observed interpretation (Phase 114C, ADR-0024).
//!
//! `DMap`/`DForward`/etc. remain the ONE authority for what SHOULD exist.
//! `DMonitor` remains evidence of what WAS OBSERVED. This module ONLY
//! compares them -- it grants DMonitor no desired-state authority, produces
//! no placement decision, and performs no host/reconciliation action.
//!
//! # Derived, never persisted
//!
//! Every type here is a READ MODEL computed on demand from three explicit
//! inputs: the current active [`PlacementAuthorityView`]
//! (`ddeploy::active_placement_authority` -- the ONE shared v1/v2
//! interpretation; this module never reads `active_by_application`/
//! `phase113_v2.active` directly), the current persisted [`DMonitorStateV1`],
//! and an explicit [`DMonitorFreshnessPolicyV1`] + evaluation time. Desired
//! authority can change, evidence ages, policy can change, and evaluation
//! time advances -- persisting a convergence boolean would immediately
//! create another staleness problem of its own. Nothing in this module
//! writes to `storage::control_state` or any other persistence layer.
//!
//! # Four separate dimensions
//!
//! This module never collapses everything into one boolean. Kept separate:
//!
//! - **freshness** (`dmonitor_freshness`): is the evidence recent enough to
//!   trust right now?
//! - **authority correlation** ([`AuthorityCorrelationV1`]): does the
//!   evidence's own referenced authority (`dmap_ref`) exactly match the
//!   CURRENT active authority?
//! - **placement convergence** ([`DMonitorConvergenceV1`]): does admitted
//!   evidence prove the desired placement is what is actually running?
//! - **observed health** (`DMonitorHealthV1`): what the collector believed
//!   about the subject's OWN health, entirely independent of placement.
//!
//! `Fresh != Healthy`, `Healthy != Converged`, `Converged != Ready`, and
//! `Diverged` does not necessarily imply `Unhealthy` -- a DMap may correctly
//! place a service on a node while that service happens to be currently
//! unhealthy, which must remain representable as `convergence = Converged`
//! + `health = Unhealthy` simultaneously (proven by
//! `exact_placement_with_unhealthy_health_still_converges`).
//!
//! # Freshness precedence (section 9)
//!
//! Freshness qualification happens BEFORE authority-correlation evidence can
//! prove CURRENT convergence. Stale or clock-skew-invalid evidence remains
//! historically valid (never deleted, never rejected from persistence -- see
//! `dmonitor_state`), but cannot prove `Converged`. If the only relevant
//! evidence is stale/clock-skew-invalid, the result is `Unknown` (with
//! `stale_evidence`/`clock_skew` findings), NEVER `Diverged` -- absence of
//! CURRENT proof is not proof of drift.
//!
//! # No implicit monitor leader (section 14)
//!
//! Multiple independent monitor streams may report on the same subject. This
//! module never picks "the latest received wins" as an implicit leader
//! election: every relevant stream's OWN latest observation of a given
//! subject is evaluated independently (mirroring `DMonitorStateV1`'s own
//! "latest per subject per stream, not the stream head" rule from Phase
//! 114B1.1), stale streams never veto fresh streams, and materially
//! conflicting fresh streams are surfaced as `conflicting_fresh_evidence`
//! rather than silently resolved by recency.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::canonical_dmonitor::{DMonitorHealthV1, DMonitorSubjectV1};
use crate::core::ddeploy_v2::{PlacementAuthorityV2Extras, PlacementAuthorityView};
use crate::core::dmonitor_freshness::{evaluate_freshness, DMonitorFreshnessPolicyV1, DMonitorFreshnessV1};
use crate::core::dmonitor_state::{DMonitorObservationRecordV1, DMonitorStateV1};

pub const DMONITOR_CONVERGENCE_SCHEMA: &str = "datum.dmonitor-convergence/1";

// ------------------------------------------------------------- FINDING CODES
// Typed reason/finding codes (section 6): the main state stays a small
// closed enum; every "why" is one of these stable string codes instead of
// being folded into the state itself.
//
// Phase 114C.1 closure audit: this is the authoritative inventory -- exactly
// 13 stable finding codes (the Phase 114C final report's prose summary said
// "12", which was a counting error in that summary text, not in this
// module; the code itself, the ADR, and the test suite all already agreed
// on these same 13 names). `FINDING_UNSUPPORTED_SUBJECT_EVIDENCE` is kept
// (never removed merely to make the count round) but is not currently
// emitted by any code path in this module: both `evaluate_service_placement`
// and the unexpected-realization scan only ever receive records already
// filtered to `DMonitorSubjectV1::DServRealization` by
// `latest_dserv_realizations_for_application`, so there is no reachable
// point today where an "unsupported subject" is actually encountered. It
// remains declared, `#[allow(dead_code)]`, for a future generalization of
// this module that dispatches across multiple subject kinds and would need
// to reject one it does not know how to interpret.
pub const FINDING_EXACT_MATCH: &str = "exact_match";
pub const FINDING_NO_EVIDENCE: &str = "no_evidence";
pub const FINDING_STALE_EVIDENCE: &str = "stale_evidence";
pub const FINDING_CLOCK_SKEW: &str = "clock_skew";
pub const FINDING_AUTHORITY_REF_MISSING: &str = "authority_ref_missing";
pub const FINDING_AUTHORITY_MISMATCH: &str = "authority_mismatch";
pub const FINDING_WRONG_NODE: &str = "wrong_node";
pub const FINDING_UNEXPECTED_REALIZATION: &str = "unexpected_realization";
pub const FINDING_CONFLICTING_FRESH_EVIDENCE: &str = "conflicting_fresh_evidence";
#[allow(dead_code)]
pub const FINDING_UNSUPPORTED_SUBJECT_EVIDENCE: &str = "unsupported_subject_evidence";
pub const FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY: &str = "no_active_placement_authority";
pub const FINDING_NO_DESIRED_PLACEMENTS: &str = "no_desired_placements";
pub const FINDING_INSUFFICIENT_CANONICAL_CORRELATION: &str = "insufficient_canonical_correlation";

// Phase 114D0A closure: DIoT convergence reuses every finding code above
// that is generic to "evidence vs authority" comparison (exact match,
// no/stale/clock-skewed evidence, an authority ref present but missing or
// mismatched, wrong/conflicting observed node, an unexpected realization,
// no active placement authority at all) unchanged. These six are the ones
// with no D-Serv equivalent: DIoT correlation additionally requires an
// exact match against the CURRENTLY REGISTERED `DIoTRuntimeBindingV1` (a
// separate operational authority from DMap/DForward, never reachable from
// D-Serv's comparison at all) and that registration must itself still be
// current against DMap/2 + D-Node authority.
pub const FINDING_RUNTIME_BINDING_NOT_REGISTERED: &str = "runtime_binding_not_registered";
pub const FINDING_RUNTIME_BINDING_REF_MISSING: &str = "runtime_binding_ref_missing";
pub const FINDING_RUNTIME_BINDING_MISMATCH: &str = "runtime_binding_mismatch";
pub const FINDING_RUNTIME_BINDING_STALE: &str = "runtime_binding_stale";
pub const FINDING_DMAP_V1_NO_DIOT_SUPPORT: &str = "dmap_v1_no_diot_support";
pub const FINDING_NO_DESIRED_DIOT_PLACEMENTS: &str = "no_desired_diot_placements";

/// The complete, stable finding-code inventory (Phase 114C.1, section 1;
/// extended by Phase 114D0A section 1 with the six DIoT-only codes above --
/// never redefined, only appended to). Exists so a test can assert the
/// inventory's exact size and membership stay in sync with the constants
/// themselves, rather than the count only ever being asserted informally in
/// prose (which is exactly how the Phase 114C final report's "12" miscount
/// happened).
pub const ALL_FINDING_CODES: &[&str] = &[
    FINDING_EXACT_MATCH,
    FINDING_NO_EVIDENCE,
    FINDING_STALE_EVIDENCE,
    FINDING_CLOCK_SKEW,
    FINDING_AUTHORITY_REF_MISSING,
    FINDING_AUTHORITY_MISMATCH,
    FINDING_WRONG_NODE,
    FINDING_UNEXPECTED_REALIZATION,
    FINDING_CONFLICTING_FRESH_EVIDENCE,
    FINDING_UNSUPPORTED_SUBJECT_EVIDENCE,
    FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY,
    FINDING_NO_DESIRED_PLACEMENTS,
    FINDING_INSUFFICIENT_CANONICAL_CORRELATION,
    FINDING_RUNTIME_BINDING_NOT_REGISTERED,
    FINDING_RUNTIME_BINDING_REF_MISSING,
    FINDING_RUNTIME_BINDING_MISMATCH,
    FINDING_RUNTIME_BINDING_STALE,
    FINDING_DMAP_V1_NO_DIOT_SUPPORT,
    FINDING_NO_DESIRED_DIOT_PLACEMENTS,
];

fn finding(code: &'static str, message: impl Into<String>) -> DMonitorConvergenceFindingV1 {
    DMonitorConvergenceFindingV1 {
        code,
        message: message.into(),
    }
}

/// Small typed convergence state (section 6). Deliberately just three
/// values -- every "why" lives in `findings`, never in a fourth/fifth main
/// state value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorConvergenceV1 {
    Converged,
    Diverged,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DMonitorConvergenceFindingV1 {
    pub code: &'static str,
    pub message: String,
}

/// How one piece of evidence's OWN referenced authority (`dmap_ref` for
/// `DServRealization`) relates to the CURRENT active authority. Never
/// "helpfully" widened to a partial/by-name match (section 11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityCorrelationV1 {
    /// The evidence carries an exact `{id, revision, digest}` match against
    /// the current active authority.
    ExactMatch,
    /// The evidence's subject has no authority reference at all
    /// (`dmap_ref: None`).
    RefMissing,
    /// The evidence carries a reference, but it does not exactly match
    /// current authority (a stale/historical reference, or a fabricated
    /// one) -- valid historical evidence, never current-authority proof.
    Mismatch,
    /// Phase 114D0A: reserved for a correlation dimension that does not
    /// apply at all in the current context (e.g. DForward correlation when
    /// the active authority has no DForward concept). Never currently
    /// constructed: `evaluate_application_diot_convergence` handles its one
    /// "not applicable" case (`datum.dmap/1` has no DIoT placements at all)
    /// at the WHOLE-APPLICATION level, before any per-placement DForward
    /// comparison is reached, so this variant is declared for API
    /// completeness/future use rather than currently reachable -- mirroring
    /// this module's existing `FINDING_UNSUPPORTED_SUBJECT_EVIDENCE`
    /// precedent.
    #[allow(dead_code)]
    NotApplicable,
}

/// Evidence-oriented summary of one admitted-or-excluded observation
/// (section 7). Deliberately NEVER includes the observation's own `facts`
/// blob -- this stays evidence-oriented, not a raw telemetry dump, and
/// never risks re-exposing anything facts might carry.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorEvidenceSummaryV1 {
    pub observation_id: String,
    pub observation_digest: String,
    /// The DNode the OBSERVATION was produced on (the observer's own
    /// identity) -- see `observed_subject_node_id` for the node the
    /// SUBJECT claims to run on, which for `DServRealization` is the field
    /// that actually matters for placement comparison.
    pub node_id: String,
    pub monitor_instance_id: String,
    pub sequence: u64,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub freshness: DMonitorFreshnessV1,
    pub health: DMonitorHealthV1,
    pub authority_correlation: AuthorityCorrelationV1,
    /// The node_id the SUBJECT itself claims (e.g.
    /// `DServRealization.node_id`) -- the field placement comparison
    /// actually keys on.
    pub observed_subject_node_id: String,
    pub admitted: bool,
    /// Present only when `admitted` is `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<&'static str>,
}

/// One desired D-Serv placement's derived convergence (section 25).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorServicePlacementConvergenceV1 {
    pub service_id: String,
    pub desired_node_id: String,
    pub convergence: DMonitorConvergenceV1,
    pub findings: Vec<DMonitorConvergenceFindingV1>,
    pub admitted_evidence: Vec<DMonitorEvidenceSummaryV1>,
    pub excluded_evidence: Vec<DMonitorEvidenceSummaryV1>,
    /// Every distinct `(monitor_instance_id, health)` claim seen among ALL
    /// relevant evidence (admitted or not) for this service -- preserved
    /// separately so health disagreement between monitor instances is
    /// visible without ever influencing `convergence` (section 15).
    pub observed_health_claims: Vec<DMonitorHealthClaimV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DMonitorHealthClaimV1 {
    pub monitor_instance_id: String,
    pub health: DMonitorHealthV1,
}

/// `{id, revision, digest}` summary of the DMap authority this evaluation
/// ran against -- `None` when there was no active authority to compare
/// against at all.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DMapRefSummaryV1 {
    pub dmap_id: String,
    pub revision: u64,
    pub digest: String,
}

/// Evidence for a `DServRealization` `service_id` that has NO desired
/// placement in the active authority at all (Phase 114C.1 closure, sections
/// 2-10) -- distinct from [`DMonitorServicePlacementConvergenceV1`], which
/// always has a `desired_node_id` to compare against. An unexpected service
/// has no desired node, so there is nothing to compare "wrong node" against
/// -- its mere presence, if admitted, IS the drift.
///
/// Included here for EVERY unexpected `service_id` that has ANY relevant
/// evidence at all (admitted or excluded), for transparency (section 4/6
/// Case C: stale/old-authority unexpected evidence is preserved, never
/// silently dropped) -- but only an entry with a non-empty
/// `admitted_evidence` actually escalates the enclosing application's
/// `convergence` to `Diverged` (section 6 Case C: historical/stale evidence
/// alone must never prove CURRENT divergence).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorUnexpectedServiceEvidenceV1 {
    pub service_id: String,
    pub admitted_evidence: Vec<DMonitorEvidenceSummaryV1>,
    pub excluded_evidence: Vec<DMonitorEvidenceSummaryV1>,
}

/// The top-level derived diagnostic view (section 25). This is NOT canonical
/// desired-state authority -- it is a derived, point-in-time interpretation
/// of authority-vs-evidence, recomputed fresh on every evaluation.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorApplicationConvergenceV1 {
    pub schema: String,
    pub project_id: String,
    pub application_id: String,
    /// Server-generated; never caller-supplied (section 27/8).
    pub evaluated_at_utc: String,
    pub dmap_ref: Option<DMapRefSummaryV1>,
    pub freshness_policy: DMonitorFreshnessPolicyV1,
    pub convergence: DMonitorConvergenceV1,
    pub findings: Vec<DMonitorConvergenceFindingV1>,
    pub services: Vec<DMonitorServicePlacementConvergenceV1>,
    /// `DServRealization` evidence for a `service_id` NOT present in the
    /// active desired placement set (Phase 114C.1 closure) -- see
    /// [`DMonitorUnexpectedServiceEvidenceV1`]. Every desired service having
    /// one correct observation is NOT sufficient for `Converged` if this is
    /// non-empty with admitted evidence: an unexpected but currently-proven
    /// realization is drift regardless of how the DESIRED services fared.
    pub unexpected_realizations: Vec<DMonitorUnexpectedServiceEvidenceV1>,
}

/// Evidence-oriented summary of one `DIoTRealization` observation (Phase
/// 114D0A, mirroring [`DMonitorEvidenceSummaryV1`] but tracking DIoT's THREE
/// independent correlation dimensions -- DMap, DForward, and runtime-binding
/// -- separately, plus whether the CURRENTLY REGISTERED runtime binding
/// itself is still current against DMap/2 + D-Node authority. Deliberately
/// never includes the observation's own `facts` blob (same rule as
/// [`DMonitorEvidenceSummaryV1`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorDiotEvidenceSummaryV1 {
    pub observation_id: String,
    pub observation_digest: String,
    pub node_id: String,
    pub monitor_instance_id: String,
    pub sequence: u64,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub freshness: DMonitorFreshnessV1,
    pub health: DMonitorHealthV1,
    pub dmap_correlation: AuthorityCorrelationV1,
    pub dforward_correlation: AuthorityCorrelationV1,
    pub runtime_binding_correlation: AuthorityCorrelationV1,
    /// `true` only when a `DIoTRuntimeBindingV1` is currently registered for
    /// this exact (project, application, placement) AND
    /// `verify_runtime_binding_still_current` accepts it against the
    /// current DMap/2 + D-Node authority. `false` whenever no binding is
    /// registered at all (in which case `runtime_binding_correlation` is
    /// always `RefMissing`) or when the registered binding is itself stale.
    pub runtime_binding_current: bool,
    /// The node_id the SUBJECT itself claims (`DIoTRealization.node_id`) --
    /// the field placement comparison keys on, mirroring D-Serv's
    /// `observed_subject_node_id`.
    pub observed_subject_node_id: String,
    pub admitted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclusion_reason: Option<&'static str>,
}

/// One desired DIoT placement's derived convergence (Phase 114D0A, mirroring
/// [`DMonitorServicePlacementConvergenceV1`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorDiotConvergenceV1 {
    pub project_id: String,
    pub application_id: String,
    pub diot_id: String,
    pub diot_placement_id: String,
    pub desired_node_id: String,
    pub convergence: DMonitorConvergenceV1,
    pub findings: Vec<DMonitorConvergenceFindingV1>,
    pub admitted_evidence: Vec<DMonitorDiotEvidenceSummaryV1>,
    pub excluded_evidence: Vec<DMonitorDiotEvidenceSummaryV1>,
    pub observed_health_claims: Vec<DMonitorHealthClaimV1>,
}

/// `DIoTRealization` evidence for a `(diot_id, diot_placement_id)` that has
/// NO desired placement in the active DMap/2 authority at all (Phase 114D0A,
/// mirroring [`DMonitorUnexpectedServiceEvidenceV1`]).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorUnexpectedDiotEvidenceV1 {
    pub diot_id: String,
    pub diot_placement_id: String,
    pub admitted_evidence: Vec<DMonitorDiotEvidenceSummaryV1>,
    pub excluded_evidence: Vec<DMonitorDiotEvidenceSummaryV1>,
}

/// Top-level derived DIoT convergence view for one application (Phase
/// 114D0A, mirroring [`DMonitorApplicationConvergenceV1`]). Kept as a
/// SEPARATE result type from the D-Serv one (section 31): DIoT evidence
/// being missing or Unknown must never change an already-computed D-Serv
/// `Converged`/`Diverged` result, and vice versa.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorApplicationDiotConvergenceV1 {
    pub schema: String,
    pub project_id: String,
    pub application_id: String,
    pub evaluated_at_utc: String,
    pub dmap_ref: Option<DMapRefSummaryV1>,
    pub freshness_policy: DMonitorFreshnessPolicyV1,
    pub convergence: DMonitorConvergenceV1,
    pub findings: Vec<DMonitorConvergenceFindingV1>,
    pub placements: Vec<DMonitorDiotConvergenceV1>,
    pub unexpected_realizations: Vec<DMonitorUnexpectedDiotEvidenceV1>,
}

// ---------------------------------------------------------------- DSERV CORE

/// Evaluates one application's desired D-Serv placements against currently
/// persisted DMonitor evidence (sections 10-18). Works identically for
/// `datum.dmap/1` and `datum.dmap/2` active authority: only
/// `PlacementAuthorityView`'s shared fields (`dmap_id`/`revision`/`digest`/
/// `placements`) are read, never `v2`-specific state -- there is no
/// DMap1/DMap2 fork in this comparison (section 16).
///
/// `authority == None` means the project/application has no active accepted
/// placement authority at all: this is ABSENCE OF DESIRED DEPLOYMENT
/// AUTHORITY, never runtime drift (section 30) -- the result is `Unknown`
/// with [`FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY`], never `Diverged`.
///
/// `dmonitor_state` MUST already be scoped/trusted as belonging to
/// `project_scope` by the caller in the sense that this function only reads
/// records whose OWN `server_project_scope` equals `project_scope` --
/// project isolation is enforced HERE, per-record, not merely assumed from
/// how the caller obtained `dmonitor_state` (section 29).
pub fn evaluate_application_dserv_convergence(
    project_id: &str,
    application_id: &str,
    project_scope: &str,
    authority: Option<&PlacementAuthorityView>,
    dmonitor_state: &DMonitorStateV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorApplicationConvergenceV1 {
    let Some(authority) = authority else {
        return DMonitorApplicationConvergenceV1 {
            schema: DMONITOR_CONVERGENCE_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            evaluated_at_utc: evaluated_at_utc.to_string(),
            dmap_ref: None,
            freshness_policy: *freshness_policy,
            convergence: DMonitorConvergenceV1::Unknown,
            findings: vec![finding(
                FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY,
                format!(
                    "project '{project_id}' application '{application_id}' has no active accepted placement authority"
                ),
            )],
            services: Vec::new(),
            // No dmap_ref to correlate ANY evidence against exactly, so no
            // evidence could ever be admitted as current-authority proof
            // either -- unexpected-realization detection is moot here, not
            // merely skipped.
            unexpected_realizations: Vec::new(),
        };
    };

    let dmap_ref = DMapRefSummaryV1 {
        dmap_id: authority.dmap_id.clone(),
        revision: authority.revision,
        digest: authority.digest.clone(),
    };

    // Phase 114C.1 closure (sections 2/9): ONE enumeration of every
    // currently-relevant `DServRealization` observation for this
    // project/application, for ANY service_id and ANY node -- not merely a
    // per-desired-service lookup, which by construction could never
    // discover a `service_id` absent from the desired placement set at all.
    // Both desired-service evaluation and unexpected-service detection are
    // built from this SAME scan, grouped by `service_id` below.
    let all_records = latest_dserv_realizations_for_application(
        dmonitor_state,
        project_scope,
        project_id,
        application_id,
    );
    let mut by_service: BTreeMap<String, Vec<DMonitorObservationRecordV1>> = BTreeMap::new();
    for record in all_records {
        if let DMonitorSubjectV1::DServRealization { service_id, .. } = &record.observation.subject {
            by_service.entry(service_id.clone()).or_default().push(record);
        }
    }

    let desired_service_ids: BTreeSet<&str> = authority
        .placements
        .iter()
        .map(|placement| placement.service_id.as_str())
        .collect();

    // Stable ordering (section 26): `authority.placements` order is already
    // deterministic (it is the accepted DMap's own persisted `Vec` order,
    // not a HashMap), so services are evaluated and returned in that same
    // order without re-sorting.
    let empty: Vec<DMonitorObservationRecordV1> = Vec::new();
    let services: Vec<DMonitorServicePlacementConvergenceV1> = authority
        .placements
        .iter()
        .map(|placement| {
            let relevant = by_service.get(&placement.service_id).unwrap_or(&empty);
            evaluate_service_placement(
                &placement.service_id,
                &placement.node_id,
                &dmap_ref,
                relevant,
                freshness_policy,
                evaluated_at_utc,
                project_scope,
            )
        })
        .collect();

    // Section 6 Case B: `by_service` iterates a `BTreeMap`, already
    // deterministically ordered by `service_id`.
    let unexpected_realizations: Vec<DMonitorUnexpectedServiceEvidenceV1> = by_service
        .iter()
        .filter(|(service_id, _)| !desired_service_ids.contains(service_id.as_str()))
        .map(|(service_id, records)| {
            evaluate_unexpected_service(service_id, records, &dmap_ref, freshness_policy, evaluated_at_utc)
        })
        .collect();

    let has_proven_unexpected = unexpected_realizations
        .iter()
        .any(|unexpected| !unexpected.admitted_evidence.is_empty());

    // Section 17/section 6: Diverged beats Unknown beats Converged; a
    // PROVEN unexpected realization is drift regardless of how the desired
    // services fared -- every desired service having one correct
    // observation is NOT sufficient for `Converged` if this is true.
    let convergence = if services
        .iter()
        .any(|service| service.convergence == DMonitorConvergenceV1::Diverged)
        || has_proven_unexpected
    {
        DMonitorConvergenceV1::Diverged
    } else if services
        .iter()
        .any(|service| service.convergence == DMonitorConvergenceV1::Unknown)
    {
        DMonitorConvergenceV1::Unknown
    } else if authority.placements.is_empty() {
        // Section 18/6 Case A: `all([]) == true` is NOT operational
        // convergence. Zero desired placements and no proven unexpected
        // realization means nothing this comparison can prove either way.
        DMonitorConvergenceV1::Unknown
    } else {
        DMonitorConvergenceV1::Converged
    };

    let findings = if authority.placements.is_empty() && !has_proven_unexpected {
        vec![finding(
            FINDING_NO_DESIRED_PLACEMENTS,
            "active placement authority declares zero D-Serv placements; nothing to converge",
        )]
    } else if has_proven_unexpected {
        let unexpected_ids: Vec<&str> = unexpected_realizations
            .iter()
            .filter(|unexpected| !unexpected.admitted_evidence.is_empty())
            .map(|unexpected| unexpected.service_id.as_str())
            .collect();
        vec![finding(
            FINDING_UNEXPECTED_REALIZATION,
            format!(
                "fresh, exact-authority-matching D-Serv realization evidence exists for service_id(s) {unexpected_ids:?}, which have no desired placement in the active authority at all"
            ),
        )]
    } else {
        Vec::new()
    };

    DMonitorApplicationConvergenceV1 {
        schema: DMONITOR_CONVERGENCE_SCHEMA.to_string(),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        evaluated_at_utc: evaluated_at_utc.to_string(),
        dmap_ref: Some(dmap_ref),
        freshness_policy: *freshness_policy,
        convergence,
        findings,
        services,
        unexpected_realizations,
    }
}

/// Assesses ONE `DServRealization` record's freshness + authority
/// correlation against `dmap_ref`, producing the shared
/// [`DMonitorEvidenceSummaryV1`] used identically by desired-service
/// evaluation ([`evaluate_service_placement`]) and unexpected-service
/// detection ([`evaluate_unexpected_service`]) -- Phase 114C.1 closure,
/// extracted so both paths apply IDENTICAL admission rules (section 4:
/// same server project scope, same project/application, `DServRealization`
/// subject, Fresh, exact current D-Map id/revision/digest).
fn assess_dserv_evidence(
    record: &DMonitorObservationRecordV1,
    dmap_ref: &DMapRefSummaryV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorEvidenceSummaryV1 {
    let DMonitorSubjectV1::DServRealization {
        node_id: observed_node_id,
        dmap_ref: observed_dmap_ref,
        ..
    } = &record.observation.subject
    else {
        unreachable!("assess_dserv_evidence is only ever called with DServRealization records, sourced from latest_dserv_realizations_for_application's own filter");
    };

    let freshness_evaluation = evaluate_freshness(record, freshness_policy, evaluated_at_utc);
    let (freshness, freshness_ok) = match &freshness_evaluation {
        Ok(evaluation) => (evaluation.freshness, evaluation.freshness == DMonitorFreshnessV1::Fresh),
        Err(_) => (DMonitorFreshnessV1::Stale, false),
    };

    let authority_correlation = match observed_dmap_ref {
        None => AuthorityCorrelationV1::RefMissing,
        Some(observed_ref) => {
            if observed_ref.id == dmap_ref.dmap_id
                && observed_ref.revision == dmap_ref.revision
                && observed_ref.digest == dmap_ref.digest
            {
                AuthorityCorrelationV1::ExactMatch
            } else {
                AuthorityCorrelationV1::Mismatch
            }
        }
    };

    let admitted = freshness_ok && authority_correlation == AuthorityCorrelationV1::ExactMatch;
    let exclusion_reason = if admitted {
        None
    } else if !freshness_ok {
        Some(match freshness {
            DMonitorFreshnessV1::ClockSkew => FINDING_CLOCK_SKEW,
            _ => FINDING_STALE_EVIDENCE,
        })
    } else {
        Some(match authority_correlation {
            AuthorityCorrelationV1::RefMissing => FINDING_AUTHORITY_REF_MISSING,
            AuthorityCorrelationV1::Mismatch => FINDING_AUTHORITY_MISMATCH,
            AuthorityCorrelationV1::ExactMatch => unreachable!("admitted case handled above"),
            AuthorityCorrelationV1::NotApplicable => unreachable!("D-Serv dmap_ref comparison never produces NotApplicable"),
        })
    };

    DMonitorEvidenceSummaryV1 {
        observation_id: record.observation.observation_id.clone(),
        observation_digest: record.observation_digest.clone(),
        node_id: record.observation.node_id.clone(),
        monitor_instance_id: record.observation.monitor_instance_id.clone(),
        sequence: record.observation.sequence,
        observed_at_utc: record.observation.observed_at_utc.clone(),
        received_at_utc: record.received_at_utc.clone(),
        freshness,
        health: record.observation.health,
        authority_correlation,
        observed_subject_node_id: observed_node_id.clone(),
        admitted,
        exclusion_reason,
    }
}

/// Evaluates evidence for a `service_id` that has NO desired placement at
/// all (Phase 114C.1 closure). Every record is assessed with the exact same
/// admission rule as a desired service; only records with `admitted ==
/// true` can prove CURRENT unexpected drift (section 4/6 Case C: stale or
/// old-authority evidence is preserved as excluded evidence, never treated
/// as proof).
fn evaluate_unexpected_service(
    service_id: &str,
    records: &[DMonitorObservationRecordV1],
    dmap_ref: &DMapRefSummaryV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorUnexpectedServiceEvidenceV1 {
    let mut admitted_evidence = Vec::new();
    let mut excluded_evidence = Vec::new();
    for record in records {
        let summary = assess_dserv_evidence(record, dmap_ref, freshness_policy, evaluated_at_utc);
        if summary.admitted {
            admitted_evidence.push(summary);
        } else {
            excluded_evidence.push(summary);
        }
    }
    admitted_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    excluded_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    DMonitorUnexpectedServiceEvidenceV1 {
        service_id: service_id.to_string(),
        admitted_evidence,
        excluded_evidence,
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_service_placement(
    service_id: &str,
    desired_node_id: &str,
    dmap_ref: &DMapRefSummaryV1,
    relevant: &[DMonitorObservationRecordV1],
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
    project_scope: &str,
) -> DMonitorServicePlacementConvergenceV1 {
    let mut admitted_evidence = Vec::new();
    let mut excluded_evidence = Vec::new();
    let mut health_claims: Vec<DMonitorHealthClaimV1> = Vec::new();

    for record in relevant {
        let health_claim = DMonitorHealthClaimV1 {
            monitor_instance_id: record.observation.monitor_instance_id.clone(),
            health: record.observation.health,
        };
        if !health_claims.contains(&health_claim) {
            health_claims.push(health_claim);
        }

        let summary = assess_dserv_evidence(record, dmap_ref, freshness_policy, evaluated_at_utc);
        if summary.admitted {
            admitted_evidence.push(summary);
        } else {
            excluded_evidence.push(summary);
        }
    }

    // Stable ordering (section 26): sort by (observation_id) -- derived
    // from BTreeMap iteration already, but sorted explicitly so ordering
    // never silently depends on iteration order of any collection type
    // used internally.
    admitted_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    excluded_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    health_claims.sort_by(|a, b| a.monitor_instance_id.cmp(&b.monitor_instance_id));

    let matching: Vec<&DMonitorEvidenceSummaryV1> = admitted_evidence
        .iter()
        .filter(|evidence| evidence.observed_subject_node_id == desired_node_id)
        .collect();
    let other: Vec<&DMonitorEvidenceSummaryV1> = admitted_evidence
        .iter()
        .filter(|evidence| evidence.observed_subject_node_id != desired_node_id)
        .collect();
    let distinct_other_nodes: BTreeSet<&str> = other
        .iter()
        .map(|evidence| evidence.observed_subject_node_id.as_str())
        .collect();

    let (convergence, findings) = if !matching.is_empty() && other.is_empty() {
        (
            DMonitorConvergenceV1::Converged,
            vec![finding(
                FINDING_EXACT_MATCH,
                format!("fresh, exact-authority-matching evidence confirms service '{service_id}' on desired node '{desired_node_id}'"),
            )],
        )
    } else if !matching.is_empty() && !other.is_empty() {
        (
            DMonitorConvergenceV1::Diverged,
            vec![finding(
                FINDING_UNEXPECTED_REALIZATION,
                format!(
                    "service '{service_id}' has fresh, exact-authority-matching evidence on the desired node '{desired_node_id}' AND on unexpected node(s) {distinct_other_nodes:?}"
                ),
            )],
        )
    } else if matching.is_empty() && distinct_other_nodes.len() == 1 {
        let wrong_node = distinct_other_nodes.iter().next().copied().unwrap_or("");
        (
            DMonitorConvergenceV1::Diverged,
            vec![finding(
                FINDING_WRONG_NODE,
                format!(
                    "service '{service_id}' has fresh, exact-authority-matching evidence on node '{wrong_node}', not the desired node '{desired_node_id}'"
                ),
            )],
        )
    } else if matching.is_empty() && distinct_other_nodes.len() > 1 {
        (
            DMonitorConvergenceV1::Diverged,
            vec![finding(
                FINDING_CONFLICTING_FRESH_EVIDENCE,
                format!(
                    "service '{service_id}' has fresh, exact-authority-matching evidence on multiple non-desired nodes {distinct_other_nodes:?}, none of them the desired node '{desired_node_id}'"
                ),
            )],
        )
    } else {
        // No admitted evidence at all. Unknown, with every distinct
        // exclusion reason actually observed among the excluded evidence,
        // in a fixed priority order (most specific/actionable first) --
        // never a guess, and never Diverged merely because nothing was
        // admitted (section 9/19: absence of proof is not proof of drift).
        let mut reasons = Vec::new();
        for code in [
            FINDING_AUTHORITY_REF_MISSING,
            FINDING_AUTHORITY_MISMATCH,
            FINDING_CLOCK_SKEW,
            FINDING_STALE_EVIDENCE,
        ] {
            if excluded_evidence
                .iter()
                .any(|evidence| evidence.exclusion_reason == Some(code))
            {
                reasons.push(finding(
                    code,
                    format!("service '{service_id}' has excluded evidence: {code}"),
                ));
            }
        }
        if reasons.is_empty() {
            reasons.push(finding(
                FINDING_NO_EVIDENCE,
                format!("no DServRealization evidence exists for service '{service_id}' in project scope '{project_scope}'"),
            ));
        }
        (DMonitorConvergenceV1::Unknown, reasons)
    };

    DMonitorServicePlacementConvergenceV1 {
        service_id: service_id.to_string(),
        desired_node_id: desired_node_id.to_string(),
        convergence,
        findings,
        admitted_evidence,
        excluded_evidence,
        observed_health_claims: health_claims,
    }
}

/// Scans every project-scoped stream for the LATEST occurrence, PER STREAM
/// AND PER EXACT `(service_id, node_id)` SUBJECT, of a `DServRealization`
/// subject belonging to `(project_id, application_id)` -- for ANY
/// `service_id` and ANY observed node, not filtered to one desired service
/// (Phase 114C.1 closure, sections 2/9: unexpected-service-id detection
/// requires evidence for the WHOLE application, not just a lookup keyed by
/// one already-known desired service).
///
/// This deliberately does not use [`DMonitorStateV1::subject_candidates`]:
/// that accessor requires an EXACT subject key, which for `DServRealization`
/// bakes in both `service_id` AND `node_id` -- looking up only one known
/// (service, node) pair could never discover evidence of an ENTIRELY
/// DIFFERENT service_id (one absent from the desired placement set
/// altogether), nor of the same service claiming to run on a different
/// node. Instead this replicates `subject_index`'s own "latest per exact
/// subject per stream" construction (Phase 114B1.1's rule) directly against
/// the typed subject enum, keyed by `(service_id, node_id)` -- exactly
/// `DServRealization`'s own real subject-identity granularity -- so:
///
/// - a stream that interleaves seq1=service X / seq2=node health /
///   seq3=service Y / seq4=service X again correctly yields ONLY seq4 for
///   service X in that stream (never the superseded seq1), exactly matching
///   Phase 114B1.1's existing test precedent
///   (`subject_index_tracks_the_latest_observation_per_subject_per_stream_not_the_stream_head`);
/// - a stream that reports the SAME service on TWO DIFFERENT nodes (e.g.
///   seq1=X@node-A, seq3=X@node-B) correctly yields BOTH as independent,
///   equally "current for their own stream" subjects -- Phase 114C.1's
///   closure fix: the ORIGINAL Phase 114C implementation tracked only one
///   "latest for this stream" value per `service_id` regardless of node,
///   which would have silently discarded X@node-A the moment X@node-B was
///   seen later in the same stream, even though they are two DIFFERENT
///   canonical subjects (subject_key embeds `node_id`), not a
///   supersede-in-place update of the same one.
///
/// Project isolation (section 29) is enforced here: only streams whose
/// `project_scope` exactly equals `project_scope` are considered.
fn latest_dserv_realizations_for_application(
    dmonitor_state: &DMonitorStateV1,
    project_scope: &str,
    project_id: &str,
    application_id: &str,
) -> Vec<DMonitorObservationRecordV1> {
    let mut results = Vec::new();
    for stream in dmonitor_state.streams.values() {
        if stream.project_scope != project_scope {
            continue;
        }
        // (service_id, node_id) -> latest record within this stream.
        // Ascending sequence order (BTreeMap's natural order): later
        // sequences overwrite earlier ones for the EXACT same (service,
        // node) subject only, reproducing "latest per exact subject per
        // stream" without needing a pre-computed subject key.
        let mut latest_per_subject: BTreeMap<(String, String), DMonitorObservationRecordV1> = BTreeMap::new();
        for record in stream.observations.values() {
            if let DMonitorSubjectV1::DServRealization {
                project_id: p,
                application_id: a,
                service_id: s,
                node_id: n,
                ..
            } = &record.observation.subject
            {
                if p == project_id && a == application_id {
                    latest_per_subject.insert((s.clone(), n.clone()), record.clone());
                }
            }
        }
        results.extend(latest_per_subject.into_values());
    }
    // Stable ordering (section 26): sort by (node_id, monitor_instance_id,
    // observation_id) -- BTreeMap iteration over `streams` is already
    // key-ordered, but this makes the ordering guarantee explicit and
    // independent of stream_key encoding details.
    results.sort_by(|a, b| {
        (
            a.observation.node_id.as_str(),
            a.observation.monitor_instance_id.as_str(),
            a.observation.observation_id.as_str(),
        )
            .cmp(&(
                b.observation.node_id.as_str(),
                b.observation.monitor_instance_id.as_str(),
                b.observation.observation_id.as_str(),
            ))
    });
    results
}

// ------------------------------------------------------------------- DIOT CORE
//
// Phase 114D0A closure: the Phase 114C audit conclusion (the OLD
// `DIoTRealization` shape carried no observed node_id, no DMap/2 reference,
// and no runtime-binding reference, so exact current DIoT convergence could
// never be proven) is closed by the canonical model change in
// `canonical_dmonitor.rs` (required `node_id`/`diot_id`/`diot_placement_id`,
// optional `dmap_ref`/`dforward_ref`/`runtime_binding_ref`). This module now
// implements the real 9-point exact-correlation algorithm documented in
// ADR-0024's "Phase 114D0A" section:
//
// 1. Enumerate every currently-relevant `DIoTRealization` observation for
//    the application (ANY diot_id/placement/node), mirroring
//    `latest_dserv_realizations_for_application`'s "latest per exact
//    subject per stream" rule exactly (`latest_diot_realizations_for_application`).
// 2. Enumerate desired DIoT placements from `authority.v2.diot_placements`.
//    `authority.v2 == None` (a `datum.dmap/1` active authority) means DIoT
//    placement is NOT APPLICABLE at all -- `Unknown` with
//    `FINDING_DMAP_V1_NO_DIOT_SUPPORT`, never `Diverged` (section 30's rule
//    extended: absence of a concept a DMap version doesn't support is not
//    drift).
// 3. Freshness-gate each observation exactly like D-Serv
//    (`dmonitor_freshness::evaluate_freshness`).
// 4. DMap correlation: observed `dmap_ref` vs current `{dmap_id, revision,
//    digest}` -- identical rule to D-Serv's `dmap_ref` comparison.
// 5. DForward correlation: observed `dforward_ref` vs current
//    `{dforward_id, dforward_revision, dforward_digest}` from
//    `authority.v2`.
// 6. Runtime-binding correlation: observed `runtime_binding_ref` vs the
//    CURRENTLY REGISTERED `DIoTRuntimeBindingV1` for this exact
//    `(project_id, application_id, diot_placement_id)`, looked up from
//    `DIoTRuntimeBindingRegistryStateV1::get`. No registration at all is
//    `FINDING_RUNTIME_BINDING_NOT_REGISTERED` (never treated as "not
//    applicable" -- a DMap/2 DIoT placement always has a governed runtime
//    binding once realized).
// 7. Runtime-binding currentness: the registered binding (if any) is passed
//    through `verify_runtime_binding_still_current` UNMODIFIED (never
//    reimplemented) against the SAME `ddeploy_state`/`dnode_registry_state`
//    this evaluation was given. A binding that fails this check can never
//    contribute to `admitted` evidence even if the observation's own
//    `runtime_binding_ref` matches it exactly -- the registration itself is
//    stale.
// 8. Node match: `observed_subject_node_id` (the subject's OWN `node_id`,
//    never the observation's top-level observer `node_id`) vs the desired
//    placement's `node_id` -- identical matching/other/wrong-node/
//    conflicting-node combinatorics to
//    `evaluate_service_placement` (section 18/19 carried over unchanged).
// 9. Evidence for a `(diot_id, diot_placement_id)` absent from the desired
//    set is unexpected-realization drift, evaluated with the SAME admission
//    rule (section 9, mirroring `evaluate_unexpected_service`).
//
// Evidence is `admitted` (able to prove CURRENT convergence) only when
// ALL of: Fresh, DMap correlation ExactMatch, DForward correlation
// ExactMatch-or-NotApplicable, runtime-binding correlation ExactMatch, AND
// the currently-registered binding verifies still-current. Any one failure
// excludes the evidence with the single most specific reason (section 9's
// "no server-synthesized observation" rule: a registered binding alone,
// with no admitted fresh evidence naming it, never synthesizes Converged).
#[allow(clippy::too_many_arguments)]
pub fn evaluate_application_diot_convergence(
    project_id: &str,
    application_id: &str,
    project_scope: &str,
    authority: Option<&PlacementAuthorityView>,
    dmonitor_state: &DMonitorStateV1,
    runtime_binding_registry: &crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1,
    ddeploy_state: &crate::core::ddeploy::DDeployStateV1,
    dnode_registry_state: &crate::core::dnode_registry::DNodeRegistryStateV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorApplicationDiotConvergenceV1 {
    let Some(authority) = authority else {
        return DMonitorApplicationDiotConvergenceV1 {
            schema: DMONITOR_CONVERGENCE_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            evaluated_at_utc: evaluated_at_utc.to_string(),
            dmap_ref: None,
            freshness_policy: *freshness_policy,
            convergence: DMonitorConvergenceV1::Unknown,
            findings: vec![finding(
                FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY,
                format!(
                    "project '{project_id}' application '{application_id}' has no active accepted placement authority"
                ),
            )],
            placements: Vec::new(),
            unexpected_realizations: Vec::new(),
        };
    };

    let dmap_ref = DMapRefSummaryV1 {
        dmap_id: authority.dmap_id.clone(),
        revision: authority.revision,
        digest: authority.digest.clone(),
    };

    // Section 30-style rule (point 2): `datum.dmap/1` has no DForward/DIoT
    // placement concept at all. This is absence of a supported concept, not
    // observed drift.
    let Some(v2) = authority.v2.as_ref() else {
        return DMonitorApplicationDiotConvergenceV1 {
            schema: DMONITOR_CONVERGENCE_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            evaluated_at_utc: evaluated_at_utc.to_string(),
            dmap_ref: Some(dmap_ref),
            freshness_policy: *freshness_policy,
            convergence: DMonitorConvergenceV1::Unknown,
            findings: vec![finding(
                FINDING_DMAP_V1_NO_DIOT_SUPPORT,
                format!(
                    "project '{project_id}' application '{application_id}' active authority is datum.dmap/1, which has no DIoT placement concept"
                ),
            )],
            placements: Vec::new(),
            unexpected_realizations: Vec::new(),
        };
    };

    let all_records = latest_diot_realizations_for_application(dmonitor_state, project_scope, project_id, application_id);
    let mut by_placement: BTreeMap<(String, String), Vec<DMonitorObservationRecordV1>> = BTreeMap::new();
    for record in all_records {
        if let DMonitorSubjectV1::DIoTRealization { diot_id, diot_placement_id, .. } = &record.observation.subject {
            by_placement
                .entry((diot_id.clone(), diot_placement_id.clone()))
                .or_default()
                .push(record);
        }
    }

    let desired: BTreeSet<(&str, &str)> = v2
        .diot_placements
        .iter()
        .map(|placement| (placement.diot_id.as_str(), placement.placement_id.as_str()))
        .collect();

    let empty: Vec<DMonitorObservationRecordV1> = Vec::new();
    let placements: Vec<DMonitorDiotConvergenceV1> = v2
        .diot_placements
        .iter()
        .map(|placement| {
            let relevant = by_placement
                .get(&(placement.diot_id.clone(), placement.placement_id.clone()))
                .unwrap_or(&empty);
            evaluate_diot_placement(
                project_id,
                application_id,
                &placement.diot_id,
                &placement.placement_id,
                &placement.node_id,
                &dmap_ref,
                v2,
                relevant,
                runtime_binding_registry,
                ddeploy_state,
                dnode_registry_state,
                freshness_policy,
                evaluated_at_utc,
                project_scope,
            )
        })
        .collect();

    let unexpected_realizations: Vec<DMonitorUnexpectedDiotEvidenceV1> = by_placement
        .iter()
        .filter(|((diot_id, placement_id), _)| !desired.contains(&(diot_id.as_str(), placement_id.as_str())))
        .map(|((diot_id, placement_id), records)| {
            evaluate_unexpected_diot(
                diot_id,
                placement_id,
                records,
                &dmap_ref,
                v2,
                runtime_binding_registry,
                project_id,
                application_id,
                ddeploy_state,
                dnode_registry_state,
                freshness_policy,
                evaluated_at_utc,
            )
        })
        .collect();

    let has_proven_unexpected = unexpected_realizations
        .iter()
        .any(|unexpected| !unexpected.admitted_evidence.is_empty());

    let convergence = if placements
        .iter()
        .any(|placement| placement.convergence == DMonitorConvergenceV1::Diverged)
        || has_proven_unexpected
    {
        DMonitorConvergenceV1::Diverged
    } else if placements
        .iter()
        .any(|placement| placement.convergence == DMonitorConvergenceV1::Unknown)
    {
        DMonitorConvergenceV1::Unknown
    } else if v2.diot_placements.is_empty() {
        DMonitorConvergenceV1::Unknown
    } else {
        DMonitorConvergenceV1::Converged
    };

    let findings = if v2.diot_placements.is_empty() && !has_proven_unexpected {
        vec![finding(
            FINDING_NO_DESIRED_DIOT_PLACEMENTS,
            "active DMap/2 placement authority declares zero DIoT placements; nothing to converge",
        )]
    } else if has_proven_unexpected {
        let unexpected_ids: Vec<String> = unexpected_realizations
            .iter()
            .filter(|unexpected| !unexpected.admitted_evidence.is_empty())
            .map(|unexpected| format!("{}/{}", unexpected.diot_id, unexpected.diot_placement_id))
            .collect();
        vec![finding(
            FINDING_UNEXPECTED_REALIZATION,
            format!(
                "fresh, exact-authority-matching DIoT realization evidence exists for placement(s) {unexpected_ids:?}, which have no desired placement in the active authority at all"
            ),
        )]
    } else {
        Vec::new()
    };

    DMonitorApplicationDiotConvergenceV1 {
        schema: DMONITOR_CONVERGENCE_SCHEMA.to_string(),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        evaluated_at_utc: evaluated_at_utc.to_string(),
        dmap_ref: Some(dmap_ref),
        freshness_policy: *freshness_policy,
        convergence,
        findings,
        placements,
        unexpected_realizations,
    }
}

/// Assesses ONE `DIoTRealization` record's freshness + three-dimensional
/// authority correlation (point 3-7 of the algorithm above), producing the
/// shared [`DMonitorDiotEvidenceSummaryV1`] used identically by
/// desired-placement evaluation and unexpected-placement detection.
#[allow(clippy::too_many_arguments)]
fn assess_diot_evidence(
    record: &DMonitorObservationRecordV1,
    dmap_ref: &DMapRefSummaryV1,
    v2: &PlacementAuthorityV2Extras,
    runtime_binding_registry: &crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1,
    project_id: &str,
    application_id: &str,
    diot_placement_id: &str,
    ddeploy_state: &crate::core::ddeploy::DDeployStateV1,
    dnode_registry_state: &crate::core::dnode_registry::DNodeRegistryStateV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorDiotEvidenceSummaryV1 {
    let DMonitorSubjectV1::DIoTRealization {
        node_id: observed_node_id,
        dmap_ref: observed_dmap_ref,
        dforward_ref: observed_dforward_ref,
        runtime_binding_ref: observed_runtime_binding_ref,
        ..
    } = &record.observation.subject
    else {
        unreachable!("assess_diot_evidence is only ever called with DIoTRealization records");
    };

    let freshness_evaluation = evaluate_freshness(record, freshness_policy, evaluated_at_utc);
    let (freshness, freshness_ok) = match &freshness_evaluation {
        Ok(evaluation) => (evaluation.freshness, evaluation.freshness == DMonitorFreshnessV1::Fresh),
        Err(_) => (DMonitorFreshnessV1::Stale, false),
    };

    let dmap_correlation = match observed_dmap_ref {
        None => AuthorityCorrelationV1::RefMissing,
        Some(observed_ref) => {
            if observed_ref.id == dmap_ref.dmap_id && observed_ref.revision == dmap_ref.revision && observed_ref.digest == dmap_ref.digest {
                AuthorityCorrelationV1::ExactMatch
            } else {
                AuthorityCorrelationV1::Mismatch
            }
        }
    };

    let dforward_correlation = match observed_dforward_ref {
        None => AuthorityCorrelationV1::RefMissing,
        Some(observed_ref) => {
            if observed_ref.id == v2.dforward_id && observed_ref.revision == v2.dforward_revision && observed_ref.digest == v2.dforward_digest {
                AuthorityCorrelationV1::ExactMatch
            } else {
                AuthorityCorrelationV1::Mismatch
            }
        }
    };

    let registered_binding = runtime_binding_registry.get(project_id, application_id, diot_placement_id);
    let runtime_binding_current = registered_binding
        .map(|binding| {
            crate::core::diot_runtime_binding::verify_runtime_binding_still_current(binding, ddeploy_state, dnode_registry_state).is_ok()
        })
        .unwrap_or(false);

    let runtime_binding_correlation = match (observed_runtime_binding_ref, registered_binding) {
        (_, None) => AuthorityCorrelationV1::RefMissing,
        (None, Some(_)) => AuthorityCorrelationV1::RefMissing,
        (Some(observed_ref), Some(registered)) => {
            if observed_ref.binding_id == registered.binding_id
                && observed_ref.binding_revision == registered.binding_revision
                && observed_ref.descriptor_digest == registered.descriptor_digest
            {
                AuthorityCorrelationV1::ExactMatch
            } else {
                AuthorityCorrelationV1::Mismatch
            }
        }
    };

    let admitted = freshness_ok
        && dmap_correlation == AuthorityCorrelationV1::ExactMatch
        && dforward_correlation == AuthorityCorrelationV1::ExactMatch
        && runtime_binding_correlation == AuthorityCorrelationV1::ExactMatch
        && runtime_binding_current;

    let exclusion_reason = if admitted {
        None
    } else if !freshness_ok {
        Some(match freshness {
            DMonitorFreshnessV1::ClockSkew => FINDING_CLOCK_SKEW,
            _ => FINDING_STALE_EVIDENCE,
        })
    } else if dmap_correlation != AuthorityCorrelationV1::ExactMatch {
        Some(match dmap_correlation {
            AuthorityCorrelationV1::RefMissing => FINDING_AUTHORITY_REF_MISSING,
            AuthorityCorrelationV1::Mismatch => FINDING_AUTHORITY_MISMATCH,
            _ => unreachable!("admitted case handled above"),
        })
    } else if dforward_correlation != AuthorityCorrelationV1::ExactMatch {
        Some(match dforward_correlation {
            AuthorityCorrelationV1::RefMissing => FINDING_AUTHORITY_REF_MISSING,
            AuthorityCorrelationV1::Mismatch => FINDING_AUTHORITY_MISMATCH,
            _ => unreachable!("admitted case handled above"),
        })
    } else if registered_binding.is_none() {
        Some(FINDING_RUNTIME_BINDING_NOT_REGISTERED)
    } else if runtime_binding_correlation != AuthorityCorrelationV1::ExactMatch {
        Some(match runtime_binding_correlation {
            AuthorityCorrelationV1::RefMissing => FINDING_RUNTIME_BINDING_REF_MISSING,
            AuthorityCorrelationV1::Mismatch => FINDING_RUNTIME_BINDING_MISMATCH,
            _ => unreachable!("admitted case handled above"),
        })
    } else {
        Some(FINDING_RUNTIME_BINDING_STALE)
    };

    DMonitorDiotEvidenceSummaryV1 {
        observation_id: record.observation.observation_id.clone(),
        observation_digest: record.observation_digest.clone(),
        node_id: record.observation.node_id.clone(),
        monitor_instance_id: record.observation.monitor_instance_id.clone(),
        sequence: record.observation.sequence,
        observed_at_utc: record.observation.observed_at_utc.clone(),
        received_at_utc: record.received_at_utc.clone(),
        freshness,
        health: record.observation.health,
        dmap_correlation,
        dforward_correlation,
        runtime_binding_correlation,
        runtime_binding_current,
        observed_subject_node_id: observed_node_id.clone(),
        admitted,
        exclusion_reason,
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_unexpected_diot(
    diot_id: &str,
    diot_placement_id: &str,
    records: &[DMonitorObservationRecordV1],
    dmap_ref: &DMapRefSummaryV1,
    v2: &PlacementAuthorityV2Extras,
    runtime_binding_registry: &crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1,
    project_id: &str,
    application_id: &str,
    ddeploy_state: &crate::core::ddeploy::DDeployStateV1,
    dnode_registry_state: &crate::core::dnode_registry::DNodeRegistryStateV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorUnexpectedDiotEvidenceV1 {
    let mut admitted_evidence = Vec::new();
    let mut excluded_evidence = Vec::new();
    for record in records {
        let summary = assess_diot_evidence(
            record,
            dmap_ref,
            v2,
            runtime_binding_registry,
            project_id,
            application_id,
            diot_placement_id,
            ddeploy_state,
            dnode_registry_state,
            freshness_policy,
            evaluated_at_utc,
        );
        if summary.admitted {
            admitted_evidence.push(summary);
        } else {
            excluded_evidence.push(summary);
        }
    }
    admitted_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    excluded_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    DMonitorUnexpectedDiotEvidenceV1 {
        diot_id: diot_id.to_string(),
        diot_placement_id: diot_placement_id.to_string(),
        admitted_evidence,
        excluded_evidence,
    }
}

#[allow(clippy::too_many_arguments)]
fn evaluate_diot_placement(
    project_id: &str,
    application_id: &str,
    diot_id: &str,
    diot_placement_id: &str,
    desired_node_id: &str,
    dmap_ref: &DMapRefSummaryV1,
    v2: &PlacementAuthorityV2Extras,
    relevant: &[DMonitorObservationRecordV1],
    runtime_binding_registry: &crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1,
    ddeploy_state: &crate::core::ddeploy::DDeployStateV1,
    dnode_registry_state: &crate::core::dnode_registry::DNodeRegistryStateV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    evaluated_at_utc: &str,
    project_scope: &str,
) -> DMonitorDiotConvergenceV1 {
    let mut admitted_evidence = Vec::new();
    let mut excluded_evidence = Vec::new();
    let mut health_claims: Vec<DMonitorHealthClaimV1> = Vec::new();

    for record in relevant {
        let health_claim = DMonitorHealthClaimV1 {
            monitor_instance_id: record.observation.monitor_instance_id.clone(),
            health: record.observation.health,
        };
        if !health_claims.contains(&health_claim) {
            health_claims.push(health_claim);
        }

        let summary = assess_diot_evidence(
            record,
            dmap_ref,
            v2,
            runtime_binding_registry,
            project_id,
            application_id,
            diot_placement_id,
            ddeploy_state,
            dnode_registry_state,
            freshness_policy,
            evaluated_at_utc,
        );
        if summary.admitted {
            admitted_evidence.push(summary);
        } else {
            excluded_evidence.push(summary);
        }
    }

    admitted_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    excluded_evidence.sort_by(|a, b| a.observation_id.cmp(&b.observation_id));
    health_claims.sort_by(|a, b| a.monitor_instance_id.cmp(&b.monitor_instance_id));

    let matching: Vec<&DMonitorDiotEvidenceSummaryV1> = admitted_evidence
        .iter()
        .filter(|evidence| evidence.observed_subject_node_id == desired_node_id)
        .collect();
    let other: Vec<&DMonitorDiotEvidenceSummaryV1> = admitted_evidence
        .iter()
        .filter(|evidence| evidence.observed_subject_node_id != desired_node_id)
        .collect();
    let distinct_other_nodes: BTreeSet<&str> = other
        .iter()
        .map(|evidence| evidence.observed_subject_node_id.as_str())
        .collect();

    let (convergence, findings) = if !matching.is_empty() && other.is_empty() {
        (
            DMonitorConvergenceV1::Converged,
            vec![finding(
                FINDING_EXACT_MATCH,
                format!("fresh, exact-authority-matching (DMap+DForward+runtime-binding, currently-verified) evidence confirms DIoT placement '{diot_placement_id}' on desired node '{desired_node_id}'"),
            )],
        )
    } else if !matching.is_empty() && !other.is_empty() {
        (
            DMonitorConvergenceV1::Diverged,
            vec![finding(
                FINDING_UNEXPECTED_REALIZATION,
                format!(
                    "DIoT placement '{diot_placement_id}' has fresh, exact-authority-matching evidence on the desired node '{desired_node_id}' AND on unexpected node(s) {distinct_other_nodes:?}"
                ),
            )],
        )
    } else if matching.is_empty() && distinct_other_nodes.len() == 1 {
        let wrong_node = distinct_other_nodes.iter().next().copied().unwrap_or("");
        (
            DMonitorConvergenceV1::Diverged,
            vec![finding(
                FINDING_WRONG_NODE,
                format!(
                    "DIoT placement '{diot_placement_id}' has fresh, exact-authority-matching evidence on node '{wrong_node}', not the desired node '{desired_node_id}'"
                ),
            )],
        )
    } else if matching.is_empty() && distinct_other_nodes.len() > 1 {
        (
            DMonitorConvergenceV1::Diverged,
            vec![finding(
                FINDING_CONFLICTING_FRESH_EVIDENCE,
                format!(
                    "DIoT placement '{diot_placement_id}' has fresh, exact-authority-matching evidence on multiple non-desired nodes {distinct_other_nodes:?}, none of them the desired node '{desired_node_id}'"
                ),
            )],
        )
    } else {
        let mut reasons = Vec::new();
        for code in [
            FINDING_RUNTIME_BINDING_NOT_REGISTERED,
            FINDING_RUNTIME_BINDING_STALE,
            FINDING_RUNTIME_BINDING_REF_MISSING,
            FINDING_RUNTIME_BINDING_MISMATCH,
            FINDING_AUTHORITY_REF_MISSING,
            FINDING_AUTHORITY_MISMATCH,
            FINDING_CLOCK_SKEW,
            FINDING_STALE_EVIDENCE,
        ] {
            if excluded_evidence.iter().any(|evidence| evidence.exclusion_reason == Some(code)) {
                reasons.push(finding(
                    code,
                    format!("DIoT placement '{diot_placement_id}' has excluded evidence: {code}"),
                ));
            }
        }
        if reasons.is_empty() {
            reasons.push(finding(
                FINDING_NO_EVIDENCE,
                format!("no DIoTRealization evidence exists for placement '{diot_placement_id}' (diot '{diot_id}') in project scope '{project_scope}'"),
            ));
        }
        (DMonitorConvergenceV1::Unknown, reasons)
    };

    DMonitorDiotConvergenceV1 {
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        diot_id: diot_id.to_string(),
        diot_placement_id: diot_placement_id.to_string(),
        desired_node_id: desired_node_id.to_string(),
        convergence,
        findings,
        admitted_evidence,
        excluded_evidence,
        observed_health_claims: health_claims,
    }
}

/// Scans every project-scoped stream for the LATEST occurrence, PER STREAM
/// AND PER EXACT `(diot_id, diot_placement_id, node_id)` SUBJECT, of a
/// `DIoTRealization` subject belonging to `(project_id, application_id)` --
/// mirroring `latest_dserv_realizations_for_application` exactly (see that
/// function's doc comment for the full rationale; the same "latest per exact
/// subject per stream, never the stream head" and "same placement on two
/// different nodes are two independent subjects" rules apply unchanged).
/// Project isolation (section 29) is enforced here identically.
fn latest_diot_realizations_for_application(
    dmonitor_state: &DMonitorStateV1,
    project_scope: &str,
    project_id: &str,
    application_id: &str,
) -> Vec<DMonitorObservationRecordV1> {
    let mut results = Vec::new();
    for stream in dmonitor_state.streams.values() {
        if stream.project_scope != project_scope {
            continue;
        }
        let mut latest_per_subject: BTreeMap<(String, String, String), DMonitorObservationRecordV1> = BTreeMap::new();
        for record in stream.observations.values() {
            if let DMonitorSubjectV1::DIoTRealization {
                project_id: p,
                application_id: a,
                diot_id: d,
                diot_placement_id: pl,
                node_id: n,
                ..
            } = &record.observation.subject
            {
                if p == project_id && a == application_id {
                    latest_per_subject.insert((d.clone(), pl.clone(), n.clone()), record.clone());
                }
            }
        }
        results.extend(latest_per_subject.into_values());
    }
    results.sort_by(|a, b| {
        (
            a.observation.node_id.as_str(),
            a.observation.monitor_instance_id.as_str(),
            a.observation.observation_id.as_str(),
        )
            .cmp(&(
                b.observation.node_id.as_str(),
                b.observation.monitor_instance_id.as_str(),
                b.observation.observation_id.as_str(),
            ))
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorContractRefV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use crate::core::ddeploy_v2::PlacementAuthorityV2Extras;
    use crate::core::canonical_dmap_v2::{DCallRealizationV2, DIoTPlacementV2, DServPlacementV2};
    use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};

    const PROJECT: &str = "proj:lab";
    const OTHER_PROJECT: &str = "proj:other";
    const APP: &str = "app:pipeline";
    const SVC_A: &str = "sensor";
    const SVC_B: &str = "decoder";
    const NODE_A: &str = "fog-01";
    const NODE_B: &str = "fog-02";
    const DMAP_ID: &str = "dmap:1";
    const DMAP_REVISION: u64 = 3;
    fn dmap_digest() -> String {
        format!("sha256:{}", "a".repeat(64))
    }
    const OLD_DMAP_REVISION: u64 = 2;
    fn old_dmap_digest() -> String {
        format!("sha256:{}", "b".repeat(64))
    }

    fn authority(placements: Vec<(&str, &str)>) -> PlacementAuthorityView {
        PlacementAuthorityView {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            dmap_id: DMAP_ID.to_string(),
            revision: DMAP_REVISION,
            digest: dmap_digest(),
            placements: placements
                .into_iter()
                .map(|(service_id, node_id)| DServPlacementV2 {
                    service_id: service_id.to_string(),
                    node_id: node_id.to_string(),
                })
                .collect(),
            proposal_id: None,
            acceptance_id: None,
            dgraph_id: None,
            dgraph_revision: None,
            dgraph_digest: None,
            dgraph_snapshot: None,
            v2: None,
        }
    }

    /// Same authority, but with `v2` populated -- proves the comparison
    /// engine reads only the shared fields and does not fork on DMap1 vs
    /// DMap2 (section 16).
    fn authority_v2(placements: Vec<(&str, &str)>) -> PlacementAuthorityView {
        let mut view = authority(placements);
        view.v2 = Some(PlacementAuthorityV2Extras {
            dforward_id: "dforward:1".to_string(),
            dforward_revision: 1,
            dforward_digest: format!("sha256:{}", "c".repeat(64)),
            dcall_realizations: Vec::<DCallRealizationV2>::new(),
            diot_placements: Vec::<DIoTPlacementV2>::new(),
            dforward_snapshot: None,
            external_port_bindings: Vec::new(),
        });
        view
    }

    fn policy() -> DMonitorFreshnessPolicyV1 {
        DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 60,
            max_received_age_seconds: 60,
            max_future_clock_skew_seconds: 10,
        }
    }

    fn exact_ref() -> DMonitorContractRefV1 {
        DMonitorContractRefV1 {
            id: DMAP_ID.to_string(),
            revision: DMAP_REVISION,
            digest: dmap_digest(),
        }
    }

    fn old_ref() -> DMonitorContractRefV1 {
        DMonitorContractRefV1 {
            id: DMAP_ID.to_string(),
            revision: OLD_DMAP_REVISION,
            digest: old_dmap_digest(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dserv_observation(
        node_id: &str,
        monitor_instance_id: &str,
        sequence: u64,
        service_id: &str,
        observed_node_id: &str,
        dmap_ref: Option<DMonitorContractRefV1>,
        health: DMonitorHealthV1,
        observed_at_utc: &str,
        observation_id: &str,
    ) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: observation_id.to_string(),
            project_id: Some(PROJECT.to_string()),
            node_id: node_id.to_string(),
            monitor_instance_id: monitor_instance_id.to_string(),
            sequence,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "test_dserv_realization".to_string(),
            subject: DMonitorSubjectV1::DServRealization {
                project_id: PROJECT.to_string(),
                application_id: APP.to_string(),
                service_id: service_id.to_string(),
                node_id: observed_node_id.to_string(),
                dmap_ref,
            },
            health,
            facts: serde_json::json!({}),
        }
    }

    fn dnode_observation(node_id: &str, monitor_instance_id: &str, sequence: u64, observed_at_utc: &str, observation_id: &str) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: observation_id.to_string(),
            project_id: None,
            node_id: node_id.to_string(),
            monitor_instance_id: monitor_instance_id.to_string(),
            sequence,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "test_host".to_string(),
            subject: DMonitorSubjectV1::DNode { node_id: node_id.to_string() },
            health: DMonitorHealthV1::Unknown,
            facts: serde_json::json!({}),
        }
    }

    /// `NOW` (the evaluation time every test uses) and `FRESH` (an
    /// `observed_at_utc`/`received_at_utc` well within `policy()`'s 60s
    /// window) are fixed constants so every test's freshness math is
    /// trivial to reason about.
    const NOW: &str = "2026-09-25T00:10:00Z";
    const FRESH: &str = "2026-09-25T00:09:50Z";
    const STALE: &str = "2026-09-25T00:00:00Z";

    fn ingest(state: &mut DMonitorStateV1, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, PROJECT, observation, received_at_utc).unwrap();
    }

    fn ingest_scope(state: &mut DMonitorStateV1, project_scope: &str, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, project_scope, observation, received_at_utc).unwrap();
    }

    // --------------------------------------------------------------- CONVERGED

    #[test]
    fn dmap1_exact_ref_correct_node_fresh_is_converged() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_EXACT_MATCH));
    }

    #[test]
    fn dmap2_exact_ref_correct_node_fresh_is_converged() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority_v2(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged);
    }

    // ----------------------------------------------------- HEALTH INDEPENDENCE

    #[test]
    fn exact_placement_with_unhealthy_health_still_converges() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Unhealthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.services[0].observed_health_claims[0].health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn exact_placement_with_degraded_health_still_converges() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Degraded, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
    }

    // ------------------------------------------------------------- NO EVIDENCE

    #[test]
    fn zero_dserv_evidence_is_unknown_no_evidence() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_NO_EVIDENCE));
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
    }

    // ------------------------------------------------------------------- STALE

    #[test]
    fn stale_otherwise_correct_observation_is_unknown_stale_evidence() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, STALE, "obs-1"),
            STALE,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_STALE_EVIDENCE));
        assert_ne!(result.services[0].convergence, DMonitorConvergenceV1::Diverged, "stale evidence must never prove drift");
    }

    // ------------------------------------------------------------------- CLOCK

    #[test]
    fn clock_skew_otherwise_correct_observation_is_unknown_clock_skew() {
        let mut state = DMonitorStateV1::empty();
        // observed_at far ahead of received_at -> ClockSkew per dmonitor_freshness.
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, "2026-09-25T01:00:00Z", "obs-1"),
            "2026-09-25T00:09:50Z",
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_CLOCK_SKEW));
    }

    // --------------------------------------------------------------- AUTHORITY

    #[test]
    fn missing_dmap_ref_is_unknown_authority_ref_missing() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, None, DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_AUTHORITY_REF_MISSING));
    }

    #[test]
    fn old_revision_is_authority_mismatch_not_converged() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(old_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_ne!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_AUTHORITY_MISMATCH));
    }

    #[test]
    fn wrong_digest_same_id_and_revision_is_authority_mismatch() {
        let mut state = DMonitorStateV1::empty();
        let wrong_digest_ref = DMonitorContractRefV1 {
            id: DMAP_ID.to_string(),
            revision: DMAP_REVISION,
            digest: format!("sha256:{}", "9".repeat(64)),
        };
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(wrong_digest_ref), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_AUTHORITY_MISMATCH));
    }

    // -------------------------------------------------------------- PLACEMENT

    #[test]
    fn wrong_node_is_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_B, "mon-1", 1, SVC_A, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Diverged);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_WRONG_NODE));
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged);
    }

    #[test]
    fn expected_node_plus_extra_fresh_wrong_node_realization_is_unexpected_realization() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        ingest(
            &mut state,
            dserv_observation(NODE_B, "mon-2", 1, SVC_A, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Diverged);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_UNEXPECTED_REALIZATION));
    }

    // ---------------------------------------------- PHASE 114C.1 CLOSURE
    // (unexpected service_id, not merely wrong node)

    const SVC_X: &str = "unexpected-service-x";

    #[test]
    fn desired_correct_plus_unexpected_service_is_app_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_B, "mon-2", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged, "{result:#?}");
        assert!(result.findings.iter().any(|f| f.code == FINDING_UNEXPECTED_REALIZATION));
        // The DESIRED service itself remains correctly Converged on its own
        // terms -- it is the APPLICATION level that must not claim
        // Converged merely because every desired service has one correct
        // observation.
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.unexpected_realizations.len(), 1);
        assert_eq!(result.unexpected_realizations[0].service_id, SVC_X);
        assert_eq!(result.unexpected_realizations[0].admitted_evidence.len(), 1);
        assert_eq!(result.unexpected_realizations[0].admitted_evidence[0].observed_subject_node_id, NODE_B);
    }

    #[test]
    fn unexpected_service_only_with_fresh_current_evidence_is_app_diverged() {
        let mut state = DMonitorStateV1::empty();
        // No evidence at all for the (only) desired service SVC_A.
        ingest(&mut state, dserv_observation(NODE_B, "mon-1", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged);
        assert!(result.findings.iter().any(|f| f.code == FINDING_UNEXPECTED_REALIZATION));
    }

    #[test]
    fn stale_unexpected_service_does_not_prove_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_B, "mon-2", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, STALE, "obs-2"), STALE);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged, "stale unexpected evidence must not prove current drift: {result:#?}");
        assert_eq!(result.unexpected_realizations.len(), 1, "the stale unexpected evidence must still be preserved as excluded evidence, not silently dropped");
        assert!(result.unexpected_realizations[0].admitted_evidence.is_empty());
        assert_eq!(result.unexpected_realizations[0].excluded_evidence.len(), 1);
        assert_eq!(result.unexpected_realizations[0].excluded_evidence[0].exclusion_reason, Some(FINDING_STALE_EVIDENCE));
    }

    #[test]
    fn old_dmap_unexpected_service_does_not_prove_current_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_B, "mon-2", 1, SVC_X, NODE_B, Some(old_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged, "an old-DMap-correlated unexpected observation must not prove CURRENT drift: {result:#?}");
        assert_eq!(result.unexpected_realizations[0].excluded_evidence[0].exclusion_reason, Some(FINDING_AUTHORITY_MISMATCH));
    }

    #[test]
    fn zero_desired_and_zero_evidence_is_unknown_no_desired_placements() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_DESIRED_PLACEMENTS));
        assert!(result.unexpected_realizations.is_empty());
    }

    #[test]
    fn zero_desired_but_fresh_current_unexpected_service_is_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_B, "mon-1", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged, "{result:#?}");
        assert!(result.findings.iter().any(|f| f.code == FINDING_UNEXPECTED_REALIZATION));
        assert!(!result.findings.iter().any(|f| f.code == FINDING_NO_DESIRED_PLACEMENTS));
    }

    #[test]
    fn zero_desired_with_only_stale_evidence_does_not_claim_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_B, "mon-1", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, STALE, "obs-1"), STALE);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown, "{result:#?}");
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_DESIRED_PLACEMENTS));
        assert_eq!(result.unexpected_realizations.len(), 1, "stale evidence for the non-desired service is still preserved, just non-admitted");
        assert!(result.unexpected_realizations[0].admitted_evidence.is_empty());
    }

    #[test]
    fn wrong_node_desired_service_is_distinct_from_unexpected_service_id() {
        let mut state = DMonitorStateV1::empty();
        // SVC_A (desired on NODE_A) observed on the WRONG node (NODE_B) --
        // this is `wrong_node`, not an unexpected SERVICE.
        ingest(&mut state, dserv_observation(NODE_B, "mon-1", 1, SVC_A, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        // SVC_X has no desired placement at all -- this IS an unexpected
        // service.
        ingest(&mut state, dserv_observation(NODE_B, "mon-2", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Diverged);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_WRONG_NODE), "SVC_A's own result must be classified wrong_node, not conflated with the unexpected service");
        assert_eq!(result.unexpected_realizations.len(), 1);
        assert_eq!(result.unexpected_realizations[0].service_id, SVC_X);
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged);
    }

    #[test]
    fn runtime_process_and_unbound_subjects_never_become_unexpected_dserv_evidence() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);

        // A RuntimeProcess observation whose facts/description happen to
        // resemble a service_id must never be treated as DServ evidence.
        let mut runtime_process_obs = dnode_observation(NODE_B, "mon-2", 1, FRESH, "obs-2");
        runtime_process_obs.subject = DMonitorSubjectV1::RuntimeProcess {
            node_id: NODE_B.to_string(),
            runtime_ref: "entity:container:looks-like-a-service".to_string(),
        };
        ingest(&mut state, runtime_process_obs, FRESH);

        let mut unbound_obs = dnode_observation(NODE_B, "mon-3", 1, FRESH, "obs-3");
        unbound_obs.subject = DMonitorSubjectV1::Unbound {
            node_id: NODE_B.to_string(),
            description: Some(SVC_X.to_string()),
        };
        ingest(&mut state, unbound_obs, FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged, "{result:#?}");
        assert!(result.unexpected_realizations.is_empty(), "RuntimeProcess/Unbound must never be discovered as unexpected DServ evidence");
    }

    #[test]
    fn multiple_streams_preserve_no_leader_semantics_for_unexpected_services() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        // monitor A: fresh unexpected service-x.
        ingest(&mut state, dserv_observation(NODE_B, "mon-a", 1, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);
        // monitor B: stale, reports nothing about service-x (a DNode
        // observation instead) -- must not veto monitor A's fresh proof.
        ingest(&mut state, dnode_observation("cloud-01", "mon-b", 1, STALE, "obs-3"), STALE);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged);
        assert_eq!(result.unexpected_realizations[0].admitted_evidence.len(), 1);
    }

    #[test]
    fn same_service_two_different_nodes_within_one_stream_are_both_tracked() {
        // Phase 114C.1 closure regression: a SINGLE stream reporting the
        // SAME service on TWO DIFFERENT nodes, interleaved, must track BOTH
        // as independent subjects -- never silently collapse to only the
        // sequence-wise-last one regardless of node.
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, SVC_X, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        // SVC_A@NODE_A must still be found (not discarded merely because
        // the SAME stream later reported a different service on a
        // different node).
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged, "{result:#?}");
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged, "SVC_X@NODE_B must still be discovered as unexpected");
    }

    // ------------------------------------------------------- FINDING INVENTORY

    #[test]
    fn finding_code_inventory_has_exactly_nineteen_stable_codes() {
        // Phase 114C.1 fixed the count at 13 D-Serv/generic codes; Phase
        // 114D0A appends exactly 6 DIoT-only codes (never redefining any of
        // the original 13) -- see `ALL_FINDING_CODES`'s own doc comment.
        assert_eq!(ALL_FINDING_CODES.len(), 19, "{ALL_FINDING_CODES:?}");
        let unique: BTreeSet<&str> = ALL_FINDING_CODES.iter().copied().collect();
        assert_eq!(unique.len(), ALL_FINDING_CODES.len(), "every finding code must be distinct");
    }

    #[test]
    fn every_finding_actually_emitted_in_this_test_suite_is_in_the_inventory() {
        // Cross-check: every FINDING_* constant this test module actually
        // asserts on must be a member of ALL_FINDING_CODES -- this is what
        // keeps "the report said 12, the code has 13" from silently
        // recurring.
        let emitted = [
            FINDING_EXACT_MATCH,
            FINDING_NO_EVIDENCE,
            FINDING_STALE_EVIDENCE,
            FINDING_CLOCK_SKEW,
            FINDING_AUTHORITY_REF_MISSING,
            FINDING_AUTHORITY_MISMATCH,
            FINDING_WRONG_NODE,
            FINDING_UNEXPECTED_REALIZATION,
            FINDING_CONFLICTING_FRESH_EVIDENCE,
            FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY,
            FINDING_NO_DESIRED_PLACEMENTS,
            FINDING_INSUFFICIENT_CANONICAL_CORRELATION,
        ];
        for code in emitted {
            assert!(ALL_FINDING_CODES.contains(&code), "finding code '{code}' must be listed in ALL_FINDING_CODES");
        }
    }

    // --------------------------------------------------------- MULTIPLE STREAMS

    #[test]
    fn two_fresh_agreeing_streams_are_converged() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-2", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.services[0].admitted_evidence.len(), 2);
    }

    #[test]
    fn fresh_good_evidence_is_not_vetoed_by_stale_conflicting_evidence() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        // A stale stream claiming the WRONG node must not veto the fresh,
        // correct evidence.
        ingest(
            &mut state,
            dserv_observation(NODE_B, "mon-2", 1, SVC_A, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, STALE, "obs-2"),
            STALE,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.services[0].excluded_evidence.len(), 1);
        assert!(result.services[0].excluded_evidence[0].exclusion_reason == Some(FINDING_STALE_EVIDENCE));
    }

    #[test]
    fn two_fresh_materially_conflicting_streams_are_diverged_conflicting_fresh_evidence() {
        let mut state = DMonitorStateV1::empty();
        // Neither stream reports the desired node; two DIFFERENT wrong
        // nodes disagree with each other.
        ingest(
            &mut state,
            dserv_observation("fog-03", "mon-1", 1, SVC_A, "fog-03", Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        ingest(
            &mut state,
            dserv_observation("fog-04", "mon-2", 1, SVC_A, "fog-04", Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Diverged);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_CONFLICTING_FRESH_EVIDENCE));
    }

    #[test]
    fn health_disagreement_does_not_alter_placement_convergence() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-2", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Unhealthy, FRESH, "obs-2"),
            FRESH,
        );
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        let healths: BTreeSet<_> = result.services[0]
            .observed_health_claims
            .iter()
            .map(|c| c.health)
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| format!("{h:?}"))
            .collect();
        assert!(healths.contains("Healthy") && healths.contains("Unhealthy"));
    }

    // ------------------------------------------------------------ AGGREGATION

    #[test]
    fn all_desired_services_converged_is_app_converged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, SVC_B, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A), (SVC_B, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged);
    }

    #[test]
    fn one_divergent_service_is_app_diverged() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_B, "mon-1", 2, SVC_B, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A), (SVC_B, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged);
    }

    #[test]
    fn one_unknown_rest_converged_is_app_unknown() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        // SVC_B has zero evidence.
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A), (SVC_B, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
    }

    // ---------------------------------------------------------- EMPTY / NONE

    #[test]
    fn empty_placement_set_is_unknown_not_converged() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![])), &state, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_DESIRED_PLACEMENTS));
        assert!(result.services.is_empty());
    }

    #[test]
    fn no_active_authority_is_unknown_no_active_placement_authority() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_application_dserv_convergence(PROJECT, APP, PROJECT, None, &state, &policy(), NOW);
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY));
        assert!(result.dmap_ref.is_none());
        assert_ne!(result.convergence, DMonitorConvergenceV1::Diverged);
    }

    // ------------------------------------------------------------ PROJECT ISOLATION

    #[test]
    fn evidence_from_another_server_project_scope_is_ignored() {
        // Phase 114B1/B1.1's own ingestion invariant already guarantees a
        // project-bearing subject's `project_id` must equal the server
        // scope it is ingested under -- so this evidence is itself a
        // genuinely valid, DIFFERENT project's own DServRealization
        // (same application_id/service_id/node_id as PROJECT's authority
        // purely by coincidence), ingested honestly under OTHER_PROJECT's
        // own scope. This proves BOTH of this module's independent guards
        // (the `project_id` filter AND the `server_project_scope` filter,
        // section 29) correctly keep it from ever being treated as
        // PROJECT's evidence.
        let mut state = DMonitorStateV1::empty();
        let mut other_project_observation = dserv_observation(
            NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1",
        );
        other_project_observation.project_id = Some(OTHER_PROJECT.to_string());
        other_project_observation.subject = DMonitorSubjectV1::DServRealization {
            project_id: OTHER_PROJECT.to_string(),
            application_id: APP.to_string(),
            service_id: SVC_A.to_string(),
            node_id: NODE_A.to_string(),
            dmap_ref: Some(exact_ref()),
        };
        ingest_scope(&mut state, OTHER_PROJECT, other_project_observation, FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.services[0].findings.iter().any(|f| f.code == FINDING_NO_EVIDENCE));
        assert!(result.services[0].admitted_evidence.is_empty());
        assert!(result.services[0].excluded_evidence.is_empty(), "the other project's evidence must not even appear as excluded evidence -- it must be invisible to this evaluation");
    }

    // -------------------------------------------------------------- HISTORICAL

    #[test]
    fn old_dmap_observation_preserved_but_does_not_prove_current_convergence() {
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(old_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"),
            FRESH,
        );
        // The observation is genuinely persisted (historical evidence, never deleted).
        assert!(state.by_observation_id("obs-1").is_some());
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_ne!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
    }

    // ----------------------------------------------------- EVIDENCE SELECTION

    #[test]
    fn interleaved_stream_uses_latest_sequence_for_the_subject_not_stream_head() {
        let mut state = DMonitorStateV1::empty();
        // stream A: seq1 service X, seq2 node health, seq3 service Y, seq4 service X again.
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_B, Some(old_ref()), DMonitorHealthV1::Healthy, STALE, "obs-1"), STALE);
        ingest(&mut state, dnode_observation(NODE_A, "mon-1", 2, FRESH, "obs-2"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 3, SVC_B, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-3"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 4, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-4"), FRESH);

        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        // Must use seq4 (fresh, correct node) for service X, NOT seq1 (stale, wrong node).
        assert_eq!(result.services[0].convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.services[0].admitted_evidence.len(), 1);
        assert_eq!(result.services[0].admitted_evidence[0].observation_id, "obs-4");
    }

    #[test]
    fn two_monitor_instance_ids_are_evaluated_independently() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-a", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-a"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-b", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Degraded, FRESH, "obs-b"), FRESH);
        let result = evaluate_application_dserv_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &policy(), NOW,
        );
        assert_eq!(result.services[0].admitted_evidence.len(), 2);
        let instances: BTreeSet<&str> = result.services[0]
            .admitted_evidence
            .iter()
            .map(|e| e.monitor_instance_id.as_str())
            .collect();
        assert_eq!(instances, BTreeSet::from(["mon-a", "mon-b"]));
    }

    // --------------------------------------------------------- DIOT CORE (114D0A)

    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1,
        CanonicalNodeCapacityV1, CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dforward::{
        CanonicalArtifactRequirementV1, CanonicalDForwardInterfaceV1, CanonicalDForwardV1,
        CanonicalDIoTV1, CANONICAL_DFORWARD_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        recompute_dgraph_digest, CanonicalDCallV1, CanonicalDGraphV1,
        CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1,
        CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
    };
    use crate::core::canonical_dmap_v2::{ContractRefV2, DIoTArtifactBindingV2};
    use crate::core::ddeploy::{active_placement_authority, ProposalSource};
    use crate::core::ddeploy_v2::DDeployProposalV2;
    use crate::core::diot_runtime_binding::{register_runtime_binding, DIoTRuntimeBindingRegistryStateV1};
    use crate::core::dnode_registry::declare_dnode;
    use crate::core::canonical_dmonitor::DMonitorRuntimeBindingRefV1;
    use crate::core::service_artifact_registry::{
        NativeProcessDeploymentSpecV1, ServiceArtifact, ServiceArtifactControlPlane,
        ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec, SERVICE_ARTIFACT_SCHEMA_VERSION,
    };

    const DIOT_ID: &str = "diot:broker";
    const DIOT_PLACEMENT_ID: &str = "diotp:broker";
    const OTHER_DIOT_PLACEMENT_ID: &str = "diotp:unexpected";
    const REALIZATION_INTERFACE: &str = "sensor-bus";
    const ARTIFACT_ID: &str = "dserv:pipeline:broker";

    fn diot_node(node_id: &str) -> CanonicalDNodeV1 {
        CanonicalDNodeV1 {
            node_id: node_id.to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "x86_64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_000_000_000 },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        }
    }

    fn minimal_broker_artifact(project_id: &str, execution_generation: u64, execution_projection_digest: &str) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: ARTIFACT_ID.to_string(),
            service_id: ARTIFACT_ID.to_string(),
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
                execution: ServiceProbeSpec { kind: "process_running".to_string(), params: BTreeMap::new(), vantage_point: Some("local_host".to_string()) },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                project_id: project_id.to_string(),
                execution_generation,
                execution_projection_digest: execution_projection_digest.to_string(),
                ..Default::default()
            },
        }
    }

    struct DiotFixture {
        ddeploy: crate::core::ddeploy::DDeployStateV1,
        dnode: crate::core::dnode_registry::DNodeRegistryStateV1,
    }

    /// Builds one REAL accepted DMap/2 authority (via the actual
    /// `create_v2_proposal`/`accept_v2_proposal` production path, never
    /// hand-assembled) with exactly one DIoT broker placement on
    /// `broker_node`, mirroring `diot_runtime_binding::tests::build_fixture`
    /// (duplicated rather than shared across files, matching this
    /// codebase's existing precedent -- see that function's own doc
    /// comment). Building through the real acceptance path (rather than
    /// constructing a `PlacementAuthorityView` by hand, as the D-Serv tests
    /// above do) is required here because `verify_runtime_binding_still_current`
    /// re-derives active authority from `DDeployStateV1` itself.
    fn build_diot_fixture(broker_node: &str) -> DiotFixture {
        let dg = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: APP.to_string(),
            dgraph_id: "dgraph:prop-diot-1".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 {
                    service_id: SVC_A.to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
                },
                CanonicalDServV1 {
                    service_id: SVC_B.to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
                },
            ],
            calls: vec![CanonicalDCallV1 { call_id: "sensor-to-decoder".to_string(), source_service_id: SVC_A.to_string(), target_service_id: SVC_B.to_string() }],
        };
        let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();

        let dcontinuum = CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dcontinuum_id: "dcontinuum:prop-diot-1".to_string(),
            revision: 1,
            nodes: vec![diot_node(broker_node)],
            links: Vec::new(),
        };
        let dcontinuum_digest = crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum).unwrap();

        let dforward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dforward_id: "dforward:prop-diot-1".to_string(),
            revision: 1,
            diots: vec![CanonicalDIoTV1 {
                diot_id: DIOT_ID.to_string(),
                diot_kind: "broker".to_string(),
                artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: ARTIFACT_ID.to_string() },
                provides: vec![CanonicalDForwardInterfaceV1 { interface: REALIZATION_INTERFACE.to_string() }],
                requires: Vec::new(),
            }],
            external_endpoints: Vec::new(),
            chains: Vec::new(),
            support_jobs: Vec::new(),
        };
        let dforward_digest = dforward.digest().unwrap();

        let proposal = DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: "prop-diot-1".to_string(),
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            dgraph_ref: ContractRefV2 { id: dg.dgraph_id.clone(), revision: dg.revision, digest: dgraph_digest },
            dcontinuum_ref: ContractRefV2 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: dcontinuum_digest },
            dforward_ref: ContractRefV2 { id: dforward.dforward_id.clone(), revision: dforward.revision, digest: dforward_digest },
            placements: vec![
                DServPlacementV2 { service_id: SVC_A.to_string(), node_id: broker_node.to_string() },
                DServPlacementV2 { service_id: SVC_B.to_string(), node_id: broker_node.to_string() },
            ],
            diot_placements: vec![DIoTPlacementV2 {
                placement_id: DIOT_PLACEMENT_ID.to_string(),
                diot_id: DIOT_ID.to_string(),
                node_id: broker_node.to_string(),
                artifact_binding: DIoTArtifactBindingV2 { artifact_id: ARTIFACT_ID.to_string(), descriptor_digest: format!("sha256:{}", "b".repeat(64)), content_digest: None },
            }],
            dcall_realizations: vec![DCallRealizationV2 {
                call_id: "sensor-to-decoder".to_string(),
                realization_kind: "via_diot_pubsub".to_string(),
                diot_placement_id: DIOT_PLACEMENT_ID.to_string(),
                interface: REALIZATION_INTERFACE.to_string(),
                channel: "pipeline/sensor-to-decoder".to_string(),
            }],
            external_port_bindings: Vec::new(),
            expected_active_dmap_revision: None,
            diot_artifact_authority: {
                let mut m = BTreeMap::new();
                m.insert(
                    ARTIFACT_ID.to_string(),
                    crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2 {
                        artifact_id: ARTIFACT_ID.to_string(),
                        execution_generation: 1,
                        execution_projection_digest: format!("sha256:{}", "c".repeat(64)),
                    },
                );
                m
            },
        };

        let broker_artifact = minimal_broker_artifact(PROJECT, 1, &format!("sha256:{}", "c".repeat(64)));

        let mut ddeploy = crate::core::ddeploy::DDeployStateV1::empty();
        ddeploy.create_v2_proposal(proposal, dg, dcontinuum.clone(), dforward).unwrap();
        ddeploy
            .accept_v2_proposal("prop-diot-1", "acc-prop-diot-1".to_string(), "test".to_string(), "2026-01-01T00:00:01Z".to_string(), &dcontinuum, std::slice::from_ref(&broker_artifact), None)
            .unwrap();

        let mut dnode = crate::core::dnode_registry::DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode, diot_node(broker_node)).unwrap();
        DiotFixture { ddeploy, dnode }
    }

    #[allow(clippy::too_many_arguments)]
    fn diot_realization_observation(
        node_id: &str,
        monitor_instance_id: &str,
        sequence: u64,
        diot_id: &str,
        diot_placement_id: &str,
        observed_node_id: &str,
        dmap_ref: Option<DMonitorContractRefV1>,
        dforward_ref: Option<DMonitorContractRefV1>,
        runtime_binding_ref: Option<DMonitorRuntimeBindingRefV1>,
        observed_at_utc: &str,
        observation_id: &str,
    ) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: observation_id.to_string(),
            project_id: Some(PROJECT.to_string()),
            node_id: node_id.to_string(),
            monitor_instance_id: monitor_instance_id.to_string(),
            sequence,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "test_diot_realization".to_string(),
            subject: DMonitorSubjectV1::DIoTRealization {
                project_id: PROJECT.to_string(),
                application_id: APP.to_string(),
                diot_id: diot_id.to_string(),
                diot_placement_id: diot_placement_id.to_string(),
                node_id: observed_node_id.to_string(),
                dmap_ref,
                dforward_ref,
                runtime_binding_ref,
            },
            health: DMonitorHealthV1::Healthy,
            facts: serde_json::json!({}),
        }
    }

    /// Fixture + real registered-and-current binding, plus the exact
    /// `{dmap_ref, dforward_ref, runtime_binding_ref}` an observer would
    /// need to report for the admission rule's "all four exact matches" to
    /// hold.
    struct DiotEvidenceInputs {
        fixture: DiotFixture,
        runtime_registry: DIoTRuntimeBindingRegistryStateV1,
        dmap_ref: DMonitorContractRefV1,
        dforward_ref: DMonitorContractRefV1,
        runtime_binding_ref: DMonitorRuntimeBindingRefV1,
    }

    fn build_diot_evidence_inputs(broker_node: &str) -> DiotEvidenceInputs {
        let fixture = build_diot_fixture(broker_node);
        let authority = active_placement_authority(&fixture.ddeploy, PROJECT, APP).unwrap().unwrap();
        let v2 = authority.v2.clone().unwrap();
        let mut runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let binding = register_runtime_binding(
            &mut runtime_registry,
            &fixture.ddeploy,
            &fixture.dnode,
            PROJECT,
            APP,
            DIOT_PLACEMENT_ID,
            "mqtt://broker.lab:1883",
            "2026-01-01T00:00:02Z",
        )
        .unwrap();
        DiotEvidenceInputs {
            dmap_ref: DMonitorContractRefV1 { id: authority.dmap_id.clone(), revision: authority.revision, digest: authority.digest.clone() },
            dforward_ref: DMonitorContractRefV1 { id: v2.dforward_id.clone(), revision: v2.dforward_revision, digest: v2.dforward_digest.clone() },
            runtime_binding_ref: DMonitorRuntimeBindingRefV1 { binding_id: binding.binding_id.clone(), binding_revision: binding.binding_revision, descriptor_digest: binding.descriptor_digest.clone() },
            fixture,
            runtime_registry,
        }
    }

    #[test]
    fn dmap_v1_authority_has_no_diot_placement_concept_and_is_unknown() {
        let state = DMonitorStateV1::empty();
        let ddeploy = crate::core::ddeploy::DDeployStateV1::empty();
        let dnode = crate::core::dnode_registry::DNodeRegistryStateV1::empty();
        let runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, Some(&authority(vec![(SVC_A, NODE_A)])), &state, &runtime_registry, &ddeploy, &dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.findings.iter().any(|f| f.code == FINDING_DMAP_V1_NO_DIOT_SUPPORT));
        assert!(result.placements.is_empty());
    }

    #[test]
    fn no_active_authority_is_unknown() {
        let state = DMonitorStateV1::empty();
        let ddeploy = crate::core::ddeploy::DDeployStateV1::empty();
        let dnode = crate::core::dnode_registry::DNodeRegistryStateV1::empty();
        let runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let result = evaluate_application_diot_convergence(PROJECT, APP, PROJECT, None, &state, &runtime_registry, &ddeploy, &dnode, &policy(), NOW);
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY));
    }

    #[test]
    fn zero_desired_diot_placements_is_unknown() {
        let state = DMonitorStateV1::empty();
        let ddeploy = crate::core::ddeploy::DDeployStateV1::empty();
        let dnode = crate::core::dnode_registry::DNodeRegistryStateV1::empty();
        let runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, Some(&authority_v2(vec![(SVC_A, NODE_A)])), &state, &runtime_registry, &ddeploy, &dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_DESIRED_DIOT_PLACEMENTS));
    }

    #[test]
    fn exact_match_all_authorities_fresh_correct_node_converges() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(result.placements[0].convergence, DMonitorConvergenceV1::Converged);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_EXACT_MATCH));
        assert!(result.placements[0].admitted_evidence[0].runtime_binding_current);
    }

    #[test]
    fn missing_dmap_ref_excludes_evidence_and_is_unknown() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, None, Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_AUTHORITY_REF_MISSING));
    }

    #[test]
    fn mismatched_dforward_ref_excludes_evidence_and_is_unknown() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        let bad_dforward = DMonitorContractRefV1 { id: inputs.dforward_ref.id.clone(), revision: inputs.dforward_ref.revision + 1, digest: inputs.dforward_ref.digest.clone() };
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(bad_dforward), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_AUTHORITY_MISMATCH));
    }

    #[test]
    fn missing_runtime_binding_ref_excludes_evidence() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), None, FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_RUNTIME_BINDING_REF_MISSING));
    }

    #[test]
    fn runtime_binding_not_registered_excludes_evidence() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let empty_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &empty_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_RUNTIME_BINDING_NOT_REGISTERED));
    }

    #[test]
    fn mismatched_runtime_binding_ref_excludes_evidence() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let bad_binding_ref = DMonitorRuntimeBindingRefV1 { binding_id: inputs.runtime_binding_ref.binding_id.clone(), binding_revision: inputs.runtime_binding_ref.binding_revision + 1, descriptor_digest: inputs.runtime_binding_ref.descriptor_digest.clone() };
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(bad_binding_ref), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_RUNTIME_BINDING_MISMATCH));
    }

    #[test]
    fn wrong_node_diverges() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        // Observed on NODE_B, but the fixture's registered/current binding
        // (and desired placement) is NODE_A -- runtime_binding_ref exact
        // match is against the REGISTERED binding, independent of which
        // node the observation itself claims, so this evidence is still
        // ADMITTED (it exactly matches every authority dimension) but
        // compares as wrong-node against the desired placement.
        ingest(
            &mut state,
            diot_realization_observation(NODE_B, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_B, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Diverged);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_WRONG_NODE));
    }

    #[test]
    fn stale_evidence_is_unknown() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), STALE, "obs-1"),
            STALE,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_STALE_EVIDENCE));
    }

    #[test]
    fn unexpected_diot_evidence_can_never_admit_because_currentness_requires_placement_existence() {
        // Structural property, distinct from D-Serv: `assess_diot_evidence`
        // requires `runtime_binding_current`, and
        // `verify_runtime_binding_still_current` itself fails closed the
        // instant a binding's `placement_id` is absent from the CURRENT
        // active DMap/2's `diot_placements` (see that function's own "DIoT
        // placement ... no longer exists" check). A binding can therefore
        // never be registered-and-current for a placement that is not
        // desired, which means a "proven unexpected DIoT realization" (the
        // D-Serv analogue of `unexpected_realizations` with non-empty
        // `admitted_evidence`) is unreachable by construction, not merely
        // untested.
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        // Fresh evidence for a DIFFERENT (diot_id, placement_id) than the
        // one desired, reusing the SAME dmap_ref/dforward_ref (both exactly
        // current) but a runtime_binding_ref that cannot possibly be
        // registered-and-current for this other placement.
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, "diot:other", OTHER_DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        // The one DESIRED placement has no evidence of its own at all here
        // (only the unexpected placement was ingested), so it is Unknown
        // with FINDING_NO_EVIDENCE; the unexpected evidence never escalates
        // this to Diverged because it was never admitted.
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_NO_EVIDENCE));
        assert_eq!(result.unexpected_realizations.len(), 1);
        assert!(result.unexpected_realizations[0].admitted_evidence.is_empty());
        assert!(result.unexpected_realizations[0]
            .excluded_evidence
            .iter()
            .any(|e| e.exclusion_reason == Some(FINDING_RUNTIME_BINDING_NOT_REGISTERED)));
    }

    #[test]
    fn project_isolation_ignores_other_project_scope_evidence() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        let mut other_project_observation = diot_realization_observation(
            NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A,
            Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()),
            FRESH, "obs-1",
        );
        other_project_observation.project_id = Some(OTHER_PROJECT.to_string());
        if let DMonitorSubjectV1::DIoTRealization { project_id, .. } = &mut other_project_observation.subject {
            *project_id = OTHER_PROJECT.to_string();
        }
        ingest_scope(&mut state, OTHER_PROJECT, other_project_observation, FRESH);
        let result = evaluate_application_diot_convergence(
            PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW,
        );
        assert_eq!(result.convergence, DMonitorConvergenceV1::Unknown);
        assert!(result.placements[0].findings.iter().any(|f| f.code == FINDING_NO_EVIDENCE));
    }

    #[test]
    fn identical_inputs_produce_identical_diot_results() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let authority = active_placement_authority(&inputs.fixture.ddeploy, PROJECT, APP).unwrap();
        let mut state = DMonitorStateV1::empty();
        ingest(
            &mut state,
            diot_realization_observation(NODE_A, "mon-1", 1, DIOT_ID, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), FRESH, "obs-1"),
            FRESH,
        );
        let a = evaluate_application_diot_convergence(PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        let b = evaluate_application_diot_convergence(PROJECT, APP, PROJECT, authority.as_ref(), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(a, b);
    }

    #[test]
    fn all_finding_codes_inventory_has_no_duplicates_and_includes_diot_codes() {
        let unique: BTreeSet<&str> = ALL_FINDING_CODES.iter().copied().collect();
        assert_eq!(unique.len(), ALL_FINDING_CODES.len());
        for code in [
            FINDING_RUNTIME_BINDING_NOT_REGISTERED,
            FINDING_RUNTIME_BINDING_REF_MISSING,
            FINDING_RUNTIME_BINDING_MISMATCH,
            FINDING_RUNTIME_BINDING_STALE,
            FINDING_DMAP_V1_NO_DIOT_SUPPORT,
            FINDING_NO_DESIRED_DIOT_PLACEMENTS,
        ] {
            assert!(ALL_FINDING_CODES.contains(&code));
        }
    }

    // -------------------------------------------------------- DETERMINISM

    #[test]
    fn identical_inputs_produce_identical_results() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-2", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);
        let auth = authority(vec![(SVC_A, NODE_A)]);
        let first = evaluate_application_dserv_convergence(PROJECT, APP, PROJECT, Some(&auth), &state, &policy(), NOW);
        let second = evaluate_application_dserv_convergence(PROJECT, APP, PROJECT, Some(&auth), &state, &policy(), NOW);
        assert_eq!(first, second);
    }

    // -------------------------------------------------------- RESTART PROOF

    #[test]
    fn restart_proof_persisted_dmonitor_state_reproduces_identical_convergence_result() {
        use crate::core::dmonitor_state::{load_state, persist_state};

        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-interpretation-restart-{}.db",
            uuid::Uuid::new_v4()
        ));

        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        persist_state(&path, &state).unwrap();

        // "Prepare accepted active placement authority": this is exactly the
        // `PlacementAuthorityView` shape `ddeploy::active_placement_authority`
        // derives from a real persisted+reloaded `DDeployStateV1` -- DDeploy's
        // OWN persistence/authority-derivation determinism is already
        // exhaustively proven by its own established test suite (Phase 111-
        // 113), so this proof focuses on what Phase 114C actually introduces:
        // that THIS module's comparison, combined with a REAL reload of
        // DMonitor evidence, is deterministic.
        let auth = authority(vec![(SVC_A, NODE_A)]);

        let before_reload = evaluate_application_dserv_convergence(PROJECT, APP, PROJECT, Some(&auth), &state, &policy(), NOW);

        drop(state);
        let reloaded = load_state(&path).unwrap();
        let after_reload = evaluate_application_dserv_convergence(PROJECT, APP, PROJECT, Some(&auth), &reloaded, &policy(), NOW);

        assert_eq!(before_reload, after_reload);
        assert_eq!(after_reload.convergence, DMonitorConvergenceV1::Converged);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_extension("db-wal")).ok();
        std::fs::remove_file(path.with_extension("db-shm")).ok();
    }
}
