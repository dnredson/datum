//! `datum.dmonitor-observation/1` (Phase 114A, ADR-0024) -- the canonical
//! DMonitor OBSERVATION artifact: DATUM-native evidence of observed runtime
//! reality, never desired-state authority.
//!
//! `DMap` (and `DGraph`/`DForward`/`DContinuum`/`ServiceArtifact`/`DCode`)
//! express what SHOULD be running. `DMonitor` expresses what an observer
//! actually saw, once, at a point in time. This module intentionally
//! contains NO `desired_state` field anywhere, at any nesting level: a
//! canonical observation that disagrees with current DMap authority is
//! "observed divergence" for some later drift computation to derive, never
//! a competing placement. See ADR-0024 for the full authority-boundary
//! rationale (this is the single most important invariant in Phase 114A).
//!
//! Independently mirrored (not shared) by a future `dserver::core`
//! counterpart when DServer-side ingestion is built (114B), per this
//! project's established no-shared-crate discipline (ADR-0007). No
//! DServer-side type exists yet -- this phase is deliberately agent-side
//! model only (ADR-0024, deferred work).
//!
//! Canonical JCS+SHA-256 digest support follows the exact established
//! pattern of `canonical_dgraph.rs`/`canonical_dmap_v2.rs`/`canonical_dforward.rs`:
//! reuse `crate::agent::jcs`, never a second canonical JSON implementation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::jcs;

pub const CANONICAL_DMONITOR_OBSERVATION_SCHEMA: &str = "datum.dmonitor-observation/1";

/// Mirrors `canonical_dgraph::CanonicalValidationFinding`'s `{code, message}`
/// shape exactly -- the established project-wide validation-finding pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DMonitorValidationFinding {
    pub code: &'static str,
    pub message: String,
}

fn finding(code: &'static str, message: impl Into<String>) -> DMonitorValidationFinding {
    DMonitorValidationFinding { code, message: message.into() }
}

