//! `datum.dmonitor-observation/1` (Phase 114B1, ADR-0024) -- DServer's
//! INDEPENDENT mirror of DATUM's `agent::canonical_dmonitor` canonical
//! DMonitor observation wire contract, per this project's established
//! no-shared-crate discipline (ADR-0007): the same logical shape is typed
//! separately in each crate, and a golden-vector test in both crates proves
//! their two independent implementations produce byte-identical JSON and
//! digests for the same fixed sample input rather than silently diverging
//! (exactly the precedent `dcall_mqtt_topic.rs` already established).
//!
//! This module is the WIRE CONTRACT and structural validator only. Server
//! ingestion, persistence, stream/sequence semantics, and freshness
//! evaluation live in `dmonitor_state.rs` / `dmonitor_freshness.rs` -- kept
//! separate exactly as `dcall_endpoint`/`dcall` and
//! `diot_runtime_binding`'s own identity-vs-registry split already are.
//!
//! DMonitor expresses OBSERVED runtime reality only, never desired-state
//! authority (ADR-0024's central invariant). Nothing in this module, at any
//! nesting level, accepts a `desired_state` field.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::jcs;

pub const CANONICAL_DMONITOR_OBSERVATION_SCHEMA: &str = "datum.dmonitor-observation/1";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

/// Mirrors `DATUM::agent::canonical_dmonitor::CanonicalDMonitorObservationV1`
/// field-for-field. See that module's doc comments for the full rationale;
/// this doc comment states only what differs at the DServer boundary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDMonitorObservationV1 {
    pub schema: String,
    pub observation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub node_id: String,
    /// Identifies one monotonic SEQUENCE EPOCH (Phase 114B1.1 closure): a
    /// monitor process that restarts while PRESERVING this id must also
    /// preserve/continue its sequence counter. Resetting to a lower
    /// sequence under the SAME `monitor_instance_id` is stale/out-of-order,
    /// never a fresh epoch -- only a genuinely NEW id starts a new one.
    /// Enforced by `dmonitor_state`'s ingestion rules.
    pub monitor_instance_id: String,
    pub sequence: u64,
    pub observed_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
    pub collector: String,
    pub subject: DMonitorSubjectV1,
    pub health: DMonitorHealthV1,
    pub facts: Value,
}

/// Mirrors `DMonitorContractRefV1` -- exact `{id, revision, digest}` or
/// absent, never approximate (ADR-0024).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DMonitorContractRefV1 {
    pub id: String,
    pub revision: u64,
    pub digest: String,
}

/// Mirrors `DMonitorRuntimeBindingRefV1` (Phase 114D0A) -- a reference to
/// Phase 113B1's server-owned `DIoTRuntimeBindingV1`
/// (`core::diot_runtime_binding`), deliberately a separate type from
/// `DMonitorContractRefV1`: a runtime binding is operational authority, not
/// a canonical desired-state "contract".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DMonitorRuntimeBindingRefV1 {
    pub binding_id: String,
    pub binding_revision: u64,
    pub descriptor_digest: String,
}

/// Mirrors `DMonitorSubjectV1` variant-for-variant, tag-for-tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum DMonitorSubjectV1 {
    #[serde(rename = "dnode")]
    DNode { node_id: String },

    #[serde(rename = "runtime_process")]
    RuntimeProcess {
        node_id: String,
        runtime_ref: String,
    },

    #[serde(rename = "dserv_realization")]
    DServRealization {
        project_id: String,
        application_id: String,
        service_id: String,
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dmap_ref: Option<DMonitorContractRefV1>,
    },

    #[serde(rename = "dcode_execution")]
    DCodeExecution {
        project_id: String,
        application_id: String,
        service_id: String,
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dcode_evidence_sha256: Option<String>,
    },

    #[serde(rename = "dcall_delivery")]
    DCallDelivery {
        project_id: String,
        application_id: String,
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grant_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery_evidence_digest: Option<String>,
    },

    /// Phase 114D0A: evolved shape -- mirrors
    /// `agent::canonical_dmonitor::DMonitorSubjectV1::DIoTRealization`
    /// exactly.
    #[serde(rename = "diot_realization")]
    DIoTRealization {
        project_id: String,
        application_id: String,
        diot_id: String,
        diot_placement_id: String,
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dmap_ref: Option<DMonitorContractRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dforward_ref: Option<DMonitorContractRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime_binding_ref: Option<DMonitorRuntimeBindingRefV1>,
    },

    #[serde(rename = "network_reachability")]
    NetworkReachability { node_id: String, target: String },

    #[serde(rename = "unbound")]
    Unbound {
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

/// Mirrors `DMonitorHealthV1` exactly (four variants, `snake_case`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorHealthV1 {
    Unknown,
    Healthy,
    Degraded,
    Unhealthy,
}

