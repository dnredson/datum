//! Generic DMonitor process-epoch identity and sequence allocation (Phase
//! 114B2, extracted in Phase 114D0B). This module owns NOTHING specific to
//! any one collector: no SmartSentinel dependency, no DIoT/MQTT dependency.
//! It is the shared seam between `agent::dmonitor_projection` (the
//! SmartSentinel -> canonical DMonitor compatibility projector) and
//! `agent::diot_runtime_dmonitor` (the governed DIoT runtime evidence
//! producer) -- both need IDENTICAL "one process epoch = one
//! `monitor_instance_id` = one monotonic sequence counter" semantics, and
//! this module is the ONE place that rule is implemented, so the two
//! producers cannot subtly drift apart.
//!
//! # Monitor instance epoch
//!
//! [`DMonitorEmissionContext::new_process_epoch`] generates a NEW,
//! DMonitor-native `monitor_instance_id` per process/listener epoch. Never
//! reuse a fixed, node-derived identifier (e.g. a legacy
//! `"<node>-sentinel"`-style value) here: a restarted process must get a
//! FRESH `monitor_instance_id` with its sequence restarting at 1, otherwise
//! DServer's own stale-sequence rejection
//! (`sequence_reset_under_the_same_monitor_instance_id_is_stale_not_a_new_epoch`,
//! Phase 114B1) would reject the restarted process's first observations.
//!
//! # Sequence and observation identity
//!
//! [`DMonitorEmissionContext::allocate_sequence`] hands out one sequence per
//! EMITTED OBSERVATION, monotonically, never reused or rewound -- even if a
//! caller later decides not to publish an already-allocated observation
//! (e.g. a downstream HTTP failure, or a bounded publisher queue that was
//! full), the next observation still receives the FOLLOWING sequence.
//! [`observation_id`] is deterministic:
//! `dmonitor-obs:<monitor_instance_id>:<sequence>` -- retrying the exact
//! same projected observation (same context, same sequence) always
//! produces the identical id, which is what makes Phase-114B1's idempotent-
//! replay ingestion semantics work.

/// One DMonitor emission PROCESS EPOCH. See the module doc for the full
/// rationale on why this is never a fixed, node-derived identifier.
#[derive(Debug, Clone)]
pub struct DMonitorEmissionContext {
    monitor_instance_id: String,
    next_sequence: u64,
}

impl DMonitorEmissionContext {
    /// Starts a NEW process epoch: a freshly generated, globally-unique
    /// `monitor_instance_id` and a sequence counter beginning at 1. Call
    /// this ONCE per one-shot invocation, or ONCE at continuous-mode
    /// service-loop/listener startup -- never once per collection cycle or
    /// once per heartbeat.
    pub fn new_process_epoch(node_id: &str) -> Self {
        Self {
            monitor_instance_id: format!("dmonitor-instance:{node_id}:{}", uuid::Uuid::new_v4()),
            next_sequence: 1,
        }
    }

    /// Deterministic reconstruction of an EXISTING epoch from its already-
    /// known `monitor_instance_id`/`next_sequence` -- used only by tests
    /// that need retry-determinism against a FIXED `monitor_instance_id`
    /// (production code always starts a fresh epoch via
    /// `new_process_epoch`).
    #[cfg(any(test, feature = "dcall-test-support"))]
    pub fn from_parts(monitor_instance_id: String, next_sequence: u64) -> Self {
        Self { monitor_instance_id, next_sequence }
    }

    pub fn monitor_instance_id(&self) -> &str {
        &self.monitor_instance_id
    }

    /// The sequence the NEXT emitted observation will receive. Exposed so a
    /// caller can log/persist current position; never required for
    /// correctness (the context tracks it internally).
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }

    /// Reserves and returns the next sequence, advancing the counter.
    /// Sequences are NEVER reused or rewound: even if a caller later
    /// decides not to publish an already-allocated observation, the next
    /// observation still receives the FOLLOWING sequence, never the
    /// abandoned one -- gaps are permitted, reuse is not.
    pub(crate) fn allocate_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        sequence
    }
}

/// `dmonitor-obs:<monitor_instance_id>:<sequence>`. Deterministic and
/// stable for retry; never derived from a container/process/service name,
/// a broker URI, or a metric value.
pub(crate) fn observation_id(monitor_instance_id: &str, sequence: u64) -> String {
    format!("dmonitor-obs:{monitor_instance_id}:{sequence}")
}