/// A single canonical DMonitor observation: one collector, one subject, one
/// point in time, one sequence number. Never a batch/snapshot container --
/// batching/ingestion/persistence semantics belong to Phase 114B, not this
/// wire contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalDMonitorObservationV1 {
    pub schema: String,

    /// Unique per emitted observation (never reused, even across a restart
    /// of the same monitor instance).
    pub observation_id: String,

    /// Present when the observation concerns a specific DATUM project;
    /// absent for pure host/DNode-level observations that exist before or
    /// outside any project's governance (mirrors the same optionality
    /// `SmartSentinelTelemetryEnvelope.project_id` already uses).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// The logical DNode this observation was produced ON (the observer's
    /// own identity), resolved via the existing DNode local-identity
    /// mechanism -- never caller-supplied for a governed collector, exactly
    /// like `DCodeInvocationEvidenceV1.node_id`.
    pub node_id: String,

    /// Identifies WHICH monitor process/instance produced this observation
    /// (distinct from `node_id`: a single DNode could in principle run more
    /// than one monitor instance). `monitor_instance_id` identifies one
    /// monotonic SEQUENCE EPOCH (Phase 114B1.1 closure): `sequence` is
    /// monotonic for as long as `monitor_instance_id` stays the same, and a
    /// monitor process that restarts while PRESERVING its own
    /// `monitor_instance_id` must also preserve/continue its own sequence
    /// counter -- resetting to a lower sequence under the SAME
    /// `monitor_instance_id` is stale/out-of-order, never a fresh epoch.
    /// Only adopting a genuinely NEW `monitor_instance_id` legitimately
    /// starts a new sequence (typically at 1); see
    /// `dserver::core::dmonitor_state`'s ingestion rules, which enforce this
    /// exactly.
    pub monitor_instance_id: String,

    /// 1-based, monotonically increasing per `monitor_instance_id`. A
    /// sequence of 0 is rejected -- mirrors the existing
    /// `SmartSentinelTelemetryEnvelope.sequence` rule exactly.
    pub sequence: u64,

    /// Agent-produced observation time (RFC 3339). This is deliberately the
    /// ONLY timestamp this type carries. A future DServer-owned ingestion
    /// `received_at` is explicitly NOT modeled here (Phase 114A section 10):
    /// no agent may fabricate authoritative server receive time, and no
    /// freshness POLICY is implemented yet -- only the provenance this
    /// artifact carries makes later freshness evaluation possible.
    pub observed_at_utc: String,

    /// Optional correlation to the observing collector's own cycle/run,
    /// where the collector's implementation genuinely has one (mirrors
    /// `Meta.cycle_id` from the legacy SmartSentinel cycle model). Absent
    /// when the collector has no cycle concept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,

    /// Which collector/source produced this fact, e.g.
    /// `"host_telemetry_linux"`, `"docker_linux_cli"`,
    /// `"dcode_execution_reference"`. Free-form but required and non-empty
    /// -- this is provenance, not a canonical identity.
    pub collector: String,

    /// WHAT was observed, typed (never an untyped-property-bag-only
    /// identity -- section 12). See `DMonitorSubjectV1`.
    pub subject: DMonitorSubjectV1,

    /// Health is a SEPARATE concept from freshness (section 11/15): this
    /// field records what the collector believed AT `observed_at_utc`. A
    /// consumer must independently decide whether this observation is still
    /// fresh enough to trust that belief right now -- this type never makes
    /// that decision for them.
    pub health: DMonitorHealthV1,

    /// The observed facts/evidence payload itself (metrics, check results,
    /// raw collector output already normalized to JSON). Deliberately an
    /// opaque validated JSON value rather than a fully-typed union of every
    /// possible metric shape -- fully typing every observable fact for every
    /// subject kind would inflate this phase far beyond its scope; the
    /// CANONICAL IDENTITY of the observation (who/what/when/subject) is what
    /// section 12 requires to be typed, and it is. This field carries NO
    /// `desired_state` key and must never be used to smuggle one in --
    /// enforced by the fail-closed structural scan in `validate()`.
    ///
    /// Because this whole artifact is digested as one canonical JCS document
    /// (`recompute_dmonitor_observation_digest`), every number inside
    /// `facts` is bound by the SAME Stage 107A safe-integer-only numeric
    /// domain as the rest of DATUM's canonical model (`agent::jcs`) --
    /// fractional/exponential numbers are rejected at digest time, not
    /// silently truncated. A collector reporting a fractional metric (e.g.
    /// a CPU percentage) must scale it to an integer domain (e.g. basis
    /// points/per-mille) before placing it here.
    pub facts: Value,
}

/// Mirrors the established `ContractRefV2 { id, revision, digest }` shape
/// (`canonical_dmap_v2.rs`) under DMonitor's own name, per this project's
/// independent-mirror discipline -- an observation's correlation to
/// canonical authority is EXACT (id + revision + digest) or it is absent
/// (section 13), never a name-only or partial reference.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DMonitorContractRefV1 {
    pub id: String,
    pub revision: u64,
    pub digest: String,
}

