//! `datum.dmap/1` (Stage 110, ADR-0017) -- the canonical placement/mapping
//! CONTRACT. Independently mirrored (not shared) by
//! `dserver::core::canonical_dmap`; see that module's doc comment for the
//! full rationale (why this is contract validation only, never a placement
//! algorithm; single-placement-per-D-Serv cardinality for Stage 110/v0.1;
//! no D-IoT/D-Forward placement or ABI/host-kind negotiation -- host-
//! independent D-Node ABI COMPATIBILITY validation exists (`incompatible_dnode_abi`),
//! ABI negotiation/selection does not; no D-Deploy proposal workflow).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::canonical_dcontinuum::{recompute_dcontinuum_digest, CanonicalDContinuumV1};
use crate::agent::canonical_dgraph::{recompute_dgraph_digest, CanonicalDGraphV1, CanonicalValidationFinding};
use crate::agent::jcs;

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
    /// and placement cardinality (Stage 110/v0.1: single placement per
    /// D-Serv). Never touches the referenced D-Graph/D-Continuum.
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
    /// checks ONLY. Deliberately does NOT re-run `self.validate()`,
    /// `dgraph.validate()`, or `dcontinuum.validate()` -- this is the
    /// low-level building block, not the complete acceptance gate. See the
    /// identical dserver-side doc comment for the full Stage 110 P1 fix
    /// rationale: structurally invalid participating contracts may contain
    /// mutually matching raw values, which is why cross-reference
    /// comparison alone is not a complete acceptance gate -- exactly why
    /// `validate_against` below exists as the real entry point.
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

        let known_services: std::collections::HashMap<&str, &crate::agent::canonical_dgraph::CanonicalDServV1> =
            dgraph.services.iter().map(|s| (s.service_id.as_str(), s)).collect();
        let known_nodes: std::collections::HashMap<&str, &crate::agent::canonical_dcontinuum::CanonicalDNodeV1> =
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
                // resource readiness): FEASIBILITY validation only -- see
                // dserver::core::canonical_dmap for the full rationale.
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

        // Aggregate fit: see dserver::core::canonical_dmap for the full
        // rationale (checked arithmetic; overflow folds into "exceeded").
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
    /// `self.validate_cross_references(dgraph, dcontinuum)`. No
    /// deduplication across the four result sets -- correctness is
    /// preferred over cosmetic deduplication.
    pub fn validate_against(&self, dgraph: &CanonicalDGraphV1, dcontinuum: &CanonicalDContinuumV1) -> Vec<CanonicalValidationFinding> {
        let mut findings = self.validate();
        findings.extend(dgraph.validate());
        findings.extend(dcontinuum.validate());
        findings.extend(self.validate_cross_references(dgraph, dcontinuum));
        findings
    }
}

