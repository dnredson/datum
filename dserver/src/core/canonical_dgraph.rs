//! `datum.dgraph/1` (Stage 110, ADR-0017) -- the canonical APPLICATION-PLANE
//! D-Graph: `G = (V, E)` with `V` = D-Serv and `E` = D-Call (ADR-0001).
//!
//! This is a NEW, deliberately minimal v1 contract type, distinct from every
//! other structure in this repository that has ever been called a graph:
//!
//! - `dgraph.rs`'s `DGraph` (the closed Controlled WASM Deployment phase,
//!   C59-I44): a composite of application, infrastructure, placement and
//!   health -- labelled "controlled composite graph" in documentation
//!   (ADR-0001). NOT renamed, NOT reinterpreted by this module.
//! - `operational_dgraph.rs`'s `OperationalDGraph`: a per-node list of
//!   desired container services, no edges, no D-Serv/D-Call semantics --
//!   labelled "operational deployment document v1". NOT renamed.
//! - The snapshot-materialized "observed runtime graph" (`model.rs`,
//!   `topology.rs`): evidence-enriched interpretation of runtime state.
//!
//! `CanonicalDGraphV1` carries NONE of infrastructure inventory, runtime
//! health, node observation, container/process state, telemetry, deployment
//! placement, or local reconciliation state (Section: frozen architecture).
//! It is placement-free by construction: it has no field that could ever
//! name a node, an address, an image, or a stage (ADR-0009).
//!
//! **D-Call cycles are explicitly ALLOWED** (ADR-0001 decision D-4):
//! communication topology is not installation ordering. `validate()` never
//! runs a cycle check; a legal `A -> B -> A` D-Call cycle, and even a
//! self-call `A -> A`, are structurally valid. The deployment-dependency
//! graph derived for installation ordering is a completely separate
//! structure (out of Stage 110's scope; see ADR-0002/ADR-0017) and must
//! never be confused with D-Call topology here.
//!
//! Independently mirrored (not shared) by `DATUM::agent::canonical_dgraph`,
//! per this project's established no-shared-crate discipline (ADR-0007):
//! parity is proven by a real cross-crate fixture + test on each side, never
//! by type sharing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::jcs;