impl CanonicalDMonitorObservationV1 {
    /// Fail-closed structural validation, mirroring
    /// `agent::canonical_dmonitor::CanonicalDMonitorObservationV1::validate`
    /// finding-for-finding (same codes, so a caller sees the same rejection
    /// reason regardless of which crate rejected it).
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != CANONICAL_DMONITOR_OBSERVATION_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!(
                    "schema must be '{CANONICAL_DMONITOR_OBSERVATION_SCHEMA}', got '{}'",
                    self.schema
                ),
            ));
        }
        if self.observation_id.trim().is_empty() {
            findings.push(finding("missing_field", "observation_id must be non-empty"));
        }
        if self.node_id.trim().is_empty() {
            findings.push(finding("missing_field", "node_id must be non-empty"));
        }
        if self.monitor_instance_id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "monitor_instance_id must be non-empty",
            ));
        }
        if self.collector.trim().is_empty() {
            findings.push(finding("missing_field", "collector must be non-empty"));
        }
        if self.sequence == 0 {
            findings.push(finding("sequence_invalid", "sequence must be >= 1 (1-based)"));
        }
        if let Some(project_id) = &self.project_id {
            if project_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "project_id, if present, must be non-empty",
                ));
            }
        }
        if chrono::DateTime::parse_from_rfc3339(&self.observed_at_utc).is_err() {
            findings.push(finding(
                "invalid_timestamp",
                format!(
                    "observed_at_utc is not a valid RFC 3339 timestamp: '{}'",
                    self.observed_at_utc
                ),
            ));
        }
        if let Some(cycle_id) = &self.cycle_id {
            if cycle_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "cycle_id, if present, must be non-empty",
                ));
            }
        }

        findings.extend(self.subject.validate());

        // Phase 114B1.1 closure (section 2): mirrors the agent-side rule
        // exactly -- a project-bearing subject can never coexist with a
        // contradictory or absent top-level `project_id`.
        if let Some(subject_project_id) = self.subject.project_id() {
            match &self.project_id {
                Some(top_level) if top_level == subject_project_id => {}
                Some(top_level) => findings.push(finding(
                    "subject_project_id_inconsistent",
                    format!(
                        "observation.project_id '{top_level}' does not match subject.project_id '{subject_project_id}'"
                    ),
                )),
                None => findings.push(finding(
                    "subject_project_id_inconsistent",
                    format!(
                        "observation.project_id must be Some(\"{subject_project_id}\") to match the typed subject's project_id, got None"
                    ),
                )),
            }
        }

        // Phase 114D0A closure (section 7): mirrors the agent-side rule
        // exactly -- a governed DIoT realization must be of THIS observing
        // node's own realization.
        if let DMonitorSubjectV1::DIoTRealization { node_id: subject_node_id, .. } = &self.subject {
            if subject_node_id != &self.node_id {
                findings.push(finding(
                    "subject_node_id_inconsistent",
                    format!(
                        "observation.node_id '{}' does not match subject.DIoTRealization.node_id '{subject_node_id}' -- a governed local-runtime DIoT observation must be of THIS observing node's own realization",
                        self.node_id
                    ),
                ));
            }
        }

        if facts_contain_desired_state_key(&self.facts) {
            findings.push(finding(
                "desired_state_forbidden",
                "facts must not carry a 'desired_state' key at any depth -- DMonitor never owns desired-state authority",
            ));
        }
        if !json_numbers_are_finite(&self.facts) {
            findings.push(finding(
                "non_finite_metric",
                "facts must not contain a non-finite numeric observation",
            ));
        }

        findings
    }
}

