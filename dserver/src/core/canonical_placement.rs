//! Canonical deterministic placement v1 (roadmap conceptual phase 120,
//! foundation closure follow-on). Given an already-VALID `datum.dgraph/1`
//! and an already-VALID, server-derived-authoritative `datum.dcontinuum/1`
//! (`core::dnode_registry::build_canonical_dcontinuum`), deterministically
//! produces a candidate `Vec<CanonicalDServPlacementV1>` suitable ONLY as
//! `CreateProposalInput.placements` for the UNCHANGED `core::ddeploy::
//! create_proposal` path (ADR-0018/ADR-0020).
//!
//! This module is advisory/proposal-only. It:
//! - NEVER calls `accept_proposal` (there is no import of that function, or
//!   of anything from `service_artifact_registry`, anywhere in this file --
//!   a structural, not merely documented, guarantee);
//! - NEVER mutates a host, a D-Node registry, or any persisted D-Deploy
//!   state (every function here takes only `&CanonicalDGraphV1`/
//!   `&CanonicalDContinuumV1` and returns a value; there is no `&mut`
//!   parameter, no database handle, no I/O of any kind);
//! - reads `ServiceArtifact.control_plane.target_nodes` ONLY THROUGH the
//!   ephemeral, caller-supplied `PlacementEligibilityView` (foundation
//!   closure v2, R2 fix) -- this module still never touches
//!   `ServiceArtifact` or the artifact registry itself, and still never
//!   duplicates or shadows `core::ddeploy::check_node_eligibility`'s real
//!   per-node semantics; it only avoids the FALSE-NEGATIVE planning failure
//!   of selecting a best-fit node that `create_proposal`'s unchanged
//!   `validate_full` would then reject as `node_not_eligible`, when a
//!   different, genuinely eligible node existed. `validate_full` remains
//!   the sole real enforcement of eligibility -- a plan this module accepts
//!   can still be rejected there if the eligibility view was stale or
//!   incomplete (see the R2 fix's TOCTOU discussion in
//!   `artifact-eligibility-audit.txt`); `target_nodes` remains an
//!   eligibility constraint, never canonical placement authority
//!   (ADR-0002);
//! - NEVER reads live telemetry -- `CanonicalDContinuumV1` structurally
//!   carries none (see `canonical_dcontinuum`'s own
//!   `no_telemetry_field_exists_on_the_type` proof), and this module adds
//!   no new field of its own that could carry any;
//! - uses NO randomness and NO learned/scored weights: every decision is a
//!   pure function of its four inputs' own declared content (the two
//!   canonical `CanonicalDGraphV1`/`CanonicalDContinuumV1` contracts, plus
//!   the `committed`/`eligibility` context described below). Independent
//!   review v3 (C1E) correction: an earlier version of this comment said
//!   "the two canonical inputs' own declared content", which stopped being
//!   accurate once foundation closure v2 (R2/R3) added `committed`/
//!   `eligibility` as genuine selection inputs -- see `PlacementProvenance`'s
//!   own doc comment and `core::ddeploy::PlannerProvenanceV1` for what is
//!   and is not persisted evidence of all four.
//!
//! Future GNN/RL/LLM placement work is EXPECTED to become an alternative
//! PROPOSAL PRODUCER competing with this module for the same
//! `CreateProposalInput.placements` slot -- never an alternative
//! governance/acceptance path (ADR-0020).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::core::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1, CanonicalValidationFinding};
use crate::core::canonical_dmap::CanonicalDServPlacementV1;

pub const PLANNER_VERSION: &str = "0.1.0";
pub const PLACEMENT_POLICY_NAME: &str = "deterministic-min-residual-first-fit";
pub const PLACEMENT_POLICY_VERSION: &str = "0.1.0";

/// Foundation closure v2 (R3A/R3B): structural (never live/observed) CPU +
/// memory already committed to a D-Node by applications OTHER than the one
/// currently being planned. A plain data value -- this module never derives
/// it itself (that requires `ActiveApplicationDeployment`/`ProposalRecord`,
/// which live in `core::ddeploy` and are deliberately NOT imported here to
/// keep this module's own dependency surface at zero DATUM-subsystem
/// types); `core::ddeploy::compute_project_node_commitments` builds it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCommitmentV1 {
    pub cpu_cores: u64,
    pub memory_bytes: u64,
}

