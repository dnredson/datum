//! DMonitor observation ingestion + durable persistence (Phase 114B1,
//! ADR-0024). Server-owned storage of ACCEPTED canonical
//! `datum.dmonitor-observation/1` evidence.
//!
//! This module makes DServer able to answer "what canonical DMonitor
//! evidence was accepted, for which project/stream/sequence, and when did
//! the SERVER receive it" -- nothing more. It deliberately does NOT compute
//! drift, convergence, placement safety, or `any_element` selection (those
//! are 114C/114D). It does not correlate against DDeploy/DNode/DCall
//! authority either: a subject's optional `dmap_ref`/`dforward_ref` is
//! validated STRUCTURALLY only (by `CanonicalDMonitorObservationV1::validate`,
//! already run before this module ever sees the observation), never checked
//! against the currently active authority. That "does this still match
//! current authority" question belongs to 114C.
//!
//! # Authority boundary
//!
//! The canonical observation itself remains node-authored end to end: this
//! module never mutates it. It is wrapped in a SERVER-owned
//! [`DMonitorObservationRecordV1`] that adds exactly the three facts only
//! the server can honestly assert: which project scope accepted it, what its
//! recomputed digest is, and when the server received it
//! (`received_at_utc`). A caller can never supply or override
//! `received_at_utc` -- every production call path in `api::dmonitor`
//! generates it fresh from `Utc::now()` and passes it in; nothing here reads
//! caller input for that field.
//!
//! # Stream identity, sequence, and idempotency
//!
//! A DMonitor observation STREAM is identified by
//! `(server_project_scope, node_id, monitor_instance_id)` -- see
//! [`stream_key`]. `monitor_instance_id` identifies one monotonic SEQUENCE
//! EPOCH (Phase 114B1.1 closure, section 1): sequence numbers are only ever
//! compared WITHIN one exact stream, and a monitor process that restarts
//! while PRESERVING its own `monitor_instance_id` must also preserve/
//! continue its own sequence counter -- resetting to a lower sequence under
//! the SAME `monitor_instance_id` is correctly rejected as stale/
//! out-of-order (see `sequence_reset_under_the_same_monitor_instance_id_is_stale_not_a_new_epoch`),
//! never treated as a fresh epoch. Only adopting a genuinely NEW
//! `monitor_instance_id` starts a brand new, independent stream, even on the
//! same node. Multiple simultaneous monitor instances for the same
//! conceptual subject are tolerated as independent streams -- this module
//! never elects one "authoritative" merely because it was received later
//! (no leader election, no lease, no takeover semantics exist here or
//! anywhere in Phase 114B1).
//!
//! Within one stream, [`ingest_observation`] enforces exactly:
//!
//! - first sequence for an empty stream: accepted, becomes the head.
//! - sequence greater than the current head: accepted, becomes the new head.
//! - sequence equal to an already-stored sequence with an IDENTICAL
//!   recomputed digest: idempotent replay (the ORIGINAL `received_at_utc` is
//!   preserved, the stream is not advanced, no second semantic observation
//!   is created).
//! - sequence equal to an already-stored sequence with a DIFFERENT digest:
//!   rejected as a sequence conflict.
//! - sequence lower than the current head: rejected as stale/out-of-order,
//!   even if that exact sequence was never seen before (a gap is not an
//!   invitation to fill it retroactively).
//!
//! `observation_id` carries its own, separate identity-collision rule,
//! checked BEFORE the sequence rule above and taking precedence over it
//! (Phase 114B1.1 closure, section 6, deliberate and tested policy): the
//! same `observation_id` seen again with the same digest is a safe replay
//! REGARDLESS of whether the stream head has since advanced past it --
//! retries of an old request therefore have deterministic, idempotent
//! behavior rather than being rejected as stale merely because newer
//! observations arrived in between (see
//! `historical_exact_replay_after_head_advanced_is_idempotent_not_stale`).
//! An observation_id/digest pair that has never been seen before still goes
//! through the ordinary sequence rule, so a genuinely unseen LOW sequence is
//! correctly stale. With a DIFFERENT digest under the same `observation_id`,
//! it is rejected as an identity collision, regardless of what sequence/
//! stream it would otherwise resolve to.
//!
//! `observation_id` identity is effectively GLOBAL across every server
//! project scope, not namespaced per project (Phase 114B1.1 closure, section
//! 4): the idempotent-replay check additionally requires the FULL stream
//! identity (`stream_key`, which already encodes
//! `server_project_scope`/`node_id`/`monitor_instance_id`) and `sequence` to
//! match, not merely the digest -- this matters specifically because
//! `server_project_scope` is a server-assigned value that is NOT part of the
//! canonical digest when `observation.project_id` is `None` (section 5), so
//! digest equality alone would not prove the two requests were even for the
//! same project. Replaying the identical `observation_id`+digest under a
//! DIFFERENT project scope therefore never returns another project's
//! persisted record: the stream-key mismatch routes it to
//! `ObservationIdConflict` instead (see
//! `same_observation_id_and_digest_under_a_different_project_scope_is_rejected_not_leaked`).
//!
//! # Subject indexing
//!
//! [`subject_key`] derives a deterministic internal key from the OBSERVED
//! TYPED SUBJECT's identity fields only -- never from `facts`, a hostname, a
//! container/process name, or an image. `subject_index` records, per
//! subject key, the LATEST accepted sequence for that subject WITHIN EACH
//! STREAM independently (`BTreeMap<subject_key, BTreeMap<stream_key,
//! sequence>>`) -- NOT merely "this stream has seen this subject at some
//! point." This distinction is load-bearing (Phase 114B1.1 closure, section
//! 7): a single stream can legitimately interleave several subjects across
//! its sequence (e.g. seq 1 -> subject A, seq 2 -> subject B, seq 3 ->
//! subject A again), so the stream's global head is NOT necessarily the
//! latest observation for any one subject, and a caller resolving "subject
//! A's latest state in this stream" must get seq 3's record, never seq 2's
//! (see `subject_index_tracks_the_latest_observation_per_subject_per_stream_not_the_stream_head`).
//!
//! `DMonitorSubjectV1::Unbound` observations are DELIBERATELY EXCLUDED from
//! `subject_index` entirely (Phase 114B1.1 closure, section 8): an `Unbound`
//! observation exists specifically because the collector could not
//! establish canonical correlation, so indexing it by `node_id` alone would
//! falsely imply that every unrelated `Unbound` observation on the same node
//! is "the same subject" merely because they share a host. `Unbound`
//! observations remain fully accessible via [`DMonitorStateV1::by_observation_id`]
//! and [`DMonitorStateV1::stream_head`]/[`DMonitorStateV1::stream_heads_for_project`]
//! -- only the subject-correlation index omits them.
//!
//! # Persistence atomicity
//!
//! This module holds NO globally-shared mutable state: every production
//! call path (`api::dmonitor`'s handlers) calls [`load_state`] fresh at the
//! start of a request, mutates only that local, request-scoped copy via
//! [`ingest_observation`], and calls [`persist_state`] before returning
//! success. [`persist_state`] delegates to
//! `storage::control_state::persist_domain_json`, which writes both the
//! `control_state_current` upsert and the `control_state_history` append
//! inside ONE SQLite transaction -- so a persistence failure (e.g. a
//! filesystem error) leaves the on-disk state completely unchanged, and the
//! failed request's locally-mutated `state` value is simply dropped when the
//! handler returns an error, never observed by any other request. There is
//! therefore no scenario where "in-memory state advanced but persistence
//! failed" can cause a later request to see the failed observation as
//! already accepted/idempotent -- the next request reloads the SAME
//! pre-failure state from disk (see
//! `failed_persist_leaves_no_durable_mutation`). This module does not add a
//! transactional abstraction of its own; it relies entirely on this
//! existing per-request-reload architecture, which every other DServer
//! control-state domain (`ddeploy`, `diot_runtime_binding`, `dcall_grant`,
//! ...) already shares.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::canonical_dmonitor::{
    recompute_dmonitor_observation_digest, CanonicalDMonitorObservationV1, DMonitorSubjectV1,
};
use crate::storage::control_state::{load_domain_json, persist_domain_json, DOMAIN_DMONITOR_STATE};

