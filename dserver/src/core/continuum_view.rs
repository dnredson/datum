//! Phase 135 -- Continuum View: a derived, read-only COMPOSITION of three
//! separate, already-existing authority/observation domains into one
//! bounded response for the Console. This module defines NO new canonical
//! contract, grants NO new authority, and persists nothing -- it is a pure
//! read model, computed on demand, exactly mirroring
//! `core::dmonitor_interpretation`'s own "derived, never persisted" design.
//!
//! # Three domains, never collapsed
//!
//! - **Physical structure** (`core::dnode_registry::DNodeRegistryStateV1`):
//!   the GLOBAL, declared D-Node inventory -- node_id, platform, declared
//!   static capacity, declared D-Node ABI capability set. See that module's
//!   own doc comment for why this is deliberately NOT project-partitioned.
//! - **Accepted placement** (`core::ddeploy::active_placement_authority`):
//!   the current accepted `service_id -> node_id` binding for one
//!   `(project_id, application_id)`, enumerated here across every
//!   currently-active application (both `datum.dmap/1` and `/2`).
//! - **Observation** (`core::dmonitor_interpretation::
//!   evaluate_application_dserv_convergence`): DMonitor's own derived
//!   freshness/health/authority-correlation/placement-convergence verdict
//!   for each accepted placement -- reused EXACTLY as that module computes
//!   it, never reinterpreted or recomputed here.
//!
//! # Project-scoped physical resource observation
//!
//! The GLOBAL physical node set remains owned exclusively by
//! `DNodeRegistryStateV1`. For a project-scoped response only, an optional
//! `resource_observation` may be joined from that project's Stage-107
//! Resource Registry record for the resource currently bound to the D-Node.
//! This is live observation (CPU usage, observed memory/swap), never
//! structural capacity, placement authority, or project ownership. The
//! project scope identifies the observation source only; it never filters or
//! redefines the global D-Node set. The older
//! `core::resource_observation` Firebase/manifest pathway remains unused.
//!
//! # Global, not project-partitioned (critical ownership rule)
//!
//! The physical node set returned here is IDENTICAL regardless of
//! `project_overlay_id` -- a project overlay only adds a derived
//! [`ProjectOverlayV1`] highlighting which of the SAME global nodes that
//! project's own accepted placements currently reference. It never
//! filters, clones, or re-scopes the physical node list itself. Nothing in
//! this module ever asserts "this node belongs to project X" -- only
//! "project X's accepted workload currently references this node," which
//! is a true, narrow, already-canonically-supported fact
//! (`active_placement_authority`'s own `placements`).
//!
//! # Observation is per-placement, never one collapsed node-level verdict
//!
//! A single physical node may legitimately host accepted placements from
//! several different applications/projects simultaneously (see
//! `AcceptedPlacementRefV1`), each independently observed by DMonitor. This
//! module therefore attaches a LIST of [`NodeObservationV1`] entries to
//! each node (one per accepted placement that references it), never a
//! single overloaded per-node status -- collapsing several placements'
//! independent, potentially-divergent freshness/health verdicts into one
//! value would hide real information the brief explicitly requires stay
//! visible.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::core::canonical_dcontinuum::{CanonicalDLinkV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1};
use crate::core::ddeploy::{active_placement_authority, DDeployStateV1};
use crate::core::dmonitor_freshness::{DMonitorFreshnessPolicyV1, DMonitorFreshnessV1};
use crate::core::dmonitor_interpretation::{
    evaluate_application_dserv_convergence, AuthorityCorrelationV1, DMonitorConvergenceFindingV1,
    DMonitorConvergenceV1,
};
use crate::core::dmonitor_state::DMonitorStateV1;
use crate::core::dnode_registry::DNodeRegistryStateV1;
use crate::core::resource_registry::ResourceRegistryRecord;
use crate::core::canonical_dmonitor::DMonitorHealthV1;

pub const CONTINUUM_VIEW_SCHEMA: &str = "datum.continuum-view/1";

/// Bounds (Phase 135 brief, section 19) -- current scale (tens of nodes,
/// per existing fixtures/tests across `dnode_registry`/`ddeploy`) is far
/// below these; they exist as an explicit ceiling, not a tuned-to-capacity
/// limit. Exceeding any of them sets [`ContinuumViewV1::truncated`] rather
/// than silently growing the response or failing the request.
pub const MAX_CONTINUUM_NODES: usize = 500;
pub const MAX_PLACEMENTS_PER_NODE: usize = 200;
pub const MAX_OBSERVATIONS_PER_NODE: usize = 200;
pub const MAX_EVIDENCE_PER_OBSERVATION: usize = 50;
pub const MAX_LINKS: usize = 2000;

