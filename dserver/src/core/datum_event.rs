//! `datum.event/1` (Phase 122, hardened per the Phase 122 freeze review) --
//! the canonical, typed, versioned **occurrence** contract every future
//! Event Manager producer may emit.
//!
//! # Event != Alert != Incident
//!
//! This module defines ONLY `DatumEventV1`: a normalized record of something
//! that happened. It deliberately carries NO alert-lifecycle semantics --
//! no `status` (open/acknowledged/resolved/suppressed), no
//! `acknowledged_by`/`acknowledged_at`/`resolved_at`, no `first_seen`/
//! `last_seen` condition bookkeeping, no `occurrence_count` dedup state.
//! Those concepts describe a *persistent condition derived from one or more
//! events*, which is `DatumAlert`'s job (Phase 124), not this one's. The
//! Phase 121 audit's finding that `build_global_events` conflates a
//! stateless recomputation with event semantics (hardcoded `status: "open"`,
//! non-deterministic `impact_summary` ids) is exactly the failure mode this
//! split exists to avoid repeating.
//!
//! `DatumEventV1` represents an immutable occurrence BY CONTRACT. Phase 122
//! exposes no lifecycle-mutation method (no `pub fn` here takes `&mut
//! DatumEventV1`), and Phase 123 will enforce append-only persistence/
//! admission semantics on top of that. This is NOT a Rust-level structural
//! guarantee: every field on `DatumEventV1` remains an ordinary `pub` value
//! and can technically be reassigned by any caller holding a `&mut`
//! reference, before admission. Do not oversell the absence of mutator
//! methods as more than what it is -- a documented contract convention,
//! reinforced (not replaced) by Phase 123's future storage discipline.
//!
//! # Authority posture -- what ACTUALLY prevents this from becoming an
//! authority
//!
//! `DatumEventV1` is NOT an authority. It never becomes placement authority,
//! DMap authority, desired-state authority, runtime authority, health
//! authority, convergence authority, or remediation authority. Canonical
//! truths stay exactly where the Phase 121 audit found them:
//!
//! | Concept              | Canonical source                                |
//! |-----------------------|--------------------------------------------------|
//! | Placement              | Accepted DMap (`core::ddeploy`)                  |
//! | Desired runtime         | deployment/reconciliation desired state          |
//! | Observed runtime         | DMonitor / sentinel telemetry evidence           |
//! | Health                   | existing health/readiness/convergence evaluators |
//! | Freshness                 | existing timestamp + policy evaluators           |
//! | Convergence                 | existing convergence evaluator                   |
//! | AI/GNN conclusion             | non-authoritative inference only                 |
//!
//! The Phase 122 freeze review established that a field-level naming
//! restriction (the original module briefly banned an `attributes` key
//! literally spelled `desired_state`) is NOT what protects this boundary --
//! it is trivially bypassed by renaming a key, and it has a real cost: it
//! rejects legitimate DESCRIPTIVE use of that exact name (e.g. a
//! `desired_state_mismatch` event wanting to show the desired value
//! side-by-side with what was observed). `DatumEventV1` may DESCRIBE
//! desired state; it must never BE or MUTATE desired state. That rule is
//! now enforced architecturally, not lexically:
//!
//! - `DatumEventV1` has no control-plane mutation path: this module imports
//!   nothing from, and writes nothing to, `core::ddeploy`,
//!   `core::operational_dgraph`, or any other mutating subsystem. No
//!   function here takes a `&mut` reference to any other subsystem's state.
//! - Future producers (Phase 126) MUST emit events only AFTER observing or
//!   performing their canonical operation -- an event is a record of what
//!   already happened via the existing authoritative path, never a request
//!   for something to happen.
//! - Future consumers in the Event/Alert/Operations path (Phase 124's Alert
//!   Manager, Phase 125's scheduler, Phase 127's read models) MUST NOT gate
//!   or mutate DDeploy, DMap, or reconciliation based on event/alert
//!   content. The scheduler stays strictly read-only/observational; read
//!   models stay strictly read-only.
//!
//! A `DatumEventV1` may *refer to* the sources in the table above via
//! `evidence_refs`. It must never embed, replace, or re-derive them.
//!
//! # Relationship to existing contracts
//!
//! - **DMonitor** (`core::canonical_dmonitor`) remains the canonical source
//!   of *observed evidence*. A `DatumEventV1` produced from a DMonitor
//!   health transition references the originating observation via
//!   `evidence_refs` (`DatumEventEvidenceRefV1::DMonitorObservation`); it
//!   never copies `facts`/`health` into itself.
//! - **`OperationalEvidenceEvent`** (`core::operational_evidence`) remains
//!   the durable, deduplicated agent-evidence audit log it already is. It
//!   is a plausible *upstream input* to a future `DatumEventV1` producer
//!   (Phase 126), referenced via `evidence_refs`, not superseded here.
//! - **Accepted DMap** (`core::ddeploy`) remains the sole placement
//!   authority; a `DatumEventV1` describing `dmap_revision_changed` (a
//!   future Phase 126 producer) would reference the DMap id/revision/digest
//!   via `evidence_refs`, never carry its own competing notion of "current"
//!   placement. The DMap evidence reference reuses
//!   `canonical_dmap::CanonicalContractReferenceV1` directly (see
//!   [`DatumEventEvidenceRefV1::DMap`]) rather than duplicating its shape,
//!   once the freeze review confirmed this preserves the wire format
//!   exactly (proven by
//!   `dmap_evidence_ref_wire_representation_is_unchanged_after_reuse` below).
//! - **Future Event Store** (Phase 123) is the durable, queryable store
//!   this type will be persisted into. This module defines no persistence.
//! - **Future Alert Manager** (Phase 124) consumes streams of
//!   `DatumEventV1` to derive `DatumAlert` records with real
//!   condition-level dedup and an open/acknowledged/resolved/suppressed
//!   lifecycle. This module implements neither.
//! - **Future Active Monitor** (Phase 125) and **Event Producers** (Phase
//!   126) will construct `DatumEventV1` values at existing, already-clean
//!   commit points identified by the Phase 121 audit (e.g.
//!   `ddeploy::accept_proposal`). No producer is wired in this phase.
//! - **Future GNN/LLM producers**: `source = Gnn` or `source = Llm` simply
//!   identifies provenance, exactly like any other source -- it grants no
//!   special authority. Unlike other sources, though, `Gnn`/`Llm` MUST
//!   carry a non-empty `model_ref` (see [`DatumEventV1::validate`]),
//!   independent of whether `confidence_permille` is present, so an
//!   inference's provenance is always traceable.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::canonical_dmap::CanonicalContractReferenceV1;
use crate::core::jcs;

/// `datum.event/1` -- see module doc comment for the full contract.
pub const DATUM_EVENT_SCHEMA: &str = "datum.event/1";

/// Confidence is expressed as parts-per-thousand (0..=1000), not a bare
/// `f64`, so a `DatumEventV1` stays within the JCS safe-integer contract
/// (`core::jcs`, RFC 8785 scope note: fractional/exponential JSON numbers
/// are rejected) used by every other canonical DATUM contract's digest.
/// This mirrors the repository's own established convention for expressing
/// a fractional quantity as a scaled integer (e.g. DMonitor's
/// `cpu_usage_permille` fact), rather than inventing a new representation.
pub const CONFIDENCE_PERMILLE_MAX: u32 = 1000;

/// `attributes`' architectural purpose is small, producer-specific
/// DIAGNOSTIC metadata -- never an embedded authoritative document (a full
/// DMap, a full DMonitor observation, or any alternate state store). This
/// bound is what makes that principle mean something structurally, since
/// the Phase 122 freeze review found nothing else enforced it (see the
/// module doc comment's authority-posture section). 16 KiB comfortably
/// fits genuine diagnostic context while making it impractical to embed a
/// full canonical contract instead of referencing it via `evidence_refs`.
pub const MAX_DATUM_EVENT_ATTRIBUTES_BYTES: usize = 16 * 1024;

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

fn empty_attributes() -> Value {
    Value::Object(serde_json::Map::new())
}