impl DMonitorSubjectV1 {
    /// The typed `project_id` this subject variant carries, if any (Phase
    /// 114B1.1 closure, section 2) -- mirrors
    /// `agent::canonical_dmonitor::DMonitorSubjectV1::project_id` exactly.
    pub fn project_id(&self) -> Option<&str> {
        match self {
            DMonitorSubjectV1::DServRealization { project_id, .. }
            | DMonitorSubjectV1::DCodeExecution { project_id, .. }
            | DMonitorSubjectV1::DCallDelivery { project_id, .. }
            | DMonitorSubjectV1::DIoTRealization { project_id, .. } => Some(project_id.as_str()),
            DMonitorSubjectV1::DNode { .. }
            | DMonitorSubjectV1::RuntimeProcess { .. }
            | DMonitorSubjectV1::NetworkReachability { .. }
            | DMonitorSubjectV1::Unbound { .. } => None,
        }
    }

    fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();
        let mut require = |label: &'static str, value: &str| {
            if value.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    format!("subject.{label} must be non-empty"),
                ));
            }
        };
        match self {
            DMonitorSubjectV1::DNode { node_id } => require("node_id", node_id),
            DMonitorSubjectV1::RuntimeProcess {
                node_id,
                runtime_ref,
            } => {
                require("node_id", node_id);
                require("runtime_ref", runtime_ref);
            }
            DMonitorSubjectV1::DServRealization {
                project_id,
                application_id,
                service_id,
                node_id,
                dmap_ref,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("service_id", service_id);
                require("node_id", node_id);
                if let Some(dmap_ref) = dmap_ref {
                    findings.extend(validate_contract_ref("subject.dmap_ref", dmap_ref));
                }
            }
            DMonitorSubjectV1::DCodeExecution {
                project_id,
                application_id,
                service_id,
                node_id,
                dcode_evidence_sha256,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("service_id", service_id);
                require("node_id", node_id);
                if let Some(digest) = dcode_evidence_sha256 {
                    if digest.trim().is_empty() {
                        findings.push(finding(
                            "missing_field",
                            "subject.dcode_evidence_sha256, if present, must be non-empty",
                        ));
                    }
                }
            }
            DMonitorSubjectV1::DCallDelivery {
                project_id,
                application_id,
                call_id,
                grant_id,
                delivery_evidence_digest,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("call_id", call_id);
                if let Some(grant_id) = grant_id {
                    if grant_id.trim().is_empty() {
                        findings.push(finding(
                            "missing_field",
                            "subject.grant_id, if present, must be non-empty",
                        ));
                    }
                }
                if let Some(digest) = delivery_evidence_digest {
                    if digest.trim().is_empty() {
                        findings.push(finding(
                            "missing_field",
                            "subject.delivery_evidence_digest, if present, must be non-empty",
                        ));
                    }
                }
            }
            DMonitorSubjectV1::DIoTRealization {
                project_id,
                application_id,
                diot_id,
                diot_placement_id,
                node_id,
                dmap_ref,
                dforward_ref,
                runtime_binding_ref,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("diot_id", diot_id);
                require("diot_placement_id", diot_placement_id);
                require("node_id", node_id);
                if let Some(dmap_ref) = dmap_ref {
                    findings.extend(validate_contract_ref("subject.dmap_ref", dmap_ref));
                }
                if let Some(dforward_ref) = dforward_ref {
                    findings.extend(validate_contract_ref("subject.dforward_ref", dforward_ref));
                }
                if let Some(runtime_binding_ref) = runtime_binding_ref {
                    findings.extend(validate_runtime_binding_ref(
                        "subject.runtime_binding_ref",
                        runtime_binding_ref,
                    ));
                }
            }
            DMonitorSubjectV1::NetworkReachability { node_id, target } => {
                require("node_id", node_id);
                require("target", target);
            }
            DMonitorSubjectV1::Unbound { node_id, .. } => {
                require("node_id", node_id);
            }
        }
        findings
    }
}