/// The typed subject model (section 12). Every variant that claims
/// correlation to canonical DATUM authority does so via EXACT identity
/// fields only -- never a container name, process name, hostname, port, or
/// image name (section 13). A subject with no exact correlation available
/// must use `Unbound`, never fabricate one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum DMonitorSubjectV1 {
    /// The DNode/host runtime itself.
    #[serde(rename = "dnode")]
    DNode { node_id: String },

    /// A runtime process or container observed on a DNode. Deliberately
    /// carries NO container/process name as canonical identity (section
    /// 13) -- a descriptive label, if wanted, belongs in `facts`, never
    /// here. `runtime_ref` is an OPAQUE, collector-local identifier (e.g. a
    /// container ID) used only to correlate repeated observations of the
    /// SAME runtime entity across a monitor instance's own cycles -- it is
    /// not claimed to correlate with any canonical DATUM authority.
    #[serde(rename = "runtime_process")]
    RuntimeProcess {
        node_id: String,
        runtime_ref: String,
    },

    /// A D-Serv placement's observed realization. May correlate exactly
    /// with the DMap that placed it.
    #[serde(rename = "dserv_realization")]
    DServRealization {
        project_id: String,
        application_id: String,
        service_id: String,
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dmap_ref: Option<DMonitorContractRefV1>,
    },

    /// A D-Code invocation's observed runtime behavior. References existing
    /// `DCodeInvocationEvidenceV1` BY DIGEST ONLY (section 16) -- never
    /// re-embeds or duplicates that evidence type.
    #[serde(rename = "dcode_execution")]
    DCodeExecution {
        project_id: String,
        application_id: String,
        service_id: String,
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dcode_evidence_sha256: Option<String>,
    },

    /// A D-Call delivery's observed outcome. References existing
    /// `DCallDeliveryEvidenceV1`/grant identity BY REFERENCE ONLY.
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

    /// A DForward/DIoT runtime realization's observed state.
    #[serde(rename = "diot_realization")]
    DIoTRealization {
        project_id: String,
        application_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dforward_ref: Option<DMonitorContractRefV1>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diot_placement_id: Option<String>,
    },

    /// Network/control-plane reachability between the observing DNode and a
    /// target. `target` is an operational address/label, not a canonical
    /// identity -- reachability facts about it belong in `facts`.
    #[serde(rename = "network_reachability")]
    NetworkReachability {
        node_id: String,
        target: String,
    },

    /// An HONEST unbound observation: the collector could not establish
    /// exact canonical correlation for what it saw. Preferred over guessing
    /// (section 13). `description` is free-form provenance only.
    #[serde(rename = "unbound")]
    Unbound {
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

/// Deliberately richer than a boolean (section 15). `Unknown` is the
/// correct default for a collector that cannot assess health at all (never
/// silently coerced to `Healthy`). Kept structurally separate from
/// freshness: nothing on this enum, or on the observation as a whole,
/// claims "this is still true right now" -- only "this collector believed
/// this to be true AT `observed_at_utc`".
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorHealthV1 {
    Unknown,
    Healthy,
    Degraded,
    Unhealthy,
}

impl CanonicalDMonitorObservationV1 {
    /// Fail-closed structural validation (section 20). Collects every
    /// finding rather than stopping at the first, matching the established
    /// `CanonicalDGraphV1::validate` convention.
    pub fn validate(&self) -> Vec<DMonitorValidationFinding> {
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
            findings.push(finding("missing_field", "monitor_instance_id must be non-empty"));
        }
        if self.collector.trim().is_empty() {
            findings.push(finding("missing_field", "collector must be non-empty"));
        }
        if self.sequence == 0 {
            findings.push(finding("sequence_invalid", "sequence must be >= 1 (1-based)"));
        }
        if let Some(project_id) = &self.project_id {
            if project_id.trim().is_empty() {
                findings.push(finding("missing_field", "project_id, if present, must be non-empty"));
            }
        }
        if chrono::DateTime::parse_from_rfc3339(&self.observed_at_utc).is_err() {
            findings.push(finding(
                "invalid_timestamp",
                format!("observed_at_utc is not a valid RFC 3339 timestamp: '{}'", self.observed_at_utc),
            ));
        }
        if let Some(cycle_id) = &self.cycle_id {
            if cycle_id.trim().is_empty() {
                findings.push(finding("missing_field", "cycle_id, if present, must be non-empty"));
            }
        }

        findings.extend(self.subject.validate());

        // Phase 114B1.1 closure (section 2): a project-bearing subject can
        // never coexist with a contradictory or absent top-level
        // `project_id`. This is enforced HERE, in the canonical artifact's
        // own validator -- never left for a consumer's request-scoped
        // project isolation (e.g. DServer's ingestion path) to paper over,
        // since a self-contradictory canonical artifact must be rejected
        // regardless of which project scope it is later submitted under.
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

        // Fail-closed structural guard (section 14): `facts` must never
        // carry a `desired_state` key, at the top level or nested inside it.
        // This cannot happen through the typed fields above (none of them
        // exist), so this guards specifically against a caller trying to
        // smuggle desired state in through the one intentionally-opaque
        // field -- including by embedding a legacy-shaped blob (e.g. a raw
        // SmartSentinel `Entity.attrs` dump, which nests exactly this key
        // under a sub-object such as `service`) rather than placing the key
        // at the top level.
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
    /// 114B1.1 closure, section 2). Only the four project-governed variants
    /// (`DServRealization`, `DCodeExecution`, `DCallDelivery`,
    /// `DIoTRealization`) carry one; `DNode`/`RuntimeProcess`/
    /// `NetworkReachability`/`Unbound` have no project concept and always
    /// return `None`. Used by `CanonicalDMonitorObservationV1::validate` to
    /// enforce that the top-level `project_id` can never contradict a
    /// project-bearing subject's own `project_id` -- this cannot be left to
    /// a consumer (e.g. DServer's request-path project scope) to catch,
    /// because the canonical artifact itself must never be internally
    /// self-contradictory.
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

    fn validate(&self) -> Vec<DMonitorValidationFinding> {
        let mut findings = Vec::new();
        let mut require = |label: &'static str, value: &str| {
            if value.trim().is_empty() {
                findings.push(finding("missing_field", format!("subject.{label} must be non-empty")));
            }
        };
        match self {
            DMonitorSubjectV1::DNode { node_id } => require("node_id", node_id),
            DMonitorSubjectV1::RuntimeProcess { node_id, runtime_ref } => {
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
                        findings.push(finding("missing_field", "subject.grant_id, if present, must be non-empty"));
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
                dforward_ref,
                diot_placement_id,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                if let Some(dforward_ref) = dforward_ref {
                    findings.extend(validate_contract_ref("subject.dforward_ref", dforward_ref));
                }
                if let Some(placement_id) = diot_placement_id {
                    if placement_id.trim().is_empty() {
                        findings.push(finding(
                            "missing_field",
                            "subject.diot_placement_id, if present, must be non-empty",
                        ));
                    }
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

fn validate_contract_ref(label: &str, contract_ref: &DMonitorContractRefV1) -> Vec<DMonitorValidationFinding> {
    let mut findings = Vec::new();
    if contract_ref.id.trim().is_empty() {
        findings.push(finding("malformed_ref", format!("{label}.id must be non-empty")));
    }
    if contract_ref.revision < 1 {
        findings.push(finding("malformed_ref", format!("{label}.revision must be >= 1")));
    }
    if !digest_shape_ok(&contract_ref.digest) {
        findings.push(finding(
            "malformed_ref",
            format!("{label}.digest must be the canonical 'sha256:<64 lowercase hex>' form"),
        ));
    }
    findings
}

fn digest_shape_ok(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()))
}

/// Recursively confirms every JSON number in `value` is finite. `serde_json`
/// cannot represent NaN/Infinity in valid JSON in the first place, so this
/// is a defensive structural guard (section 20's "non-finite numeric
/// observations" requirement) rather than a check expected to ever actually
/// trip in practice -- documented here so the invariant is explicit, not
/// merely incidental to the JSON number model.
fn json_numbers_are_finite(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64().is_none_or(f64::is_finite),
        Value::Array(items) => items.iter().all(json_numbers_are_finite),
        Value::Object(map) => map.values().all(json_numbers_are_finite),
        _ => true,
    }
}

/// Recursively scans `value` for a `desired_state` object key at any depth,
/// not just the top level. A shallow, top-level-only scan would miss a
/// collector nesting one legacy-shaped sub-object (e.g. `{"service":
/// {"desired_state": "running"}}`) inside `facts` -- exactly the shape
/// `DATUM/src/core/plugins/service_builder.rs` produces on every legacy
/// `service` entity today, so this is a real smuggling path, not a
/// hypothetical one.
fn facts_contain_desired_state_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key("desired_state") || map.values().any(facts_contain_desired_state_key)
        }
        Value::Array(items) => items.iter().any(facts_contain_desired_state_key),
        _ => false,
    }
}