/// Foundation closure v2 (R2 fix): ephemeral, per-service placement
/// eligibility derived from the CURRENT project-scoped `ServiceArtifact`
/// snapshot (`core::ddeploy::derive_placement_eligibility_view`) -- NEVER
/// canonical D-Graph/D-Continuum data, never persisted, never part of this
/// module's own provenance output. A service_id present in the view
/// restricts candidate nodes to EXACTLY its recorded set (mirroring
/// `check_node_eligibility`'s real per-node semantics, including that an
/// explicitly empty set means zero eligible nodes); a service_id ABSENT
/// from the view is unrestricted BY THIS VIEW -- see
/// `derive_placement_eligibility_view`'s own doc comment for why an absent
/// entry is the correct, non-laxening behavior for a planner that never has
/// the final say over real eligibility.
/// Independent review v3 (C1B): `Serialize`/`Deserialize`/`PartialEq`/`Eq`
/// added so this exact view can be persisted verbatim (never reconstructed
/// later) as part of `core::ddeploy::PlannerProvenanceV1`. A plain
/// `BTreeMap<String, BTreeSet<String>>` serializes with the distinction this
/// type's own semantics depend on preserved automatically: a service_id key
/// ABSENT from the map is genuinely absent from the serialized JSON object
/// (unrestricted-by-this-view), never conflated with a service_id PRESENT
/// with an empty array value (zero eligible nodes) -- see
/// `eligibility_snapshot_preserves_absent_vs_present_empty_distinction`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlacementEligibilityView {
    eligible_nodes_by_service: BTreeMap<String, BTreeSet<String>>,
}

impl PlacementEligibilityView {
    /// No `ServiceArtifact`-derived restriction on any service -- every
    /// service_id is treated as absent from the view. Used by every
    /// pre-existing test in this module (none of which cares about R2) and
    /// by any future caller planning purely against ABI/capacity.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, service_id: String, eligible_nodes: BTreeSet<String>) {
        self.eligible_nodes_by_service.insert(service_id, eligible_nodes);
    }

    pub fn eligible_nodes_for(&self, service_id: &str) -> Option<&BTreeSet<String>> {
        self.eligible_nodes_by_service.get(service_id)
    }
}

/// The D-Continuum declares zero D-Nodes at all, while the D-Graph declares
/// at least one D-Serv that must be placed somewhere. Distinct from an
/// EMPTY D-Graph (see `plan_canonical_placements`'s explicit empty-graph
/// behavior), which is never an error regardless of node count.
pub const F_PLANNER_NO_NODES: &str = "no_nodes";
/// A D-Serv's required `dnode_abi` (ADR-0003) is declared by ZERO D-Nodes in
/// the continuum -- distinct from `invalid_dnode_abi`/
/// `invalid_dnode_abi_capability` (an unsupported/malformed ABI STRING,
/// already caught by `dgraph.validate()`/`dcontinuum.validate()` before
/// this module's own selection logic ever runs).
pub const F_PLANNER_NO_ABI_COMPATIBLE_NODE: &str = "no_abi_compatible_node";
/// At least one ABI-compatible node exists, but none has enough REMAINING
/// (planner-local residual, after all earlier placements in this same plan)
/// static capacity for this D-Serv's declared `resource_requirement`.
pub const F_PLANNER_UNSATISFIED_STATIC_CAPACITY: &str = "unsatisfied_static_capacity";
/// Foundation closure v2 (R2 fix): at least one D-Node is ABI-compatible for
/// this D-Serv, but NONE of the ABI-compatible nodes is eligible per the
/// CURRENT `PlacementEligibilityView` -- distinct from
/// `F_PLANNER_NO_ABI_COMPATIBLE_NODE` (no ABI match at all, independent of
/// eligibility) and from `F_PLANNER_UNSATISFIED_STATIC_CAPACITY`
/// (ABI-compatible AND eligible nodes exist, but none has enough residual
/// capacity).
pub const F_PLANNER_NO_ELIGIBLE_NODE: &str = "no_eligible_node";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

/// One placement decision, in the exact deterministic order the planner
/// made it (services processed in ascending `service_id` order) --
/// sufficient to reconstruct WHY a given node was chosen over every other
/// eligible one at that point in planning.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlacementDecisionRecord {
    pub service_id: String,
    pub node_id: String,
    pub required_dnode_abi: String,
    /// How many D-Nodes were ABI-compatible AND had sufficient planner-local
    /// residual capacity at the moment this decision was made (always >= 1
    /// for a successful decision; ties among these are broken by minimum
    /// residual-after, then lexicographic `node_id`).
    pub eligible_node_count: usize,
    pub node_residual_cpu_cores_before: u64,
    pub node_residual_memory_bytes_before: u64,
    pub node_residual_cpu_cores_after: u64,
    pub node_residual_memory_bytes_after: u64,
}

/// Evidence for a `CanonicalPlacementPlanV1`: planner/policy identity, the
/// exact D-Graph/D-Continuum id/revision/digest planned against, and every
/// individual placement decision in order. The `decisions` list alone
/// already gives the exact `placements` back out (each record IS one
/// `service_id -> node_id` decision) -- no re-running of the policy is
/// needed to read off what was decided.
///
/// Independent review v3 (C1E) correction: an earlier version of this
/// comment additionally claimed this evidence reconstructs the placements
/// "from its two canonical inputs alone" -- that phrasing wrongly implied
/// the two canonical `dgraph`/`dcontinuum` digests recorded here are
/// sufficient, together with the policy, to REPRODUCE (not merely read off)
/// the same decisions again. Since foundation closure v2 (R2/R3), the
/// policy also genuinely depends on `committed`/`eligibility`, neither of
/// which this ephemeral, HTTP-response-only struct retains. Reproducing a
/// specific past decision therefore requires the same `committed`/
/// `eligibility` context that run received -- see
/// `core::ddeploy::PlannerProvenanceV1`, the PERSISTED superset of this
/// struct's content that DOES retain both (independent review v3, C1A/C1B).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlacementProvenance {
    pub planner_version: String,
    pub policy_name: String,
    pub policy_version: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub dgraph_digest: String,
    pub dcontinuum_id: String,
    pub dcontinuum_revision: u64,
    pub dcontinuum_digest: String,
    pub decisions: Vec<PlacementDecisionRecord>,
}

