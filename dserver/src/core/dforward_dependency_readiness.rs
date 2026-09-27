//! DForward `requires[scope = any_element]` dependency readiness (Phase
//! 114D2, ADR-0010/ADR-0023/ADR-0024). Closes the Phase 113 deferred seam:
//! `dforward_projection`'s STATIC authority-only projection always reports
//! `readiness_authority = "dmonitor"` / `readiness_state =
//! "pending_fresh_evidence"` for an `any_element` requirement, by design
//! (ADR-0010: cross-node readiness can ONLY come from fresh DMonitor
//! evidence, never from static configuration). This module is the DYNAMIC,
//! evidence-aware counterpart: it resolves the CURRENT canonical provider
//! for each `any_element` requirement and asks whether THAT EXACT provider
//! placement is `Ready` under Phase 114D1 governance.
//!
//! # Central rule
//!
//! For `consumer.requires(interface = I, scope = any_element)`, the
//! question is never "is the whole application Ready?" -- it is "which
//! canonical DIoT provides I to this consumer according to the CURRENT
//! accepted DForward/DMap/2 authority, and is THAT provider placement Ready
//! under Phase 114D1?" A dependency is `Satisfied` purely on the strength of
//! its own exact provider placement's readiness; an unrelated D-Serv
//! placement being `NotReady` never blocks it (see
//! `unrelated_dserv_notready_does_not_block_a_ready_provider` and the real
//! integration proof).
//!
//! # Static projection is untouched
//!
//! This module never modifies `dforward_projection`'s existing static
//! authority-only projection or its `pending_fresh_evidence` output --
//! that projection genuinely has no DMonitor state to consult and must
//! keep saying so honestly. This module is an entirely separate,
//! additively-named evaluation that DOES consult DMonitor state (via Phase
//! 114D1's already-derived readiness), so the two can never be confused for
//! one another.
//!
//! # Consumes D1, never reimplements it
//!
//! This module calls `dmonitor_readiness::evaluate_application_readiness`
//! UNCHANGED, exactly once per evaluation (same snapshot, same
//! `evaluated_at_utc`, same server-owned policy), and only reads
//! `DMonitorDiotPlacementReadinessV1` entries from its result -- it never
//! recomputes freshness, DMap/DForward/runtime-binding correlation, health
//! aggregation, or monitor-conflict policy.
//!
//! # Provider resolution: audited, not assumed
//!
//! `CanonicalDForwardV1::validate` (see that module) proves hop-endpoint
//! interface COMPLEMENTARITY only (`(a.provides(I) && b.requires(I)) ||
//! (a.requires(I) && b.provides(I))`) -- it never fixes which side of a hop
//! is semantically "the provider". This module therefore NEVER treats
//! `hop.from`/`hop.to` position as authority: it identifies the consumer's
//! own side of a hop by exact `diot_id` match, then classifies the OTHER
//! side as a provider candidate only if it is a declared DIoT whose
//! `provides[]` actually contains the required interface.
//!
//! Likewise, canonical validation proves NOTHING about provider
//! multiplicity: `DIoTPlacementV2` uniqueness is enforced only on
//! `placement_id`, never on `diot_id` (audited directly in
//! `canonical_dmap_v2.rs` -- `duplicate_diot_placement` only fires on a
//! repeated `placement_id`), so multiple placements legitimately sharing
//! one logical `diot_id` are NOT rejected by canonical validation. Since no
//! accepted contract establishes a provider-selection or placement-
//! selection rule for that case, this module fails closed
//! (`provider_ambiguous`/`provider_placement_ambiguous`) rather than
//! inventing one (first-wins, nearest-node, healthiest-wins, etc. are all
//! explicitly forbidden -- that would be new placement/routing authority
//! this phase does not have).

use std::collections::BTreeSet;

use crate::core::canonical_dforward::{CanonicalDForwardV1, DForwardRequirementScope};
use crate::core::canonical_dmap_v2::DIoTPlacementV2;
use crate::core::ddeploy::DDeployStateV1;
use crate::core::ddeploy_v2::PlacementAuthorityView;
use crate::core::diot_runtime_binding::DIoTRuntimeBindingRegistryStateV1;
use crate::core::dmonitor_readiness::{
    evaluate_application_readiness, DMonitorDiotPlacementReadinessV1, DMonitorReadinessPolicyV1, DMonitorReadinessV1,
};
use crate::core::dmonitor_state::DMonitorStateV1;
use crate::core::dnode_registry::DNodeRegistryStateV1;

pub const DFORWARD_DEPENDENCY_READINESS_SCHEMA: &str = "datum.dforward-dependency-readiness/1";

// ------------------------------------------------------------- FINDING CODES
// A THIRD, separate finding-code layer -- distinct from both
// `dmonitor_interpretation`'s 19 convergence codes and
// `dmonitor_readiness`'s 9 readiness codes, per this phase's own explicit
// "do not overload" instruction (matching that same instruction's own
// precedent from Phase 114D1).
pub const FINDING_SATISFIED: &str = "satisfied";
pub const FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY: &str = "no_active_dmap_v2_authority";
pub const FINDING_PROVIDER_UNRESOLVED: &str = "provider_unresolved";
pub const FINDING_PROVIDER_AMBIGUOUS: &str = "provider_ambiguous";
pub const FINDING_PROVIDER_NOT_DIOT: &str = "provider_not_diot";
pub const FINDING_PROVIDER_PLACEMENT_MISSING: &str = "provider_placement_missing";
pub const FINDING_PROVIDER_PLACEMENT_AMBIGUOUS: &str = "provider_placement_ambiguous";
pub const FINDING_PROVIDER_READINESS_MISSING: &str = "provider_readiness_missing";
pub const FINDING_PROVIDER_NOT_READY: &str = "provider_not_ready";

/// The complete, stable finding-code inventory for this module, mirroring
/// `dmonitor_readiness::ALL_READINESS_FINDING_CODES`'s own precedent.
pub const ALL_DEPENDENCY_FINDING_CODES: &[&str] = &[
    FINDING_SATISFIED,
    FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY,
    FINDING_PROVIDER_UNRESOLVED,
    FINDING_PROVIDER_AMBIGUOUS,
    FINDING_PROVIDER_NOT_DIOT,
    FINDING_PROVIDER_PLACEMENT_MISSING,
    FINDING_PROVIDER_PLACEMENT_AMBIGUOUS,
    FINDING_PROVIDER_READINESS_MISSING,
    FINDING_PROVIDER_NOT_READY,
];