/// The full Continuum View response. See the module doc comment for the
/// three-domain composition this represents.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ContinuumViewV1 {
    pub schema: String,
    /// Server-generated; never caller-supplied -- mirrors
    /// `DMonitorApplicationConvergenceV1::evaluated_at_utc`'s own rule.
    pub generated_at_utc: String,
    /// The D-Node registry's own revision counter
    /// (`DNodeRegistryStateV1::revision`) -- NOT a freshly-stamped
    /// `CanonicalDContinuumV1` digest. Phase 135 deliberately does not
    /// construct a project-labeled `CanonicalDContinuumV1` wrapper here
    /// (see module doc comment): doing so would require inventing a
    /// synthetic project label for the global case, which this view never
    /// needs since it exposes structural facts directly.
    pub dnode_registry_revision: u64,
    pub nodes: Vec<ContinuumNodeV1>,
    /// Always empty today -- `core::dnode_registry::build_canonical_dcontinuum`
    /// (the one production builder of a wire `CanonicalDContinuumV1`)
    /// hardcodes `links: Vec::new()`; there is no persisted link-declaration
    /// path anywhere in this registry. Present in the schema for forward
    /// compatibility only. The Console must render an explicit "no
    /// declared physical links" state when this is empty, never silently
    /// omit the section or infer topology from layout.
    pub links: Vec<CanonicalDLinkV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_overlay: Option<ProjectOverlayV1>,
    /// `true` if any of [`MAX_CONTINUUM_NODES`]/[`MAX_LINKS`] were exceeded
    /// and the response was bounded rather than growing unbounded. Per-node
    /// placement/observation/evidence truncation is reported on the
    /// individual node instead (`ContinuumNodeV1::placements_truncated`
    /// etc.), since those are meaningful at that granularity.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ContinuumNodeV1 {
    pub node_id: String,
    pub structure: NodeStructureV1,
    /// Latest project-scoped physical resource observation for the resource
    /// currently bound to this D-Node. Observation only: never placement
    /// authority and never a replacement for declared structural capacity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_observation: Option<ProjectResourceObservationV1>,
    pub accepted_placements: Vec<AcceptedPlacementRefV1>,
    pub placements_truncated: bool,
    pub observations: Vec<NodeObservationV1>,
    pub observations_truncated: bool,
}

/// Declared, structural facts only -- see `core::canonical_dcontinuum`'s
/// own doc comment for why this never carries live telemetry. Reuses the
/// existing canonical types verbatim rather than re-declaring them.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NodeStructureV1 {
    pub platform: CanonicalNodePlatformV1,
    /// DECLARED total static capacity -- never "available"/"free"/
    /// "allocated" capacity, which no current canonical source computes
    /// (Phase 135 brief, section 7). Declared total != currently available.
    pub capacity: CanonicalNodeCapacityV1,
    pub dnode_abi_versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectResourceObservationV1 {
    pub resource_id: String,
    pub observed_at_utc: String,
    pub last_seen_utc: String,
    pub age_seconds: i64,
    pub cpu: ObservedCpuResourcesV1,
    pub memory: ObservedMemoryResourcesV1,
    pub swap: ObservedMemoryResourcesV1,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ObservedCpuResourcesV1 {
    pub logical_cores: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ObservedMemoryResourcesV1 {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RuntimeResourceUsageV1 {
    pub cpu_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_limit_bytes: u64,
    pub pids: u64,
}

/// One accepted `(project, application, service) -> this node` binding,
/// read verbatim from `active_placement_authority`'s own `placements`.
/// This is PLACEMENT AUTHORITY, never ownership -- see module doc comment.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AcceptedPlacementRefV1 {
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
}

/// DMonitor's own derived verdict for ONE accepted placement, reused
/// verbatim from `core::dmonitor_interpretation`. Every dimension kept
/// separate, exactly as that module computes it -- no new overloaded
/// status is introduced here.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NodeObservationV1 {
    pub project_id: String,
    pub application_id: String,
    pub service_id: String,
    /// `Unknown` (not `Diverged`) when there is no current evidence at all
    /// -- absence of proof is never treated as proof of drift, matching
    /// `evaluate_application_dserv_convergence`'s own rule verbatim.
    pub placement_convergence: DMonitorConvergenceV1,
    pub convergence_findings: Vec<DMonitorConvergenceFindingV1>,
    /// One entry per ADMITTED observation -- evidence DMonitor currently
    /// counts as live proof (it may still be `Stale`/`ClockSkew` in
    /// `freshness`, which is why `placement_convergence` can be `Unknown`
    /// even with non-empty `admitted_evidence`; admission and freshness are
    /// separate dimensions). There may be more than one entry if multiple
    /// monitor instances independently report on the same subject (see
    /// `dmonitor_interpretation`'s "no implicit monitor leader" rule).
    /// Empty means no current admitted evidence exists for this placement;
    /// this is reported as absence, never inferred as "offline."
    pub admitted_evidence: Vec<ObservationEvidenceV1>,
    pub admitted_evidence_truncated: bool,
    /// Evidence DMonitor saw but did NOT admit as current proof (Phase 135
    /// freeze review correction: this was originally dropped entirely,
    /// which silently hid every stale/mismatched/clock-skewed observation
    /// -- exactly the kind of collapsed-distinction the brief requires be
    /// avoided). Kept as a genuinely SEPARATE list, mirroring
    /// `DMonitorServicePlacementConvergenceV1::excluded_evidence` verbatim,
    /// each entry's own `exclusion_reason` preserved so the Console can
    /// show e.g. "stale" or "superseded" rather than nothing at all.
    pub excluded_evidence: Vec<ExcludedObservationEvidenceV1>,
    pub excluded_evidence_truncated: bool,
}

/// One admitted DMonitor observation's evidence summary for this
/// placement, reused field-for-field from
/// `dmonitor_interpretation::DMonitorEvidenceSummaryV1` (minus its
/// observation/digest identifiers, which this view has no use for).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ObservationEvidenceV1 {
    pub monitor_instance_id: String,
    /// Safe typed subset of DMonitor facts emitted by governed container
    /// realization monitoring. Raw facts are never exposed by this view.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_usage: Option<RuntimeResourceUsageV1>,
    /// The authoritative freshness verdict, copied verbatim from DMonitor's
    /// own policy evaluation -- never recomputed here (Phase 135 brief,
    /// section 12).
    pub freshness: DMonitorFreshnessV1,
    pub health: DMonitorHealthV1,
    pub authority_correlation: AuthorityCorrelationV1,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    /// Informational only, computed as `generated_at_utc - observed_at_utc`
    /// for display convenience -- NEVER used to decide `freshness` itself,
    /// which is always DMonitor's own verbatim verdict above. A negative
    /// value (a future-dated observation) is possible and intentionally
    /// not clamped -- `freshness: ClockSkew` is how that case is already
    /// honestly represented.
    pub age_seconds: i64,
}

