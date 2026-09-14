//! D-Node/D-Continuum authority foundation closure (ADR-0019): the
//! server-owned STRUCTURAL D-Node registry, and the authoritative
//! `datum.dcontinuum/1` DServer derives from it for canonical D-Deploy
//! (Stage 111, ADR-0018).
//!
//! This module closes the one open Stage 111 gap the conceptual roadmap
//! audit identified: `create_proposal`/`validate_full` (`core::ddeploy`)
//! accept a `CanonicalDContinuumV1` as a plain parameter and have never
//! trusted a caller-supplied digest -- but nothing previously stopped a
//! caller from inventing an internally-consistent node (fictional node_id,
//! or a real node_id with fabricated capacity/platform/ABI) inside that
//! parameter. This module gives DServer its own STRUCTURAL source of truth
//! for "what D-Nodes genuinely exist" so the API layer (`api::ddeploy`) can
//! reject such a claim before a proposal is ever created (see
//! `check_dcontinuum_authority`).
//!
//! STRUCTURAL vs OPERATIONAL/LIVE (non-negotiable, per ADR-0017): a
//! `CanonicalDNodeV1` declaration here is node_id + platform + static
//! capacity + declared D-Node ABI capability set -- nothing else. It is
//! NEVER Sentinel lease/liveness state:
//!   - `SentinelRegistrationRecord` (`sentinel_registration.rs`) carries
//!     `instance_id`/`lease_seconds`/free-form `capabilities: Vec<String>`
//!     tags and is not even persisted today (in-memory `OnceLock<RwLock<..>>`
//!     only) -- it has no `platform`/`capacity` fields at all and cannot
//!     serve as a `CanonicalDNodeV1` source.
//!   - `resource_registry.rs`'s `ResourceRegistryRecord` (a hardware/OS
//!     identity-binding subsystem, unrelated to this one) carries `platform:
//!     Value` (untyped JSON) and a `resources.capacity_profile_id` field
//!     that is an OPAQUE content-addressed digest of some capacity profile
//!     never itself resolved to numeric `cpu_cores`/`memory_bytes` anywhere
//!     in this repository -- reusing it here would require either inventing
//!     numbers behind that digest (explicitly forbidden) or building a new
//!     capacity-profile catalog, which is out of scope for this pass. This
//!     module therefore introduces the MINIMUM additional structural
//!     declaration needed instead of forcing that subsystem to serve a
//!     purpose it cannot yet fulfil.
//!   - Live telemetry (`resource_observation.rs`'s `NodeResourceObservation`
//!     -- CPU usage_percent, available memory, freshness) never appears
//!     here and never will: a `CanonicalDContinuumV1` revision changes only
//!     when the DECLARED inventory changes (ADR-0017).
//!
//! GLOBAL, not project-partitioned: a structural D-Node declaration is
//! keyed only by `node_id` -- exactly like Sentinel registration is keyed
//! only by `node_id` with no `project_id` at all. Nothing else in this
//! repository models "which project owns which physical node", and
//! `CanonicalDContinuumV1.project_id` is documented (ADR-0001) as a wire-
//! identity SCOPING LABEL, not a claim that infrastructure is partitioned
//! by project. `build_canonical_dcontinuum` therefore stamps the requested
//! `project_id` onto the SAME global node set for every project rather than
//! inventing a new node-to-project ownership model this pass has no
//! evidence for.
//!
//! D-Node vs Sentinel/D-Agent: declaring a structural D-Node is completely
//! independent of any Sentinel registration lifecycle. A Sentinel MAY
//! declare a D-Node descriptor (nothing here prevents an agent's own
//! bootstrap flow from calling `declare_dnode` for the node it runs on),
//! but nothing in this module requires a 1:1 relationship -- there is no
//! `instance_id` field here, and `node_id` is never derived from or checked
//! against any `SentinelRegistrationRecord`. A future design remains free to
//! let one agent instance declare/manage multiple D-Nodes, or several agent
//! instances cooperatively declare the same D-Node, without any change to
//! this module's shape.
//!
//! No placement authority here: this module never assigns a service to a
//! node (that remains exclusively `core::ddeploy`'s accepted
//! `CanonicalDMapV1.placements`, ADR-0002) and never scores/selects a node
//! (no Stage 120 deterministic placement, no GNN/RL/LLM). It only answers
//! "does this claimed structural fact match what was genuinely declared?".

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dcontinuum::{
    recompute_dcontinuum_digest, CanonicalDContinuumV1, CanonicalDNodeV1, CANONICAL_DCONTINUUM_SCHEMA,
};
use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::storage::control_state::{load_domain_json, persist_domain_json, DOMAIN_DNODE_REGISTRY};