fn finding(code: &'static str, message: impl Into<String>) -> DForwardDependencyFindingV1 {
    DForwardDependencyFindingV1 { code, message: message.into() }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DForwardDependencyFindingV1 {
    pub code: &'static str,
    pub message: String,
}

/// Fail-closed, two-state per-dependency governance -- no third `Unknown`
/// outcome, mirroring Phase 114D1's own `DMonitorReadinessV1` rationale:
/// this is an authorization-style decision (may this dependency be treated
/// as satisfied?), and an inability to positively prove it is `Blocked`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DForwardDependencyStateV1 {
    Satisfied,
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DMapRefSummaryV1 {
    pub dmap_id: String,
    pub revision: u64,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DForwardRefSummaryV1 {
    pub dforward_id: String,
    pub revision: u64,
    pub digest: String,
}

/// One consumer `any_element` requirement's derived dependency readiness.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DForwardAnyElementDependencyReadinessV1 {
    pub consumer_diot_id: String,
    pub consumer_placement_id: String,
    pub consumer_node_id: String,
    pub interface: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_diot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_placement_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_node_id: Option<String>,
    /// The EXACT provider placement's own Phase 114D1 per-placement result
    /// -- embedded for auditability (section 19), never the whole
    /// application readiness object and never unrelated D-Serv evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_readiness: Option<DMonitorDiotPlacementReadinessV1>,
    pub state: DForwardDependencyStateV1,
    pub findings: Vec<DForwardDependencyFindingV1>,
}

/// Top-level derived result for one (project, application, node) -- section
/// 18. `findings` here carries ONLY authority-level issues (e.g. no active
/// `datum.dmap/2`); per-dependency reasons live on each dependency's own
/// `findings`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DForwardDependencyReadinessV1 {
    pub schema: String,
    pub project_id: String,
    pub application_id: String,
    pub node_id: String,
    pub evaluated_at_utc: String,
    pub policy: DMonitorReadinessPolicyV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dmap_ref: Option<DMapRefSummaryV1>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dforward_ref: Option<DForwardRefSummaryV1>,
    pub findings: Vec<DForwardDependencyFindingV1>,
    pub dependencies: Vec<DForwardAnyElementDependencyReadinessV1>,
}

/// Resolves the set of distinct CANDIDATE provider `diot_id`s for
/// `consumer_diot_id`'s `interface` requirement, from the CURRENT accepted
/// DForward's own chain/hop topology -- section 11/14: hop direction never
/// decides provider identity (only exact `provides(interface)` semantics
/// do), and repeated hop references to the SAME provider are deduplicated
/// by construction (`BTreeSet`). `saw_external_endpoint` is `true` when at
/// least one matching hop's OTHER side is a declared external endpoint
/// rather than a DIoT -- used to distinguish "resolved only to an external
/// endpoint" (`provider_not_diot`) from "resolved to nothing at all"
/// (`provider_unresolved`).
fn resolve_provider_candidates(forward: &CanonicalDForwardV1, consumer_diot_id: &str, interface: &str) -> (BTreeSet<String>, bool) {
    let mut providers = BTreeSet::new();
    let mut saw_external_endpoint = false;
    for chain in &forward.chains {
        for hop in &chain.hops {
            if hop.interface != interface {
                continue;
            }
            let other_side = if hop.from == consumer_diot_id {
                Some(hop.to.as_str())
            } else if hop.to == consumer_diot_id {
                Some(hop.from.as_str())
            } else {
                None
            };
            let Some(other_side) = other_side else { continue };
            if let Some(other_diot) = forward.diots.iter().find(|d| d.diot_id == other_side) {
                if other_diot.provides.iter().any(|p| p.interface == interface) {
                    providers.insert(other_diot.diot_id.clone());
                }
            } else if forward.external_endpoints.iter().any(|endpoint| endpoint.endpoint_id == other_side) {
                saw_external_endpoint = true;
            }
        }
    }
    (providers, saw_external_endpoint)
}

/// Classifies and resolves ONE dependency row, consuming already-computed
/// D1 `diot_placements` readiness (never recomputing it -- section 8/9).
fn evaluate_one_dependency(
    consumer_diot_id: &str,
    consumer_placement_id: &str,
    consumer_node_id: &str,
    interface: &str,
    forward: &CanonicalDForwardV1,
    diot_placements: &[DIoTPlacementV2],
    diot_readiness: &[DMonitorDiotPlacementReadinessV1],
) -> DForwardAnyElementDependencyReadinessV1 {
    let (providers, saw_external_endpoint) = resolve_provider_candidates(forward, consumer_diot_id, interface);

    let base = |provider_diot_id: Option<String>, provider_placement_id: Option<String>, provider_node_id: Option<String>, provider_readiness: Option<DMonitorDiotPlacementReadinessV1>, state, findings| {
        DForwardAnyElementDependencyReadinessV1 {
            consumer_diot_id: consumer_diot_id.to_string(),
            consumer_placement_id: consumer_placement_id.to_string(),
            consumer_node_id: consumer_node_id.to_string(),
            interface: interface.to_string(),
            provider_diot_id,
            provider_placement_id,
            provider_node_id,
            provider_readiness,
            state,
            findings,
        }
    };

    if providers.len() > 1 {
        return base(
            None,
            None,
            None,
            None,
            DForwardDependencyStateV1::Blocked,
            vec![finding(FINDING_PROVIDER_AMBIGUOUS, format!("interface '{interface}' resolves to {} distinct logical providers {providers:?}; no accepted alternative-provider selection semantics exist", providers.len()))],
        );
    }
    let Some(provider_diot_id) = providers.into_iter().next() else {
        return if saw_external_endpoint {
            base(
                None,
                None,
                None,
                None,
                DForwardDependencyStateV1::Blocked,
                vec![finding(FINDING_PROVIDER_NOT_DIOT, format!("interface '{interface}' resolves only to an external endpoint, which has no DIoT placement readiness"))],
            )
        } else {
            base(
                None,
                None,
                None,
                None,
                DForwardDependencyStateV1::Blocked,
                vec![finding(FINDING_PROVIDER_UNRESOLVED, format!("no canonical DIoT chain hop resolves interface '{interface}' for consumer '{consumer_diot_id}'"))],
            )
        };
    };

    let matching_placements: Vec<&DIoTPlacementV2> = diot_placements.iter().filter(|placement| placement.diot_id == provider_diot_id).collect();
    let provider_placement = match matching_placements.len() {
        0 => {
            return base(
                Some(provider_diot_id.clone()),
                None,
                None,
                None,
                DForwardDependencyStateV1::Blocked,
                vec![finding(FINDING_PROVIDER_PLACEMENT_MISSING, format!("provider DIoT '{provider_diot_id}' has no current DMap/2 placement"))],
            );
        }
        1 => matching_placements[0],
        _ => {
            return base(
                Some(provider_diot_id.clone()),
                None,
                None,
                None,
                DForwardDependencyStateV1::Blocked,
                vec![finding(FINDING_PROVIDER_PLACEMENT_AMBIGUOUS, format!("provider DIoT '{provider_diot_id}' has {} current placements; no accepted placement-selection semantics exist for this case", matching_placements.len()))],
            );
        }
    };

    let provider_readiness = diot_readiness.iter().find(|placement| placement.diot_placement_id == provider_placement.placement_id);
    match provider_readiness {
        None => base(
            Some(provider_diot_id.clone()),
            Some(provider_placement.placement_id.clone()),
            Some(provider_placement.node_id.clone()),
            None,
            DForwardDependencyStateV1::Blocked,
            vec![finding(FINDING_PROVIDER_READINESS_MISSING, format!("provider placement '{}' has no Phase 114D1 readiness result (not a required DIoT placement in current authority)", provider_placement.placement_id))],
        ),
        Some(result) if result.readiness == DMonitorReadinessV1::Ready => base(
            Some(provider_diot_id),
            Some(provider_placement.placement_id.clone()),
            Some(provider_placement.node_id.clone()),
            Some(result.clone()),
            DForwardDependencyStateV1::Satisfied,
            vec![finding(FINDING_SATISFIED, format!("provider placement '{}' is Ready under Phase 114D1 governance", provider_placement.placement_id))],
        ),
        Some(result) => base(
            Some(provider_diot_id),
            Some(provider_placement.placement_id.clone()),
            Some(provider_placement.node_id.clone()),
            Some(result.clone()),
            DForwardDependencyStateV1::Blocked,
            vec![finding(FINDING_PROVIDER_NOT_READY, format!("provider placement '{}' is NotReady under Phase 114D1 governance", provider_placement.placement_id))],
        ),
    }
}

