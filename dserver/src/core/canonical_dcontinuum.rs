//! `datum.dcontinuum/1` (Stage 110, ADR-0017) -- the canonical
//! infrastructure/resource continuum known to DATUM: what D-Nodes exist,
//! what kind each is, what execution capabilities they declare, and what
//! static topology links are known between them.
//!
//! This is STRUCTURAL/DECLARED inventory, never a live monitoring snapshot.
//! `CanonicalDContinuumV1` carries NO changing runtime telemetry -- no CPU
//! utilization, RSS, swap, RTT, packet-loss sample, process liveness,
//! container health, temperature, or restart count. A `CanonicalDContinuumV1`
//! revision changes only when the DECLARED inventory itself changes (a node
//! added/removed, a capability declaration changed); it must never be
//! confused with a Stage 113 D-Monitor telemetry snapshot identity, which
//! this contract is deliberately structured to let Stage 113 attach
//! observations onto WITHOUT mutating this structural identity model.
//!
//! Capability-driven, not a node-class taxonomy: each `CanonicalDNodeV1`
//! declares the SET of host-independent D-Node ABI versions (ADR-0003) it
//! supports. Stage 112 placement can determine whether a node can host a
//! given D-Serv's D-Code purely from this typed set, never by inspecting an
//! arbitrary node-class string. Stage 110 P0 fix (independent review v4,
//! D-Serv host invariance): this is deliberately NOT Stage 108's
//! `RuntimeKind` (`container`/`native_process`) -- see
//! `canonical_dgraph::DNODE_ABI_V0`'s doc comment for the full rationale. A
//! node's `container`/`native_process` HOSTING details are an operational-
//! plane fact about that D-Node itself, never part of the application-plane
//! capability this module validates.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::canonical_dgraph::{CanonicalValidationFinding, DNODE_ABI_V0};
use crate::core::jcs;

