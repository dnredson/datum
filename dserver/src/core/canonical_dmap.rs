//! `datum.dmap/1` (Stage 110, ADR-0017) -- the canonical placement/mapping
//! CONTRACT: it binds application-plane D-Serv identities to continuum
//! D-Node identities.
//!
//! `CanonicalDMapV1` is NOT a placement proposal, NOT a placement algorithm,
//! NOT observed runtime state, NOT a reconciliation report, NOT an AI
//! recommendation, and NOT a mutable `target_nodes` field hidden inside
//! another object. It is the single authoritative placement carrier
//! (ADR-0002) for the Stage 110/v0.1 vertical slice, whose mandatory mapping
//! surface is exactly D-Serv -> D-Node (single placement per D-Serv, matching
//! this repository's current v0.1 execution model:
//! `OperationalArtifactAssignment.target_node` is a single node, never a
//! set -- see ADR-0017). No D-IoT/D-Forward placement, no ABI/host-kind
//! negotiation, and no D-Deploy proposal/acceptance workflow is implemented
//! here: `validate`/`validate_against` are CONTRACT validation only -- they
//! REJECT an invalid mapping; they never SELECT one. No scoring, no
//! optimization, no fallback placement, no mutation.
//!
//! Binds itself immutably to the exact D-Graph and D-Continuum revision (and
//! canonical digest) it was produced from -- never duplicates their content,
//! never embeds live runtime health, never anticipates a Stage 111 D-Deploy
//! proposal-status field.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
use crate::core::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1, CanonicalValidationFinding};
use crate::core::jcs;

pub const CANONICAL_DMAP_SCHEMA: &str = "datum.dmap/1";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDMapV1 {
    pub schema: String,
    pub project_id: String,
    pub application_id: String,
    pub dmap_id: String,
    pub revision: u64,
    pub dgraph_ref: CanonicalContractReferenceV1,
    pub dcontinuum_ref: CanonicalContractReferenceV1,
    #[serde(default)]
    pub placements: Vec<CanonicalDServPlacementV1>,
}

/// `id` + `revision` + `digest`: the exact revision AND content this D-Map
/// was produced from, immutably bound together. `digest` is the referenced
/// contract's own canonical JCS+SHA-256 digest (ADR-0006) -- never the
/// D-Map's own digest (that would be self-referential and is never computed
/// here).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalContractReferenceV1 {
    pub id: String,
    pub revision: u64,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDServPlacementV1 {
    pub service_id: String,
    pub node_id: String,
}