/// A normalized, immutable-once-admitted record of ONE occurrence. See the
/// module doc comment for the full immutability/authority contract.
///
/// `event_id` identifies exactly one admitted occurrence -- see its own doc
/// comment for the identity/idempotency contract, which is deliberately
/// distinct from any future alert/correlation identity (Phase 124).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DatumEventV1 {
    /// Must equal [`DATUM_EVENT_SCHEMA`].
    pub schema: String,

    /// Identifies exactly ONE admitted occurrence. Stable across a retried
    /// submission of that same occurrence (idempotency); a producer must
    /// mint a NEW `event_id` for a genuinely new, independent occurrence,
    /// even one that shares its `kind`/`subject`/`severity` with a prior
    /// event. This is a REPLAY identity, not a condition/correlation
    /// identity -- two independent observations of "node X is unhealthy"
    /// five minutes apart are two different `DatumEventV1`s with two
    /// different `event_id`s; recognizing them as "the same ongoing
    /// problem" is Phase 124's `DatumAlert` correlation job, not this
    /// type's. This module intentionally implements no condition-level
    /// dedup (see `two_independent_occurrences_of_the_same_condition_are_both_valid_with_distinct_ids`
    /// in the test module below).
    pub event_id: String,

    /// Present whenever the occurrence is scoped to one project; absent
    /// for genuinely DServer-global occurrences (e.g. a DNode registry
    /// mutation -- Phase 121 confirmed DNode identity is DServer-global,
    /// never project-scoped). Kept at the top level (in addition to
    /// whatever `subject` carries) so Phase 123's Event Store can
    /// partition/index by project without unpacking `subject`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,

    /// Present only when the occurrence is application-scoped. Not every
    /// project-scoped event is application-scoped (e.g. a project-level
    /// D-Graph acquisition occurrence), and not every event is even
    /// project-scoped, so this is independently optional from
    /// `project_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,

    /// Coarse, closed, typed partition -- see [`DatumEventCategoryV1`].
    pub category: DatumEventCategoryV1,

    /// The specific occurrence kind, e.g. `"telemetry_stale"`,
    /// `"proposal_accepted"`, `"dependency_unavailable"`. Deliberately a
    /// validated STRING, not a closed Rust enum: Phase 121 catalogued
    /// dozens of plausible future kinds across a dozen subsystems, and
    /// locking all of them into this contract now would force a
    /// schema-breaking change to `DatumEventV1` every time Phase 126 wires
    /// one more producer. `category` already gives callers a stable,
    /// typed, closed dimension to query/filter on; `kind` stays open
    /// within it. Must be non-empty `lower_snake_case` (validated by
    /// [`DatumEventV1::validate`]) so it stays machine-queryable and
    /// namespace-consistent even though it is not enum-closed.
    ///
    /// `category` and `kind` are validated INDEPENDENTLY -- this module
    /// guarantees `kind`'s own syntactic validity, never a semantic
    /// pairing between `category` and `kind` (e.g. `category = Model` with
    /// `kind = "proposal_accepted"` currently validates cleanly, and this
    /// is deliberate, not an oversight -- see
    /// `category_and_kind_are_validated_independently` below). Building a
    /// `category -> allowed kind` mapping now would itself be exactly the
    /// premature global taxonomy this phase is avoiding; that boundary
    /// belongs to Phase 126, once real producers exist to inform it.
    pub kind: String,

    /// The EVENT's own operational significance -- NOT the same axis as
    /// health state, readiness, convergence, or a DMonitor/DForward
    /// finding code. See [`DatumEventSeverityV1`]'s doc comment for the
    /// explicit distinction.
    pub severity: DatumEventSeverityV1,

    /// Where this occurrence's producer subsystem sits -- pure
    /// provenance, never authority. See [`DatumEventSourceV1`].
    pub source: DatumEventSourceV1,

    /// What operational entity this occurrence concerns. See
    /// [`DatumEventSubjectV1`].
    pub subject: DatumEventSubjectV1,

    /// When the occurrence happened, in the ONE canonical UTC RFC 3339
    /// textual form this contract accepts -- see
    /// [`DatumEventV1::validate`]'s timestamp rule for exactly what
    /// "canonical" means here. This is deliberately the ONLY timestamp
    /// this contract carries: when the Event Store (Phase 123) admits
    /// this record, it may additionally track its own `admitted_at`/
    /// `stored_at` alongside the record -- that is Phase 123's field to
    /// own, not this canonical envelope's, since "when the occurrence
    /// happened" and "when the store accepted it" are different facts
    /// with different owners.
    pub occurred_at_utc: String,

    /// Stable pointers into existing authoritative/observational records
    /// -- never a copy of their content. See [`DatumEventEvidenceRefV1`].
    #[serde(default)]
    pub evidence_refs: Vec<DatumEventEvidenceRefV1>,

    /// Producer-supplied hint only. It is NOT the canonical alert
    /// correlation key -- Phase 124's Alert Manager may accept, reject,
    /// normalize, or derive its own key entirely independently of this
    /// value. Present so a producer that already knows an obvious
    /// correlation grouping (e.g. "all evidence for this one reconcile
    /// cycle") isn't forced to discard that knowledge, without this
    /// module (or any future one) treating it as authoritative dedup
    /// state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_hint: Option<String>,

    /// Confidence in parts-per-thousand (`0..=1000`). Present in practice
    /// mainly for `Model`-category / `Gnn`/`Llm`-sourced events, but not
    /// hard-tied to `source`/`category` by validation on its own --
    /// `confidence_permille` remains OPTIONAL even for `Gnn`/`Llm`
    /// sources (only `model_ref` is required for those, see
    /// [`DatumEventV1::validate`]). If present, `model_ref` must also be
    /// present and non-empty (`confidence_requires_model_ref`), because a
    /// bare, unaccountable confidence number is exactly the kind of thing
    /// that could start being treated as more authoritative than it is.
    /// A present confidence value never makes the underlying evidence
    /// authoritative; it only describes how strongly the *inference
    /// producer* itself believed its own conclusion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_permille: Option<u32>,

    /// Stable identifier/description of the model that produced this
    /// occurrence, e.g. `"gnn:deterministic_one_layer_message_passing"`
    /// (matching the naming already used by `core::gnn_dry_run`'s
    /// `GNN_MODEL_KIND`). REQUIRED (non-empty) whenever `source` is `Gnn`
    /// or `Llm`, regardless of whether `confidence_permille` is present --
    /// see [`DatumEventV1::validate`]. Independent of
    /// `confidence_permille` otherwise: a deterministic model may report
    /// provenance with no confidence score at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_ref: Option<String>,

    /// Producer-specific extension payload. Deliberately still typed as a
    /// JSON object (not the whole envelope collapsing to
    /// `serde_json::Value`) and constrained the same way DMonitor's
    /// `facts` already are for structural shape: must be a JSON object,
    /// and every number in it must be finite (consistent with this
    /// contract's JCS digest support, see
    /// [`recompute_datum_event_digest`]). It must also serialize to at
    /// most [`MAX_DATUM_EVENT_ATTRIBUTES_BYTES`] bytes -- the structural
    /// enforcement of `attributes`' intended role as small diagnostic
    /// metadata, not an embedded authoritative document (see the module
    /// doc comment's authority-posture section for why a key-name ban is
    /// NOT used for this instead). There is no depth limit and no key-name
    /// restriction: `attributes` may legitimately contain a key literally
    /// named `desired_state` (e.g. for a `desired_state_mismatch`
    /// diagnostic event) -- see
    /// `descriptive_desired_state_key_in_attributes_is_now_permitted`
    /// below.
    #[serde(default = "empty_attributes")]
    pub attributes: Value,
}

/// Coarse, closed, typed partition of [`DatumEventV1::kind`], carried
/// forward directly from the Phase 121 audit's own event classification
/// (its "EVENT CLASSIFICATION" section) rather than invented fresh.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatumEventCategoryV1 {
    /// Raw pushed/observed occurrences, roughly DMonitor/sentinel-telemetry
    /// sourced (e.g. `telemetry_stale`, `peer_unreachable`).
    Observation,
    /// D-Engine/DForward/DIoT service-execution occurrences (e.g.
    /// `service_started`, `dependency_unavailable`).
    Runtime,
    /// DDeploy/reconciliation authority-transition occurrences (e.g.
    /// `proposal_accepted`, `dmap_revision_changed`).
    ControlPlane,
    /// Derived health/readiness/convergence-TRANSITION occurrences (e.g.
    /// `node_unavailable`). These describe a derived-interpretation
    /// CHANGE, never the health/readiness/convergence authority itself --
    /// that authority remains the existing evaluators referenced via
    /// `evidence_refs`.
    Health,
    /// Root-cause/impact/anomaly-style analytical occurrences (e.g.
    /// `root_cause_candidate`, `anomaly_detected`).
    Analysis,
    /// Human-originated occurrences (e.g. a future manual annotation).
    /// Deliberately distinct from alert acknowledgement, which is Phase
    /// 124's lifecycle concept, not an occurrence in its own right.
    Operator,
    /// AI/GNN/LLM inference occurrences. Carries no special authority --
    /// see `confidence_permille`/`model_ref`.
    Model,
}

/// A small, closed operational-significance vocabulary for the EVENT
/// itself -- Phase 121 proved no such vocabulary exists anywhere in the
/// codebase today (only ad hoc, mutually inconsistent strings per
/// subsystem). Deliberately kept to four levels rather than eight-to-ten.
///
/// This is NOT the same axis as:
/// - health state (`DMonitorHealthV1`: Unknown/Healthy/Degraded/Unhealthy
///   describes a SUBJECT's condition, not an event's significance -- a
///   `Healthy` observation can still be a `High`-severity event, e.g.
///   "node recovered from an extended outage" is operationally significant
///   good news);
/// - readiness (`Ready`/`NotReady` is a governance verdict, not severity);
/// - convergence (`Converged`/`Diverged`/`Unknown` describes agreement
///   between desired and observed state, not how urgently a human should
///   care);
/// - a DMonitor/DForward finding code (those are stable REASON codes
///   explaining WHY, orthogonal to how severe the resulting event is); or
/// - runtime state (`running`/`stopped`, `DockerRuntimeState`, etc. --
///   subject state, not event severity).
///
/// A producer maps whichever of the above it observed into one of these
/// four levels; it never assigns severity by simply echoing one of those
/// other vocabularies' variant names.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DatumEventSeverityV1 {
    Info,
    Warning,
    High,
    Critical,
}