pub const DNODE_REGISTRY_STATE_VERSION: &str = "0.1.0";

pub const F_DNODE_STRUCTURAL_CONFLICT: &str = "dnode_structural_conflict";
pub const F_DNODE_CAPABILITY_INVALID: &str = "dnode_capability_invalid";
pub const F_DCONTINUUM_NODE_UNREGISTERED: &str = "dcontinuum_node_unregistered";
pub const F_AUTHORITATIVE_DCONTINUUM_UNAVAILABLE: &str = "authoritative_dcontinuum_unavailable";
/// Independent review fix v2, R2: the persisted registry's `BTreeMap` key
/// does not equal the `node_id` of the `CanonicalDNodeV1` stored under it.
pub const F_DNODE_REGISTRY_KEY_MISMATCH: &str = "dnode_registry_key_mismatch";
/// Independent review fix v2, R2: a non-empty persisted node set with
/// `revision < 1` -- structurally impossible via `declare_dnode` (which
/// always bumps to >= 1 on first insert), so only reachable through direct
/// file corruption.
pub const F_DNODE_REGISTRY_REVISION_INVALID: &str = "dnode_registry_revision_invalid";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

/// Server-owned, persisted structural D-Node inventory. GLOBAL (see module
/// doc) -- keyed by `node_id`, never by project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DNodeRegistryStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub nodes: BTreeMap<String, CanonicalDNodeV1>,
    /// Bumps only when the CONTENT of the declared node set changes.
    /// Re-declaring an already-declared node with IDENTICAL content is a
    /// no-op that leaves this unchanged (see `declare_dnode`).
    #[serde(default)]
    pub revision: u64,
}

impl DNodeRegistryStateV1 {
    pub fn empty() -> Self {
        Self { state_version: DNODE_REGISTRY_STATE_VERSION.to_string(), nodes: BTreeMap::new(), revision: 0 }
    }
}

pub fn load_state(db_path: &std::path::Path) -> Result<DNodeRegistryStateV1, String> {
    match load_domain_json::<DNodeRegistryStateV1>(db_path, DOMAIN_DNODE_REGISTRY)? {
        Some(state) => {
            if state.state_version != DNODE_REGISTRY_STATE_VERSION {
                return Err(format!(
                    "unsupported dnode registry state_version '{}'; expected '{}'",
                    state.state_version, DNODE_REGISTRY_STATE_VERSION
                ));
            }
            validate_persisted_dnode_registry_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DNodeRegistryStateV1::empty()),
    }
}

/// Independent review fix v2 (R2): retained persisted structural state must
/// fail closed on any corruption `load_state` can detect -- NEVER
/// auto-repaired. At minimum: every `BTreeMap` key equals its own stored
/// `CanonicalDNodeV1.node_id`; every stored declaration independently
/// re-validates cleanly (reusing `validate_declaration`, never a second,
/// independently-maintained copy of that logic -- this also transitively
/// re-checks platform/capacity validity and ABI-set validity, including
/// duplicate/unrecognised ABI entries via `CanonicalDContinuumV1::validate`);
/// and a non-empty node set has `revision >= 1` (structurally guaranteed by
/// every legitimate `declare_dnode` call, so a violation here is only
/// reachable through direct file corruption). A legitimate empty bootstrap
/// (`nodes = {}`, `revision = 0`) remains valid and is NOT flagged.
pub fn validate_persisted_dnode_registry_integrity(state: &DNodeRegistryStateV1) -> Result<(), String> {
    for (key, node) in &state.nodes {
        if key != &node.node_id {
            return Err(format!(
                "{F_DNODE_REGISTRY_KEY_MISMATCH}: persisted D-Node registry map key '{key}' does not match its own CanonicalDNodeV1.node_id '{}'",
                node.node_id
            ));
        }
        let findings = validate_declaration(node);
        if !findings.is_empty() {
            return Err(format!(
                "persisted D-Node registry contains a structurally invalid declaration for node '{key}': {findings:?}"
            ));
        }
    }
    if !state.nodes.is_empty() && state.revision < 1 {
        return Err(format!(
            "{F_DNODE_REGISTRY_REVISION_INVALID}: persisted D-Node registry has {} declared node(s) but revision={} (a non-empty registry must have revision >= 1)",
            state.nodes.len(),
            state.revision
        ));
    }
    Ok(())
}