/// Evaluates every `any_element` dependency of every DIoT placed on
/// `node_id`, for the CURRENT accepted `datum.dmap/2` authority (section
/// 5). `authority` must be the same `Option<&PlacementAuthorityView>` a
/// caller obtained from `ddeploy::active_placement_authority` -- this
/// function never re-derives it and never accepts a caller-supplied
/// DForward (the accepted `dforward_snapshot` embedded in
/// `PlacementAuthorityV2Extras` is the ONLY DForward consulted).
#[allow(clippy::too_many_arguments)]
pub fn evaluate_dforward_dependency_readiness(
    project_id: &str,
    application_id: &str,
    project_scope: &str,
    node_id: &str,
    authority: Option<&PlacementAuthorityView>,
    dmonitor_state: &DMonitorStateV1,
    runtime_binding_registry: &DIoTRuntimeBindingRegistryStateV1,
    ddeploy_state: &DDeployStateV1,
    dnode_registry_state: &DNodeRegistryStateV1,
    policy: &DMonitorReadinessPolicyV1,
    evaluated_at_utc: &str,
) -> DForwardDependencyReadinessV1 {
    let empty_result = |findings: Vec<DForwardDependencyFindingV1>| DForwardDependencyReadinessV1 {
        schema: DFORWARD_DEPENDENCY_READINESS_SCHEMA.to_string(),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        node_id: node_id.to_string(),
        evaluated_at_utc: evaluated_at_utc.to_string(),
        policy: policy.clone(),
        dmap_ref: None,
        dforward_ref: None,
        findings,
        dependencies: Vec::new(),
    };

    let Some(authority) = authority else {
        return empty_result(vec![finding(FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY, format!("project '{project_id}' application '{application_id}' has no active accepted placement authority"))]);
    };
    let Some(extras) = authority.v2.as_ref() else {
        return empty_result(vec![finding(FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY, format!("project '{project_id}' application '{application_id}' active authority is datum.dmap/1, which has no DForward/DIoT any_element semantics"))]);
    };
    let Some(forward) = extras.dforward_snapshot.as_ref() else {
        return empty_result(vec![finding(FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY, "active DMap/2 authority is missing its accepted DForward snapshot -- persisted-state corruption".to_string())]);
    };

    // Section 9: ONE D1 evaluation, ONE snapshot, ONE evaluated_at_utc,
    // reused for every dependency row below -- never recomputed per row.
    let readiness = evaluate_application_readiness(
        project_id,
        application_id,
        project_scope,
        Some(authority),
        dmonitor_state,
        runtime_binding_registry,
        ddeploy_state,
        dnode_registry_state,
        policy,
        evaluated_at_utc,
    );

    let mut dependencies = Vec::new();
    // Section 10/31: only requirements whose CONSUMER DIoT has a CURRENT
    // placement on `node_id`, resolved by exact placement identity, never
    // by name inference -- and only within THIS application's own accepted
    // DForward, never another application's/project's.
    for consumer_placement in extras.diot_placements.iter().filter(|placement| placement.node_id == node_id) {
        let Some(consumer_diot) = forward.diots.iter().find(|d| d.diot_id == consumer_placement.diot_id) else {
            continue; // Cannot happen against a genuinely accepted authority; defensive skip, never a fabricated row.
        };
        for requirement in &consumer_diot.requires {
            if requirement.scope != DForwardRequirementScope::AnyElement {
                continue; // Section 26: same_element is entirely unchanged/out of scope here.
            }
            dependencies.push(evaluate_one_dependency(
                &consumer_diot.diot_id,
                &consumer_placement.placement_id,
                node_id,
                &requirement.interface,
                forward,
                &extras.diot_placements,
                &readiness.diot.placements,
            ));
        }
    }

    // Section 33: stable, canonical-identity-derived ordering, independent
    // of any HashMap/HashSet iteration order used internally above.
    dependencies.sort_by(|a, b| (a.consumer_placement_id.as_str(), a.interface.as_str()).cmp(&(b.consumer_placement_id.as_str(), b.interface.as_str())));

    DForwardDependencyReadinessV1 {
        schema: DFORWARD_DEPENDENCY_READINESS_SCHEMA.to_string(),
        project_id: project_id.to_string(),
        application_id: application_id.to_string(),
        node_id: node_id.to_string(),
        evaluated_at_utc: evaluated_at_utc.to_string(),
        policy: policy.clone(),
        dmap_ref: Some(DMapRefSummaryV1 { dmap_id: authority.dmap_id.clone(), revision: authority.revision, digest: authority.digest.clone() }),
        dforward_ref: Some(DForwardRefSummaryV1 { dforward_id: extras.dforward_id.clone(), revision: extras.dforward_revision, digest: extras.dforward_digest.clone() }),
        findings: Vec::new(),
        dependencies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDContinuumV1, CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1,
        CanonicalNodePlatformV1, CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dforward::{
        CanonicalArtifactRequirementV1, CanonicalDForwardChainV1, CanonicalDForwardHopV1, CanonicalDForwardInterfaceV1,
        CanonicalDForwardRequirementV1, CanonicalDIoTV1, CanonicalExternalEndpointV1, CANONICAL_DFORWARD_SCHEMA,
    };
    use crate::core::canonical_dgraph::{CanonicalDGraphV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1, CanonicalDServV1, CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0};
    use crate::core::canonical_dmap_v2::{ContractRefV2, DIoTArtifactBindingV2, DServPlacementV2};
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, DMonitorRuntimeBindingRefV1, DMonitorSubjectV1,
        CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use crate::core::ddeploy::{active_placement_authority, ProposalSource};
    use crate::core::ddeploy_v2::DDeployProposalV2;
    use crate::core::diot_runtime_binding::register_runtime_binding;
    use crate::core::dmonitor_readiness::default_readiness_policy;
    use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};
    use crate::core::dnode_registry::declare_dnode;
    use crate::core::service_artifact_registry::{
        NativeProcessDeploymentSpecV1, ServiceArtifact, ServiceArtifactControlPlane, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
        SERVICE_ARTIFACT_SCHEMA_VERSION,
    };

    const PROJECT: &str = "proj:lab";
    const OTHER_PROJECT: &str = "proj:other";
    const APP: &str = "app:pipeline";
    const SVC_A: &str = "sensor";
    const SVC_B: &str = "decoder";
    const NODE_PROVIDER: &str = "fog-01";
    const NODE_CONSUMER: &str = "cloud-01";
    const CONSUMER_DIOT_ID: &str = "diot:consumer";
    const CONSUMER_PLACEMENT_ID: &str = "diotp:consumer";
    const PROVIDER_DIOT_ID: &str = "diot:provider";
    const PROVIDER_PLACEMENT_ID: &str = "diotp:provider";
    const INTERFACE: &str = "sensor-bus";
    const CONSUMER_ARTIFACT_ID: &str = "artifact:consumer";
    const PROVIDER_ARTIFACT_ID: &str = "dserv:pipeline:broker";

    const NOW: &str = "2026-09-25T00:10:00Z";
    const FRESH: &str = "2026-09-25T00:09:50Z";
    const STALE: &str = "2026-09-25T00:00:00Z";

    fn node(node_id: &str) -> CanonicalDNodeV1 {
        CanonicalDNodeV1 {
            node_id: node_id.to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "x86_64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: 4, memory_bytes: 4_000_000_000 },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
        }
    }

    fn minimal_artifact(artifact_id: &str, execution_generation: u64, execution_projection_digest: &str) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: artifact_id.to_string(),
            service_id: artifact_id.to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "native_process".to_string(),
                container: None,
                native_process: Some(NativeProcessDeploymentSpecV1 { process_name: "proc".to_string(), entrypoint: "bin/run".to_string(), args: Vec::new(), working_directory: None }),
            },
            probes: ServiceProbeSet { execution: ServiceProbeSpec { kind: "process_running".to_string(), params: std::collections::BTreeMap::new(), vantage_point: Some("local_host".to_string()) }, health: None, availability: None },
            metadata: None,
            control_plane: ServiceArtifactControlPlane { project_id: PROJECT.to_string(), execution_generation, execution_projection_digest: execution_projection_digest.to_string(), ..Default::default() },
        }
    }

    struct DependencyFixture {
        ddeploy: DDeployStateV1,
        dnode: DNodeRegistryStateV1,
    }

    /// Builds one REAL accepted DMap/2 (via the actual production
    /// `create_v2_proposal`/`accept_v2_proposal` path) with TWO DServ
    /// placements (unrelated to the DIoT dependency under test -- used to
    /// prove the critical "unrelated D-Serv NotReady never blocks a Ready
    /// provider" case) and TWO DIoT placements: a consumer on
    /// `NODE_CONSUMER` requiring `INTERFACE`/`AnyElement`, and a provider on
    /// `NODE_PROVIDER` providing it, connected by one canonical chain hop.
    /// `extra_hops`/`extra_diots`/`extra_placements` let individual tests
    /// extend the topology (a second provider, an external endpoint, etc.)
    /// without duplicating this whole fixture.
    fn build_fixture(extra_diots: Vec<CanonicalDIoTV1>, extra_hops: Vec<CanonicalDForwardHopV1>, extra_placements: Vec<DIoTPlacementV2>, external_endpoints: Vec<CanonicalExternalEndpointV1>) -> DependencyFixture {
        let dg = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: APP.to_string(),
            dgraph_id: "dgraph:d2-1".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 { service_id: SVC_A.to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 } },
                CanonicalDServV1 { service_id: SVC_B.to_string(), execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() }, resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: 1, memory_bytes: 1 } },
            ],
            calls: Vec::new(),
        };
        let dgraph_digest = crate::core::canonical_dgraph::recompute_dgraph_digest(&dg).unwrap();

        let dcontinuum = CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dcontinuum_id: "dcontinuum:d2-1".to_string(),
            revision: 1,
            nodes: vec![node(NODE_PROVIDER), node(NODE_CONSUMER)],
            links: Vec::new(),
        };
        let dcontinuum_digest = crate::core::canonical_dcontinuum::recompute_dcontinuum_digest(&dcontinuum).unwrap();

        let mut diots = vec![
            CanonicalDIoTV1 {
                diot_id: CONSUMER_DIOT_ID.to_string(),
                diot_kind: "consumer".to_string(),
                artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: CONSUMER_ARTIFACT_ID.to_string() },
                provides: Vec::new(),
                requires: vec![CanonicalDForwardRequirementV1 { interface: INTERFACE.to_string(), scope: DForwardRequirementScope::AnyElement }],
            },
            CanonicalDIoTV1 {
                diot_id: PROVIDER_DIOT_ID.to_string(),
                diot_kind: "broker".to_string(),
                artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: PROVIDER_ARTIFACT_ID.to_string() },
                provides: vec![CanonicalDForwardInterfaceV1 { interface: INTERFACE.to_string() }],
                requires: Vec::new(),
            },
        ];
        diots.extend(extra_diots);
        let diots_for_authority = diots.clone();

        let mut hops = vec![CanonicalDForwardHopV1 { from: CONSUMER_DIOT_ID.to_string(), to: PROVIDER_DIOT_ID.to_string(), interface: INTERFACE.to_string() }];
        hops.extend(extra_hops);

        let dforward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dforward_id: "dforward:d2-1".to_string(),
            revision: 1,
            diots,
            external_endpoints,
            chains: vec![CanonicalDForwardChainV1 { chain_id: "c1".to_string(), hops }],
            support_jobs: Vec::new(),
        };
        let dforward_digest = dforward.digest().unwrap();

        let mut diot_placements = vec![
            DIoTPlacementV2 {
                placement_id: CONSUMER_PLACEMENT_ID.to_string(),
                diot_id: CONSUMER_DIOT_ID.to_string(),
                node_id: NODE_CONSUMER.to_string(),
                artifact_binding: DIoTArtifactBindingV2 { artifact_id: CONSUMER_ARTIFACT_ID.to_string(), descriptor_digest: format!("sha256:{}", "e".repeat(64)), content_digest: None },
            },
            DIoTPlacementV2 {
                placement_id: PROVIDER_PLACEMENT_ID.to_string(),
                diot_id: PROVIDER_DIOT_ID.to_string(),
                node_id: NODE_PROVIDER.to_string(),
                artifact_binding: DIoTArtifactBindingV2 { artifact_id: PROVIDER_ARTIFACT_ID.to_string(), descriptor_digest: format!("sha256:{}", "b".repeat(64)), content_digest: None },
            },
        ];
        diot_placements.extend(extra_placements);

        let proposal = DDeployProposalV2 {
            schema: crate::core::ddeploy_v2::DDEPLOY_PROPOSAL_V2_SCHEMA.to_string(),
            proposal_id: "prop-d2-1".to_string(),
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            source: ProposalSource::Operator,
            created_at_utc: "2026-01-01T00:00:00Z".to_string(),
            dgraph_ref: ContractRefV2 { id: dg.dgraph_id.clone(), revision: dg.revision, digest: dgraph_digest },
            dcontinuum_ref: ContractRefV2 { id: dcontinuum.dcontinuum_id.clone(), revision: dcontinuum.revision, digest: dcontinuum_digest },
            dforward_ref: ContractRefV2 { id: dforward.dforward_id.clone(), revision: dforward.revision, digest: dforward_digest },
            placements: vec![
                DServPlacementV2 { service_id: SVC_A.to_string(), node_id: NODE_PROVIDER.to_string() },
                DServPlacementV2 { service_id: SVC_B.to_string(), node_id: NODE_PROVIDER.to_string() },
            ],
            diot_placements,
            dcall_realizations: Vec::new(),
            external_port_bindings: Vec::new(),
            expected_active_dmap_revision: None,
            diot_artifact_authority: {
                // Every DECLARED DIoT (not only those with a current
                // placement) needs a resolved binding entry -- Phase 113A's
                // own internal-consistency rule in `create_v2_proposal`.
                let mut m = std::collections::BTreeMap::new();
                for diot in &diots_for_authority {
                    m.entry(diot.artifact_requirement.artifact_id.clone()).or_insert_with(|| crate::core::ddeploy_v2::DIoTArtifactAuthorityBindingV2 {
                        artifact_id: diot.artifact_requirement.artifact_id.clone(),
                        execution_generation: 1,
                        execution_projection_digest: format!("sha256:{}", "c".repeat(64)),
                    });
                }
                m
            },
                dcode_artifact_authority: std::collections::BTreeMap::new(),
    };

        let mut seen_artifact_ids = std::collections::BTreeSet::new();
        let artifacts: Vec<ServiceArtifact> = diots_for_authority
            .iter()
            .filter(|diot| seen_artifact_ids.insert(diot.artifact_requirement.artifact_id.clone()))
            .map(|diot| minimal_artifact(&diot.artifact_requirement.artifact_id, 1, &format!("sha256:{}", "c".repeat(64))))
            .collect();

        let mut ddeploy = DDeployStateV1::empty();
        ddeploy.create_v2_proposal(proposal, dg, dcontinuum.clone(), dforward).unwrap();
        ddeploy.accept_v2_proposal("prop-d2-1", "acc-prop-d2-1".to_string(), "test".to_string(), "2026-01-01T00:00:01Z".to_string(), &dcontinuum, &artifacts, None).unwrap();

        let mut dnode = DNodeRegistryStateV1::empty();
        declare_dnode(&mut dnode, node(NODE_PROVIDER)).unwrap();
        declare_dnode(&mut dnode, node(NODE_CONSUMER)).unwrap();
        DependencyFixture { ddeploy, dnode }
    }

    fn policy() -> DMonitorReadinessPolicyV1 {
        default_readiness_policy()
    }

    fn empty_registry() -> DIoTRuntimeBindingRegistryStateV1 {
        DIoTRuntimeBindingRegistryStateV1::empty()
    }

    struct EvidenceInputs {
        fixture: DependencyFixture,
        runtime_registry: DIoTRuntimeBindingRegistryStateV1,
        authority: PlacementAuthorityView,
        dmap_ref: DMonitorContractRefV1,
        dforward_ref: DMonitorContractRefV1,
        runtime_binding_ref: DMonitorRuntimeBindingRefV1,
    }

    fn build_evidence_inputs(fixture: DependencyFixture) -> EvidenceInputs {
        let authority = active_placement_authority(&fixture.ddeploy, PROJECT, APP).unwrap().unwrap();
        let v2 = authority.v2.clone().unwrap();
        let mut runtime_registry = DIoTRuntimeBindingRegistryStateV1::empty();
        let binding = register_runtime_binding(&mut runtime_registry, &fixture.ddeploy, &fixture.dnode, PROJECT, APP, PROVIDER_PLACEMENT_ID, "mqtt://broker.lab:1883", "2026-01-01T00:00:02Z").unwrap();
        EvidenceInputs {
            dmap_ref: DMonitorContractRefV1 { id: authority.dmap_id.clone(), revision: authority.revision, digest: authority.digest.clone() },
            dforward_ref: DMonitorContractRefV1 { id: v2.dforward_id.clone(), revision: v2.dforward_revision, digest: v2.dforward_digest.clone() },
            runtime_binding_ref: DMonitorRuntimeBindingRefV1 { binding_id: binding.binding_id.clone(), binding_revision: binding.binding_revision, descriptor_digest: binding.descriptor_digest.clone() },
            fixture,
            runtime_registry,
            authority,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn diot_observation(placement_id: &str, node_id: &str, monitor_instance_id: &str, sequence: u64, dmap_ref: Option<DMonitorContractRefV1>, dforward_ref: Option<DMonitorContractRefV1>, runtime_binding_ref: Option<DMonitorRuntimeBindingRefV1>, health: DMonitorHealthV1, observed_at_utc: &str, observation_id: &str) -> CanonicalDMonitorObservationV1 {
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
                diot_id: PROVIDER_DIOT_ID.to_string(),
                diot_placement_id: placement_id.to_string(),
                node_id: node_id.to_string(),
                dmap_ref,
                dforward_ref,
                runtime_binding_ref,
            },
            health,
            facts: serde_json::json!({}),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dserv_observation(monitor_instance_id: &str, sequence: u64, service_id: &str, node_id: &str, dmap_ref: Option<DMonitorContractRefV1>, health: DMonitorHealthV1, observed_at_utc: &str, observation_id: &str) -> CanonicalDMonitorObservationV1 {
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
                node_id: node_id.to_string(),
                dmap_ref,
            },
            health,
            facts: serde_json::json!({}),
        }
    }

    fn ingest(state: &mut DMonitorStateV1, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, PROJECT, observation, received_at_utc).unwrap();
    }

    fn ingest_scope(state: &mut DMonitorStateV1, project_scope: &str, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, project_scope, observation, received_at_utc).unwrap();
    }

    /// Ingests exact-current Healthy `DServRealization` evidence for BOTH
    /// unrelated D-Serv placements, so application-level D1 readiness is
    /// Ready overall (used by tests that are NOT specifically exercising
    /// the "unrelated D-Serv blocks nothing" case).
    fn ingest_healthy_dserv_evidence(state: &mut DMonitorStateV1, dmap_ref: &DMonitorContractRefV1) {
        ingest(state, dserv_observation("mon-dserv", 1, SVC_A, NODE_PROVIDER, Some(dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-a"), FRESH);
        ingest(state, dserv_observation("mon-dserv", 2, SVC_B, NODE_PROVIDER, Some(dmap_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-dserv-b"), FRESH);
    }

    fn evaluate(inputs: &EvidenceInputs, state: &DMonitorStateV1, node_id: &str) -> DForwardDependencyReadinessV1 {
        evaluate_dforward_dependency_readiness(PROJECT, APP, PROJECT, node_id, Some(&inputs.authority), state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW)
    }

    // ---------------------------------------------------------------- BASIC

    #[test]
    fn ready_provider_satisfies_the_dependency() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies.len(), 1, "{result:#?}");
        let dep = &result.dependencies[0];
        assert_eq!(dep.consumer_diot_id, CONSUMER_DIOT_ID);
        assert_eq!(dep.consumer_placement_id, CONSUMER_PLACEMENT_ID);
        assert_eq!(dep.provider_diot_id.as_deref(), Some(PROVIDER_DIOT_ID));
        assert_eq!(dep.provider_placement_id.as_deref(), Some(PROVIDER_PLACEMENT_ID));
        assert_eq!(dep.state, DForwardDependencyStateV1::Satisfied, "{dep:#?}");
        assert!(dep.findings.iter().any(|f| f.code == FINDING_SATISFIED));
    }

    // ------------------------------------------------------ CRITICAL: AGGREGATION

    #[test]
    fn unrelated_dserv_notready_does_not_block_a_ready_provider() {
        // Section 22: load-bearing. Application-level D1 readiness is
        // NotReady (no D-Serv evidence at all), but the exact provider DIoT
        // placement is Ready -- the dependency must still be Satisfied.
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        // Deliberately NO D-Serv evidence ingested here.
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        // Confirm the premise: whole-application readiness really is NotReady.
        let app_readiness = evaluate_application_readiness(PROJECT, APP, PROJECT, Some(&inputs.authority), &state, &inputs.runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        assert_eq!(app_readiness.readiness, DMonitorReadinessV1::NotReady, "test premise violated: {app_readiness:#?}");

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Satisfied, "an unrelated NotReady D-Serv dimension must never block a Ready provider: {result:#?}");
    }

    // -------------------------------------------------------------- HEALTH

    #[test]
    fn provider_converged_but_unhealthy_is_blocked() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Unhealthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        let dep = &result.dependencies[0];
        assert_eq!(dep.state, DForwardDependencyStateV1::Blocked, "{dep:#?}");
        assert!(dep.findings.iter().any(|f| f.code == FINDING_PROVIDER_NOT_READY));
        assert_eq!(dep.provider_readiness.as_ref().unwrap().convergence, crate::core::dmonitor_interpretation::DMonitorConvergenceV1::Converged, "convergence must remain Converged even though the dependency is Blocked");
    }

    #[test]
    fn provider_degraded_is_blocked() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Degraded, FRESH, "obs-diot"), FRESH);
        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Blocked);
    }

    #[test]
    fn provider_unknown_health_is_blocked() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Unknown, FRESH, "obs-diot"), FRESH);
        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Blocked);
    }

    #[test]
    fn provider_stale_evidence_is_blocked() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, STALE, "obs-diot"), STALE);
        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Blocked);
    }

    #[test]
    fn provider_old_authority_is_blocked() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        let old_dmap_ref = DMonitorContractRefV1 { id: inputs.dmap_ref.id.clone(), revision: inputs.dmap_ref.revision + 1, digest: format!("sha256:{}", "f".repeat(64)) };
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(old_dmap_ref), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Blocked);
    }

    #[test]
    fn no_provider_evidence_at_all_is_blocked() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Blocked);
        assert!(result.dependencies[0].findings.iter().any(|f| f.code == FINDING_PROVIDER_NOT_READY));
    }

    // ------------------------------------------------------------ TOPOLOGY

    #[test]
    fn wrong_interface_never_resolves_a_provider() {
        let forward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dforward_id: "x".to_string(),
            revision: 1,
            diots: vec![
                CanonicalDIoTV1 { diot_id: CONSUMER_DIOT_ID.to_string(), diot_kind: "consumer".to_string(), artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "a".to_string() }, provides: vec![], requires: vec![CanonicalDForwardRequirementV1 { interface: INTERFACE.to_string(), scope: DForwardRequirementScope::AnyElement }] },
                CanonicalDIoTV1 { diot_id: PROVIDER_DIOT_ID.to_string(), diot_kind: "broker".to_string(), artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "b".to_string() }, provides: vec![CanonicalDForwardInterfaceV1 { interface: INTERFACE.to_string() }], requires: vec![] },
            ],
            external_endpoints: vec![],
            chains: vec![CanonicalDForwardChainV1 { chain_id: "c1".to_string(), hops: vec![CanonicalDForwardHopV1 { from: CONSUMER_DIOT_ID.to_string(), to: PROVIDER_DIOT_ID.to_string(), interface: INTERFACE.to_string() }] }],
            support_jobs: vec![],
        };
        let (providers, saw_external) = resolve_provider_candidates(&forward, CONSUMER_DIOT_ID, "not-the-real-interface");
        assert!(providers.is_empty());
        assert!(!saw_external);
    }

    #[test]
    fn requirement_with_no_matching_hop_is_provider_unresolved() {
        // A consumer diot with an any_element requirement for an interface
        // that has NO chain hop at all.
        let extra_diot = CanonicalDIoTV1 {
            diot_id: "diot:lonely-consumer".to_string(),
            diot_kind: "consumer".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:lonely".to_string() },
            provides: Vec::new(),
            requires: vec![CanonicalDForwardRequirementV1 { interface: "unreachable-interface".to_string(), scope: DForwardRequirementScope::AnyElement }],
        };
        let extra_placement = DIoTPlacementV2 {
            placement_id: "diotp:lonely-consumer".to_string(),
            diot_id: "diot:lonely-consumer".to_string(),
            node_id: NODE_CONSUMER.to_string(),
            artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:lonely".to_string(), descriptor_digest: format!("sha256:{}", "1".repeat(64)), content_digest: None },
        };
        let inputs = build_evidence_inputs(build_fixture(vec![extra_diot], vec![], vec![extra_placement], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        let lonely = result.dependencies.iter().find(|d| d.consumer_placement_id == "diotp:lonely-consumer").unwrap();
        assert_eq!(lonely.state, DForwardDependencyStateV1::Blocked);
        assert!(lonely.findings.iter().any(|f| f.code == FINDING_PROVIDER_UNRESOLVED), "{lonely:#?}");
    }

    #[test]
    fn external_endpoint_only_provider_is_blocked_provider_not_diot() {
        let external = CanonicalExternalEndpointV1 { endpoint_id: "ext:out".to_string(), direction: "output".to_string(), interface: "external-interface".to_string() };
        let extra_diot = CanonicalDIoTV1 {
            diot_id: "diot:ext-consumer".to_string(),
            diot_kind: "consumer".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:ext-consumer".to_string() },
            provides: Vec::new(),
            requires: vec![CanonicalDForwardRequirementV1 { interface: "external-interface".to_string(), scope: DForwardRequirementScope::AnyElement }],
        };
        let extra_hop = CanonicalDForwardHopV1 { from: "diot:ext-consumer".to_string(), to: "ext:out".to_string(), interface: "external-interface".to_string() };
        let extra_placement = DIoTPlacementV2 {
            placement_id: "diotp:ext-consumer".to_string(),
            diot_id: "diot:ext-consumer".to_string(),
            node_id: NODE_CONSUMER.to_string(),
            artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:ext-consumer".to_string(), descriptor_digest: format!("sha256:{}", "2".repeat(64)), content_digest: None },
        };
        let inputs = build_evidence_inputs(build_fixture(vec![extra_diot], vec![extra_hop], vec![extra_placement], vec![external]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        let ext = result.dependencies.iter().find(|d| d.consumer_placement_id == "diotp:ext-consumer").unwrap();
        assert_eq!(ext.state, DForwardDependencyStateV1::Blocked);
        assert!(ext.findings.iter().any(|f| f.code == FINDING_PROVIDER_NOT_DIOT), "{ext:#?}");
    }

    #[test]
    fn multiple_distinct_logical_providers_fail_closed_ambiguous() {
        // A SECOND DIoT also provides the SAME interface via a SECOND hop
        // from the same consumer -- two DISTINCT logical providers.
        let second_provider = CanonicalDIoTV1 {
            diot_id: "diot:second-provider".to_string(),
            diot_kind: "broker".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:second-provider".to_string() },
            provides: vec![CanonicalDForwardInterfaceV1 { interface: INTERFACE.to_string() }],
            requires: Vec::new(),
        };
        let extra_hop = CanonicalDForwardHopV1 { from: CONSUMER_DIOT_ID.to_string(), to: "diot:second-provider".to_string(), interface: INTERFACE.to_string() };
        let extra_placement = DIoTPlacementV2 {
            placement_id: "diotp:second-provider".to_string(),
            diot_id: "diot:second-provider".to_string(),
            node_id: NODE_PROVIDER.to_string(),
            artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:second-provider".to_string(), descriptor_digest: format!("sha256:{}", "3".repeat(64)), content_digest: None },
        };
        let inputs = build_evidence_inputs(build_fixture(vec![second_provider], vec![extra_hop], vec![extra_placement], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        let dep = &result.dependencies[0];
        assert_eq!(dep.state, DForwardDependencyStateV1::Blocked, "{dep:#?}");
        assert!(dep.findings.iter().any(|f| f.code == FINDING_PROVIDER_AMBIGUOUS), "{dep:#?}");
    }

    #[test]
    fn repeated_references_to_the_same_provider_are_deduplicated_not_ambiguous() {
        // TWO separate hops/chains both referencing the consumer<->provider
        // pair for the SAME interface -- this is the SAME logical provider,
        // never ambiguous.
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        // Manually verify via resolve_provider_candidates with a DForward
        // that has TWO chains referencing the same exact provider.
        let forward = CanonicalDForwardV1 {
            schema: CANONICAL_DFORWARD_SCHEMA.to_string(),
            project_id: PROJECT.to_string(),
            dforward_id: "x".to_string(),
            revision: 1,
            diots: vec![
                CanonicalDIoTV1 { diot_id: CONSUMER_DIOT_ID.to_string(), diot_kind: "consumer".to_string(), artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "a".to_string() }, provides: vec![], requires: vec![CanonicalDForwardRequirementV1 { interface: INTERFACE.to_string(), scope: DForwardRequirementScope::AnyElement }] },
                CanonicalDIoTV1 { diot_id: PROVIDER_DIOT_ID.to_string(), diot_kind: "broker".to_string(), artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "b".to_string() }, provides: vec![CanonicalDForwardInterfaceV1 { interface: INTERFACE.to_string() }], requires: vec![] },
            ],
            external_endpoints: vec![],
            chains: vec![
                CanonicalDForwardChainV1 { chain_id: "c1".to_string(), hops: vec![CanonicalDForwardHopV1 { from: CONSUMER_DIOT_ID.to_string(), to: PROVIDER_DIOT_ID.to_string(), interface: INTERFACE.to_string() }] },
                CanonicalDForwardChainV1 { chain_id: "c2".to_string(), hops: vec![CanonicalDForwardHopV1 { from: PROVIDER_DIOT_ID.to_string(), to: CONSUMER_DIOT_ID.to_string(), interface: INTERFACE.to_string() }] },
            ],
            support_jobs: vec![],
        };
        let (providers, saw_external) = resolve_provider_candidates(&forward, CONSUMER_DIOT_ID, INTERFACE);
        assert_eq!(providers.len(), 1, "repeated references to the SAME provider must deduplicate, not appear ambiguous: {providers:?}");
        assert!(!saw_external);
        let _ = inputs;
    }

    #[test]
    fn provider_diot_with_no_current_placement_is_blocked_placement_missing() {
        // Provider DIoT is declared and provides the interface, but has NO
        // DIoTPlacementV2 at all in the current authority.
        let forward_provider_unplaced = CanonicalDIoTV1 {
            diot_id: "diot:unplaced-provider".to_string(),
            diot_kind: "broker".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:unplaced".to_string() },
            provides: vec![CanonicalDForwardInterfaceV1 { interface: "unplaced-interface".to_string() }],
            requires: Vec::new(),
        };
        let consumer_for_unplaced = CanonicalDIoTV1 {
            diot_id: "diot:consumer-of-unplaced".to_string(),
            diot_kind: "consumer".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:consumer-of-unplaced".to_string() },
            provides: Vec::new(),
            requires: vec![CanonicalDForwardRequirementV1 { interface: "unplaced-interface".to_string(), scope: DForwardRequirementScope::AnyElement }],
        };
        let hop = CanonicalDForwardHopV1 { from: "diot:consumer-of-unplaced".to_string(), to: "diot:unplaced-provider".to_string(), interface: "unplaced-interface".to_string() };
        let consumer_placement = DIoTPlacementV2 {
            placement_id: "diotp:consumer-of-unplaced".to_string(),
            diot_id: "diot:consumer-of-unplaced".to_string(),
            node_id: NODE_CONSUMER.to_string(),
            artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:consumer-of-unplaced".to_string(), descriptor_digest: format!("sha256:{}", "4".repeat(64)), content_digest: None },
        };
        // Note: NO placement is added for "diot:unplaced-provider" itself.
        let inputs = build_evidence_inputs(build_fixture(vec![forward_provider_unplaced, consumer_for_unplaced], vec![hop], vec![consumer_placement], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        let dep = result.dependencies.iter().find(|d| d.consumer_placement_id == "diotp:consumer-of-unplaced").unwrap();
        assert_eq!(dep.state, DForwardDependencyStateV1::Blocked, "{dep:#?}");
        assert!(dep.findings.iter().any(|f| f.code == FINDING_PROVIDER_PLACEMENT_MISSING), "{dep:#?}");
    }

    #[test]
    fn multiple_placements_of_the_same_provider_diot_fail_closed_ambiguous() {
        // Section 15: `diot_id` uniqueness is NOT enforced by canonical
        // validation across placements (only `placement_id` is). Two
        // placements sharing the SAME provider `diot_id` must fail closed,
        // never pick one.
        let second_placement_same_diot = DIoTPlacementV2 {
            placement_id: "diotp:provider-second-placement".to_string(),
            diot_id: PROVIDER_DIOT_ID.to_string(),
            node_id: NODE_PROVIDER.to_string(),
            artifact_binding: DIoTArtifactBindingV2 { artifact_id: PROVIDER_ARTIFACT_ID.to_string(), descriptor_digest: format!("sha256:{}", "5".repeat(64)), content_digest: None },
        };
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![second_placement_same_diot], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        let dep = &result.dependencies[0];
        assert_eq!(dep.state, DForwardDependencyStateV1::Blocked, "{dep:#?}");
        assert!(dep.findings.iter().any(|f| f.code == FINDING_PROVIDER_PLACEMENT_AMBIGUOUS), "{dep:#?}");
    }

    #[test]
    fn provider_on_same_node_as_consumer_is_satisfied_when_ready() {
        // Section 16: AnyElement does not forbid a same-node provider.
        let same_node_provider = CanonicalDIoTV1 {
            diot_id: "diot:same-node-provider".to_string(),
            diot_kind: "broker".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:same-node-provider".to_string() },
            provides: vec![CanonicalDForwardInterfaceV1 { interface: "same-node-interface".to_string() }],
            requires: Vec::new(),
        };
        let same_node_consumer = CanonicalDIoTV1 {
            diot_id: "diot:same-node-consumer".to_string(),
            diot_kind: "consumer".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:same-node-consumer".to_string() },
            provides: Vec::new(),
            requires: vec![CanonicalDForwardRequirementV1 { interface: "same-node-interface".to_string(), scope: DForwardRequirementScope::AnyElement }],
        };
        let hop = CanonicalDForwardHopV1 { from: "diot:same-node-consumer".to_string(), to: "diot:same-node-provider".to_string(), interface: "same-node-interface".to_string() };
        let provider_placement = DIoTPlacementV2 { placement_id: "diotp:same-node-provider".to_string(), diot_id: "diot:same-node-provider".to_string(), node_id: NODE_CONSUMER.to_string(), artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:same-node-provider".to_string(), descriptor_digest: format!("sha256:{}", "6".repeat(64)), content_digest: None } };
        let consumer_placement = DIoTPlacementV2 { placement_id: "diotp:same-node-consumer".to_string(), diot_id: "diot:same-node-consumer".to_string(), node_id: NODE_CONSUMER.to_string(), artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:same-node-consumer".to_string(), descriptor_digest: format!("sha256:{}", "7".repeat(64)), content_digest: None } };
        let inputs = build_evidence_inputs(build_fixture(vec![same_node_provider, same_node_consumer], vec![hop], vec![provider_placement, consumer_placement], vec![]));

        // Register a SEPARATE runtime binding for the same-node provider too.
        let mut runtime_registry = inputs.runtime_registry;
        let binding = register_runtime_binding(&mut runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, PROJECT, APP, "diotp:same-node-provider", "mqtt://same-node-broker.lab:1883", "2026-01-01T00:00:03Z").unwrap();

        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let same_node_runtime_binding_ref = DMonitorRuntimeBindingRefV1 { binding_id: binding.binding_id.clone(), binding_revision: binding.binding_revision, descriptor_digest: binding.descriptor_digest.clone() };
        let mut same_node_observation = diot_observation("diotp:same-node-provider", NODE_CONSUMER, "mon-diot-same-node", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(same_node_runtime_binding_ref), DMonitorHealthV1::Healthy, FRESH, "obs-diot-same-node");
        if let DMonitorSubjectV1::DIoTRealization { diot_id, .. } = &mut same_node_observation.subject {
            *diot_id = "diot:same-node-provider".to_string();
        }
        ingest(&mut state, same_node_observation, FRESH);

        let result = evaluate_dforward_dependency_readiness(PROJECT, APP, PROJECT, NODE_CONSUMER, Some(&inputs.authority), &state, &runtime_registry, &inputs.fixture.ddeploy, &inputs.fixture.dnode, &policy(), NOW);
        let dep = result.dependencies.iter().find(|d| d.consumer_placement_id == "diotp:same-node-consumer").unwrap();
        assert_eq!(dep.state, DForwardDependencyStateV1::Satisfied, "a same-node provider must be able to satisfy AnyElement when Ready: {dep:#?}");
        assert_eq!(dep.provider_node_id.as_deref(), Some(NODE_CONSUMER));
    }

    // ------------------------------------------------------------- ISOLATION

    #[test]
    fn another_project_cannot_satisfy() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        let mut other_project_observation = diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot");
        other_project_observation.project_id = Some(OTHER_PROJECT.to_string());
        if let DMonitorSubjectV1::DIoTRealization { project_id, .. } = &mut other_project_observation.subject {
            *project_id = OTHER_PROJECT.to_string();
        }
        ingest_scope(&mut state, OTHER_PROJECT, other_project_observation, FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(result.dependencies[0].state, DForwardDependencyStateV1::Blocked, "evidence from another project must never satisfy this project's dependency");
    }

    // --------------------------------------------------------------- SCOPE

    #[test]
    fn same_element_never_enters_dynamic_any_element_gating() {
        let same_element_diot = CanonicalDIoTV1 {
            diot_id: "diot:same-element-consumer".to_string(),
            diot_kind: "consumer".to_string(),
            artifact_requirement: CanonicalArtifactRequirementV1 { artifact_id: "artifact:same-element-consumer".to_string() },
            provides: Vec::new(),
            requires: vec![CanonicalDForwardRequirementV1 { interface: INTERFACE.to_string(), scope: DForwardRequirementScope::SameElement }],
        };
        let placement = DIoTPlacementV2 { placement_id: "diotp:same-element-consumer".to_string(), diot_id: "diot:same-element-consumer".to_string(), node_id: NODE_CONSUMER.to_string(), artifact_binding: DIoTArtifactBindingV2 { artifact_id: "artifact:same-element-consumer".to_string(), descriptor_digest: format!("sha256:{}", "8".repeat(64)), content_digest: None } };
        let inputs = build_evidence_inputs(build_fixture(vec![same_element_diot], vec![], vec![placement], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);

        let result = evaluate(&inputs, &state, NODE_CONSUMER);
        assert!(result.dependencies.iter().all(|d| d.consumer_placement_id != "diotp:same-element-consumer"), "a same_element requirement must never appear in D2's dynamic any_element result: {result:#?}");
        assert_eq!(result.dependencies.len(), 1, "only the ORIGINAL any_element requirement should be present: {result:#?}");
    }

    // ------------------------------------------------------------- AUTHORITY

    #[test]
    fn no_active_authority_is_blocked_top_level() {
        let state = DMonitorStateV1::empty();
        let ddeploy = DDeployStateV1::empty();
        let dnode = DNodeRegistryStateV1::empty();
        let registry = empty_registry();
        let result = evaluate_dforward_dependency_readiness(PROJECT, APP, PROJECT, NODE_CONSUMER, None, &state, &registry, &ddeploy, &dnode, &policy(), NOW);
        assert!(result.dependencies.is_empty());
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY));
    }

    #[test]
    fn dmap1_authority_is_blocked_top_level_not_applicable() {
        let state = DMonitorStateV1::empty();
        let ddeploy = DDeployStateV1::empty();
        let dnode = DNodeRegistryStateV1::empty();
        let registry = empty_registry();
        let authority = PlacementAuthorityView {
            project_id: PROJECT.to_string(),
            application_id: APP.to_string(),
            dmap_id: "dmap:1".to_string(),
            revision: 1,
            digest: format!("sha256:{}", "a".repeat(64)),
            placements: vec![DServPlacementV2 { service_id: SVC_A.to_string(), node_id: NODE_PROVIDER.to_string() }],
            proposal_id: None,
            acceptance_id: None,
            dgraph_id: None,
            dgraph_revision: None,
            dgraph_digest: None,
            dgraph_snapshot: None,
            v2: None,
        };
        let result = evaluate_dforward_dependency_readiness(PROJECT, APP, PROJECT, NODE_CONSUMER, Some(&authority), &state, &registry, &ddeploy, &dnode, &policy(), NOW);
        assert!(result.dependencies.is_empty());
        assert!(result.findings.iter().any(|f| f.code == FINDING_NO_ACTIVE_DMAP_V2_AUTHORITY));
    }

    // ----------------------------------------------------------- DETERMINISM

    #[test]
    fn identical_inputs_produce_identical_results() {
        let inputs = build_evidence_inputs(build_fixture(vec![], vec![], vec![], vec![]));
        let mut state = DMonitorStateV1::empty();
        ingest_healthy_dserv_evidence(&mut state, &inputs.dmap_ref);
        ingest(&mut state, diot_observation(PROVIDER_PLACEMENT_ID, NODE_PROVIDER, "mon-diot", 1, Some(inputs.dmap_ref.clone()), Some(inputs.dforward_ref.clone()), Some(inputs.runtime_binding_ref.clone()), DMonitorHealthV1::Healthy, FRESH, "obs-diot"), FRESH);
        let a = evaluate(&inputs, &state, NODE_CONSUMER);
        let b = evaluate(&inputs, &state, NODE_CONSUMER);
        assert_eq!(a, b);
    }

    #[test]
    fn finding_inventory_has_no_duplicates_and_expected_count() {
        let unique: BTreeSet<&str> = ALL_DEPENDENCY_FINDING_CODES.iter().copied().collect();
        assert_eq!(unique.len(), ALL_DEPENDENCY_FINDING_CODES.len());
        assert_eq!(ALL_DEPENDENCY_FINDING_CODES.len(), 9);
    }
}