/// Where a [`DatumEventV1`] came from -- pure provenance, never authority.
/// Known DATUM subsystems get a strongly-typed variant; anything else is
/// carried in `Other(String)`.
///
/// # Wire representation and forward compatibility
///
/// Serialization/deserialization is hand-written (not `#[derive]`), so
/// that the wire format stays exactly a bare JSON string for every case --
/// `"source": "dmonitor"`, `"source": "gnn"`, `"source": "future_subsystem"`
/// -- with NO nested wrapper distinguishing known from unknown values. A
/// known token deserializes into its typed variant; any other non-empty
/// `lower_snake_case` string deserializes into `Other(String)`, preserving
/// the exact identity of the unrecognized source rather than collapsing it
/// into a generic literal. This means:
///
/// - A future producer subsystem can start emitting `"source":
///   "future_subsystem"` TODAY, against the CURRENT `datum.event/1`
///   schema, with zero code change to this enum and zero risk of an old
///   reader hard-failing on the whole record (it deserializes into
///   `Other("future_subsystem".to_string())`, which the old reader simply
///   doesn't have a typed match arm for -- exactly the same graceful
///   degradation `DatumEventSubjectV1::Other` already provides).
/// - Giving that subsystem a dedicated typed variant later (e.g. adding
///   `FutureSubsystem` to this enum) is a purely additive, backward- and
///   forward-compatible Rust/code change: existing serialized events
///   (`"source": "future_subsystem"`) start deserializing into the new
///   typed variant instead of `Other`, with no wire-format change at all.
///   No `datum.event/2` schema bump is required merely to introduce a new
///   producer.
/// - `Other`'s string is validated as non-empty `lower_snake_case` (same
///   charset discipline as `kind`) and rejected if it collides with an
///   already-known token (construct the typed variant instead of
///   `Other("dmonitor")`, which would be ambiguous).
#[derive(Debug, Clone, PartialEq)]
pub enum DatumEventSourceV1 {
    DMonitor,
    SentinelTelemetry,
    ActiveMonitor,
    DDeploy,
    Reconciliation,
    DForward,
    DIoT,
    DCode,
    DEngine,
    Runtime,
    ServiceArtifact,
    RuleEngine,
    Operator,
    Gnn,
    Llm,
    /// An external/future producer subsystem not yet given a dedicated
    /// variant. See the enum's own doc comment for the wire-compatibility
    /// contract this exists to satisfy.
    Other(String),
}

impl DatumEventSourceV1 {
    fn wire_str(&self) -> &str {
        match self {
            DatumEventSourceV1::DMonitor => "dmonitor",
            DatumEventSourceV1::SentinelTelemetry => "sentinel_telemetry",
            DatumEventSourceV1::ActiveMonitor => "active_monitor",
            DatumEventSourceV1::DDeploy => "ddeploy",
            DatumEventSourceV1::Reconciliation => "reconciliation",
            DatumEventSourceV1::DForward => "dforward",
            DatumEventSourceV1::DIoT => "diot",
            DatumEventSourceV1::DCode => "dcode",
            DatumEventSourceV1::DEngine => "dengine",
            DatumEventSourceV1::Runtime => "runtime",
            DatumEventSourceV1::ServiceArtifact => "service_artifact",
            DatumEventSourceV1::RuleEngine => "rule_engine",
            DatumEventSourceV1::Operator => "operator",
            DatumEventSourceV1::Gnn => "gnn",
            DatumEventSourceV1::Llm => "llm",
            DatumEventSourceV1::Other(value) => value.as_str(),
        }
    }

    fn from_wire_token(token: &str) -> DatumEventSourceV1 {
        match token {
            "dmonitor" => DatumEventSourceV1::DMonitor,
            "sentinel_telemetry" => DatumEventSourceV1::SentinelTelemetry,
            "active_monitor" => DatumEventSourceV1::ActiveMonitor,
            "ddeploy" => DatumEventSourceV1::DDeploy,
            "reconciliation" => DatumEventSourceV1::Reconciliation,
            "dforward" => DatumEventSourceV1::DForward,
            "diot" => DatumEventSourceV1::DIoT,
            "dcode" => DatumEventSourceV1::DCode,
            "dengine" => DatumEventSourceV1::DEngine,
            "runtime" => DatumEventSourceV1::Runtime,
            "service_artifact" => DatumEventSourceV1::ServiceArtifact,
            "rule_engine" => DatumEventSourceV1::RuleEngine,
            "operator" => DatumEventSourceV1::Operator,
            "gnn" => DatumEventSourceV1::Gnn,
            "llm" => DatumEventSourceV1::Llm,
            other => DatumEventSourceV1::Other(other.to_string()),
        }
    }

    /// True for `Gnn`/`Llm` -- the two sources Phase 122's model-provenance
    /// rule applies to. Deliberately NOT true for `Other`: an unrecognized
    /// external source is provenance-only, not implicitly an inference
    /// producer, unless/until it is given a dedicated typed variant.
    fn requires_model_provenance(&self) -> bool {
        matches!(self, DatumEventSourceV1::Gnn | DatumEventSourceV1::Llm)
    }

    fn validate(&self) -> Vec<CanonicalValidationFinding> {
        if let DatumEventSourceV1::Other(value) = self {
            if !is_lower_snake_case(value) {
                return vec![finding(
                    "source_other_invalid_format",
                    format!("source (other) must be non-empty lower_snake_case, got '{value}'"),
                )];
            }
            if !matches!(
                DatumEventSourceV1::from_wire_token(value),
                DatumEventSourceV1::Other(_)
            ) {
                return vec![finding(
                    "source_other_collides_with_known_value",
                    format!(
                        "source (other) value '{value}' collides with a known source token -- construct the typed variant instead"
                    ),
                )];
            }
        }
        Vec::new()
    }
}

impl Serialize for DatumEventSourceV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.wire_str())
    }
}

impl<'de> Deserialize<'de> for DatumEventSourceV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let token = String::deserialize(deserializer)?;
        Ok(DatumEventSourceV1::from_wire_token(&token))
    }
}

/// What operational entity a [`DatumEventV1`] concerns. A tagged enum
/// (mirrors `DMonitorSubjectV1`'s established shape) rather than a bag of
/// scattered optional ids on the envelope itself, so a given event's
/// scope is exactly one well-formed, typed thing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum DatumEventSubjectV1 {
    /// A genuinely DServer-global occurrence with no project scope at all
    /// (e.g. a future DNode-registry-mutation event -- Phase 121 confirmed
    /// DNode identity is DServer-global, never project-scoped).
    #[serde(rename = "dserver_global")]
    DServerGlobal,

    #[serde(rename = "project")]
    Project { project_id: String },

    #[serde(rename = "application")]
    Application {
        project_id: String,
        application_id: String,
    },

    /// Infrastructure D-Node identity (`core::dnode_registry`) --
    /// deliberately carries no `project_id`, consistent with Phase 121's
    /// finding that D-Node declaration is project-agnostic infrastructure.
    #[serde(rename = "dnode")]
    DNode { node_id: String },

    /// A D-Serv (service) realization, optionally pinned to one node.
    #[serde(rename = "dserv")]
    DServ {
        project_id: String,
        application_id: String,
        service_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_id: Option<String>,
    },

    #[serde(rename = "dcall")]
    DCall {
        project_id: String,
        application_id: String,
        call_id: String,
    },

    /// A DIoT entity, optionally pinned to one placement.
    #[serde(rename = "diot")]
    DIoT {
        project_id: String,
        application_id: String,
        diot_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diot_placement_id: Option<String>,
    },

    #[serde(rename = "service_artifact")]
    ServiceArtifact {
        project_id: String,
        artifact_id: String,
    },

    /// One concrete realized runtime instance of a service on one node
    /// (distinct from `DServ`, which may be node-agnostic).
    #[serde(rename = "runtime_instance")]
    RuntimeInstance {
        project_id: String,
        application_id: String,
        service_id: String,
        node_id: String,
    },

    /// A dependency edge (e.g. a DForward-evaluated dependency between one
    /// consuming node and a producer service).
    #[serde(rename = "dependency_edge")]
    DependencyEdge {
        project_id: String,
        application_id: String,
        node_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dependency_service_id: Option<String>,
    },

    #[serde(rename = "deployment_proposal")]
    DeploymentProposal {
        project_id: String,
        application_id: String,
        proposal_id: String,
    },

    /// A DMap identity/revision (e.g. for a future `dmap_revision_changed`
    /// occurrence).
    #[serde(rename = "dmap")]
    DMap {
        project_id: String,
        application_id: String,
        dmap_id: String,
    },

    /// Escape hatch for an operational entity not yet given its own
    /// variant. Kept minimally structured (a kind label plus one stable
    /// id, optionally project-scoped) -- never an arbitrary blob -- so a
    /// genuinely new subject shape can be represented without a
    /// schema-breaking change to this enum, at the cost of losing typed
    /// field names until a dedicated variant is added (expected as each
    /// Phase 126 producer is wired).
    #[serde(rename = "other")]
    Other {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        project_id: Option<String>,
        entity_kind: String,
        entity_id: String,
    },
}

impl DatumEventSubjectV1 {
    /// The `project_id` this subject variant carries, if any. Mirrors
    /// `DMonitorSubjectV1::project_id`'s established pattern exactly.
    pub fn project_id(&self) -> Option<&str> {
        match self {
            DatumEventSubjectV1::Project { project_id }
            | DatumEventSubjectV1::Application { project_id, .. }
            | DatumEventSubjectV1::DServ { project_id, .. }
            | DatumEventSubjectV1::DCall { project_id, .. }
            | DatumEventSubjectV1::DIoT { project_id, .. }
            | DatumEventSubjectV1::ServiceArtifact { project_id, .. }
            | DatumEventSubjectV1::RuntimeInstance { project_id, .. }
            | DatumEventSubjectV1::DependencyEdge { project_id, .. }
            | DatumEventSubjectV1::DeploymentProposal { project_id, .. }
            | DatumEventSubjectV1::DMap { project_id, .. } => Some(project_id.as_str()),
            DatumEventSubjectV1::Other { project_id, .. } => project_id.as_deref(),
            DatumEventSubjectV1::DServerGlobal | DatumEventSubjectV1::DNode { .. } => None,
        }
    }