pub const DMONITOR_STATE_VERSION: &str = "dmonitor_state_v1";

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

fn length_prefixed_component(value: &str) -> String {
    format!("{}:{}", value.len(), value)
}

/// `(server_project_scope, node_id, monitor_instance_id)`. Length-prefixed
/// joining (mirrors `diot_runtime_binding::binding_key`) so no combination of
/// component values can collide with a different triple.
pub fn stream_key(project_scope: &str, node_id: &str, monitor_instance_id: &str) -> String {
    format!(
        "{}{}{}",
        length_prefixed_component(project_scope),
        length_prefixed_component(node_id),
        length_prefixed_component(monitor_instance_id)
    )
}

/// Deterministic internal key derived ONLY from typed subject identity
/// fields -- never `facts`, never a name/hostname/port/image (section 9).
/// `Unbound` keys on `node_id` alone: it is honestly non-correlating, never
/// upgraded by inspecting the payload.
pub fn subject_key(subject: &DMonitorSubjectV1) -> String {
    let parts: Vec<String> = match subject {
        DMonitorSubjectV1::DNode { node_id } => {
            vec!["dnode".to_string(), node_id.clone()]
        }
        DMonitorSubjectV1::RuntimeProcess { node_id, runtime_ref } => vec![
            "runtime_process".to_string(),
            node_id.clone(),
            runtime_ref.clone(),
        ],
        DMonitorSubjectV1::DServRealization {
            project_id,
            application_id,
            service_id,
            node_id,
            ..
        } => vec![
            "dserv_realization".to_string(),
            project_id.clone(),
            application_id.clone(),
            service_id.clone(),
            node_id.clone(),
        ],
        DMonitorSubjectV1::DCodeExecution {
            project_id,
            application_id,
            service_id,
            node_id,
            ..
        } => vec![
            "dcode_execution".to_string(),
            project_id.clone(),
            application_id.clone(),
            service_id.clone(),
            node_id.clone(),
        ],
        DMonitorSubjectV1::DCallDelivery {
            project_id,
            application_id,
            call_id,
            ..
        } => vec![
            "dcall_delivery".to_string(),
            project_id.clone(),
            application_id.clone(),
            call_id.clone(),
        ],
        DMonitorSubjectV1::DIoTRealization {
            project_id,
            application_id,
            diot_placement_id,
            ..
        } => vec![
            "diot_realization".to_string(),
            project_id.clone(),
            application_id.clone(),
            diot_placement_id.clone().unwrap_or_else(|| "-".to_string()),
        ],
        DMonitorSubjectV1::NetworkReachability { node_id, target } => vec![
            "network_reachability".to_string(),
            node_id.clone(),
            target.clone(),
        ],
        DMonitorSubjectV1::Unbound { node_id, .. } => {
            vec!["unbound".to_string(), node_id.clone()]
        }
    };
    parts
        .iter()
        .map(|part| length_prefixed_component(part))
        .collect::<Vec<_>>()
        .join("")
}

/// Server-owned wrapper around one accepted canonical observation. The
/// canonical observation itself is never mutated by this module -- these
/// three fields are exactly the facts only the server can honestly assert
/// (section 4).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DMonitorObservationRecordV1 {
    pub observation: CanonicalDMonitorObservationV1,
    pub observation_digest: String,
    pub server_project_scope: String,
    /// Server-generated. Never accepted from a caller (section 4's central
    /// invariant) -- every write path in this module takes this as an
    /// explicit parameter rather than reading it from the observation
    /// (which has no such field at all) or the request body.
    pub received_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DMonitorStreamV1 {
    pub project_scope: String,
    pub node_id: String,
    pub monitor_instance_id: String,
    pub head_sequence: u64,
    pub head_observation_id: String,
    #[serde(default)]
    pub observations: BTreeMap<u64, DMonitorObservationRecordV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DMonitorObservationIndexEntryV1 {
    pub stream_key: String,
    pub sequence: u64,
    pub observation_digest: String,
}

/// Retention/pruning is explicitly deferred (section 10): v1 keeps every
/// accepted observation per stream indefinitely. A future phase adding
/// retention must decide how that interacts with `observation_index` and
/// `subject_index` staying consistent -- not addressed here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DMonitorStateV1 {
    pub state_version: String,
    #[serde(default)]
    pub streams: BTreeMap<String, DMonitorStreamV1>,
    #[serde(default)]
    pub observation_index: BTreeMap<String, DMonitorObservationIndexEntryV1>,
    /// subject_key -> { stream_key -> latest accepted sequence for that
    /// subject WITHIN that stream }. Deliberately NOT subject_key -> a
    /// single stream_key, and NOT merely "this stream has seen this
    /// subject": section 8 forbids collapsing multiple simultaneous
    /// monitor-instance streams for the same subject into one
    /// "authoritative" answer, and section 7 requires distinguishing a
    /// stream's global head from a given subject's own latest sequence
    /// within that stream, since one stream can interleave several
    /// subjects across its sequence. `DMonitorSubjectV1::Unbound` is never
    /// present here (section 8) -- see the module doc.
    #[serde(default)]
    pub subject_index: BTreeMap<String, BTreeMap<String, u64>>,
}

impl DMonitorStateV1 {
    pub fn empty() -> Self {
        Self {
            state_version: DMONITOR_STATE_VERSION.to_string(),
            streams: BTreeMap::new(),
            observation_index: BTreeMap::new(),
            subject_index: BTreeMap::new(),
        }
    }

    pub fn stream_head(
        &self,
        project_scope: &str,
        node_id: &str,
        monitor_instance_id: &str,
    ) -> Option<&DMonitorObservationRecordV1> {
        let stream = self
            .streams
            .get(&stream_key(project_scope, node_id, monitor_instance_id))?;
        stream.observations.get(&stream.head_sequence)
    }

    pub fn by_observation_id(&self, observation_id: &str) -> Option<&DMonitorObservationRecordV1> {
        let entry = self.observation_index.get(observation_id)?;
        let stream = self.streams.get(&entry.stream_key)?;
        stream.observations.get(&entry.sequence)
    }

    /// Current head record of every stream scoped to `project_scope`. This
    /// is the minimal read surface section 13 requires -- NOT a general
    /// historical/analytics query API.
    pub fn stream_heads_for_project(&self, project_scope: &str) -> Vec<&DMonitorObservationRecordV1> {
        self.streams
            .values()
            .filter(|stream| stream.project_scope == project_scope)
            .filter_map(|stream| stream.observations.get(&stream.head_sequence))
            .collect()
    }

