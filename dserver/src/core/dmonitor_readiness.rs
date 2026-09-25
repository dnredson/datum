//! Server-owned DMonitor-backed readiness governance (Phase 114D1, ADR-0024).
//!
//! Readiness is DERIVED GOVERNANCE, never a field submitted by DMonitor, a
//! health alias, a convergence alias, or a caller-selected diagnostic
//! interpretation. This module answers exactly one question: does
//! server-owned governance currently permit this application's realizations
//! to be considered ready for downstream use? It reuses (never
//! reimplements) [`evaluate_application_dserv_convergence`] and
//! [`evaluate_application_diot_convergence`] from `dmonitor_interpretation`
//! for every admission/freshness/authority-correlation decision -- this
//! module ONLY adds the health-gating and requirement-aggregation layer on
//! top of their already-derived results.
//!
//! # Fail-closed, two-state governance (section 3)
//!
//! Unlike `DMonitorConvergenceV1` (which has a genuine third `Unknown`
//! state for diagnostic honesty), readiness has exactly two states:
//! [`DMonitorReadinessV1::Ready`] and [`DMonitorReadinessV1::NotReady`].
//! Readiness is an authorization-style decision -- if it cannot be
//! POSITIVELY proven, the answer is `NotReady`, never a third "we don't
//! know" governance state. Diagnostic uncertainty remains fully visible
//! through `findings` and the embedded convergence results, which keep
//! their own `Unknown` states unchanged.
//!
//! # Health is not readiness, convergence is not readiness
//!
//! All of the following are representable simultaneously:
//! `convergence = Converged` + `health = Unhealthy` + `readiness =
//! NotReady`; `health = Healthy` + `authority = stale/mismatched` +
//! `readiness = NotReady`; `evidence = Stale` + `last observed health =
//! Healthy` + `readiness = NotReady`. A `Healthy` health claim alone never
//! produces `Ready`.
//!
//! # No server-synthesized readiness
//!
//! This module never calls HTTP, never reads the wall clock, and never
//! persists its result (section 32) -- it is a pure function of one
//! explicit snapshot (`PlacementAuthorityView`, `DMonitorStateV1`,
//! `DIoTRuntimeBindingRegistryStateV1`, `DDeployStateV1`,
//! `DNodeRegistryStateV1`), one server-owned policy, and one explicit
//! evaluation timestamp -- recomputed fresh on every call, exactly like the
//! convergence layer it builds on.

use crate::core::canonical_dmonitor::DMonitorHealthV1;
use crate::core::ddeploy::DDeployStateV1;
use crate::core::ddeploy_v2::PlacementAuthorityView;
use crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1;
use crate::core::dmonitor_freshness::DMonitorFreshnessPolicyV1;
use crate::core::dmonitor_interpretation::{
    evaluate_application_dserv_convergence, evaluate_application_diot_convergence,
    DMonitorApplicationConvergenceV1, DMonitorApplicationDiotConvergenceV1, DMonitorConvergenceV1,
    DMonitorDiotConvergenceV1, DMonitorServicePlacementConvergenceV1,
};
use crate::core::dmonitor_state::DMonitorStateV1;
use crate::core::dnode_registry::DNodeRegistryStateV1;

pub const DMONITOR_READINESS_SCHEMA: &str = "datum.dmonitor-readiness/1";

/// The ONE server-owned readiness policy version this release implements.
/// See [`default_readiness_policy`] for the exact values and their
/// rationale -- no server configuration mechanism for DMonitor exists yet
/// (`dserver::config::AppConfig` has no `[dmonitor]` section), so this is a
/// single explicit, versioned, fully-documented default, per this phase's
/// own fallback instruction, rather than a hidden magic constant.
pub const DEFAULT_READINESS_POLICY_ID: &str = "dmonitor-readiness-v1";

// ------------------------------------------------------------- FINDING CODES
// A SEPARATE, readiness-specific finding inventory (section 24): these are
// never the same Rust constants as `dmonitor_interpretation`'s 19
// convergence finding codes, even where a string value is intentionally
// reused for human readability (e.g. "no_active_placement_authority") --
// readiness findings answer "why is/isn't this application ready", not
// "why did this one piece of evidence admit or exclude".
pub const FINDING_READY: &str = "ready";
pub const FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY: &str = "no_active_placement_authority";
pub const FINDING_NO_REQUIRED_RUNTIME_REALIZATIONS: &str = "no_required_runtime_realizations";
pub const FINDING_DSERV_NOT_CONVERGED: &str = "dserv_not_converged";
pub const FINDING_DIOT_NOT_CONVERGED: &str = "diot_not_converged";
pub const FINDING_HEALTH_UNKNOWN: &str = "health_unknown";
pub const FINDING_HEALTH_DEGRADED: &str = "health_degraded";
pub const FINDING_HEALTH_UNHEALTHY: &str = "health_unhealthy";
pub const FINDING_CONFLICTING_CURRENT_HEALTH: &str = "conflicting_current_health";

/// The complete, stable readiness finding-code inventory, in declaration
/// order -- mirrors `dmonitor_interpretation::ALL_FINDING_CODES`'s own
/// precedent so a test can assert the inventory's exact size/membership
/// never silently drifts from the constants themselves.
pub const ALL_READINESS_FINDING_CODES: &[&str] = &[
    FINDING_READY,
    FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY,
    FINDING_NO_REQUIRED_RUNTIME_REALIZATIONS,
    FINDING_DSERV_NOT_CONVERGED,
    FINDING_DIOT_NOT_CONVERGED,
    FINDING_HEALTH_UNKNOWN,
    FINDING_HEALTH_DEGRADED,
    FINDING_HEALTH_UNHEALTHY,
    FINDING_CONFLICTING_CURRENT_HEALTH,
];