impl CanonicalDMapV1 {
    /// Self-contained structural validation: schema, identity, revision,
    /// placement-cardinality (Stage 110/v0.1: single placement per D-Serv --
    /// a `service_id` may appear in `placements` at most once), and
    /// reference DIGEST SHAPE (Stage 110 P1 fix, independent review v3):
    /// `dgraph_ref.digest`/`dcontinuum_ref.digest` must be the exact
    /// ADR-0006 `sha256:<64 lowercase hex>` form -- an empty string, a bare
    /// hex digest, uppercase hex, or a wrong-length digest is a structural
    /// defect in THIS object, independent of whether it happens to match
    /// anything (that content-equality check belongs to
    /// `validate_cross_references`, below). Never touches the referenced
    /// D-Graph/D-Continuum; see `validate_against` for cross-contract checks.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DMAP_SCHEMA {
            findings.push(finding("schema_invalid", format!("schema must be '{CANONICAL_DMAP_SCHEMA}', got '{}'", self.schema)));
        }
        if self.project_id.trim().is_empty() {
            findings.push(finding("missing_field", "project_id must be a non-empty string"));
        }
        if self.application_id.trim().is_empty() {
            findings.push(finding("missing_field", "application_id must be a non-empty string"));
        }
        if self.dmap_id.trim().is_empty() {
            findings.push(finding("missing_field", "dmap_id must be a non-empty string"));
        }
        if self.revision < 1 {
            findings.push(finding("revision_invalid", "revision must be an integer >= 1"));
        }
        if self.dgraph_ref.id.trim().is_empty() || self.dgraph_ref.revision < 1 {
            findings.push(finding("missing_field", "dgraph_ref.id/revision must be present and revision >= 1"));
        }
        if !jcs::is_canonical_digest_format(&self.dgraph_ref.digest) {
            findings.push(finding("invalid_digest_format", format!("dgraph_ref.digest '{}' is not the canonical 'sha256:<64 lowercase hex>' form (ADR-0006)", self.dgraph_ref.digest)));
        }
        if self.dcontinuum_ref.id.trim().is_empty() || self.dcontinuum_ref.revision < 1 {
            findings.push(finding("missing_field", "dcontinuum_ref.id/revision must be present and revision >= 1"));
        }
        if !jcs::is_canonical_digest_format(&self.dcontinuum_ref.digest) {
            findings.push(finding("invalid_digest_format", format!("dcontinuum_ref.digest '{}' is not the canonical 'sha256:<64 lowercase hex>' form (ADR-0006)", self.dcontinuum_ref.digest)));
        }

        let mut seen_service_ids = std::collections::HashSet::new();
        for placement in &self.placements {
            if placement.service_id.trim().is_empty() {
                findings.push(finding("missing_field", "placement.service_id must be a non-empty string"));
            } else if !seen_service_ids.insert(placement.service_id.as_str()) {
                findings.push(finding(
                    "duplicate_placement",
                    format!("service '{}' is placed more than once; Stage 110/v0.1 requires exactly one placement per D-Serv", placement.service_id),
                ));
            }
            if placement.node_id.trim().is_empty() {
                findings.push(finding("missing_field", "placement.node_id must be a non-empty string"));
            }
        }

        findings
    }

    /// Cross-contract CONTRACT validation against the exact D-Graph and
    /// D-Continuum objects this D-Map claims to reference -- CROSS-CONTRACT
    /// checks ONLY (reference identity/digest, application/project match,
    /// unknown service/node, host-independent D-Node ABI compatibility,
    /// missing placement). Deliberately does NOT re-run `self.validate()`,
    /// `dgraph.validate()`, or `dcontinuum.validate()` -- this is the
    /// low-level building block, not the complete acceptance gate. Callers
    /// that need a single fail-closed entry point (Stage 111's natural
    /// acceptance-gate usage) must call `validate_against` below, not this
    /// function, directly.
    ///
    /// Stage 110 P1 fix (independent review v2): this function alone is
    /// UNSAFE as a public "is this bundle acceptable" API, because it
    /// compares raw strings/identities without first confirming either
    /// input is itself well-formed. Concretely: a D-Graph service with an
    /// invalid `dnode_abi` (itself invalid) and a D-Continuum node
    /// declaring the SAME invalid string both fail their own `validate()`,
    /// but `validate_cross_references` alone reports zero findings for that
    /// pair, because the two invalid strings compare equal as raw strings.
    /// This is why `validate_against` (below) is the complete gate and this
    /// function is named explicitly as a partial one.
    pub fn validate_cross_references(&self, dgraph: &CanonicalDGraphV1, dcontinuum: &CanonicalDContinuumV1) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.dgraph_ref.id != dgraph.dgraph_id || self.dgraph_ref.revision != dgraph.revision {
            findings.push(finding(
                "dgraph_reference_mismatch",
                format!(
                    "dgraph_ref {{id: '{}', revision: {}}} does not match the provided D-Graph {{id: '{}', revision: {}}}",
                    self.dgraph_ref.id, self.dgraph_ref.revision, dgraph.dgraph_id, dgraph.revision
                ),
            ));
        } else {
            match recompute_dgraph_digest(dgraph) {
                Ok(actual) if actual == self.dgraph_ref.digest => {}
                Ok(actual) => findings.push(finding(
                    "dgraph_digest_mismatch",
                    format!("dgraph_ref.digest '{}' does not match the D-Graph's recomputed digest '{actual}'", self.dgraph_ref.digest),
                )),
                Err(error) => findings.push(finding("dgraph_digest_mismatch", format!("failed to recompute D-Graph digest: {error}"))),
            }
        }

        if self.dcontinuum_ref.id != dcontinuum.dcontinuum_id || self.dcontinuum_ref.revision != dcontinuum.revision {
            findings.push(finding(
                "dcontinuum_reference_mismatch",
                format!(
                    "dcontinuum_ref {{id: '{}', revision: {}}} does not match the provided D-Continuum {{id: '{}', revision: {}}}",
                    self.dcontinuum_ref.id, self.dcontinuum_ref.revision, dcontinuum.dcontinuum_id, dcontinuum.revision
                ),
            ));
        } else {
            match recompute_dcontinuum_digest(dcontinuum) {
                Ok(actual) if actual == self.dcontinuum_ref.digest => {}
                Ok(actual) => findings.push(finding(
                    "dcontinuum_digest_mismatch",
                    format!("dcontinuum_ref.digest '{}' does not match the D-Continuum's recomputed digest '{actual}'", self.dcontinuum_ref.digest),
                )),
                Err(error) => findings.push(finding("dcontinuum_digest_mismatch", format!("failed to recompute D-Continuum digest: {error}"))),
            }
        }

        if self.application_id != dgraph.application_id {
            findings.push(finding(
                "application_mismatch",
                format!("D-Map application_id '{}' does not match D-Graph application_id '{}'", self.application_id, dgraph.application_id),
            ));
        }
        if self.project_id != dcontinuum.project_id {
            findings.push(finding(
                "project_mismatch",
                format!("D-Map project_id '{}' does not match D-Continuum project_id '{}'", self.project_id, dcontinuum.project_id),
            ));
        }

        let known_services: std::collections::HashMap<&str, &crate::core::canonical_dgraph::CanonicalDServV1> =
            dgraph.services.iter().map(|s| (s.service_id.as_str(), s)).collect();
        let known_nodes: std::collections::HashMap<&str, &crate::core::canonical_dcontinuum::CanonicalDNodeV1> =
            dcontinuum.nodes.iter().map(|n| (n.node_id.as_str(), n)).collect();

        let mut placed_services = std::collections::HashSet::new();
        for placement in &self.placements {
            placed_services.insert(placement.service_id.as_str());
            let service = known_services.get(placement.service_id.as_str());
            if service.is_none() {
                findings.push(finding("unknown_service", format!("placement names service '{}' not present in the referenced D-Graph", placement.service_id)));
            }
            let node = known_nodes.get(placement.node_id.as_str());
            if node.is_none() {
                findings.push(finding("unknown_node", format!("placement names node '{}' not present in the referenced D-Continuum", placement.node_id)));
            }
            if let (Some(service), Some(node)) = (service, node) {
                let required = &service.execution_requirement.dnode_abi;
                let node_supports = node.capabilities.dnode_abi_versions.iter().any(|v| v == required);
                if !node_supports {
                    findings.push(finding(
                        "incompatible_dnode_abi",
                        format!(
                            "service '{}' requires dnode_abi '{required}', but node '{}' declares capabilities {:?}",
                            placement.service_id, placement.node_id, node.capabilities.dnode_abi_versions
                        ),
                    ));
                }

                // Stage 110 P0 fix (independent review v3, Stage 112
                // resource readiness): FEASIBILITY validation only -- this
                // never selects, scores, or optimizes a placement, it only
                // rejects a caller-supplied mapping that is statically
                // impossible. Individual fit: this one service's static
                // requirement must not alone exceed the target node's
                // declared static capacity.
                let req_cpu = service.resource_requirement.cpu_cores as u64;
                let node_cpu = node.capacity.cpu_cores as u64;
                if req_cpu > node_cpu {
                    findings.push(finding(
                        "insufficient_node_cpu",
                        format!(
                            "service '{}' requires {req_cpu} cpu_cores, but node '{}' declares capacity.cpu_cores {node_cpu}",
                            placement.service_id, placement.node_id
                        ),
                    ));
                }
                let req_mem = service.resource_requirement.memory_bytes;
                let node_mem = node.capacity.memory_bytes;
                if req_mem > node_mem {
                    findings.push(finding(
                        "insufficient_node_memory",
                        format!(
                            "service '{}' requires {req_mem} memory_bytes, but node '{}' declares capacity.memory_bytes {node_mem}",
                            placement.service_id, placement.node_id
                        ),
                    ));
                }
            }
        }

        // Aggregate fit: when multiple D-Servs are placed on the SAME node,
        // their static requirements must not together exceed that node's
        // declared capacity, even if each individually fits. Checked
        // arithmetic: an overflow can never represent a satisfiable
        // aggregate, so it is folded into the same "exceeded" finding rather
        // than panicking or silently saturating to a misleading total.
        let mut node_cpu_committed: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
        let mut node_memory_committed: std::collections::HashMap<&str, u64> = std::collections::HashMap::new();
        for placement in &self.placements {
            if !known_nodes.contains_key(placement.node_id.as_str()) {
                continue;
            }
            let Some(service) = known_services.get(placement.service_id.as_str()) else {
                continue;
            };
            let cpu_entry = node_cpu_committed.entry(placement.node_id.as_str()).or_insert(0);
            *cpu_entry = match cpu_entry.checked_add(service.resource_requirement.cpu_cores as u64) {
                Some(sum) => sum,
                None => u64::MAX,
            };
            let mem_entry = node_memory_committed.entry(placement.node_id.as_str()).or_insert(0);
            *mem_entry = match mem_entry.checked_add(service.resource_requirement.memory_bytes) {
                Some(sum) => sum,
                None => u64::MAX,
            };
        }
        for (node_id, total_cpu) in &node_cpu_committed {
            if let Some(node) = known_nodes.get(node_id) {
                let capacity = node.capacity.cpu_cores as u64;
                if *total_cpu > capacity {
                    findings.push(finding(
                        "aggregate_node_cpu_exceeded",
                        format!("node '{node_id}' aggregate placed cpu_cores {total_cpu} exceeds declared capacity.cpu_cores {capacity}"),
                    ));
                }
            }
        }
        for (node_id, total_memory) in &node_memory_committed {
            if let Some(node) = known_nodes.get(node_id) {
                let capacity = node.capacity.memory_bytes;
                if *total_memory > capacity {
                    findings.push(finding(
                        "aggregate_node_memory_exceeded",
                        format!("node '{node_id}' aggregate placed memory_bytes {total_memory} exceeds declared capacity.memory_bytes {capacity}"),
                    ));
                }
            }
        }

        // Stage 110/v0.1: every D-Graph service must be placed exactly once
        // -- no missing mandatory placement. Duplicate placement is already
        // caught by `validate()`.
        for service in &dgraph.services {
            if !placed_services.contains(service.service_id.as_str()) {
                findings.push(finding("missing_placement", format!("D-Graph service '{}' has no placement in this D-Map", service.service_id)));
            }
        }

        findings
    }

    /// THE complete, fail-closed acceptance-gate entry point (Stage 110 P1
    /// fix, independent review v2). Aggregates `self.validate()`,
    /// `dgraph.validate()`, `dcontinuum.validate()`, and
    /// `self.validate_cross_references(dgraph, dcontinuum)` -- a caller
    /// (Stage 111's natural D-Deploy acceptance usage) can call ONLY this
    /// function and be certain that a non-empty result means the bundle is
    /// unsafe in SOME way, structural or cross-contract. An empty result
    /// means every one of the four checks passed.
    ///
    /// No deduplication is performed across the four result sets: a finding
    /// that happens to be reachable from more than one check is reported
    /// once per check that reaches it. Correctness (never silently dropping
    /// a real finding) is preferred over cosmetic deduplication.
    pub fn validate_against(&self, dgraph: &CanonicalDGraphV1, dcontinuum: &CanonicalDContinuumV1) -> Vec<CanonicalValidationFinding> {
        let mut findings = self.validate();
        findings.extend(dgraph.validate());
        findings.extend(dcontinuum.validate());
        findings.extend(self.validate_cross_references(dgraph, dcontinuum));
        findings
    }
}