fn validate_contract_ref(
    label: &str,
    contract_ref: &DMonitorContractRefV1,
) -> Vec<CanonicalValidationFinding> {
    let mut findings = Vec::new();
    if contract_ref.id.trim().is_empty() {
        findings.push(finding("malformed_ref", format!("{label}.id must be non-empty")));
    }
    if contract_ref.revision < 1 {
        findings.push(finding(
            "malformed_ref",
            format!("{label}.revision must be >= 1"),
        ));
    }
    if !digest_shape_ok(&contract_ref.digest) {
        findings.push(finding(
            "malformed_ref",
            format!("{label}.digest must be the canonical 'sha256:<64 lowercase hex>' form"),
        ));
    }
    findings
}

pub fn digest_shape_ok(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    })
}

/// Mirrors `agent::canonical_dmonitor::validate_runtime_binding_ref` exactly
/// (Phase 114D0A).
fn validate_runtime_binding_ref(
    label: &str,
    binding_ref: &DMonitorRuntimeBindingRefV1,
) -> Vec<CanonicalValidationFinding> {
    let mut findings = Vec::new();
    if binding_ref.binding_id.trim().is_empty() {
        findings.push(finding("malformed_ref", format!("{label}.binding_id must be non-empty")));
    }
    if binding_ref.binding_revision < 1 {
        findings.push(finding(
            "malformed_ref",
            format!("{label}.binding_revision must be >= 1"),
        ));
    }
    if !digest_shape_ok(&binding_ref.descriptor_digest) {
        findings.push(finding(
            "malformed_ref",
            format!("{label}.descriptor_digest must be the canonical 'sha256:<64 lowercase hex>' form"),
        ));
    }
    findings
}

fn json_numbers_are_finite(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64().is_none_or(f64::is_finite),
        Value::Array(items) => items.iter().all(json_numbers_are_finite),
        Value::Object(map) => map.values().all(json_numbers_are_finite),
        _ => true,
    }
}

/// Recursive at-any-depth scan, mirroring the agent-side fix from Phase
/// 114A.1: a top-level-only scan misses a nested legacy-shaped blob (e.g.
/// `{"service": {"desired_state": "running"}}`).
pub fn facts_contain_desired_state_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("desired_state") || map.values().any(facts_contain_desired_state_key)
        }
        Value::Array(items) => items.iter().any(facts_contain_desired_state_key),
        _ => false,
    }
}