fn finding(code: &'static str, message: impl Into<String>) -> DMonitorReadinessFindingV1 {
    DMonitorReadinessFindingV1 { code, message: message.into() }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DMonitorReadinessFindingV1 {
    pub code: &'static str,
    pub message: String,
}

/// Fail-closed, two-state governance main state (section 3). Deliberately
/// NOT a three-state enum -- see this module's own doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorReadinessV1 {
    Ready,
    NotReady,
}

/// v1 accepts exactly one health value as consistent with readiness --
/// `Healthy`. Declared as a real type (not a bare doc comment) so the
/// policy's `accepted_health` field is self-describing on the wire and a
/// future v2 policy could widen it without breaking the schema shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorAcceptedHealthV1 {
    Healthy,
}

/// Server-owned readiness policy (section 5). NEVER accepted from a caller
/// -- the production API has no request body and no query parameter that
/// can influence any field here (see `api::dmonitor_readiness`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorReadinessPolicyV1 {
    pub policy_id: String,
    pub freshness_policy: DMonitorFreshnessPolicyV1,
    pub accepted_health: DMonitorAcceptedHealthV1,
}

/// The one explicit, versioned, fully-documented v1 default policy.
///
/// - `max_observed_age_seconds = 90`, `max_received_age_seconds = 90`:
///   the current governed DIoT DMonitor heartbeat default
///   (`--dmonitor-heartbeat-interval-seconds`, Phase 114D0B) is 30 seconds;
///   90 seconds allows three nominal heartbeat periods before evidence is
///   treated as stale. Heartbeat cadence and this readiness freshness
///   window are INDEPENDENT server/operator-configured values: an operator
///   who configures a heartbeat interval slower than this window can
///   legitimately and correctly cause readiness to fail closed to
///   `NotReady` on otherwise-healthy evidence -- that is working as
///   designed, not a bug in either configuration.
/// - `max_future_clock_skew_seconds = 10`: unchanged from every other
///   DMonitor freshness policy already used in this codebase (Phase 114C's
///   own convergence diagnostic default reasoning applies identically
///   here).
///
/// Policy configurability beyond this one server-owned mechanism (e.g. a
/// future `[dmonitor]` section in `AppConfig`) is a product/configuration
/// concern for a later phase, never caller control.
pub fn default_readiness_policy() -> DMonitorReadinessPolicyV1 {
    DMonitorReadinessPolicyV1 {
        policy_id: DEFAULT_READINESS_POLICY_ID.to_string(),
        freshness_policy: DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 90,
            max_received_age_seconds: 90,
            max_future_clock_skew_seconds: 10,
        },
        accepted_health: DMonitorAcceptedHealthV1::Healthy,
    }
}

/// Typed current-health assessment (section 25), derived ONLY from
/// ADMITTED CURRENT evidence (never stale/excluded claims -- those are
/// already excluded from `admitted_evidence` by the convergence layer this
/// module consumes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorCurrentHealthV1 {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
    /// More than one DISTINCT health value among admitted current evidence
    /// (section 12/25) -- no monitor leader, no majority vote: any
    /// disagreement at all fails closed.
    Conflicting,
}

fn assess_current_health(healths: &[DMonitorHealthV1]) -> DMonitorCurrentHealthV1 {
    let Some(first) = healths.first() else {
        return DMonitorCurrentHealthV1::Unknown;
    };
    if healths.iter().all(|health| health == first) {
        match first {
            DMonitorHealthV1::Healthy => DMonitorCurrentHealthV1::Healthy,
            DMonitorHealthV1::Degraded => DMonitorCurrentHealthV1::Degraded,
            DMonitorHealthV1::Unhealthy => DMonitorCurrentHealthV1::Unhealthy,
            DMonitorHealthV1::Unknown => DMonitorCurrentHealthV1::Unknown,
        }
    } else {
        DMonitorCurrentHealthV1::Conflicting
    }
}

fn health_finding(health: DMonitorCurrentHealthV1, label: &str) -> DMonitorReadinessFindingV1 {
    match health {
        DMonitorCurrentHealthV1::Healthy => finding(FINDING_READY, format!("{label} is converged and admitted current evidence is Healthy")),
        DMonitorCurrentHealthV1::Degraded => finding(FINDING_HEALTH_DEGRADED, format!("{label} is converged but admitted current evidence reports Degraded")),
        DMonitorCurrentHealthV1::Unhealthy => finding(FINDING_HEALTH_UNHEALTHY, format!("{label} is converged but admitted current evidence reports Unhealthy")),
        DMonitorCurrentHealthV1::Unknown => finding(FINDING_HEALTH_UNKNOWN, format!("{label} is converged but admitted current evidence reports Unknown health")),
        DMonitorCurrentHealthV1::Conflicting => finding(FINDING_CONFLICTING_CURRENT_HEALTH, format!("{label} is converged but admitted current evidence disagrees on health (no monitor leader/majority; any disagreement fails closed)")),
    }
}

/// One desired D-Serv placement's readiness (section 10).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorPlacementReadinessV1 {
    pub service_id: String,
    pub desired_node_id: String,
    pub convergence: DMonitorConvergenceV1,
    pub current_health: DMonitorCurrentHealthV1,
    pub readiness: DMonitorReadinessV1,
    pub findings: Vec<DMonitorReadinessFindingV1>,
}