pub const CANONICAL_DGRAPH_SCHEMA: &str = "datum.dgraph/1";
#[cfg(test)]
mod catalog_reference_tests {
    use super::*;
    const FIXTURE: &str = include_str!("../../../scripts/fixtures/datum-contracts/canonical-v2/dgraph.catalog-reference.json");
    #[test]
    fn catalog_v2_cross_crate_fixture_roundtrips_without_loss() {
        let graph: CanonicalDGraphV1 = serde_json::from_str(FIXTURE).unwrap();
        assert!(graph.validate().is_empty());
        let wire = serde_json::to_value(&graph).unwrap();
        assert_eq!(wire, serde_json::from_str::<Value>(FIXTURE).unwrap());
        let reference = graph.services[0].service_definition.as_ref().unwrap();
        assert_eq!(reference.definition_id, "smart-coffee-api");
        assert_eq!(reference.revision, 3);
        let base = recompute_dgraph_digest(&graph).unwrap();
        assert_eq!(base, "sha256:95093c7937972c38bc55c9eef21283bc40f6139065af952d5012a579ccf996ee");
        for field in ["id", "revision", "digest"] {
            let mut changed = graph.clone();
            let r = changed.services[0].service_definition.as_mut().unwrap();
            match field { "id" => r.definition_id = "other".into(), "revision" => r.revision = 4, _ => r.digest = format!("sha256:{}", "b".repeat(64)) };
            assert_ne!(base, recompute_dgraph_digest(&changed).unwrap());
        }
    }
    #[test]
    fn legacy_digest_and_schema_gate_are_preserved() {
        let mut graph: CanonicalDGraphV1 = serde_json::from_str(FIXTURE).unwrap();
        graph.schema = CANONICAL_DGRAPH_SCHEMA.into();
        assert!(graph.validate().iter().any(|f| f.code == "catalog_reference_requires_v2"));
        graph.services[0].service_definition = None;
        assert!(graph.validate().is_empty());
        assert!(serde_json::to_value(&graph).unwrap()["services"][0].get("service_definition").is_none());
        let clone: CanonicalDGraphV1 = serde_json::from_value(serde_json::to_value(&graph).unwrap()).unwrap();
        assert_eq!(recompute_dgraph_digest(&graph), recompute_dgraph_digest(&clone));
    }
    #[test]
    fn unknown_reference_fields_and_malformed_refs_never_drop() {
        let mut value: Value = serde_json::from_str(FIXTURE).unwrap();
        value["services"][0]["service_definition"]["latest"] = Value::Bool(true);
        assert!(serde_json::from_value::<CanonicalDGraphV1>(value).is_err());
        let mut graph: CanonicalDGraphV1 = serde_json::from_str(FIXTURE).unwrap();
        graph.services[0].service_definition.as_mut().unwrap().revision = 0;
        assert!(graph.validate().iter().any(|f| f.code == "invalid_service_definition_reference"));
    }
    #[test]
    fn v2_nested_unknown_fields_reject_while_v1_annotations_remain_compatible() {
        for target in ["service", "execution", "resources", "call"] {
            let mut value: Value = serde_json::from_str(FIXTURE).unwrap();
            value["calls"] = serde_json::json!([{"call_id":"self","source_service_id":value["services"][0]["service_id"],"target_service_id":value["services"][0]["service_id"]}]);
            let extra = match target {
                "service" => &mut value["services"][0],
                "execution" => &mut value["services"][0]["execution_requirement"],
                "resources" => &mut value["services"][0]["resource_requirement"],
                _ => &mut value["calls"][0],
            };
            extra["future_semantics"] = Value::Bool(true);
            assert!(serde_json::from_value::<CanonicalDGraphV1>(value.clone()).is_err(), "{target}");
            value["schema"] = Value::String(CANONICAL_DGRAPH_SCHEMA.into());
            value["services"][0].as_object_mut().unwrap().remove("service_definition");
            let graph: CanonicalDGraphV1 = serde_json::from_value(value).unwrap();
            assert!(graph.validate().is_empty(), "{target}");
        }
    }
    #[test]
    fn duplicate_known_nested_fields_are_not_normalized_away() {
        for schema in [CANONICAL_DGRAPH_SCHEMA, CATALOG_DGRAPH_SCHEMA] {
            let raw = format!(r#"{{"schema":"{schema}","application_id":"a","dgraph_id":"g","revision":1,"services":[{{"service_id":"first","service_id":"second","execution_requirement":{{"dnode_abi":"datum-dnode/0"}},"resource_requirement":{{"cpu_cores":1,"memory_bytes":1}}}}],"calls":[]}}"#);
            assert!(serde_json::from_str::<CanonicalDGraphV1>(&raw).is_err());
            let raw = raw.replace(r#""service_id":"first","service_id":"second""#, r#""service_id":"one""#)
                .replace(r#""memory_bytes":1"#, r#""memory_bytes":1,"memory_bytes":2"#);
            assert!(serde_json::from_str::<CanonicalDGraphV1>(&raw).is_err());
        }
    }
}
pub const CATALOG_DGRAPH_SCHEMA: &str = "datum.dgraph/2";

/// Exact modeling provenance only. Never execution/placement authorization.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServiceDefinitionReference {
    pub definition_id: String,
    pub revision: u64,
    pub digest: String,
}

/// Stage 110 P0 fix (independent review v4, D-Serv host invariance). The
/// ONLY accepted D-Node ABI for v0.1 (ADR-0003: `datum-dnode/0`) -- the
/// host-INDEPENDENT execution contract a D-Serv's D-Code targets. This is
/// deliberately NOT Stage 108's `RuntimeKind` (`container`/`native_process`):
/// per ADR-0004, "the D-Serv execution contract is host-independent," and
/// per ADR-0015, `RuntimeKind` is the OPERATIONAL service-artifact/executor
/// dispatch seam, never the canonical application-plane execution semantics
/// of a D-Serv. `container`/`native_process` describe HOW a D-Node (or an
/// operational workload) may later be hosted -- never WHAT a D-Serv's D-Code
/// requires to run.
pub const DNODE_ABI_V0: &str = "datum-dnode/0";

/// One structured validation finding, mirroring the `{code, message}` shape
/// `scripts/check_datum_contract_fixtures.py` already uses for the Stage 107
/// hand-authored fixtures -- deliberate consistency between the Python
/// structural checker and this Rust validator, not a coincidence. `validate`
/// collects every finding rather than stopping at the first, so a single
/// fixture/test can assert on exactly the finding(s) it cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalValidationFinding {
    pub code: &'static str,
    pub message: String,
}

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding { code, message: message.into() }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CanonicalDGraphV1 {
    pub schema: String,
    /// ADR-0009: distinct from `project_id`. The D-Graph itself never
    /// carries a `project_id` -- a project references one or more
    /// application D-Graph revisions, never the reverse.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_definition: Option<ServiceDefinitionReference>,
    /// WHAT application execution this D-Serv requires, never WHERE it
    /// currently runs -- no operational/reconciliation state (PID,
    /// container status, target node) belongs here. v1 deliberately carries
    /// only the host-independent D-Node ABI requirement; a D-Code/artifact
    /// content reference is a separate future contract, not folded in here.
    pub execution_requirement: CanonicalDServExecutionRequirementV1,
    /// Stage 110 P0 fix (independent review v4, Stage 112 resource
    /// readiness): the SMALLEST static
    /// resource surface Stage 112's deterministic resource-FEASIBILITY
    /// placement needs. This is desired/declared application semantics --
    /// exactly like `execution_requirement` -- never an observed/live value;
    /// no CPU-utilization/RSS/current-free-memory field is or will ever be
    /// added here (those are Stage 113 observations, attached elsewhere).
    pub resource_requirement: CanonicalDServResourceRequirementV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalDServExecutionRequirementV1 {
    /// Stage 110 P0 fix (independent review v4): the host-INDEPENDENT
    /// D-Node ABI (ADR-0003) this D-Serv's D-Code targets -- for v0.1,
    /// always `DNODE_ABI_V0` ("datum-dnode/0"). This is deliberately NOT
    /// `runtime_kind`/`host_kind`/`"container"`/`"native_process"`: those
    /// name HOW a host is later realized (ADR-0015's operational
    /// service-artifact dispatch seam), never WHAT a D-Serv's application
    /// semantics require. Validated by direct comparison against
    /// `DNODE_ABI_V0`, never by `RuntimeKind::parse` -- there is no import
    /// of `crate::core::runtime_kind` anywhere in this module.
    pub dnode_abi: String,
}

/// Stage 110 P0 fix (independent review v3, Stage 112 resource readiness).
/// The DESIRED/declared minimum resource this D-Serv requires to run --
/// static application/deployment semantics, exactly like
/// `CanonicalDServExecutionRequirementV1`. Exact integer quantities only:
/// `cpu_cores` is a whole core count, `memory_bytes` is an exact byte count.
/// Never a percentage, a live/current/available/used value, or any other
/// observation -- those remain Stage 113 D-Monitor territory and structurally
/// cannot appear here (no such field exists on this type).
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
    /// Structural, self-contained validation -- never touches another
    /// contract. Collects every finding; returns `Vec::new()` iff valid.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DGRAPH_SCHEMA && self.schema != CATALOG_DGRAPH_SCHEMA {
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
            if let Some(reference) = &service.service_definition {
                if self.schema != CATALOG_DGRAPH_SCHEMA {
                    findings.push(finding("catalog_reference_requires_v2", "Catalog references require datum.dgraph/2."));
                }
                if !valid_definition_reference(reference) {
                    findings.push(finding("invalid_service_definition_reference", "Expected exact definition ID, revision and sha256 digest."));
                }
            }
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
            // D-4 (ADR-0001): a D-Call cycle -- including a self-call
            // (source == target) -- is NEVER a validation failure. Only a
            // DANGLING endpoint (one that names no declared service_id) is
            // ever rejected here.
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

// Keep frozen v1 annotation handling; v2 rejects unknown semantics at every level.
impl<'de> Deserialize<'de> for CanonicalDGraphV1 {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct ExecutionWire {
            dnode_abi: String,
            #[serde(flatten)] extra: std::collections::BTreeMap<String, Value>,
        }
        #[derive(Deserialize)]
        struct ResourceWire {
            cpu_cores: u32, memory_bytes: u64,
            #[serde(flatten)] extra: std::collections::BTreeMap<String, Value>,
        }
        #[derive(Deserialize)]
        struct ServiceWire {
            service_id: String,
            #[serde(default)] service_definition: Option<ServiceDefinitionReference>,
            execution_requirement: ExecutionWire,
            resource_requirement: ResourceWire,
            #[serde(flatten)] extra: std::collections::BTreeMap<String, Value>,
        }
        #[derive(Deserialize)]
        struct CallWire {
            call_id: String, source_service_id: String, target_service_id: String,
            #[serde(flatten)] extra: std::collections::BTreeMap<String, Value>,
        }
        #[derive(Deserialize)]
        struct Wire {
            schema: String, application_id: String, dgraph_id: String, revision: u64,
            #[serde(default)] services: Vec<ServiceWire>,
            #[serde(default)] calls: Vec<CallWire>,
            #[serde(flatten)] extra: std::collections::BTreeMap<String, Value>,
        }
        let w = Wire::deserialize(deserializer)?;
        if w.schema == CATALOG_DGRAPH_SCHEMA && !w.extra.is_empty() {
            return Err(serde::de::Error::custom("Unknown v2 DGraph field."));
        }
        let strict = w.schema == CATALOG_DGRAPH_SCHEMA;
        let decode = |service: ServiceWire| -> Result<CanonicalDServV1, D::Error> {
            if strict && (!service.extra.is_empty() || !service.execution_requirement.extra.is_empty()
                || !service.resource_requirement.extra.is_empty()) {
                return Err(serde::de::Error::custom("Unknown v2 DServ field."));
            }
            Ok(CanonicalDServV1 {
                service_id: service.service_id, service_definition: service.service_definition,
                execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: service.execution_requirement.dnode_abi },
                resource_requirement: CanonicalDServResourceRequirementV1 { cpu_cores: service.resource_requirement.cpu_cores, memory_bytes: service.resource_requirement.memory_bytes },
            })
        };
        let services = w.services.into_iter().map(decode).collect::<Result<Vec<_>, _>>()?;
        let calls = w.calls.into_iter().map(|call| {
            if strict && !call.extra.is_empty() {
                return Err(serde::de::Error::custom("Unknown v2 DCall field."));
            }
            Ok(CanonicalDCallV1 { call_id: call.call_id, source_service_id: call.source_service_id, target_service_id: call.target_service_id })
        }).collect::<Result<Vec<_>, D::Error>>()?;
        Ok(Self { schema: w.schema, application_id: w.application_id, dgraph_id: w.dgraph_id,
            revision: w.revision, services, calls })
    }
}