/// Never itself embedded in another D-Map's digest input (no self-reference
/// chain): `dgraph_ref.digest`/`dcontinuum_ref.digest` are digests of OTHER
/// objects, recorded as plain string fields like any other, and this
/// function hashes the whole `CanonicalDMapV1` including those fields
/// exactly once, the same way any other field is hashed. `placements` is a
/// canonically unordered set keyed by `service_id` (Stage 110/v0.1
/// cardinality already guarantees at most one entry per key) and is sorted
/// before hashing for the same order-independence reason as the other two
/// contracts.
pub fn recompute_dmap_digest(dmap: &CanonicalDMapV1) -> Result<String, String> {
    let mut sorted = dmap.clone();
    sorted.placements.sort_by(|a, b| a.service_id.cmp(&b.service_id));
    let value: Value = serde_json::to_value(&sorted).map_err(|error| format!("serialize CanonicalDMapV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dcontinuum::{CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodePlatformV1};
    use crate::core::canonical_dgraph::{CanonicalDCallV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1, CanonicalDServV1, DNODE_ABI_V0};
    use crate::core::canonical_dcontinuum::CanonicalNodeCapacityV1;

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 { cpu_cores, memory_bytes }
    }

    fn cap(cpu_cores: u32, memory_bytes: u64) -> CanonicalNodeCapacityV1 {
        CanonicalNodeCapacityV1 { cpu_cores, memory_bytes }
    }

    fn sample_dgraph() -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: crate::core::canonical_dgraph::CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:smart-sentinel-lab".to_string(),
            dgraph_id: "dgraph:smart-sentinel-lab".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 {
                    service_definition: None, service_id: "chirpstack-gateway-bridge".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 536_870_912),
                },
                CanonicalDServV1 {
                    service_definition: None, service_id: "smart-adapter".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 268_435_456),
                },
            ],
            calls: vec![CanonicalDCallV1 {
                call_id: "bridge-to-adapter".to_string(),
                source_service_id: "chirpstack-gateway-bridge".to_string(),
                target_service_id: "smart-adapter".to_string(),
            }],
        }
    }

    fn sample_dcontinuum() -> CanonicalDContinuumV1 {
        CanonicalDContinuumV1 {
            schema: crate::core::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
            project_id: "monitor-f49c8".to_string(),
            dcontinuum_id: "continuum:smart-sentinel-lab".to_string(),
            revision: 1,
            nodes: vec![
                CanonicalDNodeV1 {
                    node_id: "fog-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                    capacity: cap(4, 4_294_967_296),
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![DNODE_ABI_V0.to_string()] },
                },
                CanonicalDNodeV1 {
                    node_id: "cloud-01".to_string(),
                    platform: CanonicalNodePlatformV1 { os: "linux".to_string(), architecture: "amd64".to_string() },
                    capacity: cap(4, 4_294_967_296),
                    // No D-Node ABI capability: this node represents a
                    // restricted/host-only element in these unit tests,
                    // used below to prove `incompatible_dnode_abi`.
                    capabilities: CanonicalNodeCapabilitiesV1 { dnode_abi_versions: vec![] },
                },
            ],
            links: vec![],
        }
    }

    fn sample_dmap() -> CanonicalDMapV1 {
        let dgraph = sample_dgraph();
        let dcontinuum = sample_dcontinuum();
        CanonicalDMapV1 {
            schema: CANONICAL_DMAP_SCHEMA.to_string(),
            project_id: dcontinuum.project_id.clone(),
            application_id: dgraph.application_id.clone(),
            dmap_id: "dmap:smart-sentinel-lab:v1".to_string(),
            revision: 1,
            dgraph_ref: CanonicalContractReferenceV1 {
                id: dgraph.dgraph_id.clone(),
                revision: dgraph.revision,
                digest: recompute_dgraph_digest(&dgraph).unwrap(),
            },
            dcontinuum_ref: CanonicalContractReferenceV1 {
                id: dcontinuum.dcontinuum_id.clone(),
                revision: dcontinuum.revision,
                digest: recompute_dcontinuum_digest(&dcontinuum).unwrap(),
            },
            placements: vec![
                CanonicalDServPlacementV1 { service_id: "chirpstack-gateway-bridge".to_string(), node_id: "fog-01".to_string() },
                CanonicalDServPlacementV1 { service_id: "smart-adapter".to_string(), node_id: "fog-01".to_string() },
            ],
        }
    }

    #[test]
    fn valid_complete_mapping_has_no_findings() {
        let dmap = sample_dmap();
        assert_eq!(dmap.validate(), Vec::new());
        assert_eq!(dmap.validate_against(&sample_dgraph(), &sample_dcontinuum()), Vec::new());
    }

    // ---- Stage 110 P1 fix (independent review v2): validate_against ---- //
    // ---- is a complete, fail-closed acceptance gate -------------------- //

    /// The exact counterexample the review reproduced: a D-Graph service
    /// and a D-Continuum node both declare the SAME invalid ABI string.
    /// `validate_cross_references` alone would report zero findings (the
    /// raw strings match each other). The complete `validate_against` MUST
    /// still reject, because it also runs
    /// `dgraph.validate()`/`dcontinuum.validate()`, both of which
    /// independently flag `invalid_dnode_abi`/`invalid_dnode_abi_capability`.
    #[test]
    fn complete_validation_rejects_matching_but_mutually_invalid_abi_strings() {
        let mut dgraph = sample_dgraph();
        // smart-adapter's requirement becomes invalid...
        dgraph.services[1].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        let mut dcontinuum = sample_dcontinuum();
        // ...and fog-01 -- smart-adapter's ALREADY-placed target node,
        // per sample_dmap() below (which ALSO places chirpstack-gateway-bridge
        // on fog-01, still requiring DNODE_ABI_V0) -- declares the SAME
        // invalid string in ADDITION to the valid one, so a raw-string
        // capability comparison for smart-adapter sees a "match" and
        // reports nothing, while chirpstack-gateway-bridge's genuine
        // requirement remains satisfied.
        dcontinuum.nodes[0].capabilities.dnode_abi_versions = vec![DNODE_ABI_V0.to_string(), "datum-dnode/999".to_string()];

        // Recompute exact refs against the (individually invalid) content,
        // exactly as the review's counterexample did -- proving this is not
        // merely a reference-mismatch that validate_cross_references would
        // catch anyway.
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        dmap.dcontinuum_ref.digest = recompute_dcontinuum_digest(&dcontinuum).unwrap();

        // The partial helper alone reports nothing -- this documents the
        // exact gap the fix closes, it does not endorse using the partial
        // helper as an acceptance gate.
        assert_eq!(
            dmap.validate_cross_references(&dgraph, &dcontinuum),
            Vec::new(),
            "documents the exact gap: raw-string comparison alone sees nothing wrong"
        );

        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "invalid_dnode_abi"), "findings: {complete:?}");
        assert!(complete.iter().any(|f| f.code == "invalid_dnode_abi_capability"), "findings: {complete:?}");
    }

    /// Duplicate D-Graph service IDs must not disappear through
    /// `validate_cross_references`'s internal `HashMap` last-wins lookup --
    /// the AGGREGATE `validate_against` must still reject via
    /// `dgraph.validate()`'s own `duplicate_service_id` finding, regardless
    /// of what the cross-reference HashMap does internally.
    #[test]
    fn complete_validation_rejects_duplicate_dgraph_service_ids_despite_hashmap_lookup() {
        let mut dgraph = sample_dgraph();
        let dup = dgraph.services[0].clone();
        dgraph.services.push(dup); // "chirpstack-gateway-bridge" now appears twice
        let dcontinuum = sample_dcontinuum();
        let dmap = sample_dmap();

        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "duplicate_service_id"), "findings: {complete:?}");
    }

    /// Symmetric proof for D-Continuum nodes.
    #[test]
    fn complete_validation_rejects_duplicate_dcontinuum_node_ids_despite_hashmap_lookup() {
        let dgraph = sample_dgraph();
        let mut dcontinuum = sample_dcontinuum();
        let dup = dcontinuum.nodes[0].clone();
        dcontinuum.nodes.push(dup); // "fog-01" now appears twice
        let dmap = sample_dmap();

        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "duplicate_node_id"), "findings: {complete:?}");
    }

    /// A structurally-invalid D-Map (duplicate placement) whose references
    /// are otherwise perfectly valid against the real D-Graph/D-Continuum
    /// must still be rejected by the complete gate.
    #[test]
    fn complete_validation_rejects_structurally_invalid_dmap_with_valid_references() {
        let dgraph = sample_dgraph();
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        let dup = dmap.placements[0].clone();
        dmap.placements.push(dup);

        assert_eq!(
            dmap.validate_cross_references(&dgraph, &dcontinuum),
            Vec::new(),
            "cross-references alone are fine; only self.validate() catches this"
        );
        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "duplicate_placement"), "findings: {complete:?}");
    }

    #[test]
    fn same_node_hosting_multiple_services_is_legal() {
        // Both services placed on fog-01 above -- must not be rejected as a
        // conflict; only a service placed MORE THAN ONCE is a violation.
        let dmap = sample_dmap();
        assert!(!dmap.validate().iter().any(|f| f.code == "duplicate_placement"));
    }

    // ---- Stage 110 P0 fix (independent review v3): resource FEASIBILITY -- //
    // ---- validation, never selection/scoring/optimization -------------- //

    #[test]
    fn multiple_services_within_aggregate_capacity_remain_valid() {
        // chirpstack-gateway-bridge (1 cpu/512MiB) + smart-adapter (1 cpu/256MiB)
        // both on fog-01 (4 cpu/4GiB): individually and in aggregate, fits.
        let findings = sample_dmap().validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(!findings.iter().any(|f| f.code.contains("cpu") || f.code.contains("memory")), "findings: {findings:?}");
    }

    #[test]
    fn service_cpu_requirement_larger_than_node_capacity_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.cpu_cores = 100; // fog-01's capacity is 4
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(findings.iter().any(|f| f.code == "insufficient_node_cpu"), "findings: {findings:?}");
    }

    #[test]
    fn service_memory_requirement_larger_than_node_capacity_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[1].resource_requirement.memory_bytes = 8 * 1024 * 1024 * 1024; // 8 GiB > fog-01's 4 GiB
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(findings.iter().any(|f| f.code == "insufficient_node_memory"), "findings: {findings:?}");
    }

    /// The exact scenario the review specified: two services that EACH fit
    /// individually (3 cores <= fog-01's 4-core capacity) but whose combined
    /// requirement (6 cores) does not.
    #[test]
    fn aggregate_cpu_overcommit_rejected_even_when_each_service_individually_fits() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.cpu_cores = 3;
        dgraph.services[1].resource_requirement.cpu_cores = 3;
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(!findings.iter().any(|f| f.code == "insufficient_node_cpu"), "individual fit must not fire: {findings:?}");
        assert!(findings.iter().any(|f| f.code == "aggregate_node_cpu_exceeded"), "findings: {findings:?}");
    }

    #[test]
    fn aggregate_memory_overcommit_rejected_even_when_each_service_individually_fits() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.memory_bytes = 3_221_225_472; // 3 GiB, fits fog-01's 4 GiB alone
        dgraph.services[1].resource_requirement.memory_bytes = 3_221_225_472; // 3 GiB, fits alone; combined = 6 GiB
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(!findings.iter().any(|f| f.code == "insufficient_node_memory"), "individual fit must not fire: {findings:?}");
        assert!(findings.iter().any(|f| f.code == "aggregate_node_memory_exceeded"), "findings: {findings:?}");
    }

    /// PASS M (independent review v3): the resource-feasibility checks only
    /// ever REJECT; they never choose a different node, reorder placements,
    /// or mutate the D-Map -- confirmed structurally by this test never
    /// observing a changed `placements` list after a rejected call.
    #[test]
    fn resource_feasibility_rejection_never_mutates_the_dmap() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.cpu_cores = 100;
        let dmap = sample_dmap();
        let before = dmap.placements.clone();
        let _ = dmap.validate_against(&dgraph, &sample_dcontinuum());
        assert_eq!(dmap.placements, before, "validate_against must never mutate placements");
    }

    #[test]
    fn graph_identity_reference_mismatch_rejected() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.revision = 999;
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dgraph_reference_mismatch"));
    }

    #[test]
    fn continuum_identity_reference_mismatch_rejected() {
        let mut dmap = sample_dmap();
        dmap.dcontinuum_ref.id = "continuum:wrong".to_string();
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dcontinuum_reference_mismatch"));
    }

    #[test]
    fn wrong_dgraph_digest_rejected_even_with_correct_id_and_revision() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = "sha256:".to_string() + &"a".repeat(64);
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dgraph_digest_mismatch"));
    }

    #[test]
    fn wrong_dcontinuum_digest_rejected_even_with_correct_id_and_revision() {
        let mut dmap = sample_dmap();
        dmap.dcontinuum_ref.digest = "sha256:".to_string() + &"b".repeat(64);
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dcontinuum_digest_mismatch"));
    }

    // ---- Stage 110 P1 fix (independent review v3): reference digest SHAPE //
    // ---- is validated structurally, before any cross-contract comparison //

    #[test]
    fn empty_dgraph_ref_digest_rejected_as_invalid_format() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = String::new();
        assert!(dmap.validate().iter().any(|f| f.code == "invalid_digest_format"));
    }

    #[test]
    fn bare_hex_dgraph_ref_digest_rejected_as_invalid_format() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = "a".repeat(64); // missing the "sha256:" prefix
        assert!(dmap.validate().iter().any(|f| f.code == "invalid_digest_format"));
    }

    #[test]
    fn uppercase_hex_dcontinuum_ref_digest_rejected_as_invalid_format() {
        let mut dmap = sample_dmap();
        dmap.dcontinuum_ref.digest = format!("sha256:{}", "A".repeat(64));
        assert!(dmap.validate().iter().any(|f| f.code == "invalid_digest_format"));
    }

    #[test]
    fn wrong_length_dgraph_ref_digest_rejected_as_invalid_format() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = format!("sha256:{}", "a".repeat(63));
        assert!(dmap.validate().iter().any(|f| f.code == "invalid_digest_format"));
    }

    /// The P1 fix must not weaken content-equality validation: a
    /// well-FORMED but WRONG digest still triggers the cross-contract
    /// mismatch finding, exactly as before.
    #[test]
    fn well_formed_but_wrong_digest_still_triggers_mismatch_not_only_format_error() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = "sha256:".to_string() + &"a".repeat(64);
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(!findings.iter().any(|f| f.code == "invalid_digest_format"), "findings: {findings:?}");
        assert!(findings.iter().any(|f| f.code == "dgraph_digest_mismatch"), "findings: {findings:?}");
    }

    #[test]
    fn unknown_service_rejected() {
        let mut dmap = sample_dmap();
        dmap.placements.push(CanonicalDServPlacementV1 { service_id: "does-not-exist".to_string(), node_id: "fog-01".to_string() });
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "unknown_service"));
    }

    #[test]
    fn unknown_node_rejected() {
        let mut dmap = sample_dmap();
        dmap.placements[0].node_id = "does-not-exist".to_string();
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "unknown_node"));
    }

    #[test]
    fn incompatible_dnode_abi_target_rejected() {
        // smart-adapter requires DNODE_ABI_V0; cloud-01 declares no D-Node
        // ABI capability at all in these unit-test fixtures.
        let mut dmap = sample_dmap();
        dmap.placements[1].node_id = "cloud-01".to_string();
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "incompatible_dnode_abi"));
    }

    #[test]
    fn duplicate_placement_cardinality_violation_rejected() {
        let mut dmap = sample_dmap();
        let dup = dmap.placements[0].clone();
        dmap.placements.push(dup);
        assert!(dmap.validate().iter().any(|f| f.code == "duplicate_placement"));
    }

    #[test]
    fn missing_mandatory_placement_rejected() {
        let mut dmap = sample_dmap();
        dmap.placements.pop();
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "missing_placement"));
    }

    #[test]
    fn application_mismatch_rejected() {
        let mut dmap = sample_dmap();
        dmap.application_id = "app:another-application".to_string();
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "application_mismatch"));
    }

    #[test]
    fn project_mismatch_rejected() {
        let mut dmap = sample_dmap();
        dmap.project_id = "another-project".to_string();
        let findings = dmap.validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "project_mismatch"));
    }

    #[test]
    fn dmap_for_another_application_graph_is_rejected() {
        let mut other_dgraph = sample_dgraph();
        other_dgraph.application_id = "app:other".to_string();
        other_dgraph.dgraph_id = "dgraph:other".to_string();
        let dmap = sample_dmap();
        let findings = dmap.validate_against(&other_dgraph, &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dgraph_reference_mismatch" || f.code == "application_mismatch"));
    }

    #[test]
    fn graph_and_continuum_from_another_project_are_rejected() {
        let mut other_dcontinuum = sample_dcontinuum();
        other_dcontinuum.project_id = "some-other-project".to_string();
        other_dcontinuum.dcontinuum_id = "continuum:other".to_string();
        let dmap = sample_dmap();
        let findings = dmap.validate_against(&sample_dgraph(), &other_dcontinuum);
        assert!(findings.iter().any(|f| f.code == "dcontinuum_reference_mismatch"));
    }

    #[test]
    fn stale_revision_with_correct_id_rejected() {
        let mut newer_dgraph = sample_dgraph();
        newer_dgraph.revision = 2;
        let dmap = sample_dmap(); // still references revision 1
        let findings = dmap.validate_against(&newer_dgraph, &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dgraph_reference_mismatch"));
    }

    #[test]
    fn correct_revision_with_wrong_digest_rejected() {
        // Same id/revision, but the underlying content actually changed
        // (a service's resource requirement flipped) without the D-Map's
        // recorded digest being updated to match -- a stale-content proof,
        // not merely a stale-revision one.
        let mut mutated_dgraph = sample_dgraph();
        mutated_dgraph.services[0].resource_requirement.cpu_cores += 1;
        let dmap = sample_dmap();
        let findings = dmap.validate_against(&mutated_dgraph, &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dgraph_digest_mismatch"));
    }

    #[test]
    fn digest_is_deterministic_and_order_independent() {
        let d1 = recompute_dmap_digest(&sample_dmap()).unwrap();
        let d2 = recompute_dmap_digest(&sample_dmap()).unwrap();
        assert_eq!(d1, d2);
        let mut reordered = sample_dmap();
        reordered.placements.reverse();
        assert_eq!(d1, recompute_dmap_digest(&reordered).unwrap());
    }

    #[test]
    fn digest_changes_when_a_placement_changes() {
        let a = sample_dmap();
        let mut b = sample_dmap();
        b.placements[0].node_id = "cloud-01".to_string();
        assert_ne!(recompute_dmap_digest(&a).unwrap(), recompute_dmap_digest(&b).unwrap());
    }

    #[test]
    fn round_trip_serialization_preserves_all_identity() {
        let dmap = sample_dmap();
        let json = serde_json::to_string(&dmap).unwrap();
        let round_tripped: CanonicalDMapV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped, dmap);
    }

    /// Real cross-crate mapper fixture, exercising all three canonical-v1
    /// fixtures together -- see
    /// `canonical_dgraph::tests::agent_cross_crate_fixture_produces_the_expected_digest`
    /// for the full pattern/rationale.
    #[test]
    fn agent_cross_crate_fixture_produces_the_expected_digest_and_validates() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("canonical-v1");

        let dgraph_text = std::fs::read_to_string(dir.join("dgraph.smart-sentinel-lab.json")).unwrap();
        let dgraph: CanonicalDGraphV1 = serde_json::from_str(&dgraph_text).unwrap();
        assert_eq!(dgraph.validate(), Vec::new());

        let dcontinuum_text = std::fs::read_to_string(dir.join("dcontinuum.smart-sentinel-lab.json")).unwrap();
        let dcontinuum: CanonicalDContinuumV1 = serde_json::from_str(&dcontinuum_text).unwrap();
        assert_eq!(dcontinuum.validate(), Vec::new());

        let dmap_text = std::fs::read_to_string(dir.join("dmap.smart-sentinel-lab.json")).unwrap();
        let raw: Value = serde_json::from_str(&dmap_text).unwrap();
        let expected_digest = raw["expected_digest_sha256"].as_str().unwrap().to_string();
        let dmap: CanonicalDMapV1 = serde_json::from_str(&dmap_text).expect("fixture must deserialize as this crate's CanonicalDMapV1");

        assert_eq!(dmap.validate(), Vec::new(), "the positive fixture must validate cleanly (structural)");
        assert_eq!(dmap.validate_against(&dgraph, &dcontinuum), Vec::new(), "the positive fixture must validate cleanly (cross-contract)");

        let digest = recompute_dmap_digest(&dmap).unwrap();
        assert_eq!(digest, expected_digest, "this crate's recomputed digest must match the fixture's recorded expected_digest_sha256");
    }
}
