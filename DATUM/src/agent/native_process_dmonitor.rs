//! Governed continuously-running (native-process) D-Serv realization
//! DMonitor evidence builder (Phase 115D, ADR-0024, building on Phase 115C's
//! identity propagation). Consumes the local runtime-ownership record
//! [`ManagedProcessRuntimeStateV1`] plus a freshly-computed [`IdentityCheck`]
//! (both obtained via `native_process_runtime::observe_current_native_state`,
//! never re-derived a second way) and produces one
//! [`CanonicalDMonitorObservationV1`] with `subject = DServRealization`.
//!
//! # Identity vs. observation (Phase 115A/115D split)
//!
//! `application_id`/`dmap_ref`/`service_id` are IDENTITY -- carried verbatim
//! from `ManagedProcessRuntimeStateV1`, which itself only ever received them
//! from a governed `OperationalArtifactAssignment` (Phase 115C), never
//! derived from the process/container itself. The freshly-observed
//! `IdentityCheck` (a live `/proc` re-read, not the persisted snapshot) is
//! the OBSERVATION -- it drives `health` only, never identity. This module
//! never inspects `observed_executable`/`cmdline_digest`/`pid` for anything
//! other than logging-free health classification; they never leak into the
//! canonical subject or `facts` (avoiding exactly the "container/process
//! name as identity" mistake this project's own invariants forbid).
//!
//! # Fail-closed on missing identity (Phase 115C/115A: never guess)
//!
//! If `application_id` is empty or `dmap_ref` is `None` -- a record
//! persisted before Phase 115C, or one resolved under
//! `PlacementAuthority::LegacyTargetNodes` (no D-Map at all) --
//! [`build_dserv_realization_from_native_state`] returns `None`. This is not
//! merely a style choice: `DMonitorSubjectV1::DServRealization` REQUIRES a
//! non-empty `application_id` (`canonical_dmonitor::DMonitorSubjectV1::validate`),
//! so a record missing it could not produce a valid observation regardless.
//!
//! # Health mapping (from a live identity re-check, never invented)
//!
//! - [`IdentityCheck::Confirmed`] -> `Healthy`: PID, starttime, executable,
//!   and cmdline digest all match the governed record right now.
//! - [`IdentityCheck::ProcessAbsent`] | [`IdentityCheck::PidReused`] |
//!   [`IdentityCheck::ExecutableMismatch`] | [`IdentityCheck::ArgsMismatch`] |
//!   [`IdentityCheck::Zombie`] -> `Unhealthy`: a definite, disproven or
//!   disqualified realization.
//! - [`IdentityCheck::Unavailable`] -> `Unknown`: a procfs race meant this
//!   cycle could not confirm OR disprove identity -- honest uncertainty,
//!   never coerced to `Healthy` or `Unhealthy`.

use serde_json::json;

use crate::agent::canonical_dmonitor::{
    CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, DMonitorSubjectV1,
    CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
};
use crate::agent::dmonitor_emission::{observation_id, DMonitorEmissionContext};
use crate::agent::native_process_state::{IdentityCheck, ManagedProcessRuntimeStateV1};

pub const NATIVE_PROCESS_OBSERVE_COLLECTOR: &str = "native_process_observe";

fn health_for_identity(identity: &IdentityCheck) -> DMonitorHealthV1 {
    match identity {
        IdentityCheck::Confirmed => DMonitorHealthV1::Healthy,
        IdentityCheck::ProcessAbsent
        | IdentityCheck::PidReused
        | IdentityCheck::ExecutableMismatch
        | IdentityCheck::ArgsMismatch
        | IdentityCheck::Zombie => DMonitorHealthV1::Unhealthy,
        IdentityCheck::Unavailable => DMonitorHealthV1::Unknown,
    }
}

fn identity_as_str(identity: &IdentityCheck) -> &'static str {
    match identity {
        IdentityCheck::Confirmed => "confirmed",
        IdentityCheck::ProcessAbsent => "process_absent",
        IdentityCheck::PidReused => "pid_reused",
        IdentityCheck::ExecutableMismatch => "executable_mismatch",
        IdentityCheck::ArgsMismatch => "args_mismatch",
        IdentityCheck::Zombie => "zombie",
        IdentityCheck::Unavailable => "unavailable",
    }
}