fn valid_definition_reference(r: &ServiceDefinitionReference) -> bool {
    !r.definition_id.is_empty() && r.definition_id.len() <= 128
        && r.definition_id != "." && r.definition_id != ".."
        && r.definition_id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && (1..=1_000_000).contains(&r.revision)
        && r.digest.len() == 71 && r.digest.starts_with("sha256:")
        && r.digest[7..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Canonical JCS+SHA-256 digest (ADR-0006), never self-referential (this
/// type has no digest-of-itself field to omit). `services`/`calls` are
/// canonically UNORDERED SETS keyed by `service_id`/`call_id` respectively
/// (ADR-0017's explicit array-ordering decision): a deterministic
/// deep-sorted copy is hashed, so two semantically identical D-Graphs that
/// merely list their elements in a different order produce the IDENTICAL
/// digest. The wire JSON itself is never reordered by this function --
/// serialization/round-tripping preserves whatever order the caller
/// supplied; only the bytes fed into the digest are normalized.
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
                    service_definition: None, service_id: "chirpstack-gateway-bridge".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 536_870_912),
                },
                CanonicalDServV1 {
                    service_definition: None, service_id: "smart-adapter".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 268_435_456),
                },
                CanonicalDServV1 {
                    service_definition: None, service_id: "atom-controller".to_string(),
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
                service_definition: None, service_id: "solo".to_string(),
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

    /// D-4 (ADR-0001): a legal `A -> B -> A` cycle is ACCEPTED -- this is a
    /// regression test proving `validate()` contains no generic cycle
    /// checker copied from deployment-dependency semantics.
    #[test]
    fn legal_two_node_cycle_is_accepted() {
        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:feedback-loop".to_string(),
            dgraph_id: "dgraph:feedback-loop".to_string(),
            revision: 1,
            services: vec![
                CanonicalDServV1 {
                    service_definition: None, service_id: "estimator".to_string(),
                    execution_requirement: CanonicalDServExecutionRequirementV1 { dnode_abi: DNODE_ABI_V0.to_string() },
                    resource_requirement: res(1, 134_217_728),
                },
                CanonicalDServV1 {
                    service_definition: None, service_id: "controller".to_string(),
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
        assert_eq!(dgraph.validate(), Vec::new(), "A -> B -> A must be accepted, findings: {:?}", dgraph.validate());
    }

    /// A degenerate self-call `A -> A` must also remain legal: application
    /// topology, not installation ordering.
    #[test]
    fn self_call_is_accepted() {
        let dgraph = CanonicalDGraphV1 {
            schema: CANONICAL_DGRAPH_SCHEMA.to_string(),
            application_id: "app:self-loop".to_string(),
            dgraph_id: "dgraph:self-loop".to_string(),
            revision: 1,
            services: vec![CanonicalDServV1 {
                service_definition: None, service_id: "recursive".to_string(),
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
        dgraph.schema = "datum.dgraph/999".to_string();
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
        // Adversarial: duplicate at non-adjacent positions, not merely
        // consecutive -- the checker must not rely on sortedness.
        let mut dgraph = sample_dgraph();
        dgraph.services.push(CanonicalDServV1 {
            service_definition: None, service_id: dgraph.services[0].service_id.clone(),
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

    /// Stage 110 P0 fix (independent review v4, D-Serv host invariance):
    /// an unrecognised D-Node ABI must be rejected -- this is deliberately
    /// NOT testing `"docker"`/`"native_process"` as invalid host-kind
    /// strings; those are operational-plane concepts and irrelevant here.
    #[test]
    fn invalid_dnode_abi_rejected() {
        let mut dgraph = sample_dgraph();
        dgraph.services[0].execution_requirement.dnode_abi = "datum-dnode/999".to_string();
        assert!(dgraph.validate().iter().any(|f| f.code == "invalid_dnode_abi"));
    }

    /// Stage 110 P0 fix (independent review v3, Stage 112 resource
    /// readiness): a D-Serv declaring zero required CPU cores is nonsensical
    /// static input, not a value to silently accept as "no requirement".
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

    /// Stage 110 P0 fix (independent review v3): a D-Serv's static resource
    /// requirement is part of its identity -- changing it must change the
    /// D-Graph digest exactly like changing its dnode_abi does.
    #[test]
    fn digest_changes_when_a_service_resource_requirement_changes() {
        let a = sample_dgraph();
        let mut b = sample_dgraph();
        b.services[0].resource_requirement.cpu_cores += 1;
        assert_ne!(recompute_dgraph_digest(&a).unwrap(), recompute_dgraph_digest(&b).unwrap());
    }

    /// PASS P (independent review v3): no live/observed resource-usage field
    /// may ever contaminate this static requirement model.
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

    /// The array-ordering decision (ADR-0017): `services`/`calls` are
    /// canonically unordered sets keyed by id -- reordering either array
    /// must NOT change the digest.
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

    /// Real cross-crate mapper fixture (same pattern as
    /// `execution_projection`'s cross-crate mapper tests): the shared
    /// canonical-v1 wire shape is IDENTICAL on both sides, so one JSON file
    /// proves parity directly -- `DATUM::agent::canonical_dgraph`'s own test
    /// of this exact file (`dserver_cross_crate_fixture_produces_the_expected_digest`)
    /// independently deserializes and re-hashes it; both sides must recover
    /// the SAME recorded `expected_digest_sha256`.
    #[test]
    fn agent_cross_crate_fixture_produces_the_expected_digest() {
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