    /// The LATEST accepted record for `subject_key` in EACH stream it has
    /// been observed in, scoped to `project_scope` (Phase 114B1.1 closure,
    /// section 7) -- never the stream's global head, and never collapsed
    /// across streams into one answer (section 8's multi-instance
    /// tolerance). `Unbound` subjects never appear here by construction
    /// (they are never inserted into `subject_index`); look them up via
    /// [`Self::stream_head`]/[`Self::stream_heads_for_project`] instead.
    pub fn subject_candidates(
        &self,
        project_scope: &str,
        subject_key: &str,
    ) -> Vec<&DMonitorObservationRecordV1> {
        let Some(per_stream) = self.subject_index.get(subject_key) else {
            return Vec::new();
        };
        per_stream
            .iter()
            .filter_map(|(stream_key_value, sequence)| {
                let stream = self.streams.get(stream_key_value)?;
                if stream.project_scope != project_scope {
                    return None;
                }
                stream.observations.get(sequence)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    Accepted,
    IdempotentReplay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IngestResult {
    pub outcome: IngestOutcome,
    pub record: DMonitorObservationRecordV1,
}

#[derive(Debug)]
pub enum IngestDMonitorObservationError {
    Invalid(Vec<CanonicalValidationFinding>),
    ProjectScopeMismatch { expected: String, actual: String },
    /// `existing_stream_key`/`attempted_stream_key` are included explicitly
    /// (Phase 114B1.1 closure, section 4) so this error is diagnosable even
    /// when the conflict is a CROSS-PROJECT observation_id reuse -- the two
    /// stream keys differing only in their `server_project_scope` component
    /// is exactly what that case looks like, and the message should say so
    /// rather than leaving a caller to guess.
    ObservationIdConflict {
        observation_id: String,
        existing_stream_key: String,
        existing_sequence: u64,
        attempted_stream_key: String,
    },
    SequenceConflict { stream_key: String, sequence: u64 },
    SequenceStale { stream_key: String, sequence: u64, current_head: u64 },
}

/// Ingests one canonical DMonitor observation into `state`, scoped to
/// `project_scope` (the caller's authoritative, request-derived project
/// identity -- NOT necessarily `observation.project_id`, see section 5).
/// `received_at_utc` MUST be server-generated by the caller (this function
/// never calls `Utc::now()` itself, keeping it a pure, deterministically
/// testable function exactly like the freshness evaluator).
pub fn ingest_observation(
    state: &mut DMonitorStateV1,
    project_scope: &str,
    observation: CanonicalDMonitorObservationV1,
    received_at_utc: &str,
) -> Result<IngestResult, IngestDMonitorObservationError> {
    let findings = observation.validate();
    if !findings.is_empty() {
        return Err(IngestDMonitorObservationError::Invalid(findings));
    }

    if let Some(declared) = &observation.project_id {
        if declared != project_scope {
            return Err(IngestDMonitorObservationError::ProjectScopeMismatch {
                expected: project_scope.to_string(),
                actual: declared.clone(),
            });
        }
    }

    let digest = recompute_dmonitor_observation_digest(&observation).map_err(|error| {
        IngestDMonitorObservationError::Invalid(vec![finding(
            "digest_unrecomputable",
            error,
        )])
    })?;

    let stream_key_value = stream_key(project_scope, &observation.node_id, &observation.monitor_instance_id);
    let sequence = observation.sequence;

    // Observation-ID identity rule, checked BEFORE the sequence rule
    // (section 7): a repeated observation_id is either a safe replay
    // (identical digest) or an identity collision (different digest),
    // regardless of what the sequence rule below would otherwise decide.
    if let Some(existing_index) = state.observation_index.get(&observation.observation_id) {
        if existing_index.observation_digest == digest
            && existing_index.stream_key == stream_key_value
            && existing_index.sequence == sequence
        {
            let stream = state
                .streams
                .get(&stream_key_value)
                .expect("observation_index consistency: indexed stream must exist");
            let record = stream
                .observations
                .get(&sequence)
                .expect("observation_index consistency: indexed sequence must exist")
                .clone();
            return Ok(IngestResult {
                outcome: IngestOutcome::IdempotentReplay,
                record,
            });
        }
        return Err(IngestDMonitorObservationError::ObservationIdConflict {
            observation_id: observation.observation_id.clone(),
            existing_stream_key: existing_index.stream_key.clone(),
            existing_sequence: existing_index.sequence,
            attempted_stream_key: stream_key_value,
        });
    }

    let stream_is_new = !state.streams.contains_key(&stream_key_value);
    let stream = state.streams.entry(stream_key_value.clone()).or_insert_with(|| DMonitorStreamV1 {
        project_scope: project_scope.to_string(),
        node_id: observation.node_id.clone(),
        monitor_instance_id: observation.monitor_instance_id.clone(),
        head_sequence: 0,
        head_observation_id: String::new(),
        observations: BTreeMap::new(),
    });

    if let Some(existing_record) = stream.observations.get(&sequence) {
        if existing_record.observation_digest == digest {
            let record = existing_record.clone();
            return Ok(IngestResult {
                outcome: IngestOutcome::IdempotentReplay,
                record,
            });
        }
        return Err(IngestDMonitorObservationError::SequenceConflict {
            stream_key: stream_key_value,
            sequence,
        });
    }

    if !stream_is_new && sequence < stream.head_sequence {
        return Err(IngestDMonitorObservationError::SequenceStale {
            stream_key: stream_key_value,
            sequence,
            current_head: stream.head_sequence,
        });
    }
    // sequence == head_sequence with a different digest was already handled
    // above via `stream.observations.get(&sequence)` (the head's own record
    // lives at that key), so reaching here with `sequence == head_sequence`
    // is unreachable; this covers `sequence > head_sequence` (including the
    // stream's very first observation, where `head_sequence == 0`).

    let record = DMonitorObservationRecordV1 {
        observation: observation.clone(),
        observation_digest: digest.clone(),
        server_project_scope: project_scope.to_string(),
        received_at_utc: received_at_utc.to_string(),
    };
    stream.observations.insert(sequence, record.clone());
    stream.head_sequence = sequence;
    stream.head_observation_id = observation.observation_id.clone();

    state.observation_index.insert(
        observation.observation_id.clone(),
        DMonitorObservationIndexEntryV1 {
            stream_key: stream_key_value.clone(),
            sequence,
            observation_digest: digest,
        },
    );

    // Section 8: `Unbound` observations are never indexed by subject --
    // indexing them by `node_id` alone would falsely correlate unrelated
    // unbound observations on the same node. Every other subject variant
    // records its LATEST sequence within THIS stream (section 7): since
    // ingestion only ever reaches this point with a `sequence` that is
    // strictly greater than every sequence previously accepted for this
    // stream (the sequence rule above rejects everything else), a plain
    // overwrite is always correct -- there is no need to compare against a
    // prior value.
    if !matches!(observation.subject, DMonitorSubjectV1::Unbound { .. }) {
        let subject_key_value = subject_key(&observation.subject);
        state
            .subject_index
            .entry(subject_key_value)
            .or_default()
            .insert(stream_key_value, sequence);
    }

    Ok(IngestResult {
        outcome: IngestOutcome::Accepted,
        record,
    })
}

/// Fail-closed relational + tamper integrity check, invoked from
/// [`load_state`]. Mirrors `diot_runtime_binding`'s reload-validation
/// precedent: every stored digest is RECOMPUTED and compared, never merely
/// trusted; every index is checked for dangling/inconsistent references.
pub fn validate_persisted_dmonitor_state_integrity(state: &DMonitorStateV1) -> Result<(), String> {
    if state.state_version != DMONITOR_STATE_VERSION {
        return Err(format!(
            "unsupported dmonitor state_version '{}', expected '{DMONITOR_STATE_VERSION}'",
            state.state_version
        ));
    }

    for (key, stream) in &state.streams {
        let expected_key = stream_key(&stream.project_scope, &stream.node_id, &stream.monitor_instance_id);
        if *key != expected_key {
            return Err(format!(
                "dmonitor stream stored under key '{key}' does not match stream_key(...) = '{expected_key}'"
            ));
        }

        if stream.observations.is_empty() {
            if stream.head_sequence != 0 || !stream.head_observation_id.is_empty() {
                return Err(format!(
                    "dmonitor stream '{key}' has no observations but a non-zero/non-empty head"
                ));
            }
        } else {
            if stream.head_sequence == 0 {
                return Err(format!(
                    "dmonitor stream '{key}' has observations but head_sequence is 0"
                ));
            }
            let head_record = stream.observations.get(&stream.head_sequence).ok_or_else(|| {
                format!(
                    "dmonitor stream '{key}' head_sequence {} has no matching observation",
                    stream.head_sequence
                )
            })?;
            if head_record.observation.observation_id != stream.head_observation_id {
                return Err(format!(
                    "dmonitor stream '{key}' head_observation_id '{}' does not match the observation stored at head_sequence {} (observation_id '{}')",
                    stream.head_observation_id, stream.head_sequence, head_record.observation.observation_id
                ));
            }
            // Section 10: the head must be EXACTLY the maximum accepted
            // sequence, not merely an existing one -- `{1,2,3}` with
            // `head=2` is corrupt even though sequence 2 does exist.
            let max_sequence = *stream
                .observations
                .keys()
                .max()
                .expect("non-empty observations map has a maximum key");
            if stream.head_sequence != max_sequence {
                return Err(format!(
                    "dmonitor stream '{key}' head_sequence {} is not the maximum accepted sequence {max_sequence}",
                    stream.head_sequence
                ));
            }
        }

        for (sequence, record) in &stream.observations {
            if *sequence == 0 {
                return Err(format!("dmonitor stream '{key}' contains a zero sequence entry"));
            }
            if record.observation.sequence != *sequence {
                return Err(format!(
                    "dmonitor stream '{key}' observation stored under sequence {sequence} has mismatched observation.sequence {}",
                    record.observation.sequence
                ));
            }
            if record.observation.node_id != stream.node_id {
                return Err(format!(
                    "dmonitor stream '{key}' sequence {sequence} observation.node_id '{}' does not match stream node_id '{}'",
                    record.observation.node_id, stream.node_id
                ));
            }
            if record.observation.monitor_instance_id != stream.monitor_instance_id {
                return Err(format!(
                    "dmonitor stream '{key}' sequence {sequence} observation.monitor_instance_id '{}' does not match stream monitor_instance_id '{}'",
                    record.observation.monitor_instance_id, stream.monitor_instance_id
                ));
            }
            if record.server_project_scope != stream.project_scope {
                return Err(format!(
                    "dmonitor stream '{key}' sequence {sequence} server_project_scope '{}' does not match stream project_scope '{}'",
                    record.server_project_scope, stream.project_scope
                ));
            }
            if let Some(declared) = &record.observation.project_id {
                if declared != &stream.project_scope {
                    return Err(format!(
                        "dmonitor stream '{key}' sequence {sequence} observation.project_id '{declared}' does not match server project scope '{}'",
                        stream.project_scope
                    ));
                }
            }
            let observation_findings = record.observation.validate();
            if !observation_findings.is_empty() {
                return Err(format!(
                    "dmonitor stream '{key}' sequence {sequence} fails canonical validation: {observation_findings:?}"
                ));
            }
            let expected_digest = recompute_dmonitor_observation_digest(&record.observation).map_err(|error| {
                format!("dmonitor stream '{key}' sequence {sequence} digest could not be recomputed: {error}")
            })?;
            if expected_digest != record.observation_digest {
                return Err(format!(
                    "dmonitor stream '{key}' sequence {sequence} persisted observation_digest '{}' does not match recomputed '{expected_digest}' -- persisted state is corrupt or was tampered with",
                    record.observation_digest
                ));
            }
            if chrono::DateTime::parse_from_rfc3339(&record.received_at_utc).is_err() {
                return Err(format!(
                    "dmonitor stream '{key}' sequence {sequence} has an invalid received_at_utc: '{}'",
                    record.received_at_utc
                ));
            }
        }
    }

    for (observation_id, entry) in &state.observation_index {
        let stream = state.streams.get(&entry.stream_key).ok_or_else(|| {
            format!("dmonitor observation_index entry '{observation_id}' points to missing stream '{}'", entry.stream_key)
        })?;
        let record = stream.observations.get(&entry.sequence).ok_or_else(|| {
            format!(
                "dmonitor observation_index entry '{observation_id}' points to missing sequence {} in stream '{}'",
                entry.sequence, entry.stream_key
            )
        })?;
        if record.observation.observation_id != *observation_id {
            return Err(format!(
                "dmonitor observation_index entry '{observation_id}' does not match stored observation_id '{}'",
                record.observation.observation_id
            ));
        }
        if record.observation_digest != entry.observation_digest {
            return Err(format!(
                "dmonitor observation_index entry '{observation_id}' digest does not match the stored record's digest"
            ));
        }
    }

    for (key, stream) in &state.streams {
        for record in stream.observations.values() {
            if !state.observation_index.contains_key(&record.observation.observation_id) {
                return Err(format!(
                    "dmonitor stream '{key}' observation_id '{}' is not present in observation_index",
                    record.observation.observation_id
                ));
            }
        }
    }

    // Section 9 (bidirectional index integrity): rather than hand-rolling
    // every possible way `observation_index`/`subject_index` could
    // disagree with the streams they were derived from (a stale sequence
    // pointer, a wrongly-typed subject, an `Unbound` subject indexed
    // contrary to policy, a wrong project/stream/digest, a missing entry
    // for a real observation, ...), RECOMPUTE both indexes from `streams`
    // from scratch using the exact same rule `ingest_observation` applies,
    // and require BYTE-FOR-BYTE equality with what was persisted. Indexes
    // are exact DERIVED structures, not merely "non-dangling" -- any
    // divergence, in either direction, is corruption.
    let (expected_observation_index, expected_subject_index) = recompute_indexes_from_streams(&state.streams);
    if expected_observation_index != state.observation_index {
        return Err(
            "dmonitor observation_index does not match the index recomputed from streams -- persisted state is corrupt or was tampered with".to_string(),
        );
    }
    if expected_subject_index != state.subject_index {
        return Err(
            "dmonitor subject_index does not match the index recomputed from streams -- persisted state is corrupt or was tampered with".to_string(),
        );
    }

    Ok(())
}

/// Recomputes `observation_index`/`subject_index` from `streams` alone,
/// applying the EXACT same rules [`ingest_observation`] does (latest
/// sequence per subject per stream; `Unbound` never indexed by subject).
/// Used both by [`validate_persisted_dmonitor_state_integrity`] (as an
/// exact-equality oracle) and available for a future repair/migration tool,
/// though none exists yet.
fn recompute_indexes_from_streams(
    streams: &BTreeMap<String, DMonitorStreamV1>,
) -> (
    BTreeMap<String, DMonitorObservationIndexEntryV1>,
    BTreeMap<String, BTreeMap<String, u64>>,
) {
    let mut observation_index = BTreeMap::new();
    let mut subject_index: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for (stream_key_value, stream) in streams {
        for (sequence, record) in &stream.observations {
            observation_index.insert(
                record.observation.observation_id.clone(),
                DMonitorObservationIndexEntryV1 {
                    stream_key: stream_key_value.clone(),
                    sequence: *sequence,
                    observation_digest: record.observation_digest.clone(),
                },
            );
            if !matches!(record.observation.subject, DMonitorSubjectV1::Unbound { .. }) {
                let subject_key_value = subject_key(&record.observation.subject);
                subject_index
                    .entry(subject_key_value)
                    .or_default()
                    .insert(stream_key_value.clone(), *sequence);
            }
        }
    }
    (observation_index, subject_index)
}

pub fn load_state(db_path: &Path) -> Result<DMonitorStateV1, String> {
    match load_domain_json::<DMonitorStateV1>(db_path, DOMAIN_DMONITOR_STATE)? {
        Some(state) => {
            validate_persisted_dmonitor_state_integrity(&state)?;
            Ok(state)
        }
        None => Ok(DMonitorStateV1::empty()),
    }
}

pub fn persist_state(db_path: &Path, state: &DMonitorStateV1) -> Result<(), String> {
    persist_domain_json(db_path, DOMAIN_DMONITOR_STATE, state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PROJECT: &str = "proj:lab";
    const NODE: &str = "fog-01";
    const MONITOR: &str = "fog-01-monitor-1";

    fn observation(sequence: u64, observation_id: &str) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: crate::core::canonical_dmonitor::CANONICAL_DMONITOR_OBSERVATION_SCHEMA
                .to_string(),
            observation_id: observation_id.to_string(),
            project_id: None,
            node_id: NODE.to_string(),
            monitor_instance_id: MONITOR.to_string(),
            sequence,
            observed_at_utc: "2026-09-25T00:00:00Z".to_string(),
            cycle_id: None,
            collector: "host_telemetry_linux".to_string(),
            subject: DMonitorSubjectV1::DNode {
                node_id: NODE.to_string(),
            },
            health: crate::core::canonical_dmonitor::DMonitorHealthV1::Healthy,
            facts: json!({"cpu_usage_permille": 100 + sequence}),
        }
    }

    fn obs_id(n: u32) -> String {
        format!("dmonitor-obs:{n:08}-1111-1111-1111-111111111111")
    }

    // ------------------------------------------------------------ INGESTION

    #[test]
    fn first_sequence_is_accepted_and_becomes_head() {
        let mut state = DMonitorStateV1::empty();
        let result = ingest_observation(
            &mut state,
            PROJECT,
            observation(1, &obs_id(1)),
            "2026-09-25T00:00:01Z",
        )
        .unwrap();
        assert_eq!(result.outcome, IngestOutcome::Accepted);
        let head = state.stream_head(PROJECT, NODE, MONITOR).unwrap();
        assert_eq!(head.observation.sequence, 1);
        assert_eq!(head.received_at_utc, "2026-09-25T00:00:01Z");
    }

    #[test]
    fn increasing_sequence_is_accepted() {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        let result = ingest_observation(&mut state, PROJECT, observation(2, &obs_id(2)), "2026-09-25T00:00:02Z").unwrap();
        assert_eq!(result.outcome, IngestOutcome::Accepted);
        assert_eq!(state.stream_head(PROJECT, NODE, MONITOR).unwrap().observation.sequence, 2);
    }

    #[test]
    fn exact_same_sequence_and_digest_replay_is_idempotent_and_preserves_received_at() {
        let mut state = DMonitorStateV1::empty();
        let obs = observation(1, &obs_id(1));
        let first = ingest_observation(&mut state, PROJECT, obs.clone(), "2026-09-25T00:00:01Z").unwrap();
        let second = ingest_observation(&mut state, PROJECT, obs, "2026-09-25T00:00:09Z").unwrap();
        assert_eq!(second.outcome, IngestOutcome::IdempotentReplay);
        assert_eq!(second.record.received_at_utc, "2026-09-25T00:00:01Z");
        assert_eq!(first.record, second.record);
        assert_eq!(state.streams.len(), 1);
        assert_eq!(state.streams.values().next().unwrap().observations.len(), 1);
    }

    #[test]
    fn same_sequence_different_digest_is_rejected_as_conflict() {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        let mut different = observation(1, &obs_id(2));
        different.facts = json!({"cpu_usage_permille": 999});
        let error = ingest_observation(&mut state, PROJECT, different, "2026-09-25T00:00:02Z").unwrap_err();
        assert!(matches!(error, IngestDMonitorObservationError::SequenceConflict { sequence: 1, .. }));
        assert_eq!(state.streams.values().next().unwrap().observations.len(), 1);
    }

    #[test]
    fn lower_sequence_than_head_is_rejected_as_stale() {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(5, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        let error = ingest_observation(&mut state, PROJECT, observation(3, &obs_id(2)), "2026-09-25T00:00:02Z").unwrap_err();
        assert!(matches!(error, IngestDMonitorObservationError::SequenceStale { sequence: 3, current_head: 5, .. }));
        assert_eq!(state.streams.values().next().unwrap().observations.len(), 1);
    }

    #[test]
    fn new_monitor_instance_may_start_at_sequence_one_independently() {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(5, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        let mut restarted = observation(1, &obs_id(2));
        restarted.monitor_instance_id = "fog-01-monitor-2".to_string();
        let result = ingest_observation(&mut state, PROJECT, restarted, "2026-09-25T00:00:02Z").unwrap();
        assert_eq!(result.outcome, IngestOutcome::Accepted);
        assert_eq!(state.streams.len(), 2);
    }

    #[test]
    fn two_monitor_instances_remain_independent_streams_not_merged() {
        let mut state = DMonitorStateV1::empty();
        let mut a = observation(1, &obs_id(1));
        a.monitor_instance_id = "fog-01-monitor-a".to_string();
        let mut b = observation(1, &obs_id(2));
        b.monitor_instance_id = "fog-01-monitor-b".to_string();
        ingest_observation(&mut state, PROJECT, a, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, b, "2026-09-25T00:00:02Z").unwrap();
        let head_a = state.stream_head(PROJECT, NODE, "fog-01-monitor-a").unwrap();
        let head_b = state.stream_head(PROJECT, NODE, "fog-01-monitor-b").unwrap();
        assert_ne!(head_a.observation.observation_id, head_b.observation.observation_id);
        assert_eq!(state.streams.len(), 2);
    }

    #[test]
    fn same_observation_id_same_digest_is_a_safe_replay() {
        let mut state = DMonitorStateV1::empty();
        let obs = observation(1, &obs_id(1));
        ingest_observation(&mut state, PROJECT, obs.clone(), "2026-09-25T00:00:01Z").unwrap();
        let result = ingest_observation(&mut state, PROJECT, obs, "2026-09-25T00:00:05Z").unwrap();
        assert_eq!(result.outcome, IngestOutcome::IdempotentReplay);
    }

    #[test]
    fn same_observation_id_different_digest_is_rejected() {
        let mut state = DMonitorStateV1::empty();
        let obs = observation(1, &obs_id(1));
        ingest_observation(&mut state, PROJECT, obs.clone(), "2026-09-25T00:00:01Z").unwrap();
        let mut tampered = obs;
        tampered.facts = json!({"cpu_usage_permille": 777});
        let error = ingest_observation(&mut state, PROJECT, tampered, "2026-09-25T00:00:02Z").unwrap_err();
        assert!(matches!(error, IngestDMonitorObservationError::ObservationIdConflict { .. }));
    }

    #[test]
    fn invalid_observation_is_rejected_without_mutating_state() {
        let mut state = DMonitorStateV1::empty();
        let mut invalid = observation(1, &obs_id(1));
        invalid.sequence = 0;
        let error = ingest_observation(&mut state, PROJECT, invalid, "2026-09-25T00:00:01Z").unwrap_err();
        assert!(matches!(error, IngestDMonitorObservationError::Invalid(_)));
        assert!(state.streams.is_empty());
    }

    #[test]
    fn project_mismatch_is_rejected() {
        let mut state = DMonitorStateV1::empty();
        let mut obs = observation(1, &obs_id(1));
        obs.project_id = Some("proj:other".to_string());
        let error = ingest_observation(&mut state, PROJECT, obs, "2026-09-25T00:00:01Z").unwrap_err();
        assert!(matches!(
            error,
            IngestDMonitorObservationError::ProjectScopeMismatch { .. }
        ));
        assert!(state.streams.is_empty());
    }

    #[test]
    fn none_project_id_is_accepted_and_stored_under_request_project_scope() {
        let mut state = DMonitorStateV1::empty();
        let obs = observation(1, &obs_id(1));
        assert!(obs.project_id.is_none());
        let result = ingest_observation(&mut state, PROJECT, obs, "2026-09-25T00:00:01Z").unwrap();
        assert_eq!(result.record.server_project_scope, PROJECT);
        assert!(result.record.observation.project_id.is_none());
    }

    // ------------------------------------------------- PHASE 114B1.1 CLOSURE

    /// Section 1: the SAME `monitor_instance_id` resetting to a lower
    /// sequence after it has already advanced is stale/out-of-order, never
    /// treated as a fresh epoch -- only a genuinely NEW `monitor_instance_id`
    /// may start one.
    #[test]
    fn sequence_reset_under_the_same_monitor_instance_id_is_stale_not_a_new_epoch() {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(100, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        let error = ingest_observation(&mut state, PROJECT, observation(1, &obs_id(2)), "2026-09-25T00:00:02Z").unwrap_err();
        assert!(matches!(
            error,
            IngestDMonitorObservationError::SequenceStale { sequence: 1, current_head: 100, .. }
        ));
        assert_eq!(state.stream_head(PROJECT, NODE, MONITOR).unwrap().observation.sequence, 100);
    }

    /// Section 6: an EXACT replay (same observation_id + digest) of an
    /// already-persisted observation is idempotent even after the stream
    /// head has since advanced past it -- this is the deliberate, documented
    /// precedence (observation_id identity is checked before the sequence
    /// rule), giving retries deterministic behavior. Contrast with
    /// `lower_sequence_than_head_is_rejected_as_stale`, which covers an
    /// UNSEEN lower sequence (correctly still stale).
    #[test]
    fn historical_exact_replay_after_head_advanced_is_idempotent_not_stale() {
        let mut state = DMonitorStateV1::empty();
        let seq1 = observation(1, &obs_id(1));
        ingest_observation(&mut state, PROJECT, seq1.clone(), "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, observation(2, &obs_id(2)), "2026-09-25T00:00:02Z").unwrap();
        assert_eq!(state.stream_head(PROJECT, NODE, MONITOR).unwrap().observation.sequence, 2);

        let replay = ingest_observation(&mut state, PROJECT, seq1, "2026-09-25T00:05:00Z").unwrap();
        assert_eq!(replay.outcome, IngestOutcome::IdempotentReplay);
        assert_eq!(replay.record.received_at_utc, "2026-09-25T00:00:01Z");
        // The stream must not have been perturbed by the historical replay.
        assert_eq!(state.stream_head(PROJECT, NODE, MONITOR).unwrap().observation.sequence, 2);
    }

    /// Section 4: replaying the identical `observation_id` + digest (a
    /// project-less observation) under a DIFFERENT server project scope must
    /// never return the first project's persisted record, and must never
    /// silently create a stream under the second project either. Digest
    /// equality alone does not prove same-project identity here because
    /// `server_project_scope` is not part of the canonical digest when
    /// `project_id` is `None` (section 5) -- this is exactly the case where
    /// relying on digest alone would be unsafe.
    #[test]
    fn same_observation_id_and_digest_under_a_different_project_scope_is_rejected_not_leaked() {
        const PROJECT_B: &str = "proj:other";
        let mut state = DMonitorStateV1::empty();
        let obs = observation(1, &obs_id(1));
        assert!(obs.project_id.is_none());
        let first = ingest_observation(&mut state, PROJECT, obs.clone(), "2026-09-25T00:00:01Z").unwrap();
        assert_eq!(first.outcome, IngestOutcome::Accepted);

        let error = ingest_observation(&mut state, PROJECT_B, obs, "2026-09-25T00:00:02Z").unwrap_err();
        assert!(matches!(error, IngestDMonitorObservationError::ObservationIdConflict { .. }));

        // No leakage: project B has no stream at all, and project A's
        // record is completely unaffected.
        assert!(state.stream_head(PROJECT_B, NODE, MONITOR).is_none());
        let project_a_head = state.stream_head(PROJECT, NODE, MONITOR).unwrap();
        assert_eq!(project_a_head.received_at_utc, "2026-09-25T00:00:01Z");
        assert_eq!(project_a_head.server_project_scope, PROJECT);
        assert_eq!(state.streams.len(), 1);
    }

    /// Section 5, explicit positive control: within the SAME project scope,
    /// the observation_id+digest replay check requires stream identity
    /// (node/monitor_instance, both encoded in `stream_key`) and sequence to
    /// match too, not merely the digest -- proven here by a same-project,
    /// different-`monitor_instance_id` attempt at reusing an observation_id.
    /// Since `monitor_instance_id` is itself a canonical field, an identical
    /// digest under a different `monitor_instance_id` is actually
    /// impossible to construct honestly; this test instead directly proves
    /// the index carries full stream identity by checking the stored
    /// `observation_index` entry's `stream_key`, not merely a passing
    /// digest comparison.
    #[test]
    fn observation_index_entry_carries_full_stream_identity_not_only_digest() {
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        let entry = state.observation_index.get(&obs_id(1)).unwrap();
        assert_eq!(entry.stream_key, stream_key(PROJECT, NODE, MONITOR));
        assert_eq!(entry.sequence, 1);
    }

    // --------------------------------------------------------------- SUBJECT

    #[test]
    fn subject_key_is_deterministic_and_distinguishes_subjects() {
        let dnode_key = subject_key(&DMonitorSubjectV1::DNode { node_id: NODE.to_string() });
        let dnode_key_again = subject_key(&DMonitorSubjectV1::DNode { node_id: NODE.to_string() });
        let other_node_key = subject_key(&DMonitorSubjectV1::DNode { node_id: "cloud-01".to_string() });
        assert_eq!(dnode_key, dnode_key_again);
        assert_ne!(dnode_key, other_node_key);
    }

    #[test]
    fn unbound_subject_key_never_incorporates_facts_or_description() {
        let a = subject_key(&DMonitorSubjectV1::Unbound {
            node_id: NODE.to_string(),
            description: Some("container A".to_string()),
        });
        let b = subject_key(&DMonitorSubjectV1::Unbound {
            node_id: NODE.to_string(),
            description: Some("completely different container B".to_string()),
        });
        assert_eq!(a, b, "Unbound subject identity must depend only on node_id, never description/facts");
    }

    #[test]
    fn multiple_subjects_do_not_collide() {
        let mut state = DMonitorStateV1::empty();
        let mut obs1 = observation(1, &obs_id(1));
        obs1.subject = DMonitorSubjectV1::DNode { node_id: NODE.to_string() };
        let mut obs2 = observation(1, &obs_id(2));
        obs2.monitor_instance_id = "fog-01-monitor-2".to_string();
        obs2.subject = DMonitorSubjectV1::NetworkReachability {
            node_id: NODE.to_string(),
            target: "cloud-01:1883".to_string(),
        };
        ingest_observation(&mut state, PROJECT, obs1, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, obs2, "2026-09-25T00:00:02Z").unwrap();
        assert_eq!(state.subject_index.len(), 2);
    }

    fn subject_a() -> DMonitorSubjectV1 {
        DMonitorSubjectV1::NetworkReachability {
            node_id: NODE.to_string(),
            target: "subject-a-target".to_string(),
        }
    }

    fn subject_b() -> DMonitorSubjectV1 {
        DMonitorSubjectV1::NetworkReachability {
            node_id: NODE.to_string(),
            target: "subject-b-target".to_string(),
        }
    }

    /// Section 7: a single stream interleaving two subjects (seq1 -> A,
    /// seq2 -> B) must let each subject resolve to ITS OWN latest
    /// observation, never both collapsing onto the stream's global head.
    #[test]
    fn subject_index_resolves_each_subject_to_its_own_latest_observation_a1_b2() {
        let mut state = DMonitorStateV1::empty();
        let mut a1 = observation(1, &obs_id(1));
        a1.subject = subject_a();
        let mut b2 = observation(2, &obs_id(2));
        b2.subject = subject_b();
        ingest_observation(&mut state, PROJECT, a1, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, b2, "2026-09-25T00:00:02Z").unwrap();

        let candidates_a = state.subject_candidates(PROJECT, &subject_key(&subject_a()));
        let candidates_b = state.subject_candidates(PROJECT, &subject_key(&subject_b()));
        assert_eq!(candidates_a.len(), 1);
        assert_eq!(candidates_a[0].observation.sequence, 1, "subject A must resolve to A1, not the stream head");
        assert_eq!(candidates_b.len(), 1);
        assert_eq!(candidates_b[0].observation.sequence, 2);
        // The stream's global head is B2 -- proving A's answer above is NOT
        // merely "whatever the stream head currently is".
        assert_eq!(state.stream_head(PROJECT, NODE, MONITOR).unwrap().observation.sequence, 2);
    }

    /// Section 7, the fully load-bearing case: A1, B2, A3 -- subject A must
    /// now resolve to A3 (its own latest), while subject B remains at B2
    /// (untouched by A's later re-observation).
    #[test]
    fn subject_index_tracks_the_latest_observation_per_subject_per_stream_not_the_stream_head() {
        let mut state = DMonitorStateV1::empty();
        let mut a1 = observation(1, &obs_id(1));
        a1.subject = subject_a();
        let mut b2 = observation(2, &obs_id(2));
        b2.subject = subject_b();
        let mut a3 = observation(3, &obs_id(3));
        a3.subject = subject_a();
        ingest_observation(&mut state, PROJECT, a1, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, b2, "2026-09-25T00:00:02Z").unwrap();
        ingest_observation(&mut state, PROJECT, a3, "2026-09-25T00:00:03Z").unwrap();

        let candidates_a = state.subject_candidates(PROJECT, &subject_key(&subject_a()));
        let candidates_b = state.subject_candidates(PROJECT, &subject_key(&subject_b()));
        assert_eq!(candidates_a.len(), 1, "one stream, one candidate for A");
        assert_eq!(candidates_a[0].observation.sequence, 3, "subject A must now resolve to A3");
        assert_eq!(candidates_b.len(), 1);
        assert_eq!(candidates_b[0].observation.sequence, 2, "subject B must remain B2, untouched by A3");
    }

    /// Same interleaving proof, but across TWO independent monitor-instance
    /// streams: subject A appears in both streams, and each stream's own
    /// latest-for-A must be tracked independently (never merged into one
    /// "authoritative" answer -- section 8's multi-instance tolerance
    /// applies to subject candidates too).
    #[test]
    fn subject_index_tracks_latest_per_subject_independently_across_two_monitor_instances() {
        let mut state = DMonitorStateV1::empty();
        let mut a1_instance1 = observation(1, &obs_id(1));
        a1_instance1.subject = subject_a();
        let mut a1_instance2 = observation(1, &obs_id(2));
        a1_instance2.monitor_instance_id = "fog-01-monitor-2".to_string();
        a1_instance2.subject = subject_a();
        let mut a5_instance1 = observation(5, &obs_id(3));
        a5_instance1.subject = subject_a();

        ingest_observation(&mut state, PROJECT, a1_instance1, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, a1_instance2, "2026-09-25T00:00:02Z").unwrap();
        ingest_observation(&mut state, PROJECT, a5_instance1, "2026-09-25T00:00:03Z").unwrap();

        let candidates_a = state.subject_candidates(PROJECT, &subject_key(&subject_a()));
        assert_eq!(candidates_a.len(), 2, "two independent streams both observed subject A");
        let sequences: std::collections::BTreeSet<u64> =
            candidates_a.iter().map(|record| record.observation.sequence).collect();
        assert_eq!(sequences, std::collections::BTreeSet::from([5, 1]));
    }

    /// Section 8: two UNRELATED `Unbound` observations on the same node must
    /// never be treated as the same canonical subject -- the safest design
    /// (chosen here) excludes `Unbound` from the subject index entirely,
    /// proven by asserting the index stays completely empty.
    #[test]
    fn two_unrelated_unbound_observations_on_the_same_node_are_never_correlated() {
        let mut state = DMonitorStateV1::empty();
        let mut first = observation(1, &obs_id(1));
        first.subject = DMonitorSubjectV1::Unbound {
            node_id: NODE.to_string(),
            description: Some("an unrelated container".to_string()),
        };
        let mut second = observation(2, &obs_id(2));
        second.subject = DMonitorSubjectV1::Unbound {
            node_id: NODE.to_string(),
            description: Some("a completely different unrelated process".to_string()),
        };
        ingest_observation(&mut state, PROJECT, first, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, second, "2026-09-25T00:00:02Z").unwrap();

        assert!(
            state.subject_index.is_empty(),
            "Unbound observations must never be indexed by subject, even on the same node: {:?}",
            state.subject_index
        );
        // Both remain honestly retrievable by their own identity.
        assert!(state.by_observation_id(&obs_id(1)).is_some());
        assert!(state.by_observation_id(&obs_id(2)).is_some());
    }

    // ----------------------------------------------------------- PERSISTENCE

    #[test]
    fn round_trip_save_and_load_preserves_state() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        persist_state(&path, &state).unwrap();
        let reloaded = load_state(&path).unwrap();
        assert_eq!(reloaded, state);
        cleanup(&path);
    }

    #[test]
    fn reload_validates_every_canonical_observation() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        // Corrupt the persisted canonical observation directly (bypassing
        // ingest_observation's own validation entirely) to prove reload
        // re-validates rather than merely trusting the digest. `collector`
        // is chosen deliberately: it fails ONLY `validate()`'s
        // `missing_field` check, not the earlier sequence/node/project
        // structural checks this same loop also performs.
        for stream in state.streams.values_mut() {
            for record in stream.observations.values_mut() {
                record.observation.collector = String::new();
            }
        }
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("fails canonical validation"));
        cleanup(&path);
    }

    #[test]
    fn digest_tamper_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        for stream in state.streams.values_mut() {
            for record in stream.observations.values_mut() {
                record.observation_digest = format!("sha256:{}", "0".repeat(64));
            }
        }
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("does not match recomputed"));
        cleanup(&path);
    }

    #[test]
    fn stream_head_tamper_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        for stream in state.streams.values_mut() {
            stream.head_sequence = 99;
        }
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("head_sequence"));
        cleanup(&path);
    }