pub const CANONICAL_DCONTINUUM_SCHEMA: &str = "datum.dcontinuum/1";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDContinuumV1 {
    pub schema: String,
    /// ADR-0001: an implementation scoping choice (deployment/persistence
    /// scope), not part of the canonical reference-model meaning of
    /// D-Continuum -- but REQUIRED at this v1 wire contract level (Stage
    /// 110's identity list), unlike the looser Stage 107 hand-authored
    /// fixtures, which treated it as optional.
    pub project_id: String,
    pub dcontinuum_id: String,
    pub revision: u64,
    #[serde(default)]
    pub nodes: Vec<CanonicalDNodeV1>,
    /// Static/structural topology only -- never live RTT/loss/bandwidth
    /// samples (those belong to Stage 113 D-Monitor observations, attached
    /// elsewhere, never mixed into this identity model).
    #[serde(default)]
    pub links: Vec<CanonicalDLinkV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDNodeV1 {
    pub node_id: String,
    pub platform: CanonicalNodePlatformV1,
    /// Stage 110 P0 fix (independent review v3, Stage 112 resource
    /// readiness): the DECLARED/STRUCTURAL static capacity of this node,
    /// never a live/observed value. A `CanonicalDContinuumV1` revision
    /// changes when this declared capacity changes, exactly like it does
    /// for a capability-set change; no CPU-utilization/RSS/free-memory/
    /// load-average field is or will ever be added here (Stage 113
    /// D-Monitor territory, attached elsewhere).
    pub capacity: CanonicalNodeCapacityV1,
    pub capabilities: CanonicalNodeCapabilitiesV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalNodePlatformV1 {
    pub os: String,
    pub architecture: String,
}

/// Exact integer static capacity, mirroring
/// `canonical_dgraph::CanonicalDServResourceRequirementV1`'s units exactly
/// (`cpu_cores`, `memory_bytes`) so a feasibility comparison in
/// `canonical_dmap` never has to convert between units.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalNodeCapacityV1 {
    pub cpu_cores: u32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalNodeCapabilitiesV1 {
    /// The SET of host-independent D-Node ABI versions (ADR-0003) this node
    /// supports -- for v0.1, entries must be `DNODE_ABI_V0` ("datum-dnode/0")
    /// or the set may be empty (a node that cannot host any D-Code under the
    /// current ABI, e.g. a restricted/lightweight element). A node lacking
    /// the ABI a D-Serv requires here is one Stage 112 placement must never
    /// target for that D-Serv.
    #[serde(default)]
    pub dnode_abi_versions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDLinkV1 {
    pub link_id: String,
    pub from_node_id: String,
    pub to_node_id: String,
}

impl CanonicalDContinuumV1 {
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DCONTINUUM_SCHEMA {
            findings.push(finding("schema_invalid", format!("schema must be '{CANONICAL_DCONTINUUM_SCHEMA}', got '{}'", self.schema)));
        }
        if self.project_id.trim().is_empty() {
            findings.push(finding("missing_field", "project_id must be a non-empty string"));
        }
        if self.dcontinuum_id.trim().is_empty() {
            findings.push(finding("missing_field", "dcontinuum_id must be a non-empty string"));
        }
        if self.revision < 1 {
            findings.push(finding("revision_invalid", "revision must be an integer >= 1"));
        }

        let mut seen_node_ids = std::collections::HashSet::new();
        let mut known_nodes = std::collections::HashSet::new();
        for node in &self.nodes {
            if node.node_id.trim().is_empty() {
                findings.push(finding("missing_field", "node_id must be a non-empty string"));
            } else if !seen_node_ids.insert(node.node_id.as_str()) {
                findings.push(finding("duplicate_node_id", format!("duplicate node_id '{}'", node.node_id)));
            }
            known_nodes.insert(node.node_id.as_str());
            if node.platform.os.trim().is_empty() || node.platform.architecture.trim().is_empty() {
                findings.push(finding("invalid_platform", format!("node '{}' platform.os/architecture must be non-empty", node.node_id)));
            }
            if node.capacity.cpu_cores < 1 {
                findings.push(finding("invalid_node_cpu_capacity", format!("node '{}' capacity.cpu_cores must be >= 1, got {}", node.node_id, node.capacity.cpu_cores)));
            }
            if node.capacity.memory_bytes < 1 {
                findings.push(finding("invalid_node_memory_capacity", format!("node '{}' capacity.memory_bytes must be >= 1, got {}", node.node_id, node.capacity.memory_bytes)));
            }
            // Stage 110 P1 fix (independent review v2, re-verified v4):
            // `dnode_abi_versions` is documented (ADR-0017) as a canonically
            // UNORDERED SET -- a duplicate entry is invalid input, never
            // something to silently deduplicate during hashing. Rejecting it
            // here (rather than in the digest function) means the same
            // semantic violation is caught the same way as every other
            // invalid-input case: `validate()` reports it as a finding,
            // `recompute_dcontinuum_digest` never gets asked to paper over it.
            let mut seen_dnode_abi_versions = std::collections::HashSet::new();
            for abi in &node.capabilities.dnode_abi_versions {
                if abi != DNODE_ABI_V0 {
                    findings.push(finding(
                        "invalid_dnode_abi_capability",
                        format!("node '{}' declares unrecognised D-Node ABI capability '{}'", node.node_id, abi),
                    ));
                } else if !seen_dnode_abi_versions.insert(abi.as_str()) {
                    findings.push(finding(
                        "duplicate_dnode_abi_capability",
                        format!("node '{}' declares D-Node ABI capability '{}' more than once", node.node_id, abi),
                    ));
                }
            }
        }

        let mut seen_link_ids = std::collections::HashSet::new();
        for link in &self.links {
            if link.link_id.trim().is_empty() {
                findings.push(finding("missing_field", "link_id must be a non-empty string"));
            } else if !seen_link_ids.insert(link.link_id.as_str()) {
                findings.push(finding("duplicate_link_id", format!("duplicate link_id '{}'", link.link_id)));
            }
            if !known_nodes.contains(link.from_node_id.as_str()) {
                findings.push(finding(
                    "dangling_link_endpoint",
                    format!("link '{}' from_node_id '{}' does not name a declared node", link.link_id, link.from_node_id),
                ));
            }
            if !known_nodes.contains(link.to_node_id.as_str()) {
                findings.push(finding(
                    "dangling_link_endpoint",
                    format!("link '{}' to_node_id '{}' does not name a declared node", link.link_id, link.to_node_id),
                ));
            }
        }

        findings
    }
}

/// Canonical JCS+SHA-256 digest, never self-referential. `nodes`/`links` are
/// canonically unordered sets keyed by `node_id`/`link_id` -- see
/// `canonical_dgraph::recompute_dgraph_digest`'s identical documented
/// decision. A node's `capabilities.dnode_abi_versions` is likewise an
/// unordered SET of declared capabilities and is sorted before hashing for
/// the same reason.
pub fn recompute_dcontinuum_digest(dcontinuum: &CanonicalDContinuumV1) -> Result<String, String> {
    let mut sorted = dcontinuum.clone();
    sorted.nodes.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    for node in &mut sorted.nodes {
        node.capabilities.dnode_abi_versions.sort();
    }
    sorted.links.sort_by(|a, b| a.link_id.cmp(&b.link_id));
    let value: Value =
        serde_json::to_value(&sorted).map_err(|error| format!("serialize CanonicalDContinuumV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap(cpu_cores: u32, memory_bytes: u64) -> CanonicalNodeCapacityV1 {
        CanonicalNodeCapacityV1 { cpu_cores, memory_bytes }
    }

    fn sample_dcontinuum() -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: "monitor-f49c8".to_string(),
            dcontinuum_id: "continuum:smart-sentinel-lab".to_string(),
            revision: 1,
            nodes: vec![
                CanonicalDNodeV1 {
                    node_id: "cloud-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                    capacity: cap(8, 17_179_869_184),
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
                },
                CanonicalDNodeV1 {
                    node_id: "fog-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                    capacity: cap(4, 4_294_967_296),
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
                },
                CanonicalDNodeV1 {
                    node_id: "mist-arm-gateway-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "arm64".to_string() },
                    capacity: cap(1, 536_870_912),
                    // Restricted/lightweight element: cannot host any D-Code
                    // under the current single-ABI v0.1 model -- empty is a
                    // legal (if degenerate) capability set, distinct from an
                    // unrecognised-ABI-token violation.
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![] },
                },
            ],
            links: vec![
                CanonicalDLinkV1 { link_id: "fog-to-cloud".to_string(), from_node_id: "fog-01".to_string(), to_node_id: "cloud-01".to_string() },
                CanonicalDLinkV1 { link_id: "mist-to-fog".to_string(), from_node_id: "mist-arm-gateway-01".to_string(), to_node_id: "fog-01".to_string() },
            ],
        }
    }

    #[test]
    fn heterogeneous_node_set_validates_cleanly() {
        assert_eq!(sample_dcontinuum().validate(), Vec::new());
    }

    #[test]
    fn abi_capable_node_recognised() {
        let dcontinuum = sample_dcontinuum();
        let node = dcontinuum.nodes.iter().find(|n| n.node_id == "cloud-01").unwrap();
        assert!(node.capabilities.dnode_abi_versions.iter().any(|v| v == DNODE_ABI_V0));
    }

    #[test]
    fn fog_node_also_recognised_as_abi_capable() {
        let dcontinuum = sample_dcontinuum();
        let node = dcontinuum.nodes.iter().find(|n| n.node_id == "fog-01").unwrap();
        assert!(node.capabilities.dnode_abi_versions.iter().any(|v| v == DNODE_ABI_V0));
    }

    /// Stage 110 P0 fix (independent review v4, D-Serv host invariance): a
    /// restricted node may legally advertise ZERO ABI capabilities --
    /// distinct from advertising an unrecognised one. Heterogeneity in v0.1
    /// is now expressed via this capability set (and via static resource
    /// capacity), never via a host-executor-kind partition.
    #[test]
    fn restricted_node_capability_set_is_empty() {
        let dcontinuum = sample_dcontinuum();
        let restricted = dcontinuum.nodes.iter().find(|n| n.node_id == "mist-arm-gateway-01").unwrap();
        assert_eq!(restricted.capabilities.dnode_abi_versions.len(), 0);
        // An empty capability set is legal input (never an
        // invalid_dnode_abi_capability finding) -- confirmed by the whole
        // fixture, including this node, validating cleanly above
        // (`heterogeneous_node_set_validates_cleanly`).
    }

    #[test]
    fn wrong_schema_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.schema = "datum.dcontinuum/2".to_string();
        assert!(dcontinuum.validate().iter().any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn invalid_revision_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.revision = 0;
        assert!(dcontinuum.validate().iter().any(|f| f.code == "revision_invalid"));
    }

    #[test]
    fn duplicate_node_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.nodes[1].node_id = dcontinuum.nodes[0].node_id.clone();
        assert!(dcontinuum.validate().iter().any(|f| f.code == "duplicate_node_id"));
    }

    #[test]
    fn duplicate_node_hidden_by_ordering_is_still_caught() {
        let mut dcontinuum = sample_dcontinuum();
        let dup = dcontinuum.nodes[0].clone();
        dcontinuum.nodes.push(dup);
        assert!(dcontinuum.validate().iter().any(|f| f.code == "duplicate_node_id"));
    }

    #[test]
    fn dangling_link_endpoint_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.links[0].to_node_id = "does-not-exist".to_string();
        assert!(dcontinuum.validate().iter().any(|f| f.code == "dangling_link_endpoint"));
    }

    #[test]
    fn duplicate_link_id_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        let id = dcontinuum.links[0].link_id.clone();
        dcontinuum.links[1].link_id = id;
        assert!(dcontinuum.validate().iter().any(|f| f.code == "duplicate_link_id"));
    }

    #[test]
    fn invalid_dnode_abi_capability_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.nodes[0].capabilities.dnode_abi_versions.push("datum-dnode/999".to_string());
        assert!(dcontinuum.validate().iter().any(|f| f.code == "invalid_dnode_abi_capability"));
    }

    /// Stage 110 P1 fix (independent review v2, re-verified v4):
    /// `dnode_abi_versions` is documented as a set -- a repeated valid entry
    /// must be REJECTED, never silently accepted and deduplicated only at
    /// digest time (which would let two structurally different inputs
    /// validate successfully while one of them also silently hashes
    /// identically to its deduplicated form, masking the invalid input
    /// entirely).
    #[test]
    fn duplicate_dnode_abi_capability_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        let existing = dcontinuum.nodes[1].capabilities.dnode_abi_versions[0].clone();
        dcontinuum.nodes[1].capabilities.dnode_abi_versions.push(existing);
        assert!(dcontinuum.validate().iter().any(|f| f.code == "duplicate_dnode_abi_capability"));
    }

    /// The existing order-independence guarantee must survive the duplicate
    /// check: a genuinely unique capability set, differently ORDERED, is
    /// still valid and still hashes identically (see
    /// `digest_is_order_independent_for_nodes_links_and_capabilities` below)
    /// -- only an actual repeated value is ever rejected.
    #[test]
    fn unique_dnode_abi_capabilities_in_different_order_remain_valid() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.nodes[1].capabilities.dnode_abi_versions.reverse();
        assert_eq!(dcontinuum.validate(), Vec::new());
    }

    /// Stage 110 P0 fix (independent review v3, Stage 112 resource
    /// readiness): a node declaring zero CPU cores of capacity is
    /// nonsensical static input, not a value to silently accept.
    #[test]
    fn invalid_node_cpu_capacity_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.nodes[0].capacity.cpu_cores = 0;
        assert!(dcontinuum.validate().iter().any(|f| f.code == "invalid_node_cpu_capacity"));
    }

    #[test]
    fn invalid_node_memory_capacity_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.nodes[0].capacity.memory_bytes = 0;
        assert!(dcontinuum.validate().iter().any(|f| f.code == "invalid_node_memory_capacity"));
    }

    #[test]
    fn digest_changes_when_a_node_capacity_changes() {
        let a = sample_dcontinuum();
        let mut b = sample_dcontinuum();
        b.nodes[0].capacity.cpu_cores += 1;
        assert_ne!(recompute_dcontinuum_digest(&a).unwrap(), recompute_dcontinuum_digest(&b).unwrap());
    }

    #[test]
    fn empty_project_id_rejected() {
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.project_id = "   ".to_string();
        assert!(dcontinuum.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn revision_identity_preserved_through_round_trip() {
        let dcontinuum = sample_dcontinuum();
        let json = serde_json::to_string(&dcontinuum).unwrap();
        let round_tripped: CanonicalDContinuumV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, dcontinuum);
    }

    #[test]
    fn no_telemetry_field_exists_on_the_type() {
        // A structural/compile-time proof, not merely a claim: the JSON
        // representation of a valid continuum contains none of the
        // forbidden live-telemetry keys.
        let value = serde_json::to_value(sample_dcontinuum()).unwrap();
        let text = value.to_string();
        for forbidden in [
            "cpu_utilization", "rss", "swap", "rtt", "packet_loss", "temperature", "restart_count", "health",
            "cpu_usage", "usage_percent", "available", "current_", "load_average",
        ] {
            assert!(!text.contains(forbidden), "forbidden telemetry-shaped key '{forbidden}' found in canonical D-Continuum JSON");
        }
    }

    #[test]
    fn digest_is_deterministic() {
        let d1 = recompute_dcontinuum_digest(&sample_dcontinuum()).unwrap();
        let d2 = recompute_dcontinuum_digest(&sample_dcontinuum()).unwrap();
        assert_eq!(d1, d2);
        assert!(d1.starts_with("sha256:"));
    }

    #[test]
    fn digest_is_order_independent_for_nodes_links_and_capabilities() {
        let mut reordered = sample_dcontinuum();
        reordered.nodes.reverse();
        reordered.links.reverse();
        reordered.nodes[0].capabilities.dnode_abi_versions.reverse();
        assert_eq!(recompute_dcontinuum_digest(&sample_dcontinuum()).unwrap(), recompute_dcontinuum_digest(&reordered).unwrap());
    }

    #[test]
    fn digest_changes_when_a_node_capability_changes() {
        let a = sample_dcontinuum();
        let mut b = sample_dcontinuum();
        b.nodes[2].capabilities.dnode_abi_versions.push(DNODE_ABI_V0.to_string());
        assert_ne!(recompute_dcontinuum_digest(&a).unwrap(), recompute_dcontinuum_digest(&b).unwrap());
    }

    /// Real cross-crate mapper fixture -- see
    /// `canonical_dgraph::tests::agent_cross_crate_fixture_produces_the_expected_digest`
    /// for the full pattern/rationale.
    #[test]
    fn agent_cross_crate_fixture_produces_the_expected_digest() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("canonical-v1")
            .join("dcontinuum.smart-sentinel-lab.json");
        let text = std::fs::read_to_string(&fixture_path).unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        let raw: Value = serde_json::from_str(&text).unwrap();
        let expected_digest = raw["expected_digest_sha256"].as_str().unwrap().to_string();

        let dcontinuum: CanonicalDContinuumV1 = serde_json::from_str(&text).expect("fixture must deserialize as this crate's CanonicalDContinuumV1");
        assert_eq!(dcontinuum.validate(), Vec::new(), "the positive fixture must validate cleanly");

        let digest = recompute_dcontinuum_digest(&dcontinuum).unwrap();
        assert_eq!(digest, expected_digest, "this crate's recomputed digest must match the fixture's recorded expected_digest_sha256");
    }
}