pub fn persist_state(db_path: &std::path::Path, state: &DNodeRegistryStateV1) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DNODE_REGISTRY, state)
}

/// `dnode_abi_versions` is a canonically UNORDERED SET (ADR-0017) -- normalize
/// before storing/comparing so a re-declaration that differs only in that
/// Vec's order is recognised as the SAME declaration, never a spurious
/// `F_DNODE_STRUCTURAL_CONFLICT`.
fn normalized(mut node: CanonicalDNodeV1) -> CanonicalDNodeV1 {
    node.capabilities.dnode_abi_versions.sort();
    node
}

/// Validate a single structural declaration's own shape (platform/capacity/
/// ABI) by reusing `CanonicalDContinuumV1::validate()` verbatim -- wrapping
/// the candidate in a single-node scratch continuum with fixed,
/// always-valid `project_id`/`dcontinuum_id`/`revision` placeholders is a
/// defensive wrapper, never a second, independently-maintained copy of that
/// validation logic.
pub fn validate_declaration(node: &CanonicalDNodeV1) -> Vec<CanonicalValidationFinding> {
    let scratch = CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: "dnode-declaration-scratch".to_string(),
        dcontinuum_id: "dnode-declaration-scratch".to_string(),
        revision: 1,
        nodes: vec![node.clone()],
        links: Vec::new(),
    };
    scratch.validate()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclareOutcome {
    Inserted,
    UnchangedNoop,
}

/// Declare (or idempotently re-declare) one structural D-Node. Fails closed
/// with `F_DNODE_STRUCTURAL_CONFLICT` on an attempt to silently change an
/// already-declared node's platform/capacity/ABI -- exactly like
/// `put_sentinel_registration` refuses to silently reassign a registered
/// node's `instance_id` (`api::sentinel_registration`). To genuinely change
/// a node's declared facts, `remove_dnode` it first, then redeclare -- there
/// is no in-place structural update in this minimal v0.1 (documented
/// limitation, ADR-0019).
pub fn declare_dnode(
    state: &mut DNodeRegistryStateV1,
    node: CanonicalDNodeV1,
) -> Result<DeclareOutcome, Vec<CanonicalValidationFinding>> {
    let findings = validate_declaration(&node);
    if !findings.is_empty() {
        return Err(findings);
    }
    let node = normalized(node);

    if let Some(existing) = state.nodes.get(&node.node_id) {
        if existing == &node {
            return Ok(DeclareOutcome::UnchangedNoop);
        }
        return Err(vec![finding(
            F_DNODE_STRUCTURAL_CONFLICT,
            format!(
                "node '{}' is already declared with a different platform/capacity/capabilities; remove it before redeclaring",
                node.node_id
            ),
        )]);
    }

    state.nodes.insert(node.node_id.clone(), node);
    state.revision = state.revision.saturating_add(1);
    Ok(DeclareOutcome::Inserted)
}

/// Removes a structural declaration (the only way to change one, per
/// `declare_dnode`'s doc). Returns `false` (a no-op, not an error) if the
/// node was never declared.
pub fn remove_dnode(state: &mut DNodeRegistryStateV1, node_id: &str) -> bool {
    let removed = state.nodes.remove(node_id).is_some();
    if removed {
        state.revision = state.revision.saturating_add(1);
    }
    removed
}