    #[test]
    fn dangling_observation_index_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        state.observation_index.insert(
            "dmonitor-obs:dangling".to_string(),
            DMonitorObservationIndexEntryV1 {
                stream_key: "does-not-exist".to_string(),
                sequence: 1,
                observation_digest: format!("sha256:{}", "1".repeat(64)),
            },
        );
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("missing stream"));
        cleanup(&path);
    }

    #[test]
    fn project_scope_tamper_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        for stream in state.streams.values_mut() {
            for record in stream.observations.values_mut() {
                record.server_project_scope = "proj:tampered".to_string();
            }
        }
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("server_project_scope"));
        cleanup(&path);
    }

    /// Section 10 exact scenario: `observations = {1, 2, 3}` but
    /// `head_sequence = 2` -- corrupt even though sequence 2 genuinely
    /// exists, because the head must be exactly the MAXIMUM accepted
    /// sequence.
    #[test]
    fn stream_head_not_the_maximum_sequence_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, observation(2, &obs_id(2)), "2026-09-25T00:00:02Z").unwrap();
        ingest_observation(&mut state, PROJECT, observation(3, &obs_id(3)), "2026-09-25T00:00:03Z").unwrap();
        for stream in state.streams.values_mut() {
            stream.head_sequence = 2;
            stream.head_observation_id = obs_id(2);
        }
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("is not the maximum accepted sequence"), "{error}");
        cleanup(&path);
    }

    /// Section 9: `subject_index` recording the WRONG typed subject for an
    /// observation (a stale/mismatched entry pointing at real, existing
    /// data) must fail closed -- proving indexes are checked for EXACTNESS,
    /// not merely "does the pointer resolve to something."
    #[test]
    fn subject_index_pointing_to_the_wrong_typed_subject_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        let mut a1 = observation(1, &obs_id(1));
        a1.subject = subject_a();
        ingest_observation(&mut state, PROJECT, a1, "2026-09-25T00:00:01Z").unwrap();
        // The real subject is A, but record the index under B's key instead.
        state.subject_index.clear();
        let stream_key_value = stream_key(PROJECT, NODE, MONITOR);
        state
            .subject_index
            .entry(subject_key(&subject_b()))
            .or_default()
            .insert(stream_key_value, 1);
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("subject_index does not match"), "{error}");
        cleanup(&path);
    }

    /// Section 9: `subject_index` pointing at an OLDER sequence for a
    /// subject when a NEWER observation of that same subject in that same
    /// stream exists must fail closed -- the index must always reflect the
    /// LATEST, not merely "a" valid observation.
    #[test]
    fn subject_index_pointing_to_a_stale_sequence_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        let mut a1 = observation(1, &obs_id(1));
        a1.subject = subject_a();
        let mut a3 = observation(3, &obs_id(2));
        a3.subject = subject_a();
        ingest_observation(&mut state, PROJECT, a1, "2026-09-25T00:00:01Z").unwrap();
        ingest_observation(&mut state, PROJECT, a3, "2026-09-25T00:00:02Z").unwrap();
        let stream_key_value = stream_key(PROJECT, NODE, MONITOR);
        state
            .subject_index
            .get_mut(&subject_key(&subject_a()))
            .unwrap()
            .insert(stream_key_value, 1); // downgrade from 3 back to 1
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("subject_index does not match"), "{error}");
        cleanup(&path);
    }

    /// Section 9 + section 8: an `Unbound` observation that somehow ended up
    /// indexed by subject (violating the final policy) must fail closed on
    /// reload, not be silently tolerated.
    #[test]
    fn unbound_observation_indexed_contrary_to_policy_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        let mut unbound = observation(1, &obs_id(1));
        unbound.subject = DMonitorSubjectV1::Unbound {
            node_id: NODE.to_string(),
            description: None,
        };
        ingest_observation(&mut state, PROJECT, unbound.clone(), "2026-09-25T00:00:01Z").unwrap();
        assert!(state.subject_index.is_empty());
        // Forcibly index it anyway, contrary to policy.
        let stream_key_value = stream_key(PROJECT, NODE, MONITOR);
        state
            .subject_index
            .entry(subject_key(&unbound.subject))
            .or_default()
            .insert(stream_key_value, 1);
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("subject_index does not match"), "{error}");
        cleanup(&path);
    }

    /// Section 9: a persisted observation with NO corresponding
    /// `observation_index` entry at all must fail closed (the reverse
    /// direction of `dangling_observation_index_is_rejected_on_reload`,
    /// which tests an EXTRA dangling entry rather than a MISSING one).
    #[test]
    fn missing_observation_index_entry_for_a_persisted_observation_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        state.observation_index.clear();
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("is not present in observation_index"), "{error}");
        cleanup(&path);
    }

    /// Section 9: an `observation_index` entry whose recorded digest does
    /// not match the actual stored record's digest must fail closed -- an
    /// index entry that resolves to something real but describes it
    /// incorrectly is still corruption.
    #[test]
    fn observation_index_entry_with_wrong_digest_is_rejected_on_reload() {
        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-state-test-{}.db",
            uuid::Uuid::new_v4()
        ));
        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();
        state.observation_index.get_mut(&obs_id(1)).unwrap().observation_digest =
            format!("sha256:{}", "9".repeat(64));
        persist_state(&path, &state).unwrap();
        let error = load_state(&path).unwrap_err();
        assert!(error.contains("digest does not match"), "{error}");
        cleanup(&path);
    }

    /// Section 11: a failed persist must leave no durable mutation. The
    /// `base` path is made to exist as a plain FILE, so `open_database`'s
    /// `fs::create_dir_all(parent)` step (parent = `base`) is guaranteed to
    /// fail -- proving `persist_state` cannot partially write, and that a
    /// subsequent `load_state` against the same broken path sees no
    /// artifact from the failed attempt (it fails the same way, rather than
    /// succeeding by reading back a partial write).
    #[test]
    fn failed_persist_leaves_no_durable_mutation() {
        let base = std::env::temp_dir().join(format!(
            "datum-dmonitor-atomicity-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&base, b"not a directory").unwrap();
        let bad_path = base.join("db.sqlite");

        let mut state = DMonitorStateV1::empty();
        ingest_observation(&mut state, PROJECT, observation(1, &obs_id(1)), "2026-09-25T00:00:01Z").unwrap();

        let persist_result = persist_state(&bad_path, &state);
        assert!(persist_result.is_err(), "persist against an unwritable path must fail, not silently succeed");

        let reload_result = load_state(&bad_path);
        assert!(
            reload_result.is_err(),
            "a subsequent load against the same broken path must find nothing durable from the failed attempt"
        );

        std::fs::remove_file(&base).ok();
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