/// Builds one canonical `DServRealization` DMonitor observation from a real
/// `ManagedProcessRuntimeStateV1` + freshly-observed `IdentityCheck`, or
/// `None` if `project_id` is empty or the record predates Phase 115C's
/// identity propagation (empty `application_id` / absent `dmap_ref` --
/// fail-closed, never guessed). `project_id` is caller-supplied (from the
/// same governed `ArtifactReconciliationContext`/`OperationalReconciliationContext`
/// the reconciliation loop already threads through unmodified) rather than
/// persisted on `ManagedProcessRuntimeStateV1` itself -- unlike
/// `application_id`/`dmap_ref`, `project_id` was never dropped anywhere in
/// the existing call chain (Phase 115A Q9 found no loss for it), so nothing
/// here needed propagating; it is simply threaded like `this_node_id`.
///
/// `sequence` must come from `context.allocate_sequence()`, called exactly
/// once by the caller immediately before this -- mirrors
/// `diot_runtime_dmonitor::build_diot_realization_observation` and
/// `dcode_execution_dmonitor::build_dserv_realization_from_dcode_evidence`.
pub fn build_dserv_realization_from_native_state(
    context: &DMonitorEmissionContext,
    sequence: u64,
    this_node_id: &str,
    project_id: &str,
    observed_at_utc: &str,
    persisted: &ManagedProcessRuntimeStateV1,
    identity: &IdentityCheck,
) -> Option<CanonicalDMonitorObservationV1> {
    if project_id.trim().is_empty() || persisted.application_id.trim().is_empty() {
        return None;
    }
    let dmap_ref = persisted.dmap_ref.as_ref()?;

    Some(CanonicalDMonitorObservationV1 {
        schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
        observation_id: observation_id(context.monitor_instance_id(), sequence),
        project_id: Some(project_id.to_string()),
        node_id: this_node_id.to_string(),
        monitor_instance_id: context.monitor_instance_id().to_string(),
        sequence,
        observed_at_utc: observed_at_utc.to_string(),
        cycle_id: None,
        collector: NATIVE_PROCESS_OBSERVE_COLLECTOR.to_string(),
        subject: DMonitorSubjectV1::DServRealization {
            project_id: project_id.to_string(),
            application_id: persisted.application_id.clone(),
            service_id: persisted.service_id.clone(),
            node_id: this_node_id.to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: dmap_ref.id.clone(),
                revision: dmap_ref.revision,
                digest: dmap_ref.digest.clone(),
            }),
        },
        health: health_for_identity(identity),
        facts: json!({
            "identity_check": identity_as_str(identity),
        }),
    })
}

/// Thin wrapper around one [`DMonitorEmissionContext`] (one process epoch,
/// shared across every service this loop observes -- never one context per
/// service) so a caller iterating multiple managed services per cycle has
/// one small object to carry, mirroring `DCodeExecutionMonitorEmitter`.
/// Unlike that emitter, there is no fixed per-emitter identity to bind: each
/// call observes a DIFFERENT service, so identity is supplied fresh each
/// time from that service's own `ManagedProcessRuntimeStateV1`.
pub struct NativeProcessMonitorEmitter {
    context: DMonitorEmissionContext,
}

impl NativeProcessMonitorEmitter {
    pub fn new(context: DMonitorEmissionContext) -> Self {
        Self { context }
    }

    pub fn monitor_instance_id(&self) -> &str {
        self.context.monitor_instance_id()
    }