/// Canonical JCS+SHA-256 digest (ADR-0006), never self-referential. There
/// are no unordered-set fields in this type (unlike `CanonicalDGraphV1`'s
/// `services`/`calls`), so no pre-sort is needed before hashing.
pub fn recompute_dmonitor_observation_digest(observation: &CanonicalDMonitorObservationV1) -> Result<String, String> {
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
            subject: DMonitorSubjectV1::DNode { node_id: "fog-01".to_string() },
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
    fn digest_is_deterministic_across_two_computations() {
        let observation = minimal_dnode_observation();
        let first = recompute_dmonitor_observation_digest(&observation).unwrap();
        let second = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(first, second);
        assert!(digest_shape_ok(&first));
    }

    /// Documents (not just asserts) the Stage 107A safe-integer-only
    /// numeric domain applying transitively to `facts`, per the doc comment
    /// on `CanonicalDMonitorObservationV1::facts`: a fractional metric must
    /// be rejected at digest time, not silently truncated or accepted.
    #[test]
    fn fractional_number_in_facts_is_rejected_at_digest_time() {
        let mut observation = minimal_dnode_observation();
        observation.facts = json!({"cpu_usage_percent": 12.5});
        assert!(recompute_dmonitor_observation_digest(&observation).is_err());
    }

    /// Golden vector (Phase 114B1): the DServer-side independent mirror
    /// (`dserver::core::canonical_dmonitor`) must produce the EXACT SAME
    /// digest for this identical fixture. The literal is hardcoded in BOTH
    /// crates (mirrors `dcall_mqtt_topic.rs`'s established golden-vector
    /// precedent) -- if either implementation's JCS canonicalization or
    /// field mirroring ever silently drifts from the other, this test (or
    /// its DServer-side twin `digest_matches_datum_golden_vector`) fails.
    #[test]
    fn digest_matches_dserver_golden_vector() {
        let observation = minimal_dnode_observation();
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:1119cec82421ebf78e01cd947a956a251caf36e4309fce58b768edc3db542c7e",
            "DATUM's canonical_dmonitor digest for the shared fixture must match DServer's golden vector exactly"
        );
    }

    #[test]
    fn digest_changes_when_any_field_changes() {
        let base = minimal_dnode_observation();
        let mut changed = base.clone();
        changed.sequence = 2;
        assert_ne!(
            recompute_dmonitor_observation_digest(&base).unwrap(),
            recompute_dmonitor_observation_digest(&changed).unwrap()
        );
    }

    #[test]
    fn wrong_schema_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.schema = "datum.something-else/1".to_string();
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "schema_invalid"));
    }

    #[test]
    fn missing_identities_are_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.observation_id = "".to_string();
        observation.node_id = "  ".to_string();
        observation.monitor_instance_id = "".to_string();
        let findings = observation.validate();
        let codes: Vec<_> = findings.iter().map(|f| f.code).collect();
        assert_eq!(codes.iter().filter(|c| **c == "missing_field").count() >= 3, true, "{codes:?}");
    }

    #[test]
    fn zero_sequence_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.sequence = 0;
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "sequence_invalid"));
    }

    #[test]
    fn invalid_timestamp_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.observed_at_utc = "not-a-timestamp".to_string();
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "invalid_timestamp"));
    }

    /// Health and freshness are separate concepts (section 11/15): this
    /// type has no "is this still fresh" field at all -- the only time
    /// carried is `observed_at_utc`, and health never encodes an implicit
    /// expiry. Demonstrated structurally: two observations differing ONLY
    /// in `observed_at_utc`/`sequence` (i.e. differing only in "when", not
    /// in the health belief itself) still produce different digests, proving
    /// the model does not collapse "when" into "health".
    #[test]
    fn health_and_freshness_are_structurally_separate() {
        let mut older = minimal_dnode_observation();
        older.sequence = 1;
        older.observed_at_utc = "2026-09-25T00:00:00Z".to_string();
        let mut newer = older.clone();
        newer.sequence = 2;
        newer.observed_at_utc = "2026-09-25T00:05:00Z".to_string();
        assert_eq!(older.health, newer.health, "same reported health");
        assert_ne!(
            recompute_dmonitor_observation_digest(&older).unwrap(),
            recompute_dmonitor_observation_digest(&newer).unwrap(),
            "same health must not imply the same observation identity"
        );
    }

    #[test]
    fn dserv_realization_may_correlate_exactly_with_a_dmap_reference() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
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
    }

    #[test]
    fn malformed_dmap_reference_digest_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.subject = DMonitorSubjectV1::DServRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "decoder".to_string(),
            node_id: "cloud-01".to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: "not-a-real-digest".to_string(),
            }),
        };
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "malformed_ref"));
    }

    /// An unbound observation is a first-class, explicitly-designed
    /// possibility (section 13) -- never an error by itself.
    #[test]
    fn unbound_observation_is_allowed_when_honestly_declared() {
        let mut observation = minimal_dnode_observation();
        observation.subject = DMonitorSubjectV1::Unbound {
            node_id: "fog-01".to_string(),
            description: Some("container discovered before service correlation was possible".to_string()),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
    }

    #[test]
    fn unbound_observation_still_requires_a_node_id() {
        let mut observation = minimal_dnode_observation();
        observation.subject = DMonitorSubjectV1::Unbound { node_id: "".to_string(), description: None };
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "missing_field"));
    }

    /// No desired-state authority may exist inside DMonitor (section 14):
    /// proven two ways -- (1) `deny_unknown_fields` structurally rejects any
    /// attempt to smuggle a `desired_state` key in via the wire JSON's
    /// top-level fields (there is no such field to populate); (2) the one
    /// intentionally-open `facts` payload is explicitly scanned and
    /// rejected if it carries a top-level `desired_state` key.
    #[test]
    fn top_level_desired_state_field_is_structurally_impossible() {
        let observation = minimal_dnode_observation();
        let mut value = serde_json::to_value(&observation).unwrap();
        value.as_object_mut().unwrap().insert("desired_state".to_string(), json!("running"));
        let result: Result<CanonicalDMonitorObservationV1, _> = serde_json::from_value(value);
        assert!(result.is_err(), "deny_unknown_fields must reject an injected top-level desired_state key");
    }

    #[test]
    fn desired_state_inside_facts_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.facts = json!({"desired_state": "running", "cpu_usage_permille": 125});
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "desired_state_forbidden"));
    }

    /// Phase 114A.1 closure finding: the `desired_state_forbidden` guard
    /// must not be a top-level-only scan. A collector embedding a
    /// legacy-shaped blob inside `facts` -- e.g. exactly the
    /// `{"service": {..., "desired_state": "running", ...}}` shape
    /// `DATUM/src/core/plugins/service_builder.rs` produces on every legacy
    /// service entity -- must still be rejected even though the forbidden
    /// key is nested rather than top-level.
    #[test]
    fn desired_state_nested_inside_facts_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.facts = json!({
            "raw_legacy_entity": {
                "service": {
                    "name": "sshd",
                    "status": "available",
                    "desired_state": "running"
                }
            }
        });
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "desired_state_forbidden"));
    }

    /// Same nested-smuggling concern, but via an array rather than a nested
    /// object, to confirm the recursive scan walks both container kinds.
    #[test]
    fn desired_state_nested_inside_facts_array_is_rejected() {
        let mut observation = minimal_dnode_observation();
        observation.facts = json!({
            "raw_legacy_entities": [
                {"kind": "host", "attrs": {"hostname": "fog-01"}},
                {"kind": "service", "attrs": {"name": "sshd", "desired_state": "running"}}
            ]
        });
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "desired_state_forbidden"));
    }

    /// Execution evidence is referenced by digest, never duplicated (section
    /// 16): the observation carries only the digest string
    /// `dcode::evidence::dcode_evidence_sha256` would itself produce for a
    /// real `DCodeInvocationEvidenceV1` -- never a second embedded copy of
    /// that evidence type.
    #[test]
    fn dcode_execution_subject_references_evidence_by_digest_only() {
        let real_digest = crate::dcode::evidence::dcode_evidence_sha256(&sample_dcode_evidence());
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.subject = DMonitorSubjectV1::DCodeExecution {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "sensor".to_string(),
            node_id: "fog-01".to_string(),
            dcode_evidence_sha256: Some(real_digest.clone()),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        // Structural proof of "by reference": the subject variant has no
        // field capable of holding a DCodeInvocationEvidenceV1 at all.
        match &observation.subject {
            DMonitorSubjectV1::DCodeExecution { dcode_evidence_sha256, .. } => {
                assert_eq!(dcode_evidence_sha256.as_deref(), Some(real_digest.as_str()));
            }
            _ => panic!("expected DCodeExecution subject"),
        }
    }

    // ------------------------- PHASE 114B1.1 CLOSURE: CROSS-CRATE COVERAGE

    /// Golden vector #2: `DServRealization` with `dmap_ref` PRESENT,
    /// project-consistent, `Degraded` health. Extends cross-crate proof
    /// beyond the single `DNode` case (golden #1) to a project-bearing
    /// subject variant carrying an optional canonical ref.
    #[test]
    fn digest_matches_dserver_golden_vector_2_dserv_realization_with_dmap_ref() {
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

    /// Golden vector #3: `DCodeExecution` with `dcode_evidence_sha256`
    /// PRESENT, project-consistent, `Unknown` health.
    #[test]
    fn digest_matches_dserver_golden_vector_3_dcode_execution_with_evidence() {
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

    /// Golden vector #4: `DIoTRealization` with `dforward_ref` AND
    /// `diot_placement_id` PRESENT, project-consistent, `Unhealthy` health.
    #[test]
    fn digest_matches_dserver_golden_vector_4_diot_realization_with_refs() {
        let mut observation = minimal_dnode_observation();
        observation.project_id = Some("proj:lab".to_string());
        observation.health = DMonitorHealthV1::Unhealthy;
        observation.subject = DMonitorSubjectV1::DIoTRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            dforward_ref: Some(DMonitorContractRefV1 {
                id: "dforward:1".to_string(),
                revision: 2,
                digest: format!("sha256:{}", "b".repeat(64)),
            }),
            diot_placement_id: Some("diotp:broker".to_string()),
        };
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        let digest = recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(
            digest,
            "sha256:20529ff5fa669c9a8c9fe4f47c49da44a2aa3f17bc6c35eec899f13e0e2d7067",
        );
    }

    /// Golden vector #5: `Unbound` with `description` PRESENT, no project
    /// (permitted for non-project-bearing subjects), `Healthy` health.
    #[test]
    fn digest_matches_dserver_golden_vector_5_unbound_with_description() {
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

    /// Table-driven round-trip coverage (compact, per the task's own
    /// guidance -- not a golden digest per case) for the remaining
    /// subject-variant shapes not already exercised by a golden vector
    /// above: `RuntimeProcess`, `DCallDelivery` (evidence ABSENT), and
    /// `NetworkReachability`, plus every `DMonitorHealthV1` value. Proves
    /// each shape validates and serialization round-trips exactly --
    /// self-consistency within this crate, complementing (not replacing)
    /// the cross-crate golden vectors above.
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

    // ---------------------------------------------- PHASE 114B1.1 CLOSURE

    /// Section 2: a project-bearing subject with a top-level `project_id`
    /// naming a DIFFERENT project is a self-contradictory canonical
    /// artifact and must be rejected by the artifact's OWN validator, never
    /// left for a later consumer's request-scoped project isolation to
    /// catch.
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

    /// Same defect, but with the top-level `project_id` absent entirely
    /// rather than merely wrong -- `None` is not a safe default when the
    /// subject itself claims a project.
    #[test]
    fn subject_project_id_present_with_top_level_none_is_rejected() {
        let mut observation = minimal_dnode_observation();
        assert!(observation.project_id.is_none());
        observation.subject = DMonitorSubjectV1::DIoTRealization {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            dforward_ref: None,
            diot_placement_id: None,
        };
        let findings = observation.validate();
        assert!(findings.iter().any(|f| f.code == "subject_project_id_inconsistent"), "{findings:?}");
    }

    /// The consistent case must remain valid: this is not a blanket ban on
    /// project-bearing subjects, only on CONTRADICTORY ones.
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

    /// Non-project-bearing subjects (`DNode`, `RuntimeProcess`,
    /// `NetworkReachability`, `Unbound`) never trigger this rule -- their
    /// `project_id()` is always `None`, so a host-level observation with no
    /// project at all remains legal exactly as ADR-0024 originally intended.
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

    fn sample_dcode_evidence() -> crate::dcode::evidence::DCodeInvocationEvidenceV1 {
        use crate::dcode::evidence::{DCodeInvocationEvidenceV1, DCodeInvocationOutcome};
        DCodeInvocationEvidenceV1 {
            schema: "datum.dcode-invocation-evidence/1".to_string(),
            authorization_id: "dcode-auth:proj:lab:app:pipeline:sensor:fog-01:1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "sensor".to_string(),
            node_id: "fog-01".to_string(),
            dmap_id: "dmap:1".to_string(),
            dmap_revision: 1,
            dmap_digest: format!("sha256:{}", "a".repeat(64)),
            dgraph_id: "dgraph:1".to_string(),
            dgraph_revision: 1,
            dgraph_digest: format!("sha256:{}", "b".repeat(64)),
            dserv_artifact_id: "dserv:1:sensor".to_string(),
            dserv_artifact_digest: format!("{}", "c".repeat(64)),
            dserv_artifact_descriptor_digest: format!("sha256:{}", "d".repeat(64)),
            module_size_bytes: Some(10),
            dnode_abi: "datum-dnode/0".to_string(),
            fresh_instance_per_invocation: true,
            input_port: "trigger".to_string(),
            message_id: "dcode-msg:1".to_string(),
            correlation_id: "dcode-corr:1".to_string(),
            input_sha256: format!("{}", "e".repeat(64)),
            input_size: 0,
            input_envelope_sha256: format!("{}", "f".repeat(64)),
            input_envelope_size: 10,
            received_at_utc: "2026-09-25T00:00:00Z".to_string(),
            output_sha256: Some(format!("{}", "0".repeat(64))),
            output_size: Some(10),
            output_status: Some("ok".to_string()),
            output_reason_present: false,
            output_reason_sha256: None,
            output_reason_size: None,
            emits: Vec::new(),
            limits_applied: crate::dcode::evidence::DCodeAppliedLimits {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 1000,
                max_memory_pages: 16,
                max_message_bytes: 65536,
            },
            outcome: DCodeInvocationOutcome::Success,
            error_code: None,
            error_message: None,
            error_offending_value_sha256: None,
            error_offending_value_size: None,
            duration_ms: 1,
        }
    }
}