    /// The `application_id` this subject variant carries, if any.
    pub fn application_id(&self) -> Option<&str> {
        match self {
            DatumEventSubjectV1::Application { application_id, .. }
            | DatumEventSubjectV1::DServ { application_id, .. }
            | DatumEventSubjectV1::DCall { application_id, .. }
            | DatumEventSubjectV1::DIoT { application_id, .. }
            | DatumEventSubjectV1::RuntimeInstance { application_id, .. }
            | DatumEventSubjectV1::DependencyEdge { application_id, .. }
            | DatumEventSubjectV1::DeploymentProposal { application_id, .. }
            | DatumEventSubjectV1::DMap { application_id, .. } => Some(application_id.as_str()),
            DatumEventSubjectV1::DServerGlobal
            | DatumEventSubjectV1::Project { .. }
            | DatumEventSubjectV1::DNode { .. }
            | DatumEventSubjectV1::ServiceArtifact { .. }
            | DatumEventSubjectV1::Other { .. } => None,
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
            DatumEventSubjectV1::DServerGlobal => {}
            DatumEventSubjectV1::Project { project_id } => require("project_id", project_id),
            DatumEventSubjectV1::Application {
                project_id,
                application_id,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
            }
            DatumEventSubjectV1::DNode { node_id } => require("node_id", node_id),
            DatumEventSubjectV1::DServ {
                project_id,
                application_id,
                service_id,
                ..
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("service_id", service_id);
            }
            DatumEventSubjectV1::DCall {
                project_id,
                application_id,
                call_id,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("call_id", call_id);
            }
            DatumEventSubjectV1::DIoT {
                project_id,
                application_id,
                diot_id,
                ..
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("diot_id", diot_id);
            }
            DatumEventSubjectV1::ServiceArtifact {
                project_id,
                artifact_id,
            } => {
                require("project_id", project_id);
                require("artifact_id", artifact_id);
            }
            DatumEventSubjectV1::RuntimeInstance {
                project_id,
                application_id,
                service_id,
                node_id,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("service_id", service_id);
                require("node_id", node_id);
            }
            DatumEventSubjectV1::DependencyEdge {
                project_id,
                application_id,
                node_id,
                ..
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("node_id", node_id);
            }
            DatumEventSubjectV1::DeploymentProposal {
                project_id,
                application_id,
                proposal_id,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("proposal_id", proposal_id);
            }
            DatumEventSubjectV1::DMap {
                project_id,
                application_id,
                dmap_id,
            } => {
                require("project_id", project_id);
                require("application_id", application_id);
                require("dmap_id", dmap_id);
            }
            // Phase 122 freeze review, F9: `project_id` is validated
            // directly here when present, rather than relying solely on
            // the top-level project_id/subject cross-check (which
            // previously produced a confusing "must be Some(\"\")"
            // message for this exact case instead of a clean
            // `missing_field`).
            DatumEventSubjectV1::Other {
                project_id,
                entity_kind,
                entity_id,
            } => {
                if let Some(project_id) = project_id {
                    require("project_id", project_id);
                }
                require("entity_kind", entity_kind);
                require("entity_id", entity_id);
            }
        }
        findings
    }
}

/// A stable pointer into an existing authoritative/observational record --
/// never a copy of that record's content. Tagged enum, one variant per
/// evidence source the Phase 121 audit identified as a plausible
/// referenceable stable identifier.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields, tag = "kind")]
pub enum DatumEventEvidenceRefV1 {
    /// `CanonicalDMonitorObservationV1::observation_id`
    /// (`core::canonical_dmonitor`).
    DMonitorObservation { observation_id: String },

    /// `OperationalEvidenceEvent::event_id` (`core::operational_evidence`).
    /// Field named `operational_evidence_event_id`, not `event_id`, so a
    /// log or payload containing both this reference and the enclosing
    /// `DatumEventV1::event_id` is never ambiguous about which id belongs
    /// to which system.
    OperationalEvidence {
        operational_evidence_event_id: String,
    },

    /// A DMap identity/revision/digest triple. Phase 122's freeze review
    /// (F3) required proving that reusing `CanonicalContractReferenceV1`
    /// (`core::canonical_dmap`, already reused across 15 files in this
    /// crate) preserves the exact wire representation before accepting the
    /// reuse -- confirmed by
    /// `dmap_evidence_ref_wire_representation_is_unchanged_after_reuse`
    /// and `dmap_evidence_ref_still_rejects_unknown_fields_after_reuse`
    /// below: `#[serde(flatten)]` produces the identical flat
    /// `{"kind":"dmap","id":...,"revision":...,"digest":...}` shape the
    /// original hand-duplicated fields produced, AND still rejects an
    /// unrecognized extra field (verified empirically, not assumed --
    /// `CanonicalContractReferenceV1` itself has no `deny_unknown_fields`,
    /// but the outer enum's `#[serde(deny_unknown_fields)]` still applies
    /// to the flattened field set in this internally-tagged-enum shape).
    DMap {
        #[serde(flatten)]
        reference: CanonicalContractReferenceV1,
    },

    /// A DDeploy proposal identity/digest (`core::ddeploy::DDeployProposalV1`).
    Proposal {
        proposal_id: String,
        proposal_digest: String,
    },

    /// A `ServiceArtifact` identity/generation (`core::service_artifact_registry`).
    ServiceArtifact {
        artifact_id: String,
        execution_generation: u64,
    },

    /// A WASM/native runtime execution report id.
    RuntimeReport { report_id: String },

    /// Escape hatch for an evidence source not yet given its own variant.
    /// Still minimally structured -- a kind label plus one stable id,
    /// never an arbitrary blob.
    Other {
        evidence_kind: String,
        reference_id: String,
    },
}

impl DatumEventEvidenceRefV1 {
    fn validate(&self, index: usize) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();
        let mut require = |label: &str, value: &str| {
            if value.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    format!("evidence_refs[{index}].{label} must be non-empty"),
                ));
            }
        };
        match self {
            DatumEventEvidenceRefV1::DMonitorObservation { observation_id } => {
                require("observation_id", observation_id);
            }
            DatumEventEvidenceRefV1::OperationalEvidence {
                operational_evidence_event_id,
            } => {
                require(
                    "operational_evidence_event_id",
                    operational_evidence_event_id,
                );
            }
            DatumEventEvidenceRefV1::DMap { reference } => {
                require("id", &reference.id);
                if reference.revision < 1 {
                    findings.push(finding(
                        "malformed_ref",
                        format!("evidence_refs[{index}].revision must be >= 1"),
                    ));
                }
                if !jcs::is_canonical_digest_format(&reference.digest) {
                    findings.push(finding(
                        "malformed_ref",
                        format!(
                            "evidence_refs[{index}].digest must be 'sha256:<64 lowercase hex>'"
                        ),
                    ));
                }
            }
            DatumEventEvidenceRefV1::Proposal {
                proposal_id,
                proposal_digest,
            } => {
                require("proposal_id", proposal_id);
                if !jcs::is_canonical_digest_format(proposal_digest) {
                    findings.push(finding(
                        "malformed_ref",
                        format!(
                            "evidence_refs[{index}].proposal_digest must be 'sha256:<64 lowercase hex>'"
                        ),
                    ));
                }
            }
            DatumEventEvidenceRefV1::ServiceArtifact { artifact_id, .. } => {
                require("artifact_id", artifact_id);
            }
            DatumEventEvidenceRefV1::RuntimeReport { report_id } => {
                require("report_id", report_id);
            }
            DatumEventEvidenceRefV1::Other {
                evidence_kind,
                reference_id,
            } => {
                require("evidence_kind", evidence_kind);
                require("reference_id", reference_id);
            }
        }
        findings
    }
}