/// Server-owned, deterministic, project-scoped ONLY as a wire-identity label
/// (see module doc). Never trusts any caller-supplied digest; contains no
/// live telemetry (no field here is ever populated from
/// `resource_observation`/Sentinel lease state -- a structural compile-time
/// proof mirroring `canonical_dcontinuum::tests::no_telemetry_field_exists_on_the_type`
/// exists in this module's own tests). `links` is always empty: declarative
/// topology-link declarations are out of scope for this pass (documented
/// limitation, ADR-0019) -- links remain caller-supplied structural hints
/// only, consistent with them never participating in `CanonicalDMapV1`
/// feasibility validation today.
pub fn build_canonical_dcontinuum(
    state: &DNodeRegistryStateV1,
    project_id: &str,
) -> Result<CanonicalDContinuumV1, Vec<CanonicalValidationFinding>> {
    if state.nodes.is_empty() {
        return Err(vec![finding(
            F_AUTHORITATIVE_DCONTINUUM_UNAVAILABLE,
            format!(
                "no structural D-Node has been declared; D-Deploy cannot derive an authoritative datum.dcontinuum/1 for project '{project_id}'"
            ),
        )]);
    }

    // Independent review fix v2 (R2): no `.max(1)` here -- that would have
    // silently masked a corrupt persisted `revision=0` with non-empty
    // `nodes` into a bogus-but-plausible-looking `revision=1` continuum.
    // `validate_persisted_dnode_registry_integrity` (invoked by
    // `load_state`) now fails closed on exactly that corruption BEFORE a
    // state ever reaches this function, and every legitimate in-memory
    // construction path (`declare_dnode`) already guarantees
    // `!nodes.is_empty() => revision >= 1` by construction -- so `state.revision`
    // is used verbatim here, never coerced.
    let dcontinuum = CanonicalDContinuumV1 {
        schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
        project_id: project_id.to_string(),
        dcontinuum_id: format!("continuum:{project_id}"),
        revision: state.revision,
        nodes: state.nodes.values().cloned().collect(),
        links: Vec::new(),
    };

    let findings = dcontinuum.validate();
    if !findings.is_empty() {
        return Err(findings);
    }
    // Defensive: prove the digest is actually computable server-side before
    // handing this back (never persisted here -- the caller decides what,
    // if anything, to do with it).
    recompute_dcontinuum_digest(&dcontinuum).map_err(|error| vec![finding("internal_error", error)])?;
    Ok(dcontinuum)
}