/// Candidate placement output. NOT a canonical wire contract (no schema
/// string, never persisted, never itself digested) -- purely an ephemeral,
/// in-process handoff value from this module to
/// `CreateProposalInput.placements`, exactly like `core::ddeploy`'s own
/// `ProposalValidationOutcome` is an ephemeral handoff, never an
/// authoritative object in its own right (see this module's own doc
/// comment: the canonical authority remains the accepted
/// `CanonicalDMapV1`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CanonicalPlacementPlanV1 {
    pub placements: Vec<CanonicalDServPlacementV1>,
    pub provenance: PlacementProvenance,
}

struct NodeState {
    node_id: String,
    abi_versions: Vec<String>,
    residual_cpu_cores: u64,
    residual_memory_bytes: u64,
}

/// Deterministically plan a candidate placement for every D-Serv in
/// `dgraph` onto a D-Node in `dcontinuum`.
///
/// PURE: no I/O, no randomness, no mutation of either input, no call into
/// any other DATUM subsystem. Callers are responsible for ensuring
/// `dcontinuum` is the server-owned AUTHORITATIVE structural continuum
/// (`dnode_registry::build_canonical_dcontinuum`) rather than an unverified
/// caller claim -- this module has no way to check that itself, and does
/// not try to; it only ever plans against whatever `CanonicalDContinuumV1`
/// it is given.
///
/// Policy (`PLACEMENT_POLICY_NAME`): D-Servs are processed in ascending
/// `service_id` order (ADR-0017: `services` is a canonically unordered set
/// keyed by `service_id`, so this is the one order with no dependency on
/// caller-supplied array position). For each, every D-Node is filtered to
/// those (a) declaring the D-Serv's required `dnode_abi` capability and (b)
/// having enough planner-local RESIDUAL static capacity (starting from the
/// node's declared `capacity`, decremented by every earlier decision in
/// this same plan -- never a live/observed value) for both `cpu_cores` and
/// `memory_bytes`. Among the eligible set, the node minimizing residual
/// `cpu_cores` AFTER this placement is chosen; ties broken by minimum
/// residual `memory_bytes` after; remaining ties broken by the
/// lexicographically smallest `node_id`. This is a deterministic
/// best-fit-by-minimum-slack bin-packing heuristic -- explicitly NOT
/// latency/energy/locality/QoS-aware (deferred to a later, explicitly
/// heuristic policy version per ADR-0020), and explicitly NOT learned
/// (no GNN/RL/LLM weight of any kind).
///
/// Fails closed (returns `Err`, produces NO partial `placements`) if: the
/// D-Graph or D-Continuum is not itself independently valid; a required
/// D-Serv has no ABI-compatible D-Node at all; every ABI-compatible D-Node
/// is excluded by `eligibility` (R2 fix); or every ABI-compatible AND
/// eligible D-Node's residual capacity (after subtracting BOTH `committed`
/// and every earlier decision in this same plan) is insufficient at the
/// point that D-Serv is planned. A `dgraph` with zero services is
/// explicitly NOT an error -- it deterministically produces an empty plan,
/// regardless of how many (or how few) nodes `dcontinuum` declares, since
/// there is nothing to place.
///
/// `committed` (R3A/R3B fix): per-node structural CPU/memory already
/// committed by OTHER active applications in the same project
/// (`core::ddeploy::compute_project_node_commitments`, excluding the
/// application currently being (re)planned) -- subtracted from each node's
/// declared `capacity` BEFORE this plan's own residual accounting starts.
/// Pass an empty map for a project with no other active applications, or
/// when project-wide commitment awareness is not applicable.
///
/// `eligibility` (R2 fix): ephemeral `ServiceArtifact`-derived candidate-node
/// restriction (`core::ddeploy::derive_placement_eligibility_view`). Pass
/// `PlacementEligibilityView::unrestricted()` to plan purely on ABI/capacity
/// (e.g. when no `ServiceArtifact` exists yet for any service).
pub fn plan_canonical_placements(
    dgraph: &CanonicalDGraphV1,
    dcontinuum: &CanonicalDContinuumV1,
    committed: &BTreeMap<String, NodeCommitmentV1>,
    eligibility: &PlacementEligibilityView,
) -> Result<CanonicalPlacementPlanV1, Vec<CanonicalValidationFinding>> {
    let mut input_findings = dgraph.validate();
    input_findings.extend(dcontinuum.validate());
    if !input_findings.is_empty() {
        return Err(input_findings);
    }

    let dgraph_digest = recompute_dgraph_digest(dgraph).map_err(|error| vec![finding("internal_error", error)])?;
    let dcontinuum_digest = recompute_dcontinuum_digest(dcontinuum).map_err(|error| vec![finding("internal_error", error)])?;

    let mut provenance = PlacementProvenance {
        planner_version: PLANNER_VERSION.to_string(),
        policy_name: PLACEMENT_POLICY_NAME.to_string(),
        policy_version: PLACEMENT_POLICY_VERSION.to_string(),
        dgraph_id: dgraph.dgraph_id.clone(),
        dgraph_revision: dgraph.revision,
        dgraph_digest,
        dcontinuum_id: dcontinuum.dcontinuum_id.clone(),
        dcontinuum_revision: dcontinuum.revision,
        dcontinuum_digest,
        decisions: Vec::new(),
    };

    if dgraph.services.is_empty() {
        // Explicit, defined behavior (never an error): nothing requires a
        // node, so no node requirement -- independent of `dcontinuum.nodes`
        // being empty, non-empty, over-provisioned, or under-provisioned.
        return Ok(CanonicalPlacementPlanV1 { placements: Vec::new(), provenance });
    }

    if dcontinuum.nodes.is_empty() {
        return Err(vec![finding(
            F_PLANNER_NO_NODES,
            "datum.dcontinuum/1 declares zero D-Nodes; cannot place any D-Serv",
        )]);
    }

    // Deterministic node ordering, independent of `dcontinuum.nodes`'
    // caller-supplied array order (ADR-0017: nodes are a canonically
    // unordered set keyed by node_id).
    let mut nodes: Vec<NodeState> = dcontinuum
        .nodes
        .iter()
        .map(|node| {
            let commitment = committed.get(&node.node_id).copied().unwrap_or_default();
            NodeState {
                node_id: node.node_id.clone(),
                abi_versions: node.capabilities.dnode_abi_versions.clone(),
                // R3B: OTHER active applications' structural commitment is
                // subtracted BEFORE this plan's own residual accounting
                // starts. `saturating_sub` (never a panic/underflow): if
                // `committed` already meets or exceeds declared capacity,
                // residual is exactly zero, which correctly yields
                // `F_PLANNER_UNSATISFIED_STATIC_CAPACITY` for any service
                // requiring more than zero of that dimension on this node --
                // it is not this function's job to flag the over-commitment
                // itself (that is `core::ddeploy::accept_proposal`'s R3C
                // gate's job, at acceptance time).
                residual_cpu_cores: (node.capacity.cpu_cores as u64).saturating_sub(commitment.cpu_cores),
                residual_memory_bytes: node.capacity.memory_bytes.saturating_sub(commitment.memory_bytes),
            }
        })
        .collect();
    nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));

    // Deterministic service ordering, independent of `dgraph.services`'
    // caller-supplied array order, for the identical reason.
    let mut services = dgraph.services.clone();
    services.sort_by(|a, b| a.service_id.cmp(&b.service_id));

    let mut placements = Vec::with_capacity(services.len());

    for service in &services {
        let required_abi = &service.execution_requirement.dnode_abi;
        let required_cpu = service.resource_requirement.cpu_cores as u64;
        let required_memory = service.resource_requirement.memory_bytes;

        let abi_capable_exists = nodes.iter().any(|node| node.abi_versions.iter().any(|abi| abi == required_abi));
        if !abi_capable_exists {
            return Err(vec![finding(
                F_PLANNER_NO_ABI_COMPATIBLE_NODE,
                format!("service '{}' requires dnode_abi '{required_abi}', but no declared D-Node supports it", service.service_id),
            )]);
        }

        // R2 fix: `None` means no `ServiceArtifact`-derived restriction is
        // currently determinable for this service (unrestricted BY THIS
        // VIEW -- see `PlacementEligibilityView`'s own doc comment); `Some`
        // restricts candidates to EXACTLY that recorded node set, including
        // an empty set meaning zero eligible nodes.
        let artifact_eligible_nodes = eligibility.eligible_nodes_for(&service.service_id);
        let abi_and_eligible_exists = nodes.iter().any(|node| {
            node.abi_versions.iter().any(|abi| abi == required_abi)
                && artifact_eligible_nodes.is_none_or(|set| set.contains(&node.node_id))
        });
        if !abi_and_eligible_exists {
            return Err(vec![finding(
                F_PLANNER_NO_ELIGIBLE_NODE,
                format!(
                    "service '{}' has at least one dnode_abi '{required_abi}'-compatible D-Node, but none of them is eligible per the current ServiceArtifact target_nodes view",
                    service.service_id
                ),
            )]);
        }

        // Candidates: ABI-compatible AND artifact-eligible AND enough
        // planner-local RESIDUAL capacity for both dimensions. `nodes` is
        // already sorted by `node_id`, so iterating in this order and
        // keeping the first strictly-better candidate makes the
        // lexicographic tie-break fall out naturally; the explicit
        // `node_id` comparison below still runs so the property holds
        // regardless of iteration order, not merely as an accident of
        // `Vec` order.
        let mut best_index: Option<usize> = None;
        let mut eligible_node_count = 0usize;
        for (index, node) in nodes.iter().enumerate() {
            if !node.abi_versions.iter().any(|abi| abi == required_abi) {
                continue;
            }
            if artifact_eligible_nodes.is_some_and(|set| !set.contains(&node.node_id)) {
                continue;
            }
            if node.residual_cpu_cores < required_cpu || node.residual_memory_bytes < required_memory {
                continue;
            }
            eligible_node_count += 1;
            best_index = Some(match best_index {
                None => index,
                Some(current_best) => {
                    let candidate_cpu_after = node.residual_cpu_cores - required_cpu;
                    let candidate_memory_after = node.residual_memory_bytes - required_memory;
                    let current = &nodes[current_best];
                    let current_cpu_after = current.residual_cpu_cores - required_cpu;
                    let current_memory_after = current.residual_memory_bytes - required_memory;
                    let candidate_is_better = (candidate_cpu_after, candidate_memory_after, &node.node_id)
                        < (current_cpu_after, current_memory_after, &current.node_id);
                    if candidate_is_better {
                        index
                    } else {
                        current_best
                    }
                }
            });
        }

        let Some(chosen_index) = best_index else {
            return Err(vec![finding(
                F_PLANNER_UNSATISFIED_STATIC_CAPACITY,
                format!(
                    "service '{}' requires {{cpu_cores: {required_cpu}, memory_bytes: {required_memory}}}, but no ABI-compatible D-Node has enough residual static capacity at this point in planning",
                    service.service_id
                ),
            )]);
        };

        let node = &mut nodes[chosen_index];
        let residual_cpu_before = node.residual_cpu_cores;
        let residual_memory_before = node.residual_memory_bytes;
        // Never underflows: `chosen_index` was selected only from
        // candidates already proven to have sufficient residual capacity.
        node.residual_cpu_cores -= required_cpu;
        node.residual_memory_bytes -= required_memory;

        provenance.decisions.push(PlacementDecisionRecord {
            service_id: service.service_id.clone(),
            node_id: node.node_id.clone(),
            required_dnode_abi: required_abi.clone(),
            eligible_node_count,
            node_residual_cpu_cores_before: residual_cpu_before,
            node_residual_memory_bytes_before: residual_memory_before,
            node_residual_cpu_cores_after: node.residual_cpu_cores,
            node_residual_memory_bytes_after: node.residual_memory_bytes,
        });
        placements.push(CanonicalDServPlacementV1 { service_id: service.service_id.clone(), node_id: node.node_id.clone() });
    }

    placements.sort_by(|a, b| a.service_id.cmp(&b.service_id));
    Ok(CanonicalPlacementPlanV1 { placements, provenance })
}