/// Same shape as [`ObservationEvidenceV1`] plus the reason DMonitor did not
/// admit this evidence as current proof (e.g. stale, superseded by a newer
/// observation in the same stream). Never silently dropped -- see
/// [`NodeObservationV1::excluded_evidence`].
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ExcludedObservationEvidenceV1 {
    pub monitor_instance_id: String,
    pub freshness: DMonitorFreshnessV1,
    pub health: DMonitorHealthV1,
    pub authority_correlation: AuthorityCorrelationV1,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub age_seconds: i64,
    pub exclusion_reason: Option<&'static str>,
}

/// A project's view onto the SAME global physical node set -- never a
/// separate or filtered node list. See module doc comment's "critical
/// ownership rule."
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ProjectOverlayV1 {
    pub project_id: String,
    /// Every `node_id` referenced by at least one of this project's
    /// CURRENTLY ACCEPTED placements (across any of its applications).
    /// Deliberately named "in_use", never "owned": see module doc comment.
    pub node_ids_in_use: Vec<String>,
}

/// Builds the core Continuum View from three already-loaded authority /
/// observation states. Project-scoped Resource Registry telemetry is joined
/// additively afterward by [`attach_project_resource_observations`]. Pure/
/// synchronous -- no I/O, no locking (the caller, `api::continuum_view`,
/// owns acquiring the established lock order and loading each state
/// before calling this). `project_overlay_id`, if present, only adds a
/// [`ProjectOverlayV1`]; it never changes `nodes`/`links`.
pub fn build_continuum_view(
    dnode_registry_state: &DNodeRegistryStateV1,
    ddeploy_state: &DDeployStateV1,
    dmonitor_state: &DMonitorStateV1,
    freshness_policy: &DMonitorFreshnessPolicyV1,
    project_overlay_id: Option<&str>,
    now: DateTime<Utc>,
) -> ContinuumViewV1 {
    let generated_at_utc = now.to_rfc3339();

    let mut truncated = false;
    let mut node_ids: Vec<&str> = dnode_registry_state.nodes.keys().map(String::as_str).collect();
    if node_ids.len() > MAX_CONTINUUM_NODES {
        node_ids.truncate(MAX_CONTINUUM_NODES);
        truncated = true;
    }

    let mut nodes_by_id: BTreeMap<String, ContinuumNodeV1> = node_ids
        .into_iter()
        .filter_map(|node_id| dnode_registry_state.nodes.get(node_id).map(|n| (node_id.to_string(), n)))
        .map(|(node_id, node)| {
            (
                node_id.clone(),
                ContinuumNodeV1 {
                    node_id,
                    structure: NodeStructureV1 {
                        platform: node.platform.clone(),
                        capacity: node.capacity,
                        dnode_abi_versions: node.capabilities.dnode_abi_versions.clone(),
                    },
                    resource_observation: None,
                    accepted_placements: Vec::new(),
                    placements_truncated: false,
                    observations: Vec::new(),
                    observations_truncated: false,
                },
            )
        })
        .collect();

    // Enumerate every currently-active (project_id, application_id) pair,
    // across BOTH datum.dmap/1 and /2 active authority -- the union of
    // both maps' own entries, never a key-decoding of `application_key`
    // (which is a one-way length-prefixed encoding, not meant to be
    // parsed back; every entry already carries its own project_id/
    // application_id fields).
    let mut active_applications: Vec<(String, String)> = Vec::new();
    for active in ddeploy_state.active_by_application.values() {
        active_applications.push((active.active_dmap.project_id.clone(), active.application_id.clone()));
    }
    for active in ddeploy_state.phase113_v2.active.values() {
        active_applications.push((active.map.project_id.clone(), active.map.application_id.clone()));
    }
    active_applications.sort();
    active_applications.dedup();

    let mut project_node_ids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

    for (project_id, application_id) in &active_applications {
        let Ok(authority) = active_placement_authority(ddeploy_state, project_id, application_id) else {
            // Dual v1/v2 authority is an internal state-integrity problem,
            // not something this read-only view can resolve -- skip this
            // application's contribution rather than failing the whole
            // response; every other application's data remains correct.
            continue;
        };
        let Some(authority) = authority else { continue };

        for placement in &authority.placements {
            if let Some(node) = nodes_by_id.get_mut(&placement.node_id) {
                if node.accepted_placements.len() >= MAX_PLACEMENTS_PER_NODE {
                    node.placements_truncated = true;
                } else {
                    node.accepted_placements.push(AcceptedPlacementRefV1 {
                        project_id: project_id.clone(),
                        application_id: application_id.clone(),
                        service_id: placement.service_id.clone(),
                    });
                }
            }
            if project_overlay_id == Some(project_id.as_str()) {
                project_node_ids.insert(placement.node_id.clone());
            }
        }

        let convergence = evaluate_application_dserv_convergence(
            project_id,
            application_id,
            project_id,
            Some(&authority),
            dmonitor_state,
            freshness_policy,
            &generated_at_utc,
        );

        for service in &convergence.services {
            let Some(node) = nodes_by_id.get_mut(&service.desired_node_id) else { continue };
            if node.observations.len() >= MAX_OBSERVATIONS_PER_NODE {
                node.observations_truncated = true;
                continue;
            }
            let mut admitted_evidence: Vec<ObservationEvidenceV1> = service
                .admitted_evidence
                .iter()
                .map(|e| ObservationEvidenceV1 {
                    monitor_instance_id: e.monitor_instance_id.clone(),
                    runtime_usage: runtime_usage_for_observation(dmonitor_state, &e.observation_id),
                    freshness: e.freshness,
                    health: e.health,
                    authority_correlation: e.authority_correlation,
                    observed_at_utc: e.observed_at_utc.clone(),
                    received_at_utc: e.received_at_utc.clone(),
                    age_seconds: observation_age_seconds(&e.observed_at_utc, now),
                })
                .collect();
            let admitted_evidence_truncated = admitted_evidence.len() > MAX_EVIDENCE_PER_OBSERVATION;
            admitted_evidence.truncate(MAX_EVIDENCE_PER_OBSERVATION);

            let mut excluded_evidence: Vec<ExcludedObservationEvidenceV1> = service
                .excluded_evidence
                .iter()
                .map(|e| ExcludedObservationEvidenceV1 {
                    monitor_instance_id: e.monitor_instance_id.clone(),
                    freshness: e.freshness,
                    health: e.health,
                    authority_correlation: e.authority_correlation,
                    observed_at_utc: e.observed_at_utc.clone(),
                    received_at_utc: e.received_at_utc.clone(),
                    age_seconds: observation_age_seconds(&e.observed_at_utc, now),
                    exclusion_reason: e.exclusion_reason,
                })
                .collect();
            let excluded_evidence_truncated = excluded_evidence.len() > MAX_EVIDENCE_PER_OBSERVATION;
            excluded_evidence.truncate(MAX_EVIDENCE_PER_OBSERVATION);

            node.observations.push(NodeObservationV1 {
                project_id: project_id.clone(),
                application_id: application_id.clone(),
                service_id: service.service_id.clone(),
                placement_convergence: service.convergence,
                convergence_findings: service.findings.clone(),
                admitted_evidence,
                admitted_evidence_truncated,
                excluded_evidence,
                excluded_evidence_truncated,
            });
        }
    }

    let mut links = Vec::new();
    if links.len() > MAX_LINKS {
        links.truncate(MAX_LINKS);
        truncated = true;
    }

    let project_overlay = project_overlay_id.map(|project_id| ProjectOverlayV1 {
        project_id: project_id.to_string(),
        node_ids_in_use: project_node_ids.into_iter().collect(),
    });

    ContinuumViewV1 {
        schema: CONTINUUM_VIEW_SCHEMA.to_string(),
        generated_at_utc,
        dnode_registry_revision: dnode_registry_state.revision,
        nodes: nodes_by_id.into_values().collect(),
        links,
        project_overlay,
        truncated,
    }
}