impl DatumEventV1 {
    /// Fail-closed structural validation. Rejects empty ids, empty
    /// required scope, an invalid `kind` format, invalid subject/evidence
    /// references, an invalid confidence range, a confidence value with no
    /// accompanying provenance, a `Gnn`/`Llm` source with no `model_ref`,
    /// a malformed or non-canonical timestamp, an unknown schema version,
    /// a non-object or oversized `attributes`, and a non-finite number
    /// anywhere in `attributes`.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != DATUM_EVENT_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!(
                    "schema must be '{DATUM_EVENT_SCHEMA}', got '{}'",
                    self.schema
                ),
            ));
        }
        if self.event_id.trim().is_empty() {
            findings.push(finding("missing_field", "event_id must be non-empty"));
        }
        if let Some(project_id) = &self.project_id {
            if project_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "project_id, if present, must be non-empty",
                ));
            }
        }
        if let Some(application_id) = &self.application_id {
            if application_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "application_id, if present, must be non-empty",
                ));
            }
        }
        if self.kind.trim().is_empty() {
            findings.push(finding("missing_field", "kind must be non-empty"));
        } else if !is_lower_snake_case(&self.kind) {
            findings.push(finding(
                "kind_invalid_format",
                format!(
                    "kind must be lower_snake_case (ascii a-z, 0-9, '_'), got '{}'",
                    self.kind
                ),
            ));
        }

        match canonicalize_rfc3339_utc(&self.occurred_at_utc) {
            Err(message) => findings.push(finding("invalid_timestamp", message)),
            Ok(canonical) if canonical != self.occurred_at_utc => {
                findings.push(finding(
                    "timestamp_not_canonical",
                    format!(
                        "occurred_at_utc must be the canonical UTC RFC3339 form '{canonical}', got '{}'",
                        self.occurred_at_utc
                    ),
                ));
            }
            Ok(_) => {}
        }

        findings.extend(self.subject.validate());

        // Mirrors `CanonicalDMonitorObservationV1`'s established
        // top-level/subject project-id consistency rule exactly.
        if let Some(subject_project_id) = self.subject.project_id() {
            match &self.project_id {
                Some(top_level) if top_level == subject_project_id => {}
                Some(top_level) => findings.push(finding(
                    "subject_project_id_inconsistent",
                    format!(
                        "event.project_id '{top_level}' does not match subject.project_id '{subject_project_id}'"
                    ),
                )),
                None => findings.push(finding(
                    "subject_project_id_inconsistent",
                    format!(
                        "event.project_id must be Some(\"{subject_project_id}\") to match the typed subject's project_id, got None"
                    ),
                )),
            }
        }
        if let Some(subject_application_id) = self.subject.application_id() {
            match &self.application_id {
                Some(top_level) if top_level == subject_application_id => {}
                Some(top_level) => findings.push(finding(
                    "subject_application_id_inconsistent",
                    format!(
                        "event.application_id '{top_level}' does not match subject.application_id '{subject_application_id}'"
                    ),
                )),
                None => findings.push(finding(
                    "subject_application_id_inconsistent",
                    format!(
                        "event.application_id must be Some(\"{subject_application_id}\") to match the typed subject's application_id, got None"
                    ),
                )),
            }
        }

        for (index, evidence_ref) in self.evidence_refs.iter().enumerate() {
            findings.extend(evidence_ref.validate(index));
        }

        if let Some(correlation_hint) = &self.correlation_hint {
            if correlation_hint.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "correlation_hint, if present, must be non-empty",
                ));
            }
        }

        if let Some(confidence_permille) = self.confidence_permille {
            if confidence_permille > CONFIDENCE_PERMILLE_MAX {
                findings.push(finding(
                    "confidence_out_of_range",
                    format!(
                        "confidence_permille must be 0..={CONFIDENCE_PERMILLE_MAX}, got {confidence_permille}"
                    ),
                ));
            }
            // A confidence score with no stated provenance is exactly the
            // "unaccountable number" the authority-posture rule ("a
            // present confidence never makes the underlying evidence
            // authoritative") guards against -- requiring `model_ref`
            // keeps every confidence value traceable to what produced it.
            // Kept independent of the source-based rule below: this
            // applies to ANY source that supplies a confidence value, not
            // only Gnn/Llm, since a future non-Gnn/Llm "model-like" source
            // could plausibly want to report a confidence too.
            if self
                .model_ref
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
            {
                findings.push(finding(
                    "confidence_requires_model_ref",
                    "confidence_permille requires a non-empty model_ref",
                ));
            }
        }
        if let Some(model_ref) = &self.model_ref {
            if model_ref.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "model_ref, if present, must be non-empty",
                ));
            }
        }
        // Phase 122 freeze review, F2: independent of confidence, a
        // Gnn/Llm-sourced event must always identify the model that
        // produced it.
        if self.source.requires_model_provenance()
            && self
                .model_ref
                .as_deref()
                .is_none_or(|s| s.trim().is_empty())
        {
            findings.push(finding(
                "model_ref_required_for_model_source",
                format!(
                    "model_ref is required and must be non-empty when source is '{}'",
                    self.source.wire_str()
                ),
            ));
        }
        findings.extend(self.source.validate());

        if !self.attributes.is_object() {
            findings.push(finding(
                "attributes_not_object",
                "attributes must be a JSON object",
            ));
        }
        if !json_numbers_are_finite(&self.attributes) {
            findings.push(finding(
                "non_finite_attribute",
                "attributes must not contain a non-finite numeric value",
            ));
        }
        match serde_json::to_vec(&self.attributes) {
            Ok(bytes) if bytes.len() > MAX_DATUM_EVENT_ATTRIBUTES_BYTES => {
                findings.push(finding(
                    "attributes_too_large",
                    format!(
                        "attributes must serialize to at most {MAX_DATUM_EVENT_ATTRIBUTES_BYTES} bytes, got {}",
                        bytes.len()
                    ),
                ));
            }
            Ok(_) => {}
            Err(error) => findings.push(finding(
                "attributes_not_object",
                format!("attributes could not be serialized: {error}"),
            )),
        }

        findings
    }
}

fn is_lower_snake_case(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !trimmed.starts_with('_')
        && !trimmed.ends_with('_')
}

/// Local re-implementation of `canonical_dmonitor`'s identical finite-number
/// check: a few lines duplicated within the SAME crate is a deliberate,
/// minimal choice to keep this module self-contained (Phase 122's "small,
/// focused module" scope) rather than exporting a shared helper across an
/// otherwise-unrelated module boundary.
///
/// Stack-safety note (Phase 122 freeze review, item 6): this recursion has
/// no explicit depth limit, and neither does `serde_json`'s own `Value`
/// deserialization -- unlike deserializing into a fixed Rust struct (whose
/// recursion is bounded by the type's own shape), building a `Value` tree
/// from arbitrary JSON has no built-in depth guarantee, so a pathological
/// input (e.g. thousands of nested empty arrays) could in principle
/// exhaust the stack. Critically, that exposure exists at the JSON-PARSING
/// boundary -- wherever raw bytes are deserialized into a `DatumEventV1`
/// (an HTTP/API layer that does not exist in this phase) -- not in this
/// function specifically: by the time `attributes` reaches `validate()`,
/// the `Value` tree has already been built, so a depth limit added HERE
/// would not close the actual risk window; it would only add complexity
/// without preventing the overflow, which (if it happens) already
/// happened during deserialization. A real mitigation (e.g. a
/// depth-counting deserializer) belongs at that future ingestion
/// boundary, not in this type's validator -- consistent with "do not add
/// a depth limit unless implementation safety requires it," since adding
/// one here would not, in fact, provide the safety.
fn json_numbers_are_finite(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64().is_none_or(f64::is_finite),
        Value::Array(items) => items.iter().all(json_numbers_are_finite),
        Value::Object(map) => map.values().all(json_numbers_are_finite),
        _ => true,
    }
}

/// The Phase 122 canonical UTC timestamp rule: parse `raw` as RFC 3339,
/// convert to UTC, and re-render using `chrono`'s `SecondsFormat::AutoSi`
/// with a literal `Z` suffix (never `+00:00`). `AutoSi` snaps the
/// sub-second part to the SMALLEST of four discrete tiers that exactly
/// represents the parsed nanosecond value -- no fractional part, `.SSS`
/// (milliseconds), `.SSSSSS` (microseconds), or `.SSSSSSSSS` (nanoseconds)
/// -- rather than an arbitrary minimal digit count (verified empirically:
/// 500_000_000 ns canonicalizes to `.500Z`, not `.5Z`, since 500ms is the
/// smallest exact tier, not the smallest exact digit count). This still
/// satisfies "do not unintentionally collapse meaningful precision": a
/// value with genuine microsecond/nanosecond significance keeps that
/// tier, it just never gets fewer digits than its tier requires.
/// `DatumEventV1::validate` REJECTS any `occurred_at_utc` that does not
/// already equal this function's output byte-for-byte -- it never
/// silently rewrites the submitted value. This means a producer
/// submitting a non-UTC offset (e.g. `-03:00`) or a non-canonical
/// fractional representation (e.g. `.500000000Z`, whose canonical
/// millisecond-tier form is `.500Z`) is told exactly what canonical
/// string is required, rather than having its input silently
/// reinterpreted.
fn canonicalize_rfc3339_utc(raw: &str) -> Result<String, String> {
    let parsed = DateTime::parse_from_rfc3339(raw).map_err(|error| {
        format!("occurred_at_utc is not a valid RFC 3339 timestamp: '{raw}' ({error})")
    })?;
    Ok(parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true))
}