fn dserv_placement_readiness(service: &DMonitorServicePlacementConvergenceV1) -> DMonitorPlacementReadinessV1 {
    let label = format!("D-Serv placement '{}'", service.service_id);
    let (readiness, findings) = if service.convergence != DMonitorConvergenceV1::Converged {
        (
            DMonitorReadinessV1::NotReady,
            vec![finding(FINDING_DSERV_NOT_CONVERGED, format!("{label} convergence is {:?}, not Converged", service.convergence))],
        )
    } else {
        let healths: Vec<DMonitorHealthV1> = service.admitted_evidence.iter().map(|evidence| evidence.health).collect();
        let current_health = assess_current_health(&healths);
        let finding = health_finding(current_health, &label);
        let readiness = if current_health == DMonitorCurrentHealthV1::Healthy { DMonitorReadinessV1::Ready } else { DMonitorReadinessV1::NotReady };
        return DMonitorPlacementReadinessV1 {
            service_id: service.service_id.clone(),
            desired_node_id: service.desired_node_id.clone(),
            convergence: service.convergence,
            current_health,
            readiness,
            findings: vec![finding],
        };
    };
    DMonitorPlacementReadinessV1 {
        service_id: service.service_id.clone(),
        desired_node_id: service.desired_node_id.clone(),
        convergence: service.convergence,
        current_health: DMonitorCurrentHealthV1::Unknown,
        readiness,
        findings,
    }
}

/// One desired DIoT placement's readiness (section 11). Freshness, exact
/// DMap/DForward/runtime-binding correlation, and runtime-binding
/// currentness are ALREADY structural prerequisites of `admitted_evidence`
/// (Phase 114D0A's `assess_diot_evidence`) and of `convergence ==
/// Converged` itself -- this function never re-derives them, only the
/// health-gating layer on top.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorDiotPlacementReadinessV1 {
    pub diot_id: String,
    pub diot_placement_id: String,
    pub desired_node_id: String,
    pub convergence: DMonitorConvergenceV1,
    pub current_health: DMonitorCurrentHealthV1,
    pub readiness: DMonitorReadinessV1,
    pub findings: Vec<DMonitorReadinessFindingV1>,
}

fn diot_placement_readiness(placement: &DMonitorDiotConvergenceV1) -> DMonitorDiotPlacementReadinessV1 {
    let label = format!("DIoT placement '{}'", placement.diot_placement_id);
    if placement.convergence != DMonitorConvergenceV1::Converged {
        return DMonitorDiotPlacementReadinessV1 {
            diot_id: placement.diot_id.clone(),
            diot_placement_id: placement.diot_placement_id.clone(),
            desired_node_id: placement.desired_node_id.clone(),
            convergence: placement.convergence,
            current_health: DMonitorCurrentHealthV1::Unknown,
            readiness: DMonitorReadinessV1::NotReady,
            findings: vec![finding(FINDING_DIOT_NOT_CONVERGED, format!("{label} convergence is {:?}, not Converged", placement.convergence))],
        };
    }
    let healths: Vec<DMonitorHealthV1> = placement.admitted_evidence.iter().map(|evidence| evidence.health).collect();
    let current_health = assess_current_health(&healths);
    let finding = health_finding(current_health, &label);
    let readiness = if current_health == DMonitorCurrentHealthV1::Healthy { DMonitorReadinessV1::Ready } else { DMonitorReadinessV1::NotReady };
    DMonitorDiotPlacementReadinessV1 {
        diot_id: placement.diot_id.clone(),
        diot_placement_id: placement.diot_placement_id.clone(),
        desired_node_id: placement.desired_node_id.clone(),
        convergence: placement.convergence,
        current_health,
        readiness,
        findings: vec![finding],
    }
}

/// Per-dimension readiness detail, embedding the full already-derived
/// convergence result for audit trail (section 8) alongside the
/// readiness-specific per-placement layer.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorDservReadinessV1 {
    pub convergence: DMonitorApplicationConvergenceV1,
    pub placements: Vec<DMonitorPlacementReadinessV1>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorDiotReadinessV1 {
    /// `false` when this dimension does not apply at all in the current
    /// context (`datum.dmap/1`, or `datum.dmap/2` with zero desired DIoT
    /// placements) -- section 15/17: a `NotRequired` dimension never blocks
    /// readiness by itself.
    pub required: bool,
    pub convergence: DMonitorApplicationDiotConvergenceV1,
    pub placements: Vec<DMonitorDiotPlacementReadinessV1>,
}

/// Top-level derived readiness view for one application (section 8).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DMonitorApplicationReadinessV1 {
    pub schema: String,
    pub project_id: String,
    pub application_id: String,
    /// Server-generated; never caller-supplied.
    pub evaluated_at_utc: String,
    pub policy: DMonitorReadinessPolicyV1,
    pub readiness: DMonitorReadinessV1,
    pub findings: Vec<DMonitorReadinessFindingV1>,
    pub dserv: DMonitorDservReadinessV1,
    pub diot: DMonitorDiotReadinessV1,
}

/// A dimension is "OK" (does not by itself block application readiness)
/// iff its OWN top-level convergence is never `Diverged` (this already
/// covers BOTH an individually-diverged desired placement AND a proven
/// unexpected realization escalating a zero-desired dimension to Diverged
/// -- section 19), AND EITHER it has zero required placements (section
/// 15/17's `NotRequired` rule) OR it is fully `Converged` with every
/// required placement individually `Ready`.
fn dimension_ok(convergence: DMonitorConvergenceV1, required_count: usize, all_placements_ready: bool) -> bool {
    if convergence == DMonitorConvergenceV1::Diverged {
        return false;
    }
    required_count == 0 || (convergence == DMonitorConvergenceV1::Converged && all_placements_ready)
}

