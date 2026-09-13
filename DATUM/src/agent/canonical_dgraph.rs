//! `datum.dgraph/1` (Stage 110, ADR-0017) -- the canonical APPLICATION-PLANE
//! D-Graph: `G = (V, E)` with `V` = D-Serv and `E` = D-Call (ADR-0001).
//!
//! Independently mirrored (not shared) by `dserver::core::canonical_dgraph`,
//! per this project's established no-shared-crate discipline (ADR-0007):
//! parity is proven by a real cross-crate fixture + test on each side, never
//! by type sharing. See that module's doc comment for the full rationale
//! (why this is a NEW type distinct from `OperationalDGraph`/the legacy
//! composite graph, why D-Call cycles are accepted, why no runtime
//! observation/placement ever enters this contract).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::jcs;

pub const CANONICAL_DGRAPH_SCHEMA: &str = "datum.dgraph/1";

/// Stage 110 P0 fix (independent review v4, D-Serv host invariance). See
/// `dserver::core::canonical_dgraph::DNODE_ABI_V0` for the full rationale --
/// the ONLY accepted D-Node ABI for v0.1 (ADR-0003), host-independent,
/// deliberately NOT Stage 108's `RuntimeKind`.
pub const DNODE_ABI_V0: &str = "datum-dnode/0";

/// One structured validation finding, mirroring the `{code, message}` shape
/// `scripts/check_datum_contract_fixtures.py` already uses for the Stage 107
/// hand-authored fixtures. `validate` collects every finding rather than
/// stopping at the first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalValidationFinding {
    pub code: &'static str,
    pub message: String,
}

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDGraphV1 {
    pub schema: String,
    /// ADR-0009: distinct from `project_id`. The D-Graph itself never
    /// carries a `project_id`.
    pub application_id: String,
    pub dgraph_id: String,
    pub revision: u64,
    #[serde(default)]
    pub services: Vec<CanonicalDServV1>,
    #[serde(default)]
    pub calls: Vec<CanonicalDCallV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDServV1 {
    pub service_id: String,
    pub execution_requirement: CanonicalDServExecutionRequirementV1,
    /// Stage 110 P0 fix (independent review v3, Stage 112 resource
    /// readiness): the SMALLEST static resource surface Stage 112's
    /// deterministic resource-FEASIBILITY placement needs. Declared
    /// application semantics, never an observed/live value.
    pub resource_requirement: CanonicalDServResourceRequirementV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDServExecutionRequirementV1 {
    /// Stage 110 P0 fix (independent review v4): host-independent D-Node
    /// ABI (ADR-0003), for v0.1 always `DNODE_ABI_V0`. Deliberately NOT
    /// `runtime_kind`/`host_kind`/`"container"`/`"native_process"`.
    pub dnode_abi: String,
}

/// See `dserver::core::canonical_dgraph::CanonicalDServResourceRequirementV1`
/// for the full rationale. Exact integer quantities only; no live/current/
/// available/used field is or will ever be added here.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDServResourceRequirementV1 {
    pub cpu_cores: u32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDCallV1 {
    pub call_id: String,
    pub source_service_id: String,
    pub target_service_id: String,
}

impl CanonicalDGraphV1 {
    /// Structural, self-contained validation. Collects every finding;
    /// returns `Vec::new()` iff valid. D-Call cycles (including a self-call)
    /// are NEVER rejected here (D-4, ADR-0001) -- only a dangling endpoint.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DGRAPH_SCHEMA {
            findings.push(finding("schema_invalid", format!("schema must be '{CANONICAL_DGRAPH_SCHEMA}', got '{}'", self.schema)));
        }
        if self.application_id.trim().is_empty() {
            findings.push(finding("missing_field", "application_id must be a non-empty string"));
        }
        if self.dgraph_id.trim().is_empty() {
            findings.push(finding("missing_field", "dgraph_id must be a non-empty string"));
        }
        if self.revision < 1 {
            findings.push(finding("revision_invalid", "revision must be an integer >= 1"));
        }

        let mut seen_service_ids = std::collections::HashSet::new();
        let mut known_services = std::collections::HashSet::new();
        for service in &self.services {
            if service.service_id.trim().is_empty() {
                findings.push(finding("missing_field", "service_id must be a non-empty string"));
            } else if !seen_service_ids.insert(service.service_id.as_str()) {
                findings.push(finding("duplicate_service_id", format!("duplicate service_id '{}'", service.service_id)));
            }
            known_services.insert(service.service_id.as_str());
            if service.execution_requirement.dnode_abi != DNODE_ABI_V0 {
                findings.push(finding(
                    "invalid_dnode_abi",
                    format!(
                        "service '{}' execution_requirement.dnode_abi '{}' is not a recognised D-Node ABI (expected '{DNODE_ABI_V0}')",
                        service.service_id, service.execution_requirement.dnode_abi
                    ),
                ));
            }
            if service.resource_requirement.cpu_cores < 1 {
                findings.push(finding(
                    "invalid_cpu_requirement",
                    format!("service '{}' resource_requirement.cpu_cores must be >= 1, got {}", service.service_id, service.resource_requirement.cpu_cores),
                ));
            }
            if service.resource_requirement.memory_bytes < 1 {
                findings.push(finding(
                    "invalid_memory_requirement",
                    format!("service '{}' resource_requirement.memory_bytes must be >= 1, got {}", service.service_id, service.resource_requirement.memory_bytes),
                ));
            }
        }

        let mut seen_call_ids = std::collections::HashSet::new();
        for call in &self.calls {
            if call.call_id.trim().is_empty() {
                findings.push(finding("missing_field", "call_id must be a non-empty string"));
            } else if !seen_call_ids.insert(call.call_id.as_str()) {
                findings.push(finding("duplicate_call_id", format!("duplicate call_id '{}'", call.call_id)));
            }
            if !known_services.contains(call.source_service_id.as_str()) {
                findings.push(finding(
                    "dangling_call_source",
                    format!("D-Call '{}' source_service_id '{}' does not name a declared service", call.call_id, call.source_service_id),
                ));
            }
            if !known_services.contains(call.target_service_id.as_str()) {
                findings.push(finding(
                    "dangling_call_target",
                    format!("D-Call '{}' target_service_id '{}' does not name a declared service", call.call_id, call.target_service_id),
                ));
            }
        }

        findings
    }
}