    /// Allocates the next sequence and builds one observation for ONE
    /// service in one step (or `None` per
    /// `build_dserv_realization_from_native_state`'s own fail-closed rules)
    /// -- the only way a caller can obtain a sequence from this emitter, so
    /// a sequence is never allocated without a corresponding attempted
    /// observation. On `None`, the allocated sequence is simply left
    /// unpublished (a permitted gap), never reused.
    pub fn emit(
        &mut self,
        this_node_id: &str,
        project_id: &str,
        observed_at_utc: &str,
        persisted: &ManagedProcessRuntimeStateV1,
        identity: &IdentityCheck,
    ) -> Option<CanonicalDMonitorObservationV1> {
        let sequence = self.context.allocate_sequence();
        build_dserv_realization_from_native_state(
            &self.context,
            sequence,
            this_node_id,
            project_id,
            observed_at_utc,
            persisted,
            identity,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcode::authorization::CanonicalContractReferenceV1;

    const NODE: &str = "fog-01";
    const PROJECT: &str = "proj:lab";

    fn context() -> DMonitorEmissionContext {
        DMonitorEmissionContext::from_parts(
            "dmonitor-instance:fog-01:fixed-for-test".to_string(),
            1,
        )
    }

    fn persisted_state() -> ManagedProcessRuntimeStateV1 {
        ManagedProcessRuntimeStateV1 {
            schema: crate::agent::native_process_state::MANAGED_PROCESS_RUNTIME_STATE_SCHEMA
                .to_string(),
            service_id: "decode".to_string(),
            artifact_id: "decode-artifact".to_string(),
            execution_generation: 1,
            execution_projection_digest: format!("sha256:{}", "a".repeat(64)),
            application_id: "app:pipeline".to_string(),
            dmap_ref: Some(CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: format!("sha256:{}", "b".repeat(64)),
            }),
            content_sha256: "c".repeat(64),
            pid: 4242,
            proc_start_time_ticks: 123456,
            observed_executable: "/opt/datum/decode/bin/run".to_string(),
            cmdline_digest: crate::agent::native_process_state::compute_args_digest(&[]),
            started_at_utc: "2026-01-01T00:00:00Z".to_string(),
            state: crate::agent::native_process_state::PROCESS_STATE_RUNNING.to_string(),
        }
    }

    #[test]
    fn confirmed_identity_is_healthy_dserv_realization_with_exact_identity() {
        let state = persisted_state();
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &state,
            &IdentityCheck::Confirmed,
        )
        .expect("full identity must produce an observation");
        assert_eq!(observation.health, DMonitorHealthV1::Healthy);
        assert_eq!(observation.project_id, Some(PROJECT.to_string()));
        assert_eq!(observation.node_id, NODE);
        assert!(
            observation.validate().is_empty(),
            "{:?}",
            observation.validate()
        );
        match observation.subject {
            DMonitorSubjectV1::DServRealization {
                project_id,
                application_id,
                service_id,
                node_id,
                dmap_ref,
            } => {
                assert_eq!(project_id, PROJECT);
                assert_eq!(application_id, "app:pipeline");
                assert_eq!(service_id, "decode");
                assert_ne!(
                    service_id, state.artifact_id,
                    "service_id must never be the artifact/D-Code identity"
                );
                assert_eq!(node_id, NODE);
                let dmap_ref = dmap_ref.unwrap();
                assert_eq!(dmap_ref.id, "dmap:1");
                assert_eq!(dmap_ref.revision, 3);
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    #[test]
    fn process_absent_is_unhealthy() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::ProcessAbsent,
        )
        .unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn pid_reused_is_unhealthy() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::PidReused,
        )
        .unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn executable_mismatch_is_unhealthy() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::ExecutableMismatch,
        )
        .unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn args_mismatch_is_unhealthy() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::ArgsMismatch,
        )
        .unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn zombie_is_unhealthy_never_confused_with_running() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::Zombie,
        )
        .unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    /// A procfs race is honest uncertainty, never coerced to Healthy (which
    /// would falsely promote readiness) or Unhealthy (which would falsely
    /// deny it for a service that may well still be running fine).
    #[test]
    fn unavailable_identity_is_unknown_never_healthy_or_unhealthy() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::Unavailable,
        )
        .unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unknown);
    }

    /// Phase 115A/115C: a record that predates identity propagation (or one
    /// resolved under legacy target_nodes authority, which never had a
    /// D-Map) must never be promoted into a canonical observation --
    /// fail-closed, never guessed.
    #[test]
    fn missing_application_id_fails_closed_with_no_observation() {
        let mut state = persisted_state();
        state.application_id = String::new();
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &state,
            &IdentityCheck::Confirmed,
        );
        assert!(observation.is_none());
    }

    #[test]
    fn missing_dmap_ref_fails_closed_with_no_observation() {
        let mut state = persisted_state();
        state.dmap_ref = None;
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &state,
            &IdentityCheck::Confirmed,
        );
        assert!(observation.is_none());
    }

    #[test]
    fn empty_project_id_fails_closed_with_no_observation() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            "",
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::Confirmed,
        );
        assert!(observation.is_none());
    }

    #[test]
    fn observation_never_carries_a_desired_state_key() {
        let observation = build_dserv_realization_from_native_state(
            &context(),
            1,
            NODE,
            PROJECT,
            "2026-01-01T00:00:05Z",
            &persisted_state(),
            &IdentityCheck::Confirmed,
        )
        .unwrap();
        assert!(observation.facts.get("desired_state").is_none());
    }
}