/// Evaluates one application's DMonitor-backed readiness (section 6/7).
/// `dmonitor_state` MUST already be trusted as belonging to `project_scope`
/// in the same sense `evaluate_application_dserv_convergence` documents
/// (project isolation is additionally enforced per-record inside that
/// function and its DIoT counterpart).
#[allow(clippy::too_many_arguments)]
pub fn evaluate_application_readiness(
    project_id: &str,
    application_id: &str,
    project_scope: &str,
    authority: Option<&PlacementAuthorityView>,
    dmonitor_state: &DMonitorStateV1,
    runtime_binding_registry: &DIoTRuntimeBindingRegistryStateV1,
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    policy: &DMonitorReadinessPolicyV1,
    evaluated_at_utc: &str,
) -> DMonitorApplicationReadinessV1 {
    let dserv_convergence = evaluate_application_dserv_convergence(
        project_id,
        application_id,
        project_scope,
        authority,
        dmonitor_state,
        &policy.freshness_policy,
        evaluated_at_utc,
    );
    let diot_convergence = evaluate_application_diot_convergence(
        project_id,
        application_id,
        project_scope,
        authority,
        dmonitor_state,
        runtime_binding_registry,
        ddeploy_state,
        dnode_registry_state,
        &policy.freshness_policy,
        evaluated_at_utc,
    );

    let dserv_placements: Vec<DMonitorPlacementReadinessV1> = dserv_convergence.services.iter().map(dserv_placement_readiness).collect();
    let diot_placements: Vec<DMonitorDiotPlacementReadinessV1> = diot_convergence.placements.iter().map(diot_placement_readiness).collect();

    let dserv_required_count = dserv_convergence.services.len();
    let diot_required_count = diot_convergence.placements.len();
    let diot_required = diot_required_count > 0;

    let (readiness, findings) = if authority.is_none() {
        (
            DMonitorReadinessV1::NotReady,
            vec![finding(FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY, format!("project '{project_id}' application '{application_id}' has no active accepted placement authority"))],
        )
    } else if dserv_convergence.convergence == DMonitorConvergenceV1::Diverged {
        (DMonitorReadinessV1::NotReady, vec![finding(FINDING_DSERV_NOT_CONVERGED, "D-Serv dimension is Diverged (including a possible proven unexpected realization)")])
    } else if diot_convergence.convergence == DMonitorConvergenceV1::Diverged {
        (DMonitorReadinessV1::NotReady, vec![finding(FINDING_DIOT_NOT_CONVERGED, "DIoT dimension is Diverged (including a possible proven unexpected realization)")])
    } else if dserv_required_count == 0 && diot_required_count == 0 {
        (DMonitorReadinessV1::NotReady, vec![finding(FINDING_NO_REQUIRED_RUNTIME_REALIZATIONS, "active placement authority declares zero required DServ and zero required DIoT realizations; nothing to prove ready")])
    } else {
        let dserv_all_ready = dserv_placements.iter().all(|placement| placement.readiness == DMonitorReadinessV1::Ready);
        let diot_all_ready = diot_placements.iter().all(|placement| placement.readiness == DMonitorReadinessV1::Ready);
        let dserv_ok = dimension_ok(dserv_convergence.convergence, dserv_required_count, dserv_all_ready);
        let diot_ok = dimension_ok(diot_convergence.convergence, diot_required_count, diot_all_ready);

        if dserv_ok && diot_ok {
            (DMonitorReadinessV1::Ready, vec![finding(FINDING_READY, "every required DServ and DIoT placement is Converged with admitted current Healthy evidence")])
        } else {
            let mut reasons = Vec::new();
            if !dserv_ok {
                if dserv_convergence.convergence != DMonitorConvergenceV1::Converged {
                    reasons.push(finding(FINDING_DSERV_NOT_CONVERGED, format!("D-Serv dimension convergence is {:?}, not Converged", dserv_convergence.convergence)));
                } else {
                    for placement in dserv_placements.iter().filter(|placement| placement.readiness == DMonitorReadinessV1::NotReady) {
                        reasons.extend(placement.findings.clone());
                    }
                }
            }
            if !diot_ok {
                if diot_convergence.convergence != DMonitorConvergenceV1::Converged {
                    reasons.push(finding(FINDING_DIOT_NOT_CONVERGED, format!("DIoT dimension convergence is {:?}, not Converged", diot_convergence.convergence)));
                } else {
                    for placement in diot_placements.iter().filter(|placement| placement.readiness == DMonitorReadinessV1::NotReady) {
                        reasons.extend(placement.findings.clone());
                    }
                }
            }
            (DMonitorReadinessV1::NotReady, reasons)
        }
    };

    DMonitorApplicationReadinessV1 {
        schema: DMONITOR_READINESS_SCHEMA.to_string(),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        evaluated_at_utc: evaluated_at_utc.to_string(),
        policy: policy.clone(),
        readiness,
        findings,
        dserv: DMonitorDservReadinessV1 { convergence: dserv_convergence, placements: dserv_placements },
        diot: DMonitorDiotReadinessV1 { required: diot_required, convergence: diot_convergence, placements: diot_placements },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dmap_v2::{DCallRealizationV2, DIoTArtifactBindingV2, DIoTPlacementV2, DServPlacementV2};
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorRuntimeBindingRefV1, DMonitorSubjectV1,
        CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use crate::core::ddeploy::active_placement_authority;
    use crate::core::ddeploy_v2::PlacementAuthorityV2Extras;
    use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};
    use std::collections::BTreeSet;

    const PROJECT: &str = "proj:lab";
    const OTHER_PROJECT: &str = "proj:other";
    const APP: &str = "app:pipeline";
    const SVC_A: &str = "sensor";
    const NODE_A: &str = "fog-01";
    const NODE_B: &str = "fog-02";
    const DMAP_ID: &str = "dmap:1";
    const DMAP_REVISION: u64 = 3;
    fn dmap_digest() -> String { format!("sha256:{}", "a".repeat(64)) }

    const NOW: &str = "2026-09-25T00:10:00Z";
    const FRESH: &str = "2026-09-25T00:09:50Z";
    const STALE: &str = "2026-09-25T00:00:00Z";

    // ------------------------------------------------- LIGHTWEIGHT (DSERV-ONLY) FIXTURES

    fn authority_v1(placements: Vec<(&str, &str)>) -> PlacementAuthorityView {
        PlacementAuthorityView {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            dmap_id: DMAP_ID.to_string(),
            revision: DMAP_REVISION,
            digest: dmap_digest(),
            placements: placements.into_iter().map(|(service_id, node_id)| DServPlacementV2 { service_id: service_id.to_string(), node_id: node_id.to_string() }).collect(),
            proposal_id: None,
            acceptance_id: None,
            dgraph_id: None,
            dgraph_revision: None,
            dgraph_digest: None,
            dgraph_snapshot: None,
            v2: None,
        }
    }

    fn authority_v2_no_diot(placements: Vec<(&str, &str)>) -> PlacementAuthorityView {
        let mut view = authority_v1(placements);
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

    fn policy() -> DMonitorReadinessPolicyV1 {
        default_readiness_policy()
    }

    fn empty_ddeploy() -> DDeployStateV1 { DDeployStateV1::empty() }
    fn empty_dnode() -> DNodeRegistryStateV1 { DNodeRegistryStateV1::empty() }
    fn empty_registry() -> DIoTRuntimeBindingRegistryStateV1 { DIoTRuntimeBindingRegistryStateV1::empty() }

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

    fn exact_ref() -> DMonitorContractRefV1 {
        DMonitorContractRefV1 { id: DMAP_ID.to_string(), revision: DMAP_REVISION, digest: dmap_digest() }
    }

    fn ingest(state: &mut DMonitorStateV1, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, PROJECT, observation, received_at_utc).unwrap();
    }

    fn ingest_scope(state: &mut DMonitorStateV1, project_scope: &str, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, project_scope, observation, received_at_utc).unwrap();
    }

    fn evaluate_v1(authority: &PlacementAuthorityView, state: &DMonitorStateV1) -> DMonitorApplicationReadinessV1 {
        evaluate_application_readiness(PROJECT, APP, PROJECT, Some(authority), state, &empty_registry(), &empty_ddeploy(), &empty_dnode(), &policy(), NOW)
    }

    // ---------------------------------------------------------------- POSITIVE

    #[test]
    fn dmap1_all_required_dserv_converged_healthy_is_ready_diot_not_required() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "{result:#?}");
        assert!(!result.diot.required);
        assert_eq!(result.dserv.placements[0].readiness, DMonitorReadinessV1::Ready);
        assert!(result.findings.iter().any(|f| f.code == FINDING_READY));
    }

    #[test]
    fn dmap2_zero_diot_placements_all_dserv_ready_is_application_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v2_no_diot(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "{result:#?}");
        assert!(!result.diot.required);
        assert!(result.diot.placements.is_empty());
    }

    // --------------------------------------------------------------- FRESHNESS

    #[test]
    fn stale_otherwise_healthy_dserv_is_not_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, STALE, "obs-1"), STALE);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
        assert_eq!(result.dserv.placements[0].readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_DSERV_NOT_CONVERGED));
    }

    #[test]
    fn old_dmap_revision_is_not_ready() {
        let mut state = DMonitorStateV1::empty();
        let old_ref = DMonitorContractRefV1 { id: DMAP_ID.to_string(), revision: DMAP_REVISION - 1, digest: format!("sha256:{}", "b".repeat(64)) };
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(old_ref), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
    }

    // ------------------------------------------------------------------- HEALTH

    #[test]
    fn converged_degraded_is_not_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Degraded, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_HEALTH_DEGRADED));
    }

    #[test]
    fn converged_unhealthy_is_not_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Unhealthy, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_HEALTH_UNHEALTHY));
    }

    #[test]
    fn converged_unknown_health_is_not_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Unknown, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_HEALTH_UNKNOWN));
    }

    #[test]
    fn multiple_fresh_current_healthy_monitors_is_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-2", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "{result:#?}");
    }

    #[test]
    fn fresh_current_healthy_plus_fresh_current_unknown_is_not_ready_conflicting() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-2", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Unknown, FRESH, "obs-2"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_CONFLICTING_CURRENT_HEALTH), "{:?}", result.findings);
    }

    #[test]
    fn fresh_current_healthy_plus_stale_unhealthy_is_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        // A DIFFERENT monitor instance's STALE Unhealthy claim must never
        // veto admitted current Healthy evidence -- it is excluded from
        // admission entirely (section 12).
        ingest(&mut state, dserv_observation(NODE_A, "mon-2", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Unhealthy, STALE, "obs-2"), STALE);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "{result:#?}");
    }

    // -------------------------------------------------------------- CONVERGENCE

    #[test]
    fn wrong_node_dserv_is_not_ready() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_B, "mon-1", 1, SVC_A, NODE_B, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_DSERV_NOT_CONVERGED));
    }

    #[test]
    fn no_evidence_dserv_unknown_is_not_ready() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_DSERV_NOT_CONVERGED));
    }

    #[test]
    fn unexpected_dserv_realization_is_not_ready_even_with_desired_service_healthy() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "unexpected-svc", NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-2"), FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
        assert!(result.findings.iter().any(|f| f.code == FINDING_DSERV_NOT_CONVERGED));
    }

    // -------------------------------------------------------------------- AUTHORITY

    #[test]
    fn no_active_authority_is_not_ready() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, None, &state, &empty_registry(), &empty_ddeploy(), &empty_dnode(), &policy(), NOW);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_ACTIVE_PLACEMENT_AUTHORITY));
    }

    // ---------------------------------------------------------------------- EMPTY

    #[test]
    fn zero_dserv_and_zero_diot_is_not_ready_no_required_runtime_realizations() {
        let state = DMonitorStateV1::empty();
        let result = evaluate_v1(&authority_v2_no_diot(vec![]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_REQUIRED_RUNTIME_REALIZATIONS), "{:?}", result.findings);
    }

    #[test]
    fn zero_dserv_but_nonempty_diot_authority_and_no_diot_evidence_is_not_ready_not_vacuous() {
        // A DMap/2 declaring a desired DIoT placement but zero desired DServ
        // placements must NOT be vacuously Ready merely because DServ has
        // nothing to prove -- the DIoT placement is still required and has
        // no evidence, so it is Unknown/NotReady.
        let mut view = authority_v2_no_diot(vec![]);
        view.v2.as_mut().unwrap().diot_placements.push(DIoTPlacementV2 {
            placement_id: "diotp:broker".to_string(),
            diot_id: "diot:broker".to_string(),
            node_id: NODE_A.to_string(),
            artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:broker".to_string(), descriptor_digest: format!("sha256:{}", "e".repeat(64)), content_digest: None },
        });
        let state = DMonitorStateV1::empty();
        let result = evaluate_v1(&view, &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
        assert!(result.diot.required);
        assert!(result.findings.iter().any(|f| f.code == FINDING_DIOT_NOT_CONVERGED));
    }

    // ----------------------------------------------------------- PROJECT ISOLATION

    #[test]
    fn evidence_from_another_project_scope_does_not_participate() {
        let mut state = DMonitorStateV1::empty();
        let mut other = dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1");
        other.project_id = Some(OTHER_PROJECT.to_string());
        if let DMonitorSubjectV1::DServRealization { project_id, .. } = &mut other.subject {
            *project_id = OTHER_PROJECT.to_string();
        }
        ingest_scope(&mut state, OTHER_PROJECT, other, FRESH);
        let result = evaluate_v1(&authority_v1(vec![(SVC_A, NODE_A)]), &state);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "evidence from another project must never make this project's readiness Ready");
    }

    // ------------------------------------------------------------- DETERMINISM

    #[test]
    fn identical_inputs_produce_identical_readiness_results() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(exact_ref()), DMonitorHealthV1::Healthy, FRESH, "obs-1"), FRESH);
        let authority = authority_v1(vec![(SVC_A, NODE_A)]);
        let a = evaluate_v1(&authority, &state);
        let b = evaluate_v1(&authority, &state);
        assert_eq!(a, b);
    }

    #[test]
    fn readiness_finding_inventory_has_no_duplicates() {
        let unique: BTreeSet<&str> = ALL_READINESS_FINDING_CODES.iter().copied().collect();
        assert_eq!(unique.len(), ALL_READINESS_FINDING_CODES.len());
        assert_eq!(ALL_READINESS_FINDING_CODES.len(), 9);
    }

    #[test]
    fn default_policy_passes_freshness_validation_and_uses_healthy_only() {
        let policy = default_readiness_policy();
        assert_eq!(policy.policy_id, DEFAULT_READINESS_POLICY_ID);
        assert_eq!(policy.accepted_health, DMonitorAcceptedHealthV1::Healthy);
        crate::core::dmonitor_freshness::validate_freshness_policy(&policy.freshness_policy).unwrap();
        assert_eq!(policy.freshness_policy.max_observed_age_seconds, 90);
        assert_eq!(policy.freshness_policy.max_received_age_seconds, 90);
        assert_eq!(policy.freshness_policy.max_future_clock_skew_seconds, 10);
    }

    // ---------------------------------------------------------- REAL DIOT FIXTURE
    // Mirrors `dmonitor_interpretation::tests`' own real-fixture pattern
    // exactly (Phase 114D0A precedent) -- required here because DIoT
    // `Converged` needs a genuinely registered-and-current runtime binding,
    // which only a real `DDeployStateV1`/`DNodeRegistryStateV1` can prove.

    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
        CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dforward::{
        CanonicalArtifactRequirementV1, CanonicalDForwardInterfaceV1, CanonicalDForwardV1, CanonicalDIoTV1,
        CANONICAL_DFORWARD_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        recompute_dgraph_digest, CanonicalDCallV1, CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
        CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
    };
    use crate::core::ddeploy::ProposalSource;
    use crate::core::ddeploy_v2::{DDeployProposalV2, DIoTArtifactAuthorityBindingV2};
    use crate::core::diot_runtime_binding::register_runtime_binding;
    use crate::core::dnode_registry::declare_dnode;
    use crate::core::service_artifact_registry::{
        NativeProcessDeploymentSpecV1, ServiceArtifact, ServiceArtifactControlPlane, ServiceProbeSet,
        ServiceProbeSpec, ServiceRuntimeSpec, SERVICE_ARTIFACT_SCHEMA_VERSION,
    };

    const DIOT_ID: &str = "diot:broker";
    const DIOT_PLACEMENT_ID: &str = "diotp:broker";
    const ARTIFACT_ID: &str = "dserv:pipeline:broker";
    const SVC_B: &str = "decoder";
    const REALIZATION_INTERFACE: &str = "sensor-bus";

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
                native_process: Some(NativeProcessDeploymentSpecV1 { process_name: "broker".to_string(), entrypoint: "bin/broker-run".to_string(), args: Vec::new(), working_directory: None }),
            },
            probes: ServiceProbeSet { execution: ServiceProbeSpec { kind: "process_running".to_string(), params: std::collections::BTreeMap::new(), vantage_point: Some("local_host".to_string()) }, health: None, availability: None },
            metadata: None,
            control_plane: ServiceArtifactControlPlane { project_id: project_id.to_string(), execution_generation, execution_projection_digest: execution_projection_digest.to_string(), ..Default::default() },
        }
    }

    struct DiotFixture {
        ddeploy: DDeployStateV1,
        dnode: DNodeRegistryStateV1,
    }

    fn build_diot_fixture(broker_node: &str) -> DiotFixture {
        let dg = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: APP.to_string(),
            dgraph_id: "dgraph:prop-readiness-1".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 { service_id: SVC_A.to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 } },
                CanonicalDServV1 { service_id: SVC_B.to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 } },
            ],
            calls: vec![CanonicalDCallV1 { call_id: "sensor-to-decoder".to_string(), source_service_id: SVC_A.to_string(), target_service_id: SVC_B.to_string() }],
        };
        let dgraph_digest = recompute_dgraph_digest(&dg).unwrap();

        let dcontinuum = CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dcontinuum_id: "dcontinuum:prop-readiness-1".to_string(),
            revision: 1,
            nodes: vec![diot_node(broker_node)],
            links: Vec::new(),
        };
        let dcontinuum_digest = crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum).unwrap();

        let dforward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dforward_id: "dforward:prop-readiness-1".to_string(),
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
            proposal_id: "prop-readiness-1".to_string(),
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            dgraph_ref: crate::core::canonical_dmap_v2::ContractRefV2 { id: dg.dgraph_id.clone(), revision: dg.revision, digest: dgraph_digest },
            dcontinuum_ref: crate::core::canonical_dmap_v2::ContractRefV2 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: dcontinuum_digest },
            dforward_ref: crate::core::canonical_dmap_v2::ContractRefV2 { id: dforward.dforward_id.clone(), revision: dforward.revision, digest: dforward_digest },
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
            dcall_realizations: vec![DCallRealizationV2 { call_id: "sensor-to-decoder".to_string(), realization_kind: "via_diot_pubsub".to_string(), diot_placement_id: DIOT_PLACEMENT_ID.to_string(), interface: REALIZATION_INTERFACE.to_string(), channel: "pipeline/sensor-to-decoder".to_string() }],
            external_port_bindings: Vec::new(),
            expected_active_dmap_revision: None,
            diot_artifact_authority: {
                let mut m = std::collections::BTreeMap::new();
                m.insert(ARTIFACT_ID.to_string(), DIoTArtifactAuthorityBindingV2 { artifact_id: ARTIFACT_ID.to_string(), execution_generation: 1, execution_projection_digest: format!("sha256:{}", "c".repeat(64)) });
                m
            },
        };

        let broker_artifact = minimal_broker_artifact(PROJECT, 1, &format!("sha256:{}", "c".repeat(64)));

        let mut ddeploy = DDeployStateV1::empty();
        ddeploy.create_v2_proposal(proposal, dg, dcontinuum.clone(), dforward).unwrap();
        ddeploy
            .accept_v2_proposal("prop-readiness-1", "acc-prop-readiness-1".to_string(), "test".to_string(), "2026-01-01T00:00:01Z".to_string(), &dcontinuum, std::slice::from_ref(&broker_artifact), None)
            .unwrap();

        let mut dnode = DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode, diot_node(broker_node)).unwrap();
        DiotFixture { ddeploy, dnode }
    }

    #[allow(clippy::too_many_arguments)]
    fn diot_realization_observation(
        node_id: &str,
        monitor_instance_id: &str,
        sequence: u64,
        diot_placement_id: &str,
        observed_node_id: &str,
        dmap_ref: Option<DMonitorContractRefV1>,
        dforward_ref: Option<DMonitorContractRefV1>,
        runtime_binding_ref: Option<DMonitorRuntimeBindingRefV1>,
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
            collector: "test_diot_realization".to_string(),
            subject: DMonitorSubjectV1::DIoTRealization {
                project_id: PROJECT.to_string(),
                application_id: APP.to_string(),
                diot_id: DIOT_ID.to_string(),
                diot_placement_id: diot_placement_id.to_string(),
                node_id: observed_node_id.to_string(),
                dmap_ref,
                dforward_ref,
                runtime_binding_ref,
            },
            health,
            facts: serde_json::json!({}),
        }
    }

    struct DiotEvidenceInputs {
        fixture: DiotFixture,
        runtime_registry: DIoTRuntimeBindingRegistryStateV1,
        authority: PlacementAuthorityView,
        dmap_ref: DMonitorContractRefV1,
        dforward_ref: DMonitorContractRefV1,
        runtime_binding_ref: DMonitorRuntimeBindingRefV1,
    }

    fn build_diot_evidence_inputs(broker_node: &str) -> DiotEvidenceInputs {
        let fixture = build_diot_fixture(broker_node);
        let authority = active_placement_authority(&fixture.ddeploy, PROJECT, APP).unwrap().unwrap();
        let v2 = authority.v2.clone().unwrap();
        let mut runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let binding = register_runtime_binding(&mut runtime_registry, &fixture.ddeploy, &fixture.dnode, PROJECT, APP, DIOT_PLACEMENT_ID, "mqtt://broker.lab:1883", "2026-01-01T00:00:02Z").unwrap();
        DiotEvidenceInputs {
            dmap_ref: DMonitorContractRefV1 { id: authority.dmap_id.clone(), revision: authority.revision, digest: authority.digest.clone() },
            dforward_ref: DMonitorContractRefV1 { id: v2.dforward_id.clone(), revision: v2.dforward_revision, digest: v2.dforward_digest.clone() },
            runtime_binding_ref: DMonitorRuntimeBindingRefV1 { binding_id: binding.binding_id.clone(), binding_revision: binding.binding_revision, descriptor_digest: binding.descriptor_digest.clone() },
            fixture,
            runtime_registry,
            authority,
        }
    }

    #[test]
    fn dmap2_dserv_and_diot_converged_healthy_is_ready() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-a"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, SVC_B, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-b"), FRESH);
        ingest(&mut state, diot_realization_observation(NODE_A, "mon-2", 1, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&inputs.authority), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(result.readiness, DMonitorReadinessV1::Ready, "{result:#?}");
        assert!(result.diot.required);
        assert_eq!(result.diot.placements[0].readiness, DMonitorReadinessV1::Ready);
    }

    #[test]
    fn missing_dserv_evidence_blocks_readiness_even_with_diot_ready() {
        // Phase 114D1 section 14: the generic SmartSentinel B2 bridge does
        // not manufacture DServRealization identity, so a normal
        // environment may have zero canonical DServ evidence. Readiness
        // must remain NotReady until exact canonical DServ evidence exists
        // -- never fall back to RuntimeProcess/container/Unbound evidence.
        let inputs = build_diot_evidence_inputs(NODE_A);
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, diot_realization_observation(NODE_A, "mon-2", 1, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&inputs.authority), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
        assert!(result.findings.iter().any(|f| f.code == FINDING_DSERV_NOT_CONVERGED));
    }

    #[test]
    fn fresh_exact_current_unhealthy_diot_is_not_ready_while_convergence_stays_converged() {
        let inputs = build_diot_evidence_inputs(NODE_A);
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-a"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, SVC_B, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-b"), FRESH);
        ingest(&mut state, diot_realization_observation(NODE_A, "mon-2", 1, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Unhealthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&inputs.authority), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
        assert_eq!(result.diot.placements[0].convergence, DMonitorConvergenceV1::Converged, "convergence must remain Converged even though readiness is NotReady -- readiness is a genuinely separate dimension");
        assert_eq!(result.diot.placements[0].readiness, DMonitorReadinessV1::NotReady);
        assert!(result.findings.iter().any(|f| f.code == FINDING_HEALTH_UNHEALTHY));
    }

    #[test]
    fn stale_runtime_binding_currentness_failure_is_not_ready() {
        // A binding registered under an OLD authority (before a later
        // acceptance advanced revision) fails `runtime_binding_current`,
        // which D0A already excludes from admission -- readiness must
        // therefore be NotReady even though the observation's own health is
        // Healthy.
        let inputs = build_diot_evidence_inputs(NODE_A);
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-a"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, SVC_B, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-b"), FRESH);
        // Observation references a stale binding_id/revision that was never
        // actually registered -- runtime_binding_correlation cannot exact-match.
        let stale_binding_ref = DMonitorRuntimeBindingRefV1 { binding_id: inputs.runtime_binding_ref.binding_id.clone(), binding_revision: inputs.runtime_binding_ref.binding_revision + 1, descriptor_digest: inputs.runtime_binding_ref.descriptor_digest.clone() };
        ingest(&mut state, diot_realization_observation(NODE_A, "mon-2", 1, DIOT_PLACEMENT_ID, NODE_A, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(stale_binding_ref), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&inputs.authority), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
        assert!(result.findings.iter().any(|f| f.code == FINDING_DIOT_NOT_CONVERGED));
    }

    #[test]
    fn old_authority_but_still_healthy_session_is_not_ready() {
        // Phase 114D0B/D1's critical scenario: an MQTT session established
        // under binding N remains Healthy, but references a DMap
        // revision/digest that is no longer current -- D0A excludes it from
        // admission (authority_mismatch), so readiness must be NotReady
        // even though health itself never changed.
        let inputs = build_diot_evidence_inputs(NODE_A);
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, SVC_A, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-a"), FRESH);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, SVC_B, NODE_A, Some(inputs.dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-b"), FRESH);
        let old_dmap_ref = DMonitorContractRefV1 { id: inputs.dmap_ref.id.clone(), revision: inputs.dmap_ref.revision + 1, digest: format!("sha256:{}", "f".repeat(64)) };
        ingest(&mut state, diot_realization_observation(NODE_A, "mon-2", 1, DIOT_PLACEMENT_ID, NODE_A, Some(old_dmap_ref), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&inputs.authority), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(result.readiness, DMonitorReadinessV1::NotReady, "{result:#?}");
    }
}