/// Canonical JCS+SHA-256 digest (ADR-0006) of the FULL admitted record,
/// `event_id` included.
///
/// This fingerprints exact record CONTENT for integrity/replay
/// verification -- it is explicitly NOT a condition-correlation key. Two
/// independent occurrences of the same underlying condition (different
/// `event_id` and/or `occurred_at_utc`) are expected, and allowed, to
/// produce different digests; Phase 124's Alert Manager derives whatever
/// condition-level correlation key it needs on its own terms, independent
/// of both this digest and `event_id`. Including `event_id` in the digest
/// (rather than excluding it to produce a "condition fingerprint") is a
/// deliberate choice: it mirrors `CanonicalDMonitorObservationV1`'s own
/// digest, which includes `observation_id`, and keeps this function's
/// contract simple and unambiguous -- "the digest of this exact record" --
/// rather than trying to also serve as an implicit, undocumented
/// correlation primitive that Phase 124 would then have to either reuse
/// correctly or work around. `occurred_at_utc` participates in the digest
/// too, for the same reason -- it is an ordinary field of the record.
pub fn recompute_datum_event_digest(event: &DatumEventV1) -> Result<String, String> {
    let value: Value =
        serde_json::to_value(event).map_err(|error| format!("serialize DatumEventV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal_project_event() -> DatumEventV1 {
        DatumEventV1 {
            schema: DATUM_EVENT_SCHEMA.to_string(),
            event_id: "datumevent:11111111-1111-1111-1111-111111111111".to_string(),
            project_id: Some("proj:lab".to_string()),
            application_id: None,
            category: DatumEventCategoryV1::Observation,
            kind: "telemetry_stale".to_string(),
            severity: DatumEventSeverityV1::Warning,
            source: DatumEventSourceV1::SentinelTelemetry,
            subject: DatumEventSubjectV1::Project {
                project_id: "proj:lab".to_string(),
            },
            occurred_at_utc: "2026-09-29T00:00:00Z".to_string(),
            evidence_refs: Vec::new(),
            correlation_hint: None,
            confidence_permille: None,
            model_ref: None,
            attributes: empty_attributes(),
        }
    }

    // ------------------------------------------------------- REQUIREMENT 1
    #[test]
    fn minimal_valid_deterministic_event_validates_cleanly() {
        let event = minimal_project_event();
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ------------------------------------------------------- REQUIREMENT 2
    #[test]
    fn project_scoped_event_is_valid() {
        let event = minimal_project_event();
        assert_eq!(event.subject.project_id(), Some("proj:lab"));
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ------------------------------------------------------- REQUIREMENT 3
    #[test]
    fn application_service_scoped_event_is_valid() {
        let mut event = minimal_project_event();
        event.application_id = Some("app:pipeline".to_string());
        event.category = DatumEventCategoryV1::Runtime;
        event.kind = "service_failed".to_string();
        event.severity = DatumEventSeverityV1::Critical;
        event.source = DatumEventSourceV1::DEngine;
        event.subject = DatumEventSubjectV1::RuntimeInstance {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "decoder".to_string(),
            node_id: "fog-01".to_string(),
        };
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ------------------------------------------------------- REQUIREMENT 4
    #[test]
    fn infrastructure_dnode_event_is_globally_scoped_with_no_project_id() {
        let mut event = minimal_project_event();
        event.project_id = None;
        event.category = DatumEventCategoryV1::ControlPlane;
        event.kind = "dnode_declared".to_string();
        event.severity = DatumEventSeverityV1::Info;
        event.source = DatumEventSourceV1::DDeploy;
        event.subject = DatumEventSubjectV1::DNode {
            node_id: "fog-01".to_string(),
        };
        assert_eq!(event.subject.project_id(), None);
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn dserver_global_subject_carries_no_scope_and_is_valid() {
        let mut event = minimal_project_event();
        event.project_id = None;
        event.subject = DatumEventSubjectV1::DServerGlobal;
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ------------------------------------------------------- REQUIREMENT 5
    #[test]
    fn event_with_evidence_refs_is_valid() {
        let mut event = minimal_project_event();
        event.evidence_refs = vec![
            DatumEventEvidenceRefV1::DMonitorObservation {
                observation_id: "dmonitor-obs:11111111-1111-1111-1111-111111111111".to_string(),
            },
            DatumEventEvidenceRefV1::DMap {
                reference: CanonicalContractReferenceV1 {
                    id: "dmap:1".to_string(),
                    revision: 3,
                    digest: format!("sha256:{}", "a".repeat(64)),
                },
            },
            DatumEventEvidenceRefV1::OperationalEvidence {
                operational_evidence_event_id: "opev:test".to_string(),
            },
        ];
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn malformed_dmap_evidence_ref_is_rejected() {
        let mut event = minimal_project_event();
        event.evidence_refs = vec![DatumEventEvidenceRefV1::DMap {
            reference: CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 0,
                digest: "not-a-digest".to_string(),
            },
        }];
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "malformed_ref"),
            "{findings:?}"
        );
    }

    #[test]
    fn empty_evidence_ref_id_is_rejected() {
        let mut event = minimal_project_event();
        event.evidence_refs = vec![DatumEventEvidenceRefV1::RuntimeReport {
            report_id: String::new(),
        }];
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "missing_field"),
            "{findings:?}"
        );
    }

    // ----------------------------------------------------- F3: DMAP REUSE

    /// Proves reuse of `CanonicalContractReferenceV1` did NOT change the
    /// wire representation relative to the original hand-duplicated
    /// `{id, revision, digest}` fields -- required by the freeze review
    /// before accepting F3. The tag value asserted here is `"DMap"`
    /// (PascalCase), not `"dmap"`: unlike `DatumEventSubjectV1`'s
    /// variants, none of `DatumEventEvidenceRefV1`'s variants were ever
    /// given an explicit `#[serde(rename = ...)]` in the original Phase
    /// 122 implementation, so every tag (including this one, both before
    /// and after this patch) is the default literal Rust variant name.
    /// This was discovered empirically while writing this exact test (an
    /// initial `"dmap"` expectation failed) -- see the Phase 122 hardening
    /// freeze-review report for the follow-up recommendation this
    /// surfaced (a separate, out-of-scope naming-consistency pass across
    /// all `DatumEventEvidenceRefV1` tags, deliberately not bundled into
    /// this DRY-only change).
    #[test]
    fn dmap_evidence_ref_wire_representation_is_unchanged_after_reuse() {
        let expected_shape = json!({
            "kind": "DMap",
            "id": "dmap:1",
            "revision": 3,
            "digest": format!("sha256:{}", "a".repeat(64)),
        });
        let value = DatumEventEvidenceRefV1::DMap {
            reference: CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: format!("sha256:{}", "a".repeat(64)),
            },
        };
        assert_eq!(serde_json::to_value(&value).unwrap(), expected_shape);
    }

    /// Proves the `#[serde(flatten)]`-based reuse did NOT silently weaken
    /// `deny_unknown_fields` for this specific variant -- empirically
    /// verified rather than assumed, per the freeze review's explicit
    /// instruction not to distort the contract merely for DRY.
    #[test]
    fn dmap_evidence_ref_still_rejects_unknown_fields_after_reuse() {
        let json_with_bogus_field = json!({
            "kind": "DMap",
            "id": "dmap:1",
            "revision": 3,
            "digest": format!("sha256:{}", "a".repeat(64)),
            "bogus_extra_field": "should not be accepted",
        });
        let result: Result<DatumEventEvidenceRefV1, _> =
            serde_json::from_value(json_with_bogus_field);
        assert!(
            result.is_err(),
            "flattened DMap evidence ref must still reject an unrecognized field"
        );
    }

    // ------------------------------------------------------- REQUIREMENT 6
    #[test]
    fn inferred_gnn_event_with_confidence_and_model_provenance_is_valid() {
        let mut event = minimal_project_event();
        event.category = DatumEventCategoryV1::Model;
        event.kind = "gnn_inference".to_string();
        event.severity = DatumEventSeverityV1::Info;
        event.source = DatumEventSourceV1::Gnn;
        event.confidence_permille = Some(870);
        event.model_ref = Some("gnn:deterministic_one_layer_message_passing".to_string());
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ------------------------------------------------------- REQUIREMENT 7
    #[test]
    fn confidence_above_max_permille_is_rejected() {
        let mut event = minimal_project_event();
        event.confidence_permille = Some(1001);
        event.model_ref = Some("gnn:x".to_string());
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "confidence_out_of_range"),
            "{findings:?}"
        );
    }

    #[test]
    fn confidence_without_model_ref_is_rejected() {
        let mut event = minimal_project_event();
        event.confidence_permille = Some(500);
        event.model_ref = None;
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "confidence_requires_model_ref"),
            "{findings:?}"
        );
    }

    // -------------------------------------------- F2: GNN/LLM PROVENANCE

    #[test]
    fn gnn_without_confidence_or_model_ref_is_rejected() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Gnn;
        event.confidence_permille = None;
        event.model_ref = None;
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "model_ref_required_for_model_source"),
            "{findings:?}"
        );
    }

    #[test]
    fn gnn_with_confidence_but_no_model_ref_is_rejected() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Gnn;
        event.confidence_permille = Some(500);
        event.model_ref = None;
        let findings = event.validate();
        // Both rules fire for this input; the important assertion is that
        // the source-based rule fires regardless of the confidence-based
        // one, proving they are independent as required.
        assert!(
            findings
                .iter()
                .any(|f| f.code == "model_ref_required_for_model_source"),
            "{findings:?}"
        );
    }

    #[test]
    fn gnn_with_model_ref_and_no_confidence_is_valid() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Gnn;
        event.confidence_permille = None;
        event.model_ref = Some("gnn:deterministic_one_layer_message_passing".to_string());
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn gnn_with_model_ref_and_confidence_is_valid() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Gnn;
        event.confidence_permille = Some(750);
        event.model_ref = Some("gnn:deterministic_one_layer_message_passing".to_string());
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn llm_without_model_ref_is_rejected() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Llm;
        event.confidence_permille = None;
        event.model_ref = None;
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "model_ref_required_for_model_source"),
            "{findings:?}"
        );
    }

    #[test]
    fn llm_with_model_ref_is_valid() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Llm;
        event.model_ref = Some("llm:some-external-model".to_string());
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn non_model_source_never_requires_model_ref() {
        let event = minimal_project_event(); // source = SentinelTelemetry
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ------------------------------------------------------- REQUIREMENT 8
    #[test]
    fn empty_event_id_is_rejected() {
        let mut event = minimal_project_event();
        event.event_id = "   ".to_string();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "missing_field"),
            "{findings:?}"
        );
    }

    #[test]
    fn empty_kind_is_rejected() {
        let mut event = minimal_project_event();
        event.kind = String::new();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "missing_field"),
            "{findings:?}"
        );
    }

    #[test]
    fn empty_subject_field_is_rejected() {
        let mut event = minimal_project_event();
        event.subject = DatumEventSubjectV1::Project {
            project_id: String::new(),
        };
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "missing_field"),
            "{findings:?}"
        );
    }

    #[test]
    fn kind_with_invalid_characters_is_rejected() {
        let mut event = minimal_project_event();
        event.kind = "Telemetry-Stale".to_string();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "kind_invalid_format"),
            "{findings:?}"
        );
    }

    // ------------------------------------------------------- REQUIREMENT 9
    #[test]
    fn wrong_schema_is_rejected() {
        let mut event = minimal_project_event();
        event.schema = "datum.something-else/1".to_string();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "schema_invalid"),
            "{findings:?}"
        );
    }

    #[test]
    fn unknown_top_level_field_is_rejected_at_deserialization() {
        let event = minimal_project_event();
        let mut value = serde_json::to_value(&event).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("bogus_future_field".to_string(), json!("x"));
        let result: Result<DatumEventV1, _> = serde_json::from_value(value);
        assert!(result.is_err());
    }

    // ------------------------------------------------------ REQUIREMENT 10
    #[test]
    fn serialization_round_trips_exactly() {
        let event = minimal_project_event();
        let json_text = serde_json::to_string(&event).unwrap();
        let restored: DatumEventV1 = serde_json::from_str(&json_text).unwrap();
        assert_eq!(event, restored);
    }

    #[test]
    fn evidence_refs_and_confidence_round_trip_through_json() {
        let mut event = minimal_project_event();
        event.evidence_refs = vec![DatumEventEvidenceRefV1::ServiceArtifact {
            artifact_id: "artifact:decoder".to_string(),
            execution_generation: 4,
        }];
        event.source = DatumEventSourceV1::Gnn;
        event.confidence_permille = Some(250);
        event.model_ref = Some("gnn:x".to_string());
        event.correlation_hint = Some("reconcile-cycle:42".to_string());
        let json_text = serde_json::to_string(&event).unwrap();
        let restored: DatumEventV1 = serde_json::from_str(&json_text).unwrap();
        assert_eq!(event, restored);
    }

    // ------------------------------------------------------ REQUIREMENT 11
    #[test]
    fn digest_is_deterministic() {
        let event = minimal_project_event();
        let first = recompute_datum_event_digest(&event).unwrap();
        let second = recompute_datum_event_digest(&event).unwrap();
        assert_eq!(first, second);
        assert!(jcs::is_canonical_digest_format(&first));
    }

    /// "Same canonical event retried produces the same digest" -- the
    /// exact idempotent-replay semantics `event_id` and the digest
    /// together are supposed to guarantee.
    #[test]
    fn same_canonical_event_retried_produces_the_same_digest() {
        let first_submission = minimal_project_event();
        let retried_submission = minimal_project_event();
        assert_eq!(first_submission, retried_submission);
        assert_eq!(
            recompute_datum_event_digest(&first_submission).unwrap(),
            recompute_datum_event_digest(&retried_submission).unwrap()
        );
    }

    #[test]
    fn digest_changes_when_content_changes() {
        let a = minimal_project_event();
        let mut b = minimal_project_event();
        b.severity = DatumEventSeverityV1::Critical;
        assert_ne!(
            recompute_datum_event_digest(&a).unwrap(),
            recompute_datum_event_digest(&b).unwrap()
        );
    }

    #[test]
    fn digest_changes_when_event_id_changes() {
        let a = minimal_project_event();
        let mut b = minimal_project_event();
        b.event_id = "datumevent:22222222-2222-2222-2222-222222222222".to_string();
        assert_ne!(
            recompute_datum_event_digest(&a).unwrap(),
            recompute_datum_event_digest(&b).unwrap()
        );
    }

    #[test]
    fn digest_changes_when_occurred_at_utc_changes() {
        let a = minimal_project_event();
        let mut b = minimal_project_event();
        b.occurred_at_utc = "2026-09-29T00:05:00Z".to_string();
        assert_ne!(
            recompute_datum_event_digest(&a).unwrap(),
            recompute_datum_event_digest(&b).unwrap()
        );
    }

    #[test]
    fn non_finite_number_in_attributes_is_rejected_at_digest_time() {
        let mut event = minimal_project_event();
        event.attributes = json!({"latency_ms": 12.5});
        assert!(recompute_datum_event_digest(&event).is_err());
    }

    // ---------------------------------------------- F1: desired_state NOW
    // PERMITTED AS DESCRIPTIVE DATA

    /// Replaces the old (now-rejected) policy test that banned a
    /// `desired_state` key. `DatumEvent` may DESCRIBE desired state; it
    /// must never BE or MUTATE it -- see the module doc comment's
    /// authority-posture section for how that boundary is actually
    /// enforced (architecturally, not lexically).
    #[test]
    fn descriptive_desired_state_key_in_attributes_is_now_permitted() {
        let mut event = minimal_project_event();
        event.category = DatumEventCategoryV1::Health;
        event.kind = "desired_state_mismatch".to_string();
        event.attributes = json!({
            "desired_state": {"replicas": 3},
            "observed_state": {"replicas": 2},
        });
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn nested_diagnostic_attributes_object_is_valid() {
        let mut event = minimal_project_event();
        event.attributes = json!({
            "context": {
                "node": {"id": "fog-01", "zone": "lab-a"},
                "counters": [1, 2, 3],
            }
        });
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    // ---------------------------------------------------- F6: ATTRIBUTES
    // SIZE BOUND

    #[test]
    fn small_attributes_are_valid() {
        let mut event = minimal_project_event();
        event.attributes = json!({"note": "short diagnostic context"});
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn attributes_at_exactly_the_size_limit_are_valid() {
        let overhead = serde_json::to_vec(&json!({"p": ""})).unwrap().len();
        let pad_len = MAX_DATUM_EVENT_ATTRIBUTES_BYTES - overhead;
        let mut event = minimal_project_event();
        event.attributes = json!({"p": "x".repeat(pad_len)});
        let actual_len = serde_json::to_vec(&event.attributes).unwrap().len();
        assert_eq!(actual_len, MAX_DATUM_EVENT_ATTRIBUTES_BYTES);
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn attributes_one_byte_over_the_size_limit_are_rejected() {
        let overhead = serde_json::to_vec(&json!({"p": ""})).unwrap().len();
        let pad_len = MAX_DATUM_EVENT_ATTRIBUTES_BYTES - overhead + 1;
        let mut event = minimal_project_event();
        event.attributes = json!({"p": "x".repeat(pad_len)});
        let actual_len = serde_json::to_vec(&event.attributes).unwrap().len();
        assert_eq!(actual_len, MAX_DATUM_EVENT_ATTRIBUTES_BYTES + 1);
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "attributes_too_large"),
            "{findings:?}"
        );
    }

    #[test]
    fn attributes_well_above_the_size_limit_are_rejected() {
        let mut event = minimal_project_event();
        event.attributes = json!({"padding": "x".repeat(64 * 1024)});
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "attributes_too_large"),
            "{findings:?}"
        );
    }

    // ------------------------------------------------------ REQUIREMENT 12
    #[test]
    fn two_independent_occurrences_of_the_same_condition_are_both_valid_with_distinct_ids() {
        // Phase 122 deliberately does NOT implement condition-level dedup
        // (that is Phase 124's job). Two events describing the exact same
        // semantic condition (same kind/subject/severity/source), differing
        // only in event_id and occurred_at_utc, must both be independently
        // valid -- nothing in this module collapses, rejects, or links them.
        let mut first = minimal_project_event();
        first.event_id = "datumevent:aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".to_string();
        first.occurred_at_utc = "2026-09-29T00:00:00Z".to_string();

        let mut second = minimal_project_event();
        second.event_id = "datumevent:bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".to_string();
        second.occurred_at_utc = "2026-09-29T00:05:00Z".to_string();

        assert_ne!(first.event_id, second.event_id);
        assert!(first.validate().is_empty(), "{:?}", first.validate());
        assert!(second.validate().is_empty(), "{:?}", second.validate());
        assert_ne!(
            recompute_datum_event_digest(&first).unwrap(),
            recompute_datum_event_digest(&second).unwrap(),
            "two independent occurrences must not collapse to the same digest either"
        );
    }

    // ------------------------------------------------------ REQUIREMENT 13
    // and AUTHORITY-BOUNDARY REGRESSION
    #[test]
    fn no_alert_lifecycle_field_exists_on_the_contract() {
        // Static contract assertion: the exact set of top-level keys a
        // minimal event serializes to must match this explicit whitelist.
        // Adding any field -- lifecycle-related or not -- forces a
        // conscious update to this test, which is exactly the point: it
        // is the mechanism that keeps `status`/`acknowledged_by`/
        // `acknowledged_at`/`resolved_at`/`first_seen`/`last_seen`/
        // `occurrence_count` (Phase 124's vocabulary, explicitly forbidden
        // here per Phase 122's architectural refinement) from silently
        // reappearing on this envelope.
        let event = minimal_project_event();
        let value = serde_json::to_value(&event).unwrap();
        let keys: std::collections::BTreeSet<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        let allowed: std::collections::BTreeSet<&str> = [
            "schema",
            "event_id",
            "project_id",
            "category",
            "kind",
            "severity",
            "source",
            "subject",
            "occurred_at_utc",
            "evidence_refs",
            "attributes",
            // application_id, application/confidence/model/correlation
            // fields are all `skip_serializing_if = "Option::is_none"` and
            // therefore absent from this minimal fixture's serialized form.
        ]
        .into_iter()
        .collect();
        assert_eq!(keys, allowed, "unexpected top-level field set: {keys:?}");

        let forbidden_alert_lifecycle_fields = [
            "status",
            "open",
            "acknowledged",
            "acknowledged_by",
            "acknowledged_at",
            "resolved",
            "resolved_at",
            "resolution",
            "suppressed",
            "first_seen",
            "last_seen",
            "occurrence_count",
        ];
        for field in forbidden_alert_lifecycle_fields {
            assert!(
                !keys.contains(field),
                "DatumEventV1 must never carry alert-lifecycle field '{field}' -- that belongs to Phase 124's DatumAlert"
            );
        }
    }

    #[test]
    fn alert_lifecycle_fields_are_rejected_as_unknown_by_deserialization() {
        // Reinforces the static-shape assertion above with an actual
        // deserialization-time rejection, proving `deny_unknown_fields`
        // structurally blocks any of these from being smuggled in even if
        // a future caller tried.
        let event = minimal_project_event();
        for field in [
            "status",
            "acknowledged_by",
            "resolved_at",
            "occurrence_count",
        ] {
            let mut value = serde_json::to_value(&event).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .insert(field.to_string(), json!("x"));
            let result: Result<DatumEventV1, _> = serde_json::from_value(value);
            assert!(result.is_err(), "field '{field}' must be rejected");
        }
    }

    #[test]
    fn contract_exposes_no_mutation_or_authorization_method() {
        // Static contract assertion (compile-time, not runtime): this
        // module defines exactly the value type, its validator, and its
        // digest function -- no `authorize_*`, `accept_*`, `apply_*`,
        // `mutate_*`, or `reconcile_*` function exists anywhere in this
        // file. This test exists to be grepped by a future reviewer as a
        // documented, intentional checkpoint rather than to execute any
        // runtime logic; the real enforcement is architectural (nothing in
        // this module holds a `&mut` reference to any other subsystem's
        // state, imports no other subsystem's mutation API, and every
        // function here is either a pure validator or a pure serializer).
        let event = minimal_project_event();
        // The only operations available on a constructed value:
        let _: Vec<CanonicalValidationFinding> = event.validate();
        let _: Result<String, String> = recompute_datum_event_digest(&event);
        let _: Result<String, _> = serde_json::to_string(&event);
    }

    // -------------------------------------------- F4: SOURCE EXTENSIBILITY

    #[test]
    fn all_known_sources_serialize_to_the_original_lowercase_tokens() {
        let expectations = [
            (DatumEventSourceV1::DMonitor, "dmonitor"),
            (DatumEventSourceV1::SentinelTelemetry, "sentinel_telemetry"),
            (DatumEventSourceV1::ActiveMonitor, "active_monitor"),
            (DatumEventSourceV1::DDeploy, "ddeploy"),
            (DatumEventSourceV1::Reconciliation, "reconciliation"),
            (DatumEventSourceV1::DForward, "dforward"),
            (DatumEventSourceV1::DIoT, "diot"),
            (DatumEventSourceV1::DCode, "dcode"),
            (DatumEventSourceV1::DEngine, "dengine"),
            (DatumEventSourceV1::Runtime, "runtime"),
            (DatumEventSourceV1::ServiceArtifact, "service_artifact"),
            (DatumEventSourceV1::RuleEngine, "rule_engine"),
            (DatumEventSourceV1::Operator, "operator"),
            (DatumEventSourceV1::Gnn, "gnn"),
            (DatumEventSourceV1::Llm, "llm"),
        ];
        for (source, token) in expectations {
            let value = serde_json::to_value(&source).unwrap();
            assert_eq!(
                value,
                json!(token),
                "known-source wire format must be unchanged"
            );
            let restored: DatumEventSourceV1 = serde_json::from_value(json!(token)).unwrap();
            assert_eq!(restored, source);
        }
    }

    #[test]
    fn unknown_source_round_trips_and_preserves_identity() {
        let value = json!("future_subsystem");
        let source: DatumEventSourceV1 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            source,
            DatumEventSourceV1::Other("future_subsystem".to_string())
        );
        assert_eq!(serde_json::to_value(&source).unwrap(), value);

        let mut event = minimal_project_event();
        event.source = source;
        assert!(event.validate().is_empty(), "{:?}", event.validate());
        let json_text = serde_json::to_string(&event).unwrap();
        assert!(json_text.contains("\"future_subsystem\""));
        let restored: DatumEventV1 = serde_json::from_str(&json_text).unwrap();
        assert_eq!(event, restored);
    }

    #[test]
    fn source_other_with_invalid_charset_is_rejected() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Other("Future-Subsystem".to_string());
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "source_other_invalid_format"),
            "{findings:?}"
        );
    }

    #[test]
    fn source_other_colliding_with_known_token_is_rejected() {
        let mut event = minimal_project_event();
        event.source = DatumEventSourceV1::Other("dmonitor".to_string());
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "source_other_collides_with_known_value"),
            "{findings:?}"
        );
    }

    // -------------------------------------------- F5: CANONICAL TIMESTAMP

    #[test]
    fn canonical_utc_timestamp_is_accepted() {
        let event = minimal_project_event(); // "2026-09-29T00:00:00Z"
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn non_utc_offset_equivalent_instant_is_rejected() {
        let mut event = minimal_project_event();
        // Same instant as "2026-09-29T00:00:00Z" shifted three hours
        // earlier in wall-clock terms with a matching negative offset.
        event.occurred_at_utc = "2026-09-28T21:00:00-03:00".to_string();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "timestamp_not_canonical"),
            "{findings:?}"
        );
    }

    #[test]
    fn non_canonical_fractional_seconds_representation_is_rejected() {
        let mut event = minimal_project_event();
        // 500_000_000 ns == 500ms exactly; the canonical AutoSi tier for
        // that value is milliseconds (".500Z"), not nanoseconds
        // (".500000000Z") -- verified empirically, see
        // `canonicalize_rfc3339_utc`'s doc comment.
        event.occurred_at_utc = "2026-09-29T00:00:00.500000000Z".to_string();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "timestamp_not_canonical"),
            "{findings:?}"
        );
    }

    #[test]
    fn canonical_millisecond_fractional_seconds_representation_is_accepted() {
        let mut event = minimal_project_event();
        event.occurred_at_utc = "2026-09-29T00:00:00.500Z".to_string();
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn unparseable_timestamp_is_rejected() {
        let mut event = minimal_project_event();
        event.occurred_at_utc = "not-a-timestamp".to_string();
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "invalid_timestamp"),
            "{findings:?}"
        );
    }

    // ---------------------------------------------------- MISC / TABLE-DRIVEN

    #[test]
    fn all_severities_and_all_categories_validate_and_round_trip() {
        let severities = [
            DatumEventSeverityV1::Info,
            DatumEventSeverityV1::Warning,
            DatumEventSeverityV1::High,
            DatumEventSeverityV1::Critical,
        ];
        let categories = [
            DatumEventCategoryV1::Observation,
            DatumEventCategoryV1::Runtime,
            DatumEventCategoryV1::ControlPlane,
            DatumEventCategoryV1::Health,
            DatumEventCategoryV1::Analysis,
            DatumEventCategoryV1::Operator,
            DatumEventCategoryV1::Model,
        ];
        for &severity in &severities {
            for &category in &categories {
                let mut event = minimal_project_event();
                event.severity = severity;
                event.category = category;
                assert!(
                    event.validate().is_empty(),
                    "{:?}/{:?}: {:?}",
                    severity,
                    category,
                    event.validate()
                );
                let json_text = serde_json::to_string(&event).unwrap();
                let restored: DatumEventV1 = serde_json::from_str(&json_text).unwrap();
                assert_eq!(event, restored);
            }
        }
    }

    // ------------------------------------------- REQUIREMENT 8: CATEGORY/KIND

    /// Proves category/kind independence is a DELIBERATE, tested
    /// invariant, not an unexamined gap: a `Model`-category event with a
    /// `ControlPlane`-shaped `kind` string currently validates cleanly,
    /// and this module intentionally never cross-checks the two.
    #[test]
    fn category_and_kind_are_validated_independently() {
        let mut event = minimal_project_event();
        event.category = DatumEventCategoryV1::Model;
        event.kind = "proposal_accepted".to_string();
        assert!(event.validate().is_empty(), "{:?}", event.validate());
    }

    #[test]
    fn application_id_inconsistency_with_subject_is_rejected() {
        let mut event = minimal_project_event();
        event.application_id = Some("app:other".to_string());
        event.subject = DatumEventSubjectV1::Application {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
        };
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "subject_application_id_inconsistent"),
            "{findings:?}"
        );
    }

    #[test]
    fn project_id_inconsistency_with_subject_is_rejected() {
        let mut event = minimal_project_event();
        event.project_id = Some("proj:other".to_string());
        event.subject = DatumEventSubjectV1::Project {
            project_id: "proj:lab".to_string(),
        };
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "subject_project_id_inconsistent"),
            "{findings:?}"
        );
    }

    #[test]
    fn other_subject_and_other_evidence_ref_require_non_empty_labels() {
        let mut event = minimal_project_event();
        event.project_id = None;
        event.subject = DatumEventSubjectV1::Other {
            project_id: None,
            entity_kind: String::new(),
            entity_id: "x".to_string(),
        };
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "missing_field"),
            "{findings:?}"
        );

        let mut event = minimal_project_event();
        event.evidence_refs = vec![DatumEventEvidenceRefV1::Other {
            evidence_kind: "future_source".to_string(),
            reference_id: String::new(),
        }];
        let findings = event.validate();
        assert!(
            findings.iter().any(|f| f.code == "missing_field"),
            "{findings:?}"
        );
    }

    // -------------------------------------------------- F9: OTHER SUBJECT

    #[test]
    fn other_subject_empty_but_present_project_id_is_rejected_directly() {
        let mut event = minimal_project_event();
        event.project_id = None;
        event.subject = DatumEventSubjectV1::Other {
            project_id: Some(String::new()),
            entity_kind: "future_entity".to_string(),
            entity_id: "x".to_string(),
        };
        let findings = event.validate();
        assert!(
            findings
                .iter()
                .any(|f| f.code == "missing_field" && f.message.contains("subject.project_id")),
            "expected a direct subject.project_id missing_field finding, got: {findings:?}"
        );
    }
}