/// `placements` is a canonically unordered set keyed by `service_id`; sorted
/// before hashing for order independence, same as the other two contracts.
pub fn recompute_dmap_digest(dmap: &CanonicalDMapV1) -> Result<String, String> {
    let mut sorted = dmap.clone();
    sorted.placements.sort_by(|a, b| a.service_id.cmp(&b.service_id));
    let value: Value = serde_json::to_value(&sorted).map_err(|error| format!("serialize CanonicalDMapV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::canonical_dcontinuum::{CanonicalDNodeV1, CanonicalNodeCapabilitiesV1, CanonicalNodeCapacityV1, CanonicalNodePlatformV1};
    use crate::agent::canonical_dgraph::{CanonicalDCallV1, CanonicalDServExecutionRequirementV1, CanonicalDServResourceRequirementV1, CanonicalDServV1, DNODE_ABI_V0};

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 { cpu_cores, memory_bytes }
    }

    fn cap(cpu_cores: u32, memory_bytes: u64) -> CanonicalNodeCapacityV1 {
        CanonicalNodeCapacityV1 { cpu_cores, memory_bytes }
    }

    fn sample_dgraph() -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: crate::agent::canonical_dgraph::CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:smart-sentinel-lab".to_string(),
            dgraph_id: "dgraph:smart-sentinel-lab".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 {
                    service_id: "chirpstack-gateway-bridge".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 536_870_912),
                },
                CanonicalDServV1 {
                    service_id: "smart-adapter".to_string(),
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
            schema: crate::agent::canonical_dcontinuum::CANONICAL_DCONTINUUM_SCHEMA.to_string(),
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
                    // No D-Node ABI capability: used below to prove
                    // `incompatible_dnode_abi`.
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

    /// See the identical dserver-side test for the full rationale.
    #[test]
    fn complete_validation_rejects_matching_but_mutually_invalid_abi_strings() {
        let mut dgraph = sample_dgraph();
        dgraph.services[1].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        let mut dcontinuum = sample_dcontinuum();
        dcontinuum.nodes[0].capabilities.dnode_abi_versions = vec![DNODE_ABI_V0.to_string(), "datum-dnode/999".to_string()];

        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        dmap.dcontinuum_ref.digest = recompute_dcontinuum_digest(&dcontinuum).unwrap();

        assert_eq!(
            dmap.validate_cross_references(&dgraph, &dcontinuum),
            Vec::new(),
            "documents the exact gap: raw-string comparison alone sees nothing wrong"
        );

        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "invalid_dnode_abi"), "findings: {complete:?}");
        assert!(complete.iter().any(|f| f.code == "invalid_dnode_abi_capability"), "findings: {complete:?}");
    }

    #[test]
    fn complete_validation_rejects_duplicate_dgraph_service_ids_despite_hashmap_lookup() {
        let mut dgraph = sample_dgraph();
        let dup = dgraph.services[0].clone();
        dgraph.services.push(dup);
        let dcontinuum = sample_dcontinuum();
        let dmap = sample_dmap();

        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "duplicate_service_id"), "findings: {complete:?}");
    }

    #[test]
    fn complete_validation_rejects_duplicate_dcontinuum_node_ids_despite_hashmap_lookup() {
        let dgraph = sample_dgraph();
        let mut dcontinuum = sample_dcontinuum();
        let dup = dcontinuum.nodes[0].clone();
        dcontinuum.nodes.push(dup);
        let dmap = sample_dmap();

        let complete = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(complete.iter().any(|f| f.code == "duplicate_node_id"), "findings: {complete:?}");
    }

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
        let dmap = sample_dmap();
        assert!(!dmap.validate().iter().any(|f| f.code == "duplicate_placement"));
    }

    // ---- Stage 110 P0 fix (independent review v3): resource FEASIBILITY -- //

    #[test]
    fn multiple_services_within_aggregate_capacity_remain_valid() {
        let findings = sample_dmap().validate_against(&sample_dgraph(), &sample_dcontinuum());
        assert!(!findings.iter().any(|f| f.code.contains("cpu") || f.code.contains("memory")), "findings: {findings:?}");
    }

    #[test]
    fn service_cpu_requirement_larger_than_node_capacity_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.cpu_cores = 100;
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(findings.iter().any(|f| f.code == "insufficient_node_cpu"), "findings: {findings:?}");
    }

    #[test]
    fn service_memory_requirement_larger_than_node_capacity_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[1].resource_requirement.memory_bytes = 8 * 1024 * 1024 * 1024;
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(findings.iter().any(|f| f.code == "insufficient_node_memory"), "findings: {findings:?}");
    }

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
        dgraph.services[0].resource_requirement.memory_bytes = 3_221_225_472;
        dgraph.services[1].resource_requirement.memory_bytes = 3_221_225_472;
        let dcontinuum = sample_dcontinuum();
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = recompute_dgraph_digest(&dgraph).unwrap();
        let findings = dmap.validate_against(&dgraph, &dcontinuum);
        assert!(!findings.iter().any(|f| f.code == "insufficient_node_memory"), "individual fit must not fire: {findings:?}");
        assert!(findings.iter().any(|f| f.code == "aggregate_node_memory_exceeded"), "findings: {findings:?}");
    }

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

    #[test]
    fn empty_dgraph_ref_digest_rejected_as_invalid_format() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = String::new();
        assert!(dmap.validate().iter().any(|f| f.code == "invalid_digest_format"));
    }

    #[test]
    fn bare_hex_dgraph_ref_digest_rejected_as_invalid_format() {
        let mut dmap = sample_dmap();
        dmap.dgraph_ref.digest = "a".repeat(64);
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
        let dmap = sample_dmap();
        let findings = dmap.validate_against(&newer_dgraph, &sample_dcontinuum());
        assert!(findings.iter().any(|f| f.code == "dgraph_reference_mismatch"));
    }

    #[test]
    fn correct_revision_with_wrong_digest_rejected() {
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

    #[test]
    fn schema_constant_matches_dserver_literal() {
        assert_eq!(CANONICAL_DMAP_SCHEMA, "datum.dmap/1");
    }

    /// Real cross-crate mapper fixture, exercising ALL THREE canonical-v1
    /// fixtures together: the D-Map deserializes and validates against the
    /// SAME D-Graph/D-Continuum fixtures `canonical_dgraph`/
    /// `canonical_dcontinuum`'s own cross-crate tests independently verify.
    /// Proves not just digest parity but full referential/capability
    /// validation parity across the wire boundary.
    #[test]
    fn dserver_cross_crate_fixture_produces_the_expected_digest_and_validates() {
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