/// Informational age only -- see [`ObservationEvidenceV1::age_seconds`]'s
/// doc comment for why this never feeds back into any freshness decision.
fn observation_age_seconds(observed_at_utc: &str, now: DateTime<Utc>) -> i64 {
    match DateTime::parse_from_rfc3339(observed_at_utc) {
        Ok(observed_at) => (now - observed_at.with_timezone(&Utc)).num_seconds(),
        Err(_) => 0,
    }
}


fn runtime_usage_for_observation(
    state: &DMonitorStateV1,
    observation_id: &str,
) -> Option<RuntimeResourceUsageV1> {
    let record = state.by_observation_id(observation_id)?;
    let usage = record.observation.facts.get("resource_usage")?.as_object()?;
    let cpu_percent = usage.get("cpu_percent")?.as_f64()?;
    if !cpu_percent.is_finite() || cpu_percent < 0.0 {
        return None;
    }
    let memory_used_bytes = usage.get("memory_used_bytes")?.as_u64()?;
    let memory_limit_bytes = usage.get("memory_limit_bytes")?.as_u64()?;
    let pids = usage.get("pids")?.as_u64()?;
    if memory_limit_bytes > 0 && memory_used_bytes > memory_limit_bytes {
        return None;
    }
    Some(RuntimeResourceUsageV1 {
        cpu_percent,
        memory_used_bytes,
        memory_limit_bytes,
        pids,
    })
}

fn parse_observed_memory(value: &serde_json::Value) -> Option<ObservedMemoryResourcesV1> {
    let object = value.as_object()?;
    let total_bytes = object.get("total_bytes")?.as_u64()?;
    let used_bytes = object.get("used_bytes")?.as_u64()?;
    let available_bytes = object.get("available_bytes")?.as_u64()?;
    if used_bytes > total_bytes || available_bytes > total_bytes {
        return None;
    }
    Some(ObservedMemoryResourcesV1 {
        total_bytes,
        used_bytes,
        available_bytes,
    })
}