/// Canonical JCS+SHA-256 digest (ADR-0006), computed identically to the
/// agent-side mirror (both reuse the same numeric-domain-restricted JCS
/// algorithm, just implemented independently per-crate).
pub fn recompute_dmonitor_observation_digest(
    observation: &CanonicalDMonitorObservationV1,
) -> Result<String, String> {
    let value: Value = serde_json::to_value(observation)
        .map_err(|error| format!("serialize CanonicalDMonitorObservationV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_dnode_observation() -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: "dmonitor-obs:11111111-1111-1111-1111-111111111111".to_string(),
            project_id: None,
            node_id: "fog-01".to_string(),
            monitor_instance_id: "fog-01-monitor-1".to_string(),
            sequence: 1,
            observed_at_utc: "2026-09-25T00:00:00Z".to_string(),
            cycle_id: None,
            collector: "host_telemetry_linux".to_string(),
            subject: DMonitorSubjectV1::DNode {
                node_id: "fog-01".to_string(),
            },
            health: DMonitorHealthV1::Healthy,
            facts: json!({"cpu_usage_permille": 125}),
        }
    }

    #[test]
    fn minimal_dnode_observation_is_valid() {
        let observation = minimal_dnode_observation();
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
    }

    #[test]
    fn serialization_round_trips_exactly() {
        let observation = minimal_dnode_observation();
        let json_text = serde_json::to_string(&observation).unwrap();
        let restored: CanonicalDMonitorObservationV1 = serde_json::from_str(&json_text).unwrap();
        assert_eq!(observation, restored);
    }

    #[test]
    fn wrong_schema_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.schema = "datum.something-else/1".to_string();
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn zero_sequence_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.sequence = 0;
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "sequence_invalid"));
    }

    #[test]
    fn desired_state_nested_inside_facts_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.facts = json!({
            "raw_legacy_entity": {
                "service": {"name": "sshd", "desired_state": "running"}
            }
        });
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "desired_state_forbidden"));
    }

    #[test]
    fn unknown_top_level_field_is_rejected() {
        let observation = minimal_dnode_observation();
        let mut value = serde_json::to_value(&observation).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("desired_state".to_string(), json!("running"));
        let result: Result<CanonicalDMonitorObservationV1, _> = serde_json::from_value(value);
        assert!(result.is_err());
    }

    #[test]
    fn fractional_number_in_facts_is_rejected_at_digest_time() {
        let mut observation = minimal_dnode_observation();
        observation.facts = json!({"cpu_usage_percent": 12.5});
        assert!(recompute_dmonitor_observation_digest(&observation).is_err());
    }

    #[test]
    fn digest_is_deterministic() {
        let observation = minimal_dnode_observation();
        let first = recompute_dmonitor_observation_digest(&observation).unwrap();
        let second = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(first, second);
        assert!(digest_shape_ok(&first));
    }

    // ---------------------------------------------- PHASE 114B1.1 CLOSURE

    #[test]
    fn subject_project_id_mismatch_against_top_level_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("project-a".to_string());
        observation.subject = DMonitorSubjectV1::DCallDelivery {
            project_id: "project-b".to_string(),
            application_id: "app:pipeline".to_string(),
            call_id: "sensor-to-decoder".to_string(),
            grant_id: None,
            delivery_evidence_digest: None,
        };
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "subject_project_id_inconsistent"), "{findings:?}");
    }

    #[test]
    fn subject_project_id_present_with_top_level_none_is_rejected() {
        let mut observation = minimal_dnode_observation();
        assert!(observation.project_id.is_none());
        observation.subject = DMonitorSubjectV1::DIoTRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            diot_id: "diot:broker".to_string(),
            diot_placement_id: "diotp:broker".to_string(),
            node_id: "fog-01".to_string(),
            dmap_ref: None,
            dforward_ref: None,
            runtime_binding_ref: None,
        };
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "subject_project_id_inconsistent"), "{findings:?}");
    }

    #[test]
    fn subject_project_id_matching_top_level_is_accepted() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.subject = DMonitorSubjectV1::DCallDelivery {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            call_id: "sensor-to-decoder".to_string(),
            grant_id: None,
            delivery_evidence_digest: None,
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
    }

    #[test]
    fn non_project_bearing_subjects_permit_top_level_project_id_none() {
        let mut observation = minimal_dnode_observation();
        assert!(observation.project_id.is_none());
        observation.subject = DMonitorSubjectV1::NetworkReachability {
            node_id: "fog-01".to_string(),
            target: "cloud-01:1883".to_string(),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
    }

    // ------------------------- PHASE 114B1.1 CLOSURE: CROSS-CRATE COVERAGE

    #[test]
    fn digest_matches_datum_golden_vector_2_dserv_realization_with_dmap_ref() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.health = DMonitorHealthV1::Degraded;
        observation.subject = DMonitorSubjectV1::DServRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "decoder".to_string(),
            node_id: "cloud-01".to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: format!("sha256:{}", "a".repeat(64)),
            }),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:a7d1d6ce7e56eabe44656efab47c4d21a603503a902cd44f8b86be3123d0ab38",
        );
    }

    #[test]
    fn digest_matches_datum_golden_vector_3_dcode_execution_with_evidence() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.health = DMonitorHealthV1::Unknown;
        observation.subject = DMonitorSubjectV1::DCodeExecution {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "sensor".to_string(),
            node_id: "fog-01".to_string(),
            dcode_evidence_sha256: Some(format!("{}", "e".repeat(64))),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:44be20b5f30d0bbbe1a09301499e43aa322c9913035a1474426a8c476dd2139e",
        );
    }

    #[test]
    fn digest_matches_datum_golden_vector_4_diot_realization_with_refs() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.health = DMonitorHealthV1::Unhealthy;
        observation.subject = DMonitorSubjectV1::DIoTRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            diot_id: "diot:broker".to_string(),
            diot_placement_id: "diotp:broker".to_string(),
            node_id: "fog-01".to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: format!("sha256:{}", "a".repeat(64)),
            }),
            dforward_ref: Some(DMonitorContractRefV1 {
                id: "dforward:1".to_string(),
                revision: 2,
                digest: format!("sha256:{}", "b".repeat(64)),
            }),
            runtime_binding_ref: Some(DMonitorRuntimeBindingRefV1 {
                binding_id: "diot-runtime-binding:proj:lab:app:pipeline:diotp:broker".to_string(),
                binding_revision: 1,
                descriptor_digest: format!("sha256:{}", "d".repeat(64)),
            }),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:5e20cf673907d9e89975176a3caf9af31cd647f2a9df378cb3c87872848c2ada",
        );
    }

    #[test]
    fn digest_matches_datum_golden_vector_6_diot_realization_missing_authority_refs() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.subject = DMonitorSubjectV1::DIoTRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            diot_id: "diot:broker".to_string(),
            diot_placement_id: "diotp:broker".to_string(),
            node_id: "fog-01".to_string(),
            dmap_ref: None,
            dforward_ref: None,
            runtime_binding_ref: None,
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:d71df9036a34815005af3e6972af4d5f7e0d043147deda6fc419967ae0da0e61",
        );
    }

    // ---------------------------------------- PHASE 114D0A CLOSURE
    // (canonical DIoT correlation)

    fn valid_diot_realization() -> DMonitorSubjectV1 {
        DMonitorSubjectV1::DIoTRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            diot_id: "diot:broker".to_string(),
            diot_placement_id: "diotp:broker".to_string(),
            node_id: "fog-01".to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: format!("sha256:{}", "a".repeat(64)),
            }),
            dforward_ref: Some(DMonitorContractRefV1 {
                id: "dforward:1".to_string(),
                revision: 2,
                digest: format!("sha256:{}", "b".repeat(64)),
            }),
            runtime_binding_ref: Some(DMonitorRuntimeBindingRefV1 {
                binding_id: "diot-runtime-binding:proj:lab:app:pipeline:diotp:broker".to_string(),
                binding_revision: 1,
                descriptor_digest: format!("sha256:{}", "d".repeat(64)),
            }),
        }
    }

    fn diot_observation() -> CanonicalDMonitorObservationV1 {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.subject = valid_diot_realization();
        observation
    }

    #[test]
    fn complete_valid_diot_realization_validates_cleanly() {
        assert!(diot_observation().validate().is_empty(), "{:?}", diot_observation().validate());
    }

    #[test]
    fn diot_required_identity_fields_must_be_non_empty() {
        for mutator in [
            (|s: &mut DMonitorSubjectV1| if let DMonitorSubjectV1::DIoTRealization { diot_id, .. } = s { *diot_id = String::new(); }) as fn(&mut DMonitorSubjectV1),
            |s: &mut DMonitorSubjectV1| if let DMonitorSubjectV1::DIoTRealization { diot_placement_id, .. } = s { *diot_placement_id = String::new(); },
            |s: &mut DMonitorSubjectV1| if let DMonitorSubjectV1::DIoTRealization { node_id, .. } = s { *node_id = String::new(); },
        ] {
            let mut observation = diot_observation();
            mutator(&mut observation.subject);
            let findings = observation.validate();
            assert!(findings.iter().any(|f| f.code == "missing_field"), "{findings:?}");
        }
    }

    #[test]
    fn diot_node_consistency_is_enforced() {
        let mut observation = diot_observation();
        if let DMonitorSubjectV1::DIoTRealization { node_id, .. } = &mut observation.subject {
            *node_id = "cloud-01".to_string();
        }
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "subject_node_id_inconsistent"), "{findings:?}");
    }

    #[test]
    fn diot_valid_dmap_ref_is_accepted_and_malformed_one_is_rejected() {
        assert!(diot_observation().validate().is_empty());
        let mut observation = diot_observation();
        if let DMonitorSubjectV1::DIoTRealization { dmap_ref, .. } = &mut observation.subject {
            *dmap_ref = Some(DMonitorContractRefV1 { id: String::new(), revision: 0, digest: "bad".to_string() });
        }
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "malformed_ref"), "{findings:?}");
    }

    #[test]
    fn diot_valid_dforward_ref_is_accepted_and_malformed_one_is_rejected() {
        let mut observation = diot_observation();
        if let DMonitorSubjectV1::DIoTRealization { dforward_ref, .. } = &mut observation.subject {
            *dforward_ref = Some(DMonitorContractRefV1 { id: "x".to_string(), revision: 1, digest: "not-a-digest".to_string() });
        }
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "malformed_ref"), "{findings:?}");
    }

    #[test]
    fn diot_valid_runtime_binding_ref_is_accepted() {
        assert!(diot_observation().validate().is_empty());
    }

    #[test]
    fn diot_zero_binding_revision_is_rejected() {
        let mut observation = diot_observation();
        if let DMonitorSubjectV1::DIoTRealization { runtime_binding_ref, .. } = &mut observation.subject {
            runtime_binding_ref.as_mut().unwrap().binding_revision = 0;
        }
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "malformed_ref"), "{findings:?}");
    }

    #[test]
    fn diot_malformed_binding_digest_is_rejected() {
        let mut observation = diot_observation();
        if let DMonitorSubjectV1::DIoTRealization { runtime_binding_ref, .. } = &mut observation.subject {
            runtime_binding_ref.as_mut().unwrap().descriptor_digest = "not-sha256".to_string();
        }
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "malformed_ref"), "{findings:?}");
    }

    #[test]
    fn diot_realization_digest_is_deterministic() {
        let a = recompute_dmonitor_observation_digest(&diot_observation()).unwrap();
        let b = recompute_dmonitor_observation_digest(&diot_observation()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn digest_matches_datum_golden_vector_5_unbound_with_description() {
        let mut observation = minimal_dnode_observation();
        observation.subject = DMonitorSubjectV1::Unbound {
            node_id: "fog-01".to_string(),
            description: Some("container discovered before correlation".to_string()),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:847c0f899e8a7ca846d10e30978a769767789ed93c9eced064d2deb8295d98ab",
        );
    }

    /// Table-driven round-trip coverage mirroring DATUM's own
    /// `table_driven_remaining_subject_variants_and_all_health_values_round_trip`
    /// test.
    #[test]
    fn table_driven_remaining_subject_variants_and_all_health_values_round_trip() {
        let subjects = vec![
            DMonitorSubjectV1::RuntimeProcess {
                node_id: "fog-01".to_string(),
                runtime_ref: "container:abc123".to_string(),
            },
            DMonitorSubjectV1::DCallDelivery {
                project_id: "proj:lab".to_string(),
                application_id: "app:pipeline".to_string(),
                call_id: "sensor-to-decoder".to_string(),
                grant_id: None,
                delivery_evidence_digest: None,
            },
            DMonitorSubjectV1::NetworkReachability {
                node_id: "fog-01".to_string(),
                target: "cloud-01:1883".to_string(),
            },
        ];
        let health_values = [
            DMonitorHealthV1::Unknown,
            DMonitorHealthV1::Healthy,
            DMonitorHealthV1::Degraded,
            DMonitorHealthV1::Unhealthy,
        ];
        for subject in subjects {
            for &health in &health_values {
                let mut observation = minimal_dnode_observation();
                if let Some(project_id) = subject.project_id() {
                    observation.project_id = Some(project_id.to_string());
                }
                observation.subject = subject.clone();
                observation.health = health;
                assert!(observation.validate().is_empty(), "{:?} / {:?}: {:?}", subject, health, observation.validate());
                let json_text = serde_json::to_string(&observation).unwrap();
                let restored: CanonicalDMonitorObservationV1 = serde_json::from_str(&json_text).unwrap();
                assert_eq!(observation, restored);
                assert!(recompute_dmonitor_observation_digest(&observation).is_ok());
            }
        }
    }

    /// Golden vector: this DServer-side independent implementation must
    /// produce the EXACT SAME digest as `DATUM::agent::canonical_dmonitor`'s
    /// own `digest_is_deterministic_across_two_computations` fixture for the
    /// identical `minimal_dnode_observation()` input. The literal is
    /// hardcoded in BOTH crates (mirrors `dcall_mqtt_topic.rs`'s precedent
    /// exactly) -- if either implementation's JCS canonicalization or field
    /// mirroring ever silently drifts from the other, this test (or its
    /// DATUM-side twin) fails.
    #[test]
    fn digest_matches_datum_golden_vector() {
        let observation = minimal_dnode_observation();
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:1119cec82421ebf78e01cd947a956a251caf36e4309fce58b768edc3db542c7e",
            "DServer's canonical_dmonitor digest for the shared fixture must match DATUM's golden vector exactly"
        );
    }
}