/// Canonical JCS+SHA-256 digest (ADR-0006), never self-referential.
/// `services`/`calls` are canonically UNORDERED SETS keyed by
/// `service_id`/`call_id` (ADR-0017): a deterministic deep-sorted copy is
/// hashed, so reordering either array never changes the digest. The wire
/// JSON itself is never reordered by this function.
pub fn recompute_dgraph_digest(dgraph: &CanonicalDGraphV1) -> Result<String, String> {
    let mut sorted = dgraph.clone();
    sorted.services.sort_by(|a, b| a.service_id.cmp(&b.service_id));
    sorted.calls.sort_by(|a, b| a.call_id.cmp(&b.call_id));
    let value: Value = serde_json::to_value(&sorted).map_err(|error| format!("serialize CanonicalDGraphV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn res(cpu_cores: u32, memory_bytes: u64) -> CanonicalDServResourceRequirementV1 {
        CanonicalDServResourceRequirementV1 { cpu_cores, memory_bytes }
    }

    fn sample_dgraph() -> CanonicalDGraphV1 {
        CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
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
                CanonicalDServV1 {
                    service_id: "atom-controller".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(2, 1_073_741_824),
                },
            ],
            calls: vec![
                CanonicalDCallV1 {
                    call_id: "bridge-to-adapter".to_string(),
                    source_service_id: "chirpstack-gateway-bridge".to_string(),
                    target_service_id: "smart-adapter".to_string(),
                },
                CanonicalDCallV1 {
                    call_id: "adapter-to-atom".to_string(),
                    source_service_id: "smart-adapter".to_string(),
                    target_service_id: "atom-controller".to_string(),
                },
                CanonicalDCallV1 {
                    call_id: "atom-to-adapter-feedback".to_string(),
                    source_service_id: "atom-controller".to_string(),
                    target_service_id: "smart-adapter".to_string(),
                },
            ],
        }
    }

    #[test]
    fn valid_minimal_graph_has_no_findings() {
        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:x".to_string(),
            dgraph_id: "dgraph:x".to_string(),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_id: "solo".to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: res(1, 134_217_728),
            }],
            calls: vec![],
        };
        assert_eq!(dgraph.validate(), Vec::new());
    }

    #[test]
    fn valid_heterogeneous_graph_has_no_findings() {
        assert_eq!(sample_dgraph().validate(), Vec::new());
    }

    #[test]
    fn legal_two_node_cycle_is_accepted() {
        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:feedback-loop".to_string(),
            dgraph_id: "dgraph:feedback-loop".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 {
                    service_id: "estimator".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 134_217_728),
                },
                CanonicalDServV1 {
                    service_id: "controller".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 134_217_728),
                },
            ],
            calls: vec![
                CanonicalDCallV1 {
                    call_id: "estimate-to-controller".to_string(),
                    source_service_id: "estimator".to_string(),
                    target_service_id: "controller".to_string(),
                },
                CanonicalDCallV1 {
                    call_id: "correction-to-estimator".to_string(),
                    source_service_id: "controller".to_string(),
                    target_service_id: "estimator".to_string(),
                },
            ],
        };
        assert_eq!(dgraph.validate(), Vec::new());
    }

    #[test]
    fn self_call_is_accepted() {
        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:self-loop".to_string(),
            dgraph_id: "dgraph:self-loop".to_string(),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_id: "recursive".to_string(),
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                resource_requirement: res(1, 134_217_728),
            }],
            calls: vec![CanonicalDCallV1 {
                call_id: "self".to_string(),
                source_service_id: "recursive".to_string(),
                target_service_id: "recursive".to_string(),
            }],
        };
        assert_eq!(dgraph.validate(), Vec::new());
    }

    #[test]
    fn wrong_schema_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.schema = "datum.dgraph/2".to_string();
        assert!(dgraph.validate().iter().any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn invalid_revision_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.revision = 0;
        assert!(dgraph.validate().iter().any(|f| f.code == "revision_invalid"));
    }

    #[test]
    fn duplicate_service_id_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[1].service_id = dgraph.services[0].service_id.clone();
        assert!(dgraph.validate().iter().any(|f| f.code == "duplicate_service_id"));
    }

    #[test]
    fn duplicate_service_id_hidden_by_ordering_is_still_caught() {
        let mut dgraph = sample_dgraph();
        dgraph.services.push(CanonicalDServV1 {
            service_id: dgraph.services[0].service_id.clone(),
            execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
            resource_requirement: res(1, 134_217_728),
        });
        assert!(dgraph.validate().iter().any(|f| f.code == "duplicate_service_id"));
    }

    #[test]
    fn duplicate_call_id_rejected() {
        let mut dgraph = sample_dgraph();
        let id = dgraph.calls[0].call_id.clone();
        dgraph.calls[1].call_id = id;
        assert!(dgraph.validate().iter().any(|f| f.code == "duplicate_call_id"));
    }

    #[test]
    fn dangling_call_source_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.calls[0].source_service_id = "does-not-exist".to_string();
        assert!(dgraph.validate().iter().any(|f| f.code == "dangling_call_source"));
    }

    #[test]
    fn dangling_call_target_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.calls[0].target_service_id = "does-not-exist".to_string();
        assert!(dgraph.validate().iter().any(|f| f.code == "dangling_call_target"));
    }

    #[test]
    fn invalid_dnode_abi_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        assert!(dgraph.validate().iter().any(|f| f.code == "invalid_dnode_abi"));
    }

    #[test]
    fn invalid_cpu_requirement_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.cpu_cores = 0;
        assert!(dgraph.validate().iter().any(|f| f.code == "invalid_cpu_requirement"));
    }

    #[test]
    fn invalid_memory_requirement_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].resource_requirement.memory_bytes = 0;
        assert!(dgraph.validate().iter().any(|f| f.code == "invalid_memory_requirement"));
    }

    #[test]
    fn empty_service_id_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].service_id = "   ".to_string();
        assert!(dgraph.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn project_and_application_identity_preserved_through_round_trip() {
        let dgraph = sample_dgraph();
        let json = serde_json::to_string(&dgraph).unwrap();
        let round_tripped: CanonicalDGraphV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(round_tripped.application_id, dgraph.application_id);
        assert_eq!(round_tripped.dgraph_id, dgraph.dgraph_id);
        assert_eq!(round_tripped, dgraph);
    }

    #[test]
    fn digest_is_deterministic_for_the_same_semantic_input() {
        let d1 = recompute_dgraph_digest(&sample_dgraph()).unwrap();
        let d2 = recompute_dgraph_digest(&sample_dgraph()).unwrap();
        assert_eq!(d1, d2);
        assert!(d1.starts_with("sha256:"));
        assert_eq!(d1.len(), 7 + 64);
    }

    #[test]
    fn digest_changes_when_service_semantics_change() {
        let a = sample_dgraph();
        let mut b = sample_dgraph();
        b.services[0].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        assert_ne!(recompute_dgraph_digest(&a).unwrap(), recompute_dgraph_digest(&b).unwrap());
    }

    #[test]
    fn digest_changes_when_a_service_resource_requirement_changes() {
        let a = sample_dgraph();
        let mut b = sample_dgraph();
        b.services[0].resource_requirement.cpu_cores += 1;
        assert_ne!(recompute_dgraph_digest(&a).unwrap(), recompute_dgraph_digest(&b).unwrap());
    }

    #[test]
    fn no_live_resource_usage_field_exists_on_the_type() {
        let value = serde_json::to_value(sample_dgraph()).unwrap();
        let text = value.to_string();
        for forbidden in ["cpu_usage", "usage_percent", "available", "current_", "rss", "load_average", "temperature", "rtt", "packet_loss"] {
            assert!(!text.contains(forbidden), "forbidden live-resource key '{forbidden}' found in canonical D-Graph JSON");
        }
    }

    #[test]
    fn digest_changes_when_call_semantics_change() {
        let a = sample_dgraph();
        let mut b = sample_dgraph();
        b.calls[0].target_service_id = "atom-controller".to_string();
        assert_ne!(recompute_dgraph_digest(&a).unwrap(), recompute_dgraph_digest(&b).unwrap());
    }

    #[test]
    fn digest_is_order_independent_for_services_and_calls() {
        let mut reordered = sample_dgraph();
        reordered.services.reverse();
        reordered.calls.reverse();
        assert_eq!(recompute_dgraph_digest(&sample_dgraph()).unwrap(), recompute_dgraph_digest(&reordered).unwrap());
    }

    #[test]
    fn digest_changes_when_revision_changes() {
        let a = sample_dgraph();
        let mut b = sample_dgraph();
        b.revision = 2;
        assert_ne!(recompute_dgraph_digest(&a).unwrap(), recompute_dgraph_digest(&b).unwrap());
    }

    #[test]
    fn empty_collections_validate_cleanly() {
        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:empty".to_string(),
            dgraph_id: "dgraph:empty".to_string(),
            revision: 1,
            services: vec![],
            calls: vec![],
        };
        assert_eq!(dgraph.validate(), Vec::new());
    }

    /// Cross-crate wire-vocabulary parity check (mirrors
    /// `runtime_kind::tests::runtime_kind_wire_values_match_dserver`): this
    /// crate's schema constant must be the exact literal dserver's own
    /// module pins independently.
    #[test]
    fn schema_constant_matches_dserver_literal() {
        assert_eq!(CANONICAL_DGRAPH_SCHEMA, "datum.dgraph/1");
    }

    /// Real cross-crate mapper fixture (same pattern as
    /// `execution_projection`'s `dserver_cross_crate_fixture_produces_the_expected_projection_and_digest`):
    /// `dserver::core::canonical_dgraph`'s own test of the SAME file
    /// (`dserver_cross_crate_fixture_produces_the_expected_digest`)
    /// independently deserializes and re-hashes this exact JSON. Unlike the
    /// Stage 107A execution-projection fixture (which needs two different
    /// `dserver_X`/`agent_X` sections because the two crates' SOURCE types
    /// genuinely differ in shape), this canonical-v1 wire shape is
    /// deliberately IDENTICAL on both sides -- one shared JSON file proves
    /// parity directly: both crates deserialize it into their own
    /// `CanonicalDGraphV1`, and both must recompute the SAME recorded
    /// `expected_digest_sha256`.
    #[test]
    fn dserver_cross_crate_fixture_produces_the_expected_digest() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("canonical-v1")
            .join("dgraph.smart-sentinel-lab.json");
        let text = std::fs::read_to_string(&fixture_path).unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        let raw: Value = serde_json::from_str(&text).unwrap();
        let expected_digest = raw["expected_digest_sha256"].as_str().unwrap().to_string();

        let dgraph: CanonicalDGraphV1 = serde_json::from_str(&text).expect("fixture must deserialize as this crate's CanonicalDGraphV1");
        assert_eq!(dgraph.validate(), Vec::new(), "the positive fixture must validate cleanly");

        let digest = recompute_dgraph_digest(&dgraph).unwrap();
        assert_eq!(digest, expected_digest, "this crate's recomputed digest must match the fixture's recorded expected_digest_sha256");
    }
}