/// Convenience index, `service_id -> node_id`, occasionally useful to
/// callers/tests that want O(1) lookup rather than a linear scan of
/// `placements`. Never used by `plan_canonical_placements` itself.
pub fn placements_by_service(plan: &CanonicalPlacementPlanV1) -> BTreeMap<String, String> {
    plan.placements.iter().map(|placement| (placement.service_id.clone(), placement.node_id.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{
        CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1,
        CANONICAL_DCONTINUUM_SCHEMA,
    };
    use crate::core::canonical_dgraph::{
        CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1, CanonicalDServV1,
        CANONICAL_DGRAPH_SCHEMA, DNODE_ABI_V0,
    };

    fn dserv(service_id: &str, cpu_cores: u32, memory_bytes: u64) -> CanonicalDServV1 {
        CanonicalDServV1 {
            service_id: service_id.to_string(),
            execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
            resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores, memory_bytes },
        }
    }

    fn dnode(node_id: &str, cpu_cores: u32, memory_bytes: u64, abi: Vec<&str>) -> CanonicalDNodeV1 {
        CanonicalDNodeV1 {
            node_id: node_id.to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores, memory_bytes },
            capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: abi.into_iter().map(str::to_string).collect() },
        }
    }

    fn dgraph(services: Vec<CanonicalDServV1>) -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:placement-lab".to_string(),
            dgraph_id: "dgraph:placement-lab".to_string(),
            revision: 1,
            services,
            calls: vec![],
        }
    }

    fn dcontinuum(nodes: Vec<CanonicalDNodeV1>) -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: "monitor-f49c8".to_string(),
            dcontinuum_id: "continuum:placement-lab".to_string(),
            revision: 1,
            nodes,
            links: vec![],
        }
    }

    // ---- 1: one service / one eligible node ------------------------------

    #[test]
    fn one_service_one_eligible_node() {
        let plan = plan_canonical_placements(
            &dgraph(vec![dserv("svc-a", 1, 128)]),
            &dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]),
        &BTreeMap::new(), &PlacementEligibilityView::unrestricted())
        .unwrap();
        assert_eq!(plan.placements, vec![CanonicalDServPlacementV1 { service_id: "svc-a".to_string(), node_id: "fog-01".to_string() }]);
        assert_eq!(plan.provenance.decisions.len(), 1);
        assert_eq!(plan.provenance.decisions[0].node_residual_cpu_cores_after, 3);
        assert_eq!(plan.provenance.decisions[0].node_residual_memory_bytes_after, 3968);
    }

    // ---- 2: one service / multiple nodes deterministic tie-break ---------

    #[test]
    fn identical_capacity_nodes_tie_break_lexicographically() {
        let plan = plan_canonical_placements(
            &dgraph(vec![dserv("svc-a", 1, 128)]),
            &dcontinuum(vec![
                dnode("node-b", 4, 4096, vec![DNODE_ABI_V0]),
                dnode("node-a", 4, 4096, vec![DNODE_ABI_V0]),
            ]),
        &BTreeMap::new(), &PlacementEligibilityView::unrestricted())
        .unwrap();
        assert_eq!(plan.placements[0].node_id, "node-a", "the lexicographically smaller node_id must win an exact tie");
    }

    #[test]
    fn min_residual_after_wins_over_lexicographically_smaller_but_worse_fit_node() {
        // node-a has more absolute capacity but node-b is a tighter fit
        // (smaller residual after placement) -- min-residual-after must
        // win over the lexicographic tie-break, which only applies to a
        // genuine tie.
        let plan = plan_canonical_placements(
            &dgraph(vec![dserv("svc-a", 2, 1024)]),
            &dcontinuum(vec![
                dnode("node-a", 8, 8192, vec![DNODE_ABI_V0]),
                dnode("node-b", 2, 1024, vec![DNODE_ABI_V0]),
            ]),
        &BTreeMap::new(), &PlacementEligibilityView::unrestricted())
        .unwrap();
        assert_eq!(plan.placements[0].node_id, "node-b");
    }

    // ---- 3: multiple services deterministic placement --------------------

    #[test]
    fn multiple_services_deterministic_placement() {
        let plan = plan_canonical_placements(
            &dgraph(vec![dserv("svc-a", 1, 128), dserv("svc-b", 1, 128), dserv("svc-c", 6, 6144)]),
            &dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]),
        &BTreeMap::new(), &PlacementEligibilityView::unrestricted())
        .unwrap();
        let by_service = placements_by_service(&plan);
        // svc-c (6 cpu) only fits cloud-01 (8 cpu); svc-a/svc-b (1 cpu each)
        // both fit fog-01 with a smaller residual-after than cloud-01 once
        // cloud-01 is otherwise untouched -- fog-01 (4 cpu) vs cloud-01
        // (8 cpu) each minus 1 => fog-01 residual 3 < cloud-01 residual 7,
        // so fog-01 wins for both.
        assert_eq!(by_service.get("svc-a").map(String::as_str), Some("fog-01"));
        assert_eq!(by_service.get("svc-b").map(String::as_str), Some("fog-01"));
        assert_eq!(by_service.get("svc-c").map(String::as_str), Some("cloud-01"));
    }

    // ---- 4/5/18: determinism under reordering + identical digests --------

    #[test]
    fn reversed_service_order_produces_identical_output() {
        let g = dgraph(vec![dserv("svc-a", 1, 128), dserv("svc-b", 2, 256), dserv("svc-c", 1, 512)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let forward = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();

        let mut reversed_g = g.clone();
        reversed_g.services.reverse();
        let reversed = plan_canonical_placements(&reversed_g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();

        assert_eq!(forward.placements, reversed.placements);
        assert_eq!(forward.provenance.decisions, reversed.provenance.decisions);
    }

    #[test]
    fn reversed_node_order_produces_identical_output() {
        let g = dgraph(vec![dserv("svc-a", 1, 128), dserv("svc-b", 2, 256)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let forward = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();

        let mut reversed_c = c.clone();
        reversed_c.nodes.reverse();
        let reversed = plan_canonical_placements(&g, &reversed_c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();

        assert_eq!(forward.placements, reversed.placements);
        assert_eq!(forward.provenance.decisions, reversed.provenance.decisions);
    }

    #[test]
    fn same_canonical_inputs_produce_the_same_plan_every_time() {
        let g = dgraph(vec![dserv("svc-a", 1, 128), dserv("svc-b", 2, 256)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let a = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();
        let b = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();
        assert_eq!(a, b);
    }

    // ---- 6: ABI-compatible-node-absent rejected (valid ABI, no match) ----

    #[test]
    fn no_abi_compatible_node_rejected() {
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![])]); // no ABI declared
        let error = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_PLANNER_NO_ABI_COMPATIBLE_NODE), "findings: {error:?}");
    }

    // ---- 7: malformed/unsupported ABI string rejected by input validation

    #[test]
    fn unsupported_dnode_abi_string_rejected_by_input_validation() {
        let mut g = dgraph(vec![dserv("svc-a", 1, 128)]);
        g.services[0].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]);
        let error = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap_err();
        assert!(error.iter().any(|f| f.code == "invalid_dnode_abi"), "findings: {error:?}");
    }

    // ---- 8: empty continuum (with services to place) rejected -----------

    #[test]
    fn empty_continuum_with_services_to_place_rejected() {
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![]);
        let error = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_PLANNER_NO_NODES), "findings: {error:?}");
    }

    // ---- 9: empty graph explicit behavior ---------------------------------

    #[test]
    fn empty_graph_produces_an_empty_plan_even_with_nonempty_continuum() {
        let plan = plan_canonical_placements(&dgraph(vec![]), &dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]), &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();
        assert_eq!(plan.placements, Vec::new());
        assert_eq!(plan.provenance.decisions, Vec::new());
    }

    #[test]
    fn empty_graph_produces_an_empty_plan_even_with_empty_continuum() {
        let plan = plan_canonical_placements(&dgraph(vec![]), &dcontinuum(vec![]), &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();
        assert_eq!(plan.placements, Vec::new());
    }

    // ---- 10/11: capacity exhaustion + no oversubscription -----------------

    #[test]
    fn capacity_exhaustion_rejected() {
        let g = dgraph(vec![dserv("svc-a", 3, 1024), dserv("svc-b", 3, 1024)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]); // 4 cores total; 3+3=6 > 4
        let error = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_PLANNER_UNSATISFIED_STATIC_CAPACITY), "findings: {error:?}");
    }

    #[test]
    fn successful_plan_never_oversubscribes_a_node_per_canonical_dmap_validation() {
        use crate::core::canonical_dmap::{CanonicalContractReferenceV1, CanonicalDMapV1, CANONICAL_DMAP_SCHEMA};
        use crate::core::canonical_dgraph::recompute_dgraph_digest as digest_dgraph;
        use crate::core::canonical_dcontinuum::recompute_dcontinuum_digest as digest_dcontinuum;

        let g = dgraph(vec![dserv("svc-a", 2, 1024), dserv("svc-b", 2, 1024), dserv("svc-c", 2, 1024)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let plan = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();

        // Cross-validate the plan's placements against the EXISTING,
        // independently-reviewed `CanonicalDMapV1::validate_against`
        // aggregate feasibility check, rather than re-implementing the
        // same arithmetic a second time in this test.
        let dmap = CanonicalDMapV1 {
            schema: CANONICAL_DMAP_SCHEMA.to_string(),
            project_id: c.project_id.clone(),
            application_id: g.application_id.clone(),
            dmap_id: "dmap:placement-lab:plan-check".to_string(),
            revision: 1,
            dgraph_ref: CanonicalContractReferenceV1 { id: g.dgraph_id.clone(), revision: g.revision, digest: digest_dgraph(&g).unwrap() },
            dcontinuum_ref: CanonicalContractReferenceV1 { id: c.dcontinuum_id.clone(), revision: c.revision, digest: digest_dcontinuum(&c).unwrap() },
            placements: plan.placements.clone(),
        };
        let findings = dmap.validate_against(&g, &c);
        assert!(!findings.iter().any(|f| f.code.contains("cpu") || f.code.contains("memory")), "findings: {findings:?}");
    }

    // ---- 19: changed D-Continuum digest changes provenance even when -----
    // ---- the resulting placement is identical -----------------------------

    #[test]
    fn irrelevant_dcontinuum_field_change_still_changes_provenance_digest() {
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]);
        let mut c2 = c.clone();
        c2.nodes[0].platform.os = "windows".to_string(); // irrelevant to placement selection, but part of digest

        let plan_a = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();
        let plan_b = plan_canonical_placements(&g, &c2, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();

        assert_eq!(plan_a.placements, plan_b.placements, "the placement decision itself is unaffected by an unrelated platform field");
        assert_ne!(plan_a.provenance.dcontinuum_digest, plan_b.provenance.dcontinuum_digest, "but provenance must reflect the actual continuum planned against");
    }

    // ---- structural leakage guards ----------------------------------------

    #[test]
    fn plan_output_never_contains_a_capacity_or_platform_string_leak() {
        // A cheap structural proof that provenance carries only identity/
        // digest/decision evidence, never a raw copy of node platform or
        // capacity content beyond what a decision record already reports.
        let plan = plan_canonical_placements(
            &dgraph(vec![dserv("svc-a", 1, 128)]),
            &dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]),
        &BTreeMap::new(), &PlacementEligibilityView::unrestricted())
        .unwrap();
        let value = serde_json::to_value(&plan).unwrap();
        let text = value.to_string();
        for forbidden in ["cpu_usage", "usage_percent", "available", "current_", "rss", "load_average", "temperature", "rtt", "packet_loss", "platform"] {
            assert!(!text.contains(forbidden), "forbidden key/field-shaped text '{forbidden}' found in plan JSON: {text}");
        }
    }

    // ---- foundation closure v2, R2: artifact eligibility ------------------

    #[test]
    fn eligibility_view_skips_a_tighter_fit_ineligible_node_for_a_larger_eligible_one() {
        // fog-01 is the min-residual-after best fit by capacity ALONE, but
        // only cloud-01 is artifact-eligible -- the planner must select
        // cloud-01, not fail closed with a false-negative
        // `unsatisfied_static_capacity`/wrong-node result.
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let mut eligibility = PlacementEligibilityView::unrestricted();
        eligibility.insert("svc-a".to_string(), std::collections::BTreeSet::from(["cloud-01".to_string()]));
        let plan = plan_canonical_placements(&g, &c, &BTreeMap::new(), &eligibility).unwrap();
        assert_eq!(plan.placements[0].node_id, "cloud-01");
    }

    #[test]
    fn all_abi_and_capacity_compatible_nodes_ineligible_fails_closed() {
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let mut eligibility = PlacementEligibilityView::unrestricted();
        eligibility.insert("svc-a".to_string(), std::collections::BTreeSet::from(["edge-99".to_string()])); // exists in neither
        let error = plan_canonical_placements(&g, &c, &BTreeMap::new(), &eligibility).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_PLANNER_NO_ELIGIBLE_NODE), "findings: {error:?}");
    }

    #[test]
    fn service_with_no_resolved_artifact_is_unrestricted_by_eligibility() {
        // Absent from the view entirely (e.g. no ServiceArtifact registered
        // yet for a brand-new application) -- the planner must NOT restrict
        // candidates for it; real enforcement remains `create_proposal`'s
        // own `resolve_artifact_for_service` call, downstream.
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]);
        let plan = plan_canonical_placements(&g, &c, &BTreeMap::new(), &PlacementEligibilityView::unrestricted()).unwrap();
        assert_eq!(plan.placements[0].node_id, "fog-01");
    }

    #[test]
    fn an_artifact_with_an_explicitly_empty_target_nodes_set_still_means_zero_eligible_nodes() {
        // Guards against silently reinterpreting `check_node_eligibility`'s
        // real semantics: a service_id PRESENT in the view with an empty
        // set is NOT "unrestricted" -- it is zero eligible nodes, exactly
        // like the real per-node check.
        let g = dgraph(vec![dserv("svc-a", 1, 128)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]);
        let mut eligibility = PlacementEligibilityView::unrestricted();
        eligibility.insert("svc-a".to_string(), std::collections::BTreeSet::new());
        let error = plan_canonical_placements(&g, &c, &BTreeMap::new(), &eligibility).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_PLANNER_NO_ELIGIBLE_NODE), "findings: {error:?}");
    }

    // ---- foundation closure v2, R3B: project commitment awareness --------

    #[test]
    fn other_active_applications_committed_capacity_reduces_planning_residual() {
        // fog-01 has 4 cores total; 3 are already committed by another
        // active application -- only 1 core of residual remains, so a
        // 2-core service must be placed on cloud-01 instead, even though
        // fog-01 alone (ignoring commitments) would otherwise be the
        // min-residual-after winner.
        let g = dgraph(vec![dserv("svc-a", 2, 256)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0]), dnode("cloud-01", 8, 8192, vec![DNODE_ABI_V0])]);
        let mut committed = BTreeMap::new();
        committed.insert("fog-01".to_string(), NodeCommitmentV1 { cpu_cores: 3, memory_bytes: 0 });
        let plan = plan_canonical_placements(&g, &c, &committed, &PlacementEligibilityView::unrestricted()).unwrap();
        assert_eq!(plan.placements[0].node_id, "cloud-01");
    }

    // ---- independent review v3, C1B: eligibility snapshot representation --

    #[test]
    fn eligibility_snapshot_preserves_absent_vs_present_empty_distinction() {
        let mut view = PlacementEligibilityView::unrestricted();
        view.insert("svc-restricted-empty".to_string(), BTreeSet::new());
        // "svc-absent" is deliberately never inserted at all.
        let json = serde_json::to_string(&view).unwrap();
        assert!(json.contains("svc-restricted-empty"), "a present-but-empty service_id key must appear in the serialized form: {json}");
        assert!(!json.contains("svc-absent"), "an absent service_id must never appear in the serialized form at all: {json}");

        let round_tripped: PlacementEligibilityView = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.eligible_nodes_for("svc-restricted-empty"), Some(&BTreeSet::new()), "present-with-empty-set must round-trip as Some(empty set), never None");
        assert_eq!(round_tripped.eligible_nodes_for("svc-absent"), None, "an absent service_id must round-trip as None, never Some(empty set)");
        assert_eq!(view, round_tripped);
    }

    #[test]
    fn commitment_meeting_or_exceeding_declared_capacity_saturates_to_zero_residual_rather_than_panicking() {
        let g = dgraph(vec![dserv("svc-a", 1, 1)]);
        let c = dcontinuum(vec![dnode("fog-01", 4, 4096, vec![DNODE_ABI_V0])]);
        let mut committed = BTreeMap::new();
        committed.insert("fog-01".to_string(), NodeCommitmentV1 { cpu_cores: 999, memory_bytes: 999_999_999 });
        let error = plan_canonical_placements(&g, &c, &committed, &PlacementEligibilityView::unrestricted()).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_PLANNER_UNSATISFIED_STATIC_CAPACITY), "findings: {error:?}");
    }
}