fn project_resource_observation(
    record: &ResourceRegistryRecord,
    now: DateTime<Utc>,
) -> Option<ProjectResourceObservationV1> {
    let resources = record.discovery.resources.as_object()?;
    let cpu = resources.get("cpu")?.as_object()?;
    let logical_cores = cpu.get("logical_cores")?.as_u64()?;
    if logical_cores == 0 {
        return None;
    }
    let usage_percent = match cpu.get("usage_percent") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => {
            let value = value.as_f64()?;
            if !value.is_finite() || !(0.0..=100.0).contains(&value) {
                return None;
            }
            Some(value)
        }
    };
    let observed_at = DateTime::parse_from_rfc3339(&record.discovery.observed_at_utc)
        .ok()?
        .with_timezone(&Utc);
    Some(ProjectResourceObservationV1 {
        resource_id: record.resource_id.clone(),
        observed_at_utc: record.discovery.observed_at_utc.clone(),
        last_seen_utc: record.last_seen_utc.clone(),
        age_seconds: (now - observed_at).num_seconds().max(0),
        cpu: ObservedCpuResourcesV1 {
            logical_cores,
            usage_percent,
        },
        memory: parse_observed_memory(resources.get("memory")?)?,
        swap: parse_observed_memory(resources.get("swap")?)?,
    })
}