/// D-Deploy authority transition (foundation closure Subtranche A4): does a
/// CALLER-SUPPLIED `datum.dcontinuum/1` claim only nodes that genuinely
/// match the server-owned structural registry? SUBSET semantics -- `claimed`
/// need not enumerate every authoritative node, only the ones it actually
/// references (typically the placement target nodes); this never allows
/// fabricating a node or lying about an existing one's structural facts,
/// which is the entire safety property required here. A claimed node absent
/// from the registry fails `F_DCONTINUUM_NODE_UNREGISTERED`; a claimed node
/// present but with a different platform/capacity fails
/// `F_DNODE_STRUCTURAL_CONFLICT`; a different `dnode_abi_versions` set
/// (order-independent) fails `F_DNODE_CAPABILITY_INVALID`.
pub fn check_dcontinuum_authority(
    claimed: &CanonicalDContinuumV1,
    authoritative: &CanonicalDContinuumV1,
) -> Vec<CanonicalValidationFinding> {
    let mut findings = Vec::new();
    for node in &claimed.nodes {
        match authoritative.nodes.iter().find(|candidate| candidate.node_id == node.node_id) {
            None => findings.push(finding(
                F_DCONTINUUM_NODE_UNREGISTERED,
                format!("claimed dcontinuum node '{}' is not a registered structural D-Node", node.node_id),
            )),
            Some(registered) => {
                if registered.platform != node.platform || registered.capacity != node.capacity {
                    findings.push(finding(
                        F_DNODE_STRUCTURAL_CONFLICT,
                        format!(
                            "claimed dcontinuum node '{}' platform/capacity does not match the registered structural declaration (registered platform={:?} capacity={:?}, claimed platform={:?} capacity={:?})",
                            node.node_id, registered.platform, registered.capacity, node.platform, node.capacity
                        ),
                    ));
                }
                let mut registered_abi = registered.capabilities.dnode_abi_versions.clone();
                registered_abi.sort();
                let mut claimed_abi = node.capabilities.dnode_abi_versions.clone();
                claimed_abi.sort();
                if registered_abi != claimed_abi {
                    findings.push(finding(
                        F_DNODE_CAPABILITY_INVALID,
                        format!(
                            "claimed dcontinuum node '{}' dnode_abi_versions {:?} does not match the registered declaration {:?}",
                            node.node_id, node.capabilities.dnode_abi_versions, registered.capabilities.dnode_abi_versions
                        ),
                    ));
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::core::canonical_dcontinuum::{
        CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1,
    };
    use crate::core::canonical_dgraph::DNODE_ABI_V0;

    use super::*;

    fn temp_db_path(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-dnode-registry-test-{tag}-{}.db", Uuid::new_v4()))
    }

    fn node(node_id: &str, cpu: u32, mem: u64, abi: &[&str]) -> CanonicalDNodeV1 {
        CanonicalDNodeV1 {
            node_id: node_id.to_string(),
            platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
            capacity: CanonicalNodeCapacityV1 { cpu_cores: cpu, memory_bytes: mem },
            capabilities: CanonicalNodeCapabilitiesV1 {
                dnode_abi_versions: abi.iter().map(|value| value.to_string()).collect(),
            },
        }
    }

    #[test]
    fn valid_structural_declaration_is_accepted() {
        let mut state = DNodeRegistryStateV1::empty();
        let outcome = declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        assert_eq!(outcome, DeclareOutcome::Inserted);
        assert_eq!(state.revision, 1);
        assert!(state.nodes.contains_key("fog-01"));
    }

    #[test]
    fn identical_redeclaration_is_an_idempotent_noop() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let outcome = declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        assert_eq!(outcome, DeclareOutcome::UnchangedNoop);
        assert_eq!(state.revision, 1, "an unchanged redeclaration must not bump the revision");
    }

    #[test]
    fn reordered_abi_set_redeclaration_is_still_a_noop() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let mut reordered = node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0]);
        reordered.capabilities.dnode_abi_versions.push("extra-would-differ".to_string());
        reordered.capabilities.dnode_abi_versions.pop();
        let outcome = declare_dnode(&mut state, reordered).unwrap();
        assert_eq!(outcome, DeclareOutcome::UnchangedNoop);
        assert_eq!(state.revision, 1);
    }

    #[test]
    fn duplicate_node_declaration_with_different_capacity_conflicts() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let error = declare_dnode(&mut state, node("fog-01", 8, 4_294_967_296, &[DNODE_ABI_V0])).unwrap_err();
        assert!(error.iter().any(|f| f.code == F_DNODE_STRUCTURAL_CONFLICT));
        assert_eq!(state.nodes.get("fog-01").unwrap().capacity.cpu_cores, 4, "the conflicting declaration must not have overwritten the original");
    }

    #[test]
    fn malformed_platform_is_rejected() {
        let mut state = DNodeRegistryStateV1::empty();
        let mut bad = node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0]);
        bad.platform.os = String::new();
        let error = declare_dnode(&mut state, bad).unwrap_err();
        assert!(error.iter().any(|f| f.code == "invalid_platform"));
        assert!(state.nodes.is_empty());
    }

    #[test]
    fn zero_capacity_is_rejected() {
        let mut state = DNodeRegistryStateV1::empty();
        let mut bad = node("fog-01", 0, 4_294_967_296, &[DNODE_ABI_V0]);
        let error1 = declare_dnode(&mut state, bad.clone()).unwrap_err();
        assert!(error1.iter().any(|f| f.code == "invalid_node_cpu_capacity"));
        bad.capacity.cpu_cores = 4;
        bad.capacity.memory_bytes = 0;
        let error2 = declare_dnode(&mut state, bad).unwrap_err();
        assert!(error2.iter().any(|f| f.code == "invalid_node_memory_capacity"));
    }

    #[test]
    fn unrecognised_abi_capability_is_rejected() {
        let mut state = DNodeRegistryStateV1::empty();
        let bad = node("fog-01", 4, 4_294_967_296, &["datum-dnode/999"]);
        let error = declare_dnode(&mut state, bad).unwrap_err();
        assert!(error.iter().any(|f| f.code == "invalid_dnode_abi_capability"));
    }

    #[test]
    fn empty_abi_capability_set_is_legal() {
        let mut state = DNodeRegistryStateV1::empty();
        let restricted = node("mist-01", 1, 536_870_912, &[]);
        let outcome = declare_dnode(&mut state, restricted).unwrap();
        assert_eq!(outcome, DeclareOutcome::Inserted);
    }

    #[test]
    fn authoritative_dcontinuum_is_deterministic_and_stably_ordered_regardless_of_registration_order() {
        let mut state_a = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state_a, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        declare_dnode(&mut state_a, node("cloud-01", 8, 17_179_869_184, &[DNODE_ABI_V0])).unwrap();

        let mut state_b = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state_b, node("cloud-01", 8, 17_179_869_184, &[DNODE_ABI_V0])).unwrap();
        declare_dnode(&mut state_b, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();

        let dcontinuum_a = build_canonical_dcontinuum(&state_a, "project-x").unwrap();
        let dcontinuum_b = build_canonical_dcontinuum(&state_b, "project-x").unwrap();
        assert_eq!(dcontinuum_a.nodes, dcontinuum_b.nodes, "node ordering must not depend on registration order");
        assert_eq!(
            recompute_dcontinuum_digest(&dcontinuum_a).unwrap(),
            recompute_dcontinuum_digest(&dcontinuum_b).unwrap()
        );
    }

    #[test]
    fn empty_registry_produces_authoritative_dcontinuum_unavailable() {
        let state = DNodeRegistryStateV1::empty();
        let error = build_canonical_dcontinuum(&state, "project-x").unwrap_err();
        assert!(error.iter().any(|f| f.code == F_AUTHORITATIVE_DCONTINUUM_UNAVAILABLE));
    }

    #[test]
    fn authoritative_dcontinuum_contains_no_live_telemetry_field() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let dcontinuum = build_canonical_dcontinuum(&state, "project-x").unwrap();
        let text = serde_json::to_value(&dcontinuum).unwrap().to_string();
        for forbidden in ["cpu_utilization", "usage_percent", "available", "current_", "rtt", "temperature"] {
            assert!(!text.contains(forbidden), "forbidden telemetry-shaped key '{forbidden}' found");
        }
    }

    #[test]
    fn proposal_using_registered_node_passes_authority_check() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let authoritative = build_canonical_dcontinuum(&state, "project-x").unwrap();

        let claimed = CanonicalDContinuumV1 { nodes: vec![node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])], ..authoritative.clone() };
        assert!(check_dcontinuum_authority(&claimed, &authoritative).is_empty());
    }

    #[test]
    fn proposal_inventing_unknown_node_fails_authority_check() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let authoritative = build_canonical_dcontinuum(&state, "project-x").unwrap();

        let claimed = CanonicalDContinuumV1 {
            nodes: vec![node("fabricated-node", 999, 999_999_999_999, &[DNODE_ABI_V0])],
            ..authoritative.clone()
        };
        let findings = check_dcontinuum_authority(&claimed, &authoritative);
        assert!(findings.iter().any(|f| f.code == F_DCONTINUUM_NODE_UNREGISTERED));
    }

    #[test]
    fn proposal_altering_registered_node_capacity_fails_authority_check() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let authoritative = build_canonical_dcontinuum(&state, "project-x").unwrap();

        let claimed = CanonicalDContinuumV1 { nodes: vec![node("fog-01", 999, 4_294_967_296, &[DNODE_ABI_V0])], ..authoritative.clone() };
        let findings = check_dcontinuum_authority(&claimed, &authoritative);
        assert!(findings.iter().any(|f| f.code == F_DNODE_STRUCTURAL_CONFLICT));
    }

    #[test]
    fn proposal_altering_registered_node_platform_fails_authority_check() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let authoritative = build_canonical_dcontinuum(&state, "project-x").unwrap();

        let mut altered = node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0]);
        altered.platform.architecture = "arm64".to_string();
        let claimed = CanonicalDContinuumV1 { nodes: vec![altered], ..authoritative.clone() };
        let findings = check_dcontinuum_authority(&claimed, &authoritative);
        assert!(findings.iter().any(|f| f.code == F_DNODE_STRUCTURAL_CONFLICT));
    }

    #[test]
    fn proposal_altering_registered_node_abi_capabilities_fails_authority_check() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        let authoritative = build_canonical_dcontinuum(&state, "project-x").unwrap();

        let claimed = CanonicalDContinuumV1 { nodes: vec![node("fog-01", 4, 4_294_967_296, &[])], ..authoritative.clone() };
        let findings = check_dcontinuum_authority(&claimed, &authoritative);
        assert!(findings.iter().any(|f| f.code == F_DNODE_CAPABILITY_INVALID));
    }

    #[test]
    fn claimed_subset_of_registered_nodes_is_sufficient() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        declare_dnode(&mut state, node("cloud-01", 8, 17_179_869_184, &[DNODE_ABI_V0])).unwrap();
        let authoritative = build_canonical_dcontinuum(&state, "project-x").unwrap();

        // Claiming only ONE of the two registered nodes (a legitimate
        // proposal referencing a single placement target) must still pass.
        let claimed = CanonicalDContinuumV1 { nodes: vec![node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])], ..authoritative.clone() };
        assert!(check_dcontinuum_authority(&claimed, &authoritative).is_empty());
    }

    #[test]
    fn structural_inventory_survives_restart() {
        let path = temp_db_path("restart");
        let mut state = load_state(&path).unwrap();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        declare_dnode(&mut state, node("cloud-01", 8, 17_179_869_184, &[DNODE_ABI_V0])).unwrap();
        persist_state(&path, &state).unwrap();

        let reloaded = load_state(&path).unwrap();
        assert_eq!(reloaded, state);
        assert_eq!(reloaded.nodes.len(), 2);
        assert_eq!(reloaded.revision, 2);
    }

    #[test]
    fn bootstrap_with_no_prior_state_is_empty() {
        let path = temp_db_path("bootstrap-empty");
        let state = load_state(&path).unwrap();
        assert!(state.nodes.is_empty());
        assert_eq!(state.revision, 0);
    }

    #[test]
    fn unsupported_state_version_fails_closed_on_reload() {
        let path = temp_db_path("unknown-version");
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        state.state_version = "9.9.9".to_string();
        persist_state(&path, &state).unwrap();
        assert!(load_state(&path).is_err());
    }

    #[test]
    fn remove_dnode_allows_a_clean_redeclaration_with_new_facts() {
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        assert!(remove_dnode(&mut state, "fog-01"));
        assert_eq!(state.revision, 2);
        let outcome = declare_dnode(&mut state, node("fog-01", 8, 8_589_934_592, &[DNODE_ABI_V0])).unwrap();
        assert_eq!(outcome, DeclareOutcome::Inserted);
        assert_eq!(state.nodes.get("fog-01").unwrap().capacity.cpu_cores, 8);
    }

    #[test]
    fn removing_an_undeclared_node_is_a_harmless_noop() {
        let mut state = DNodeRegistryStateV1::empty();
        assert!(!remove_dnode(&mut state, "never-declared"));
        assert_eq!(state.revision, 0);
    }

    // ---- independent review fix v2, R2: persisted-state integrity -------

    #[test]
    fn persisted_map_key_node_id_mismatch_fails_closed_on_reload() {
        let path = temp_db_path("key-mismatch");
        let mut state = DNodeRegistryStateV1::empty();
        state.nodes.insert("wrong-key".to_string(), node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0]));
        state.revision = 1;
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a map key that does not match its own stored node_id must fail closed");
        assert!(error.contains("map key") && error.contains("node_id"), "expected the map-key/node_id mismatch check to fire, got a different error: {error}");
    }

    #[test]
    fn persisted_invalid_node_fails_closed_on_reload() {
        let path = temp_db_path("invalid-node");
        let mut state = DNodeRegistryStateV1::empty();
        let mut bad = node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0]);
        bad.platform.os = String::new();
        state.nodes.insert("fog-01".to_string(), bad);
        state.revision = 1;
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a persisted node that fails its own structural validation must fail closed on reload");
        assert!(error.contains("structurally invalid declaration"), "expected the per-node structural validity check to fire, got a different error: {error}");
    }

    #[test]
    fn persisted_non_empty_registry_with_revision_zero_fails_closed_on_reload() {
        let path = temp_db_path("revision-zero");
        let mut state = DNodeRegistryStateV1::empty();
        state.nodes.insert("fog-01".to_string(), node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0]));
        state.revision = 0;
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).expect_err("a non-empty node set with revision=0 must fail closed on reload, never be silently coerced to revision=1");
        assert!(error.contains("revision=0"), "expected the non-empty-implies-revision>=1 check to fire, got a different error: {error}");
    }

    #[test]
    fn valid_multi_node_registry_reload_remains_green() {
        // Positive control: the integrity checks above must not falsely
        // reject a real, uncorrupted multi-node registry.
        let path = temp_db_path("valid-multi-node-reload");
        let mut state = DNodeRegistryStateV1::empty();
        declare_dnode(&mut state, node("fog-01", 4, 4_294_967_296, &[DNODE_ABI_V0])).unwrap();
        declare_dnode(&mut state, node("cloud-01", 8, 17_179_869_184, &[DNODE_ABI_V0])).unwrap();
        declare_dnode(&mut state, node("mist-01", 1, 536_870_912, &[])).unwrap();
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).expect("a real, uncorrupted multi-node registry must reload cleanly");
        assert_eq!(reloaded, state);
        let dcontinuum = build_canonical_dcontinuum(&reloaded, "project-x").unwrap();
        assert_eq!(dcontinuum.nodes.len(), 3);
    }
}