/// Adds live physical-resource observation to the SAME global D-Node objects
/// for a project-scoped Continuum response. The project scope only identifies
/// which Resource Registry binding supplied the observation; it never changes
/// the structural node set or placement authority.
pub fn attach_project_resource_observations(
    view: &mut ContinuumViewV1,
    records: &[ResourceRegistryRecord],
    project_id: &str,
    now: DateTime<Utc>,
) {
    let mut newest_by_node: BTreeMap<&str, &ResourceRegistryRecord> = BTreeMap::new();
    for record in records.iter().filter(|record| record.project_id == project_id) {
        let Some(node_id) = record.assigned_node_id.as_deref() else { continue };
        let replace = newest_by_node
            .get(node_id)
            .and_then(|existing| DateTime::parse_from_rfc3339(&existing.last_seen_utc).ok())
            .zip(DateTime::parse_from_rfc3339(&record.last_seen_utc).ok())
            .map(|(existing, candidate)| candidate > existing)
            .unwrap_or(!newest_by_node.contains_key(node_id));
        if replace {
            newest_by_node.insert(node_id, record);
        }
    }
    for node in &mut view.nodes {
        node.resource_observation = newest_by_node
            .get(node.node_id.as_str())
            .and_then(|record| project_resource_observation(record, now));
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;
    use crate::core::canonical_dcontinuum::{CanonicalNodeCapabilitiesV1, CanonicalNodePlatformV1};
    use crate::core::canonical_dgraph::DNODE_ABI_V0;
    use crate::core::canonical_dmap::CanonicalContractReferenceV1;
    use crate::core::canonical_dmap::{CanonicalDMapV1, CanonicalDServPlacementV1, CANONICAL_DMAP_SCHEMA};
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorSubjectV1,
    };
    use crate::core::ddeploy::{ActiveApplicationDeployment, DDeployStateV1};
    use crate::core::dmonitor_readiness::default_readiness_policy;
    use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};
    use crate::core::dnode_registry::{declare_dnode, DNodeRegistryStateV1};
    use uuid::Uuid;

    fn policy() -> DMonitorFreshnessPolicyV1 {
        default_readiness_policy().freshness_policy
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 4, 12, 0, 0).unwrap()
    }

    fn node(node_id: &str, cpu: u32, mem: u64) -> crate::core::canonical_dcontinuum::CanonicalDNodeV1 {
        crate::core::canonical_dcontinuum::CanonicalDNodeV1 {
            node_id: node_id.to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: cpu, memory_bytes: mem },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        }
    }

    fn registry_with(nodes: &[(&str, u32, u64)]) -> DNodeRegistryStateV1 {
        let mut state = DNodeRegistryStateV1::empty();
        for (id, cpu, mem) in nodes {
            declare_dnode(&mut state, node(id, *cpu, *mem)).unwrap();
        }
        state
    }

    /// Builds a FULLY VALID, self-consistent accepted v1 application --
    /// including a real `ProposalRecord` in `state.proposals` (required by
    /// `active_placement_authority`, which looks the proposal up by id and
    /// recomputes its `dgraph_snapshot`'s digest) -- rather than hand-
    /// inserting only `active_by_application` in isolation. Bypasses
    /// `create_proposal`/`accept_proposal` themselves (which additionally
    /// require D-Node eligibility/D-Code registry wiring this test module
    /// has no need of), but every digest/reference below is genuinely
    /// recomputed, never a placeholder string, so `active_placement_authority`
    /// exercises its real lookup/digest-recomputation path exactly as
    /// production does.
    /// Returns `(dmap_id, dmap_digest)` -- the EXACT `{id, digest}` pair
    /// (`revision` is always `1`) a DMonitor observation's `dmap_ref` must
    /// carry for `AuthorityCorrelationV1::ExactMatch` against this
    /// application's now-active authority, so callers can construct a
    /// genuinely matching (or deliberately mismatching) reference instead
    /// of guessing at placeholder values.
    fn placed_application(
        state: &mut DDeployStateV1,
        project_id: &str,
        application_id: &str,
        service_id: &str,
        node_id: &str,
    ) -> (String, String) {
        use crate::core::canonical_dgraph::{
            recompute_dgraph_digest, CanonicalDGraphV1, CanonicalDServExecutionRequirementV1,
            CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA,
        };
        use crate::core::canonical_dcontinuum::recompute_dcontinuum_digest;
        use crate::core::ddeploy::{
            compute_proposal_digest, DDeployProposalV1, ProposalLifecycleState, ProposalRecord,
            ProposalSource,
        };

        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: application_id.to_string(),
            dgraph_id: format!("dgraph:{application_id}"),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_id: service_id.to_string(),
                service_definition: None,
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 },
            }],
            calls: Vec::new(),
        };
        let dcontinuum = crate::core::canonical_dcontinuum::CanonicalDContinuumV1 {
            schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            dcontinuum_id: format!("continuum:{project_id}"),
            revision: 1,
            nodes: vec![node(node_id, 1, 1)],
            links: Vec::new(),
        };
        assert!(dgraph.validate().is_empty(), "{:?}", dgraph.validate());
        assert!(dcontinuum.validate().is_empty(), "{:?}", dcontinuum.validate());

        let dgraph_digest = recompute_dgraph_digest(&dgraph).unwrap();
        let dcontinuum_digest = recompute_dcontinuum_digest(&dcontinuum).unwrap();

        let proposal_id = format!("proposal:{}", Uuid::new_v4());
        let acceptance_id = format!("acceptance:{}", Uuid::new_v4());
        let proposal = DDeployProposalV1 {
            schema: "datum.ddeploy-proposal/1".to_string(),
            proposal_id: proposal_id.clone(),
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-10-04T00:00:00Z".to_string(),
            expected_active_dmap_revision: None,
            dgraph_ref: CanonicalContractReferenceV1 { id: dgraph.dgraph_id.clone(), revision: dgraph.revision, digest: dgraph_digest },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: dcontinuum_digest },
            placements: vec![CanonicalDServPlacementV1 { service_id: service_id.to_string(), node_id: node_id.to_string() }],
            dcode_artifact_authority: BTreeMap::new(),
        };
        let proposal_digest = compute_proposal_digest(&proposal).unwrap();
        state.proposals.insert(
            proposal_id.clone(),
            ProposalRecord {
                proposal,
                planner_provenance: None,
                proposal_digest: proposal_digest.clone(),
                dgraph_snapshot: dgraph,
                dcontinuum_snapshot: dcontinuum,
                state: ProposalLifecycleState::Accepted,
                accepted_acceptance_id: Some(acceptance_id.clone()),
            },
        );

        let dmap_id = crate::core::ddeploy::canonical_dmap_id(project_id, application_id);
        let dmap = CanonicalDMapV1 {
            schema: CANONICAL_DMAP_SCHEMA.to_string(),
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            dmap_id: dmap_id.clone(),
            revision: 1,
            // `active_placement_authority`'s v1 path never reads these two
            // refs' own content (only `a.dmap_digest`/`a.active_dmap.{revision,dmap_id,placements}`
            // and the real proposal's `dgraph_snapshot`) -- these exist only
            // to satisfy `CanonicalDMapV1`'s required fields structurally.
            dgraph_ref: CanonicalContractReferenceV1 { id: format!("dgraph:{application_id}"), revision: 1, digest: format!("sha256:{}", "0".repeat(64)) },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: "continuum:global".to_string(), revision: 1, digest: format!("sha256:{}", "0".repeat(64)) },
            placements: vec![CanonicalDServPlacementV1 { service_id: service_id.to_string(), node_id: node_id.to_string() }],
        };
        let key = crate::core::ddeploy::application_key(project_id, application_id);
        state.active_by_application.insert(
            key,
            ActiveApplicationDeployment {
                application_id: application_id.to_string(),
                active_dmap: dmap,
                dmap_digest: proposal_digest.clone(),
                proposal_id,
                acceptance_id,
                accepted_at_utc: "2026-10-04T00:00:00Z".to_string(),
                operational_services: BTreeMap::new(),
            },
        );
        (dmap_id, proposal_digest)
    }

    #[allow(clippy::too_many_arguments)]
    fn dserv_observation(
        project_id: &str,
        node_id: &str,
        application_id: &str,
        service_id: &str,
        dmap_id: &str,
        dmap_revision: u64,
        dmap_digest: &str,
        observed_at_utc: &str,
        monitor_instance_id: &str,
    ) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: crate::core::canonical_dmonitor::CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: format!("obs:{}", Uuid::new_v4()),
            project_id: Some(project_id.to_string()),
            node_id: node_id.to_string(),
            monitor_instance_id: monitor_instance_id.to_string(),
            sequence: 1,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "test-collector".to_string(),
            subject: DMonitorSubjectV1::DServRealization {
                project_id: project_id.to_string(),
                application_id: application_id.to_string(),
                service_id: service_id.to_string(),
                node_id: node_id.to_string(),
                dmap_ref: Some(DMonitorContractRefV1 {
                    id: dmap_id.to_string(),
                    revision: dmap_revision,
                    digest: dmap_digest.to_string(),
                }),
            },
            health: crate::core::canonical_dmonitor::DMonitorHealthV1::Healthy,
            facts: json!({}),
        }
    }

    #[test]
    fn empty_registry_produces_an_honest_empty_view() {
        let registry = DNodeRegistryStateV1::empty();
        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        assert!(view.nodes.is_empty());
        assert!(view.links.is_empty());
        assert!(!view.truncated);
        assert_eq!(view.schema, CONTINUUM_VIEW_SCHEMA);
    }

    #[test]
    fn global_view_lists_every_registered_node_regardless_of_placement() {
        let registry = registry_with(&[("node-a", 4, 4_294_967_296), ("node-b", 8, 8_589_934_592)]);
        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        let mut ids: Vec<_> = view.nodes.iter().map(|n| n.node_id.clone()).collect();
        ids.sort();
        assert_eq!(ids, vec!["node-a".to_string(), "node-b".to_string()]);
    }

    #[test]
    fn node_without_placement_has_no_accepted_placements_and_no_observations() {
        let registry = registry_with(&[("node-c", 2, 2_147_483_648)]);
        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        let node = view.nodes.iter().find(|n| n.node_id == "node-c").unwrap();
        assert!(node.accepted_placements.is_empty());
        assert!(node.observations.is_empty());
    }

    #[test]
    fn shared_node_exposes_two_distinct_accepted_placements() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let mut ddeploy = DDeployStateV1::empty();
        placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-01");
        placed_application(&mut ddeploy, "project-b", "app-9", "service-bar", "node-01");
        let dmonitor = DMonitorStateV1::empty();

        let global = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        assert_eq!(global.nodes.len(), 1, "one physical node, never duplicated");
        let node = &global.nodes[0];
        let mut projects: Vec<_> = node.accepted_placements.iter().map(|p| p.project_id.clone()).collect();
        projects.sort();
        assert_eq!(projects, vec!["project-a".to_string(), "project-b".to_string()]);
    }

    #[test]
    fn project_overlay_highlights_in_use_nodes_without_filtering_the_global_set() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184), ("node-02", 4, 4_294_967_296)]);
        let mut ddeploy = DDeployStateV1::empty();
        placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-01");
        let dmonitor = DMonitorStateV1::empty();

        let overlay = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), Some("project-a"), now());
        assert_eq!(overlay.nodes.len(), 2, "project overlay must not reduce the global physical node set");
        let project_overlay = overlay.project_overlay.expect("overlay requested");
        assert_eq!(project_overlay.project_id, "project-a");
        assert_eq!(project_overlay.node_ids_in_use, vec!["node-01".to_string()]);
    }

    #[test]
    fn project_overlay_for_a_project_with_no_placements_is_empty_not_an_error() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let overlay = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), Some("project-z"), now());
        assert!(overlay.project_overlay.unwrap().node_ids_in_use.is_empty());
        assert_eq!(overlay.nodes.len(), 1, "physical nodes remain visible regardless of this project's own usage");
    }

    #[test]
    fn node_without_any_dmonitor_evidence_reports_unknown_convergence_and_no_offline_inference() {
        let registry = registry_with(&[("node-02", 4, 4_294_967_296)]);
        let mut ddeploy = DDeployStateV1::empty();
        placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-02");
        let dmonitor = DMonitorStateV1::empty();

        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        let node = view.nodes.iter().find(|n| n.node_id == "node-02").unwrap();
        assert_eq!(node.observations.len(), 1);
        let observation = &node.observations[0];
        assert_eq!(observation.placement_convergence, DMonitorConvergenceV1::Unknown);
        assert!(observation.admitted_evidence.is_empty(), "no evidence must mean an empty list, never a fabricated offline status");
        assert!(observation.excluded_evidence.is_empty());
    }

    #[test]
    fn fresh_matching_observation_converges() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let mut ddeploy = DDeployStateV1::empty();
        let (dmap_id, dmap_digest) = placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-01");

        let mut dmonitor = DMonitorStateV1::empty();
        let observed_at = "2026-10-04T11:59:30Z";
        let observation = dserv_observation(
            "project-a", "node-01", "app-1", "service-foo", &dmap_id, 1,
            &dmap_digest,
            observed_at, "monitor-1",
        );
        ingest_observation(&mut dmonitor, "project-a", observation, &now().to_rfc3339()).unwrap();

        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        let node = view.nodes.iter().find(|n| n.node_id == "node-01").unwrap();
        let observation = &node.observations[0];
        assert_eq!(observation.placement_convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(observation.admitted_evidence.len(), 1);
        assert_eq!(observation.admitted_evidence[0].freshness, DMonitorFreshnessV1::Fresh);
        assert_eq!(observation.admitted_evidence[0].health, DMonitorHealthV1::Healthy);
        assert_eq!(observation.admitted_evidence[0].authority_correlation, AuthorityCorrelationV1::ExactMatch);
        assert_eq!(observation.admitted_evidence[0].age_seconds, 30);
    }

    #[test]
    fn stale_observation_is_reported_as_stale_not_recomputed() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let mut ddeploy = DDeployStateV1::empty();
        let (dmap_id, dmap_digest) = placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-01");

        let mut dmonitor = DMonitorStateV1::empty();
        // Far older than the default 90s freshness window.
        let observed_at = "2026-10-04T10:00:00Z";
        let observation = dserv_observation(
            "project-a", "node-01", "app-1", "service-foo", &dmap_id, 1,
            &dmap_digest,
            observed_at, "monitor-1",
        );
        ingest_observation(&mut dmonitor, "project-a", observation, &now().to_rfc3339()).unwrap();

        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        let node = view.nodes.iter().find(|n| n.node_id == "node-01").unwrap();
        let observation = &node.observations[0];
        assert!(observation.admitted_evidence.is_empty(), "stale evidence must not be admitted as current proof");
        assert_eq!(observation.excluded_evidence.len(), 1);
        assert_eq!(observation.excluded_evidence[0].freshness, DMonitorFreshnessV1::Stale);
        // Staleness means this cannot currently PROVE convergence.
        assert_eq!(observation.placement_convergence, DMonitorConvergenceV1::Unknown);
    }

    #[test]
    fn mismatched_authority_reference_is_unknown_never_fabricated_as_converged_or_diverged() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let mut ddeploy = DDeployStateV1::empty();
        let (dmap_id, _dmap_digest) = placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-01");

        let mut dmonitor = DMonitorStateV1::empty();
        let observed_at = "2026-10-04T11:59:30Z";
        // Same dmap_id, but a DIFFERENT digest than the currently active one.
        let observation = dserv_observation(
            "project-a", "node-01", "app-1", "service-foo", &dmap_id, 1,
            &format!("sha256:{}", "d".repeat(64)),
            observed_at, "monitor-1",
        );
        ingest_observation(&mut dmonitor, "project-a", observation, &now().to_rfc3339()).unwrap();

        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        let node = view.nodes.iter().find(|n| n.node_id == "node-01").unwrap();
        let observation = &node.observations[0];
        assert!(observation.admitted_evidence.is_empty(), "a mismatched reference is not current-authority proof, so it is not admitted either");
        assert_eq!(observation.excluded_evidence.len(), 1);
        assert_eq!(observation.excluded_evidence[0].authority_correlation, AuthorityCorrelationV1::Mismatch);
        // A stale/historical/mismatched reference is valid HISTORICAL
        // evidence but never current-authority proof of EITHER state --
        // `Unknown`, never fabricated as `Diverged` (absence of current
        // proof is not proof of drift, exactly like absent evidence).
        assert_eq!(observation.placement_convergence, DMonitorConvergenceV1::Unknown);
    }

    #[test]
    fn links_are_present_but_empty_today() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        assert!(view.links.is_empty());
    }

    #[test]
    fn placement_referencing_an_unregistered_node_is_not_silently_fabricated_as_a_node() {
        // A drift scenario: an accepted placement references a node_id that
        // is not (or no longer) a registered structural D-Node. This must
        // not create a phantom node entry out of thin air.
        let registry = registry_with(&[("node-01", 8, 17_179_869_184)]);
        let mut ddeploy = DDeployStateV1::empty();
        placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-removed");
        let dmonitor = DMonitorStateV1::empty();

        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        assert_eq!(view.nodes.len(), 1);
        assert!(view.nodes.iter().all(|n| n.node_id != "node-removed"));
    }

    #[test]
    fn deterministic_ordering_regardless_of_registration_order() {
        let mut registry_a = DNodeRegistryStateV1::empty();
        declare_dnode(&mut registry_a, node("fog-01", 4, 4_294_967_296)).unwrap();
        declare_dnode(&mut registry_a, node("cloud-01", 8, 17_179_869_184)).unwrap();
        let mut registry_b = DNodeRegistryStateV1::empty();
        declare_dnode(&mut registry_b, node("cloud-01", 8, 17_179_869_184)).unwrap();
        declare_dnode(&mut registry_b, node("fog-01", 4, 4_294_967_296)).unwrap();

        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let view_a = build_continuum_view(&registry_a, &ddeploy, &dmonitor, &policy(), None, now());
        let view_b = build_continuum_view(&registry_b, &ddeploy, &dmonitor, &policy(), None, now());
        let ids_a: Vec<_> = view_a.nodes.iter().map(|n| n.node_id.clone()).collect();
        let ids_b: Vec<_> = view_b.nodes.iter().map(|n| n.node_id.clone()).collect();
        assert_eq!(ids_a, ids_b, "node ordering must not depend on registration order");
    }

    #[test]
    fn response_is_bounded_when_node_count_exceeds_the_limit() {
        let mut registry = DNodeRegistryStateV1::empty();
        for i in 0..(MAX_CONTINUUM_NODES + 5) {
            declare_dnode(&mut registry, node(&format!("node-{i}"), 1, 1_073_741_824)).unwrap();
        }
        let ddeploy = DDeployStateV1::empty();
        let dmonitor = DMonitorStateV1::empty();
        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());
        assert_eq!(view.nodes.len(), MAX_CONTINUUM_NODES);
        assert!(view.truncated);
    }

    /// Phase 135 brief, section 30: a node with matching accepted/observed
    /// state and a node with missing observation must remain independently,
    /// simultaneously correct in the SAME response -- one node's state must
    /// never bleed into another's.
    #[test]
    fn accepted_and_observed_states_remain_independent_across_nodes_in_one_response() {
        let registry = registry_with(&[("node-01", 8, 17_179_869_184), ("node-02", 4, 4_294_967_296)]);
        let mut ddeploy = DDeployStateV1::empty();
        let (dmap_id, dmap_digest) = placed_application(&mut ddeploy, "project-a", "app-1", "service-foo", "node-01");
        placed_application(&mut ddeploy, "project-a", "app-2", "service-bar", "node-02");

        let mut dmonitor = DMonitorStateV1::empty();
        let observation = dserv_observation(
            "project-a", "node-01", "app-1", "service-foo", &dmap_id, 1,
            &dmap_digest, "2026-10-04T11:59:30Z", "monitor-1",
        );
        ingest_observation(&mut dmonitor, "project-a", observation, &now().to_rfc3339()).unwrap();
        // node-02's service has an accepted placement but NO DMonitor
        // evidence was ever ingested for it.

        let view = build_continuum_view(&registry, &ddeploy, &dmonitor, &policy(), None, now());

        let node_a = view.nodes.iter().find(|n| n.node_id == "node-01").unwrap();
        assert_eq!(node_a.observations.len(), 1);
        assert_eq!(node_a.observations[0].placement_convergence, DMonitorConvergenceV1::Converged);
        assert_eq!(node_a.observations[0].admitted_evidence.len(), 1);

        let node_b = view.nodes.iter().find(|n| n.node_id == "node-02").unwrap();
        assert_eq!(node_b.observations.len(), 1);
        assert_eq!(node_b.observations[0].placement_convergence, DMonitorConvergenceV1::Unknown);
        assert!(node_b.observations[0].admitted_evidence.is_empty());
        assert!(node_b.observations[0].excluded_evidence.is_empty());
        assert!(
            node_a.observations[0].admitted_evidence[0].freshness == DMonitorFreshnessV1::Fresh,
            "node-a's real evidence must not be affected by node-b having none"
        );
    }
}
