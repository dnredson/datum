//! Governed DIoT runtime DMonitor evidence builder (Phase 114D0B, ADR-0024).
//! Pure/testable: no HTTP, no MQTT, no file I/O. Consumes an explicit
//! [`DiotRuntimeBindingIdentity`] (the exact governed identity/authority the
//! calling MQTT listener session was established under -- see
//! `dcall::delivery::run_mqtt_dcall_listener`) plus emission context, and
//! produces one [`CanonicalDMonitorObservationV1`] per call.
//!
//! # Not routed through `dmonitor_projection`
//!
//! This producer deliberately does NOT go through
//! `agent::dmonitor_projection` (the generic SmartSentinel -> DMonitor
//! compatibility projector). It has exact governed runtime identity
//! (`diot_id`/`diot_placement_id`/`dmap_ref`/`dforward_ref`/
//! `runtime_binding_ref`) that generic SmartSentinel telemetry intentionally
//! does not carry -- projecting through that generic path would either lose
//! this identity or require bolting governed-authority awareness onto a
//! module whose entire design point is staying subject-agnostic. It DOES
//! reuse `agent::dmonitor_emission` for process-epoch/sequence/
//! observation-id semantics, so both producers share identical rules there
//! (see that module's doc).
//!
//! # Health means transport/session liveness only (section 13)
//!
//! `Healthy` here means exactly one thing: THIS governed MQTT realization
//! session is currently established and subscribed, per the protocol/
//! session state available to DATUM. It does NOT mean D-Serv healthy, whole
//! application healthy, DCall success, placement readiness, or
//! `any_element` eligibility -- those remain entirely out of scope.
//!
//! # No secrets, no broker coordinates (section 15/34)
//!
//! The canonical subject never carries `broker_uri`, credentials, raw
//! topic/filter strings, tokens, grants, or secret configuration --
//! correlation is entirely through `dmap_ref`/`dforward_ref`/
//! `runtime_binding_ref`, exactly as Phase 114D0A's canonical model
//! requires. `facts` is a small fixed allowlist: `runtime_state` (a closed
//! set of category strings, never raw error text) and `heartbeat` (bool).

use serde_json::json;

use crate::agent::canonical_dmonitor::{
    CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1,
    DMonitorRuntimeBindingRefV1, DMonitorSubjectV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
};
use crate::agent::dmonitor_emission::{observation_id, DMonitorEmissionContext};

/// The exact governed identity/authority ONE MQTT listener session was
/// established under -- captured ONCE at session-readiness time and reused
/// unchanged for every observation this session ever emits (section 8's
/// session-binding-immutability rule; see `dcall::delivery` for how this is
/// derived from `client::CurrentRuntimeBindingResponse` and never refreshed
/// mid-session).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiotRuntimeBindingIdentity {
    pub project_id: String,
    pub application_id: String,
    pub diot_id: String,
    pub diot_placement_id: String,
    /// The governed node this binding is FOR (must already have been
    /// verified by the caller to equal the local D-Node's own identity --
    /// this builder does not re-derive or re-check that; see section 7).
    pub node_id: String,
    pub dmap_id: String,
    pub dmap_revision: u64,
    pub dmap_digest: String,
    pub dforward_id: String,
    pub dforward_revision: u64,
    pub dforward_digest: String,
    pub binding_id: String,
    pub binding_revision: u64,
    pub descriptor_digest: String,
}

/// Closed set of non-secret runtime-state categories (section 34's facts
/// allowlist) -- never raw error text, never broker/host details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiotRuntimeState {
    /// Emitted with `health = Healthy`: the session is confirmed
    /// established and subscribed.
    Subscribed,
    /// Emitted with `health = Unhealthy`: a previously-established session
    /// hit a definite transport/session failure.
    TransportFailure,
}

impl DiotRuntimeState {
    fn as_str(self) -> &'static str {
        match self {
            DiotRuntimeState::Subscribed => "subscribed",
            DiotRuntimeState::TransportFailure => "transport_failure",
        }
    }
}

pub const DIOT_MQTT_LISTENER_COLLECTOR: &str = "dcall_mqtt_listener";

/// Builds one canonical `DIoTRealization` DMonitor observation. `sequence`
/// must come from `context.allocate_sequence()` (called exactly once, by
/// the caller, immediately before this -- see
/// `DiotRuntimeMonitorEmitter::emit`), never invented independently, so
/// sequence allocation stays the ONE responsibility of
/// `agent::dmonitor_emission`.
#[allow(clippy::too_many_arguments)]
pub fn build_diot_realization_observation(
    context: &DMonitorEmissionContext,
    sequence: u64,
    observed_at_utc: &str,
    health: DMonitorHealthV1,
    runtime_state: DiotRuntimeState,
    heartbeat: bool,
    binding: &DiotRuntimeBindingIdentity,
) -> CanonicalDMonitorObservationV1 {
    CanonicalDMonitorObservationV1 {
        schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
        observation_id: observation_id(context.monitor_instance_id(), sequence),
        project_id: Some(binding.project_id.clone()),
        // The observer/subject node MUST be identical (Phase 114D0A's
        // `subject_node_id_inconsistent` rule) -- `binding.node_id` is
        // already the caller-verified local D-Node identity (section 7), so
        // both top-level and subject use it directly rather than trusting
        // two independently-passed values to agree.
        node_id: binding.node_id.clone(),
        monitor_instance_id: context.monitor_instance_id().to_string(),
        sequence,
        observed_at_utc: observed_at_utc.to_string(),
        cycle_id: None,
        collector: DIOT_MQTT_LISTENER_COLLECTOR.to_string(),
        subject: DMonitorSubjectV1::DIoTRealization {
            project_id: binding.project_id.clone(),
            application_id: binding.application_id.clone(),
            diot_id: binding.diot_id.clone(),
            diot_placement_id: binding.diot_placement_id.clone(),
            node_id: binding.node_id.clone(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: binding.dmap_id.clone(),
                revision: binding.dmap_revision,
                digest: binding.dmap_digest.clone(),
            }),
            dforward_ref: Some(DMonitorContractRefV1 {
                id: binding.dforward_id.clone(),
                revision: binding.dforward_revision,
                digest: binding.dforward_digest.clone(),
            }),
            runtime_binding_ref: Some(DMonitorRuntimeBindingRefV1 {
                binding_id: binding.binding_id.clone(),
                binding_revision: binding.binding_revision,
                descriptor_digest: binding.descriptor_digest.clone(),
            }),
        },
        health,
        facts: json!({
            "runtime_state": runtime_state.as_str(),
            "heartbeat": heartbeat,
        }),
    }
}

/// Convenience wrapper pairing a [`DMonitorEmissionContext`] with the
/// immutable session binding identity, so `dcall::delivery` has one small
/// object to carry through its listener loop instead of threading both
/// separately at every call site.
pub struct DiotRuntimeMonitorEmitter {
    context: DMonitorEmissionContext,
    binding: DiotRuntimeBindingIdentity,
}

impl DiotRuntimeMonitorEmitter {
    pub fn new(context: DMonitorEmissionContext, binding: DiotRuntimeBindingIdentity) -> Self {
        Self { context, binding }
    }

    pub fn monitor_instance_id(&self) -> &str {
        self.context.monitor_instance_id()
    }

    pub fn binding(&self) -> &DiotRuntimeBindingIdentity {
        &self.binding
    }

    /// Allocates the next sequence and builds one observation in one step
    /// -- the only way a caller can obtain a sequence from this emitter,
    /// so a sequence can never be allocated without an observation being
    /// built for it.
    pub fn emit(
        &mut self,
        observed_at_utc: &str,
        health: DMonitorHealthV1,
        runtime_state: DiotRuntimeState,
        heartbeat: bool,
    ) -> CanonicalDMonitorObservationV1 {
        let sequence = self.context.allocate_sequence();
        build_diot_realization_observation(
            &self.context,
            sequence,
            observed_at_utc,
            health,
            runtime_state,
            heartbeat,
            &self.binding,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding() -> DiotRuntimeBindingIdentity {
        DiotRuntimeBindingIdentity {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            diot_id: "diot:broker".to_string(),
            diot_placement_id: "diotp:broker".to_string(),
            node_id: "fog-01".to_string(),
            dmap_id: "dmap:1".to_string(),
            dmap_revision: 3,
            dmap_digest: format!("sha256:{}", "a".repeat(64)),
            dforward_id: "dforward:1".to_string(),
            dforward_revision: 2,
            dforward_digest: format!("sha256:{}", "b".repeat(64)),
            binding_id: "diot-runtime-binding:proj:lab:app:pipeline:diotp:broker".to_string(),
            binding_revision: 1,
            descriptor_digest: format!("sha256:{}", "d".repeat(64)),
        }
    }

    fn emitter() -> DiotRuntimeMonitorEmitter {
        DiotRuntimeMonitorEmitter::new(
            DMonitorEmissionContext::from_parts("dmonitor-instance:fog-01:fixed-for-test".to_string(), 1),
            binding(),
        )
    }

    #[test]
    fn complete_healthy_observation_validates_and_matches_expected_shape() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        assert_eq!(observation.health, DMonitorHealthV1::Healthy);
        assert_eq!(observation.facts["runtime_state"], "subscribed");
        assert_eq!(observation.facts["heartbeat"], false);
    }

    #[test]
    fn complete_unhealthy_observation_validates_and_matches_expected_shape() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Unhealthy, DiotRuntimeState::TransportFailure, false);
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
        assert_eq!(observation.facts["runtime_state"], "transport_failure");
    }

    #[test]
    fn exact_local_node_is_used_for_both_observer_and_subject() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        assert_eq!(observation.node_id, "fog-01");
        let DMonitorSubjectV1::DIoTRealization { node_id, .. } = &observation.subject else { panic!("wrong subject") };
        assert_eq!(node_id, "fog-01");
    }

    #[test]
    fn session_binding_reference_is_immutable_across_repeated_heartbeats() {
        // Phase 114D0B section 8/26: once a session is established under
        // binding revision N, EVERY observation that session ever emits --
        // the initial one and every subsequent heartbeat -- must keep
        // referencing EXACTLY N, never a later "current" revision the
        // server might advance to while this session stays open. There is
        // no API on `DiotRuntimeMonitorEmitter` that could change
        // `binding` after construction (private field, no setter) --
        // this test confirms that structural guarantee holds in practice
        // across many emits, standing in for "authority advanced to N+1
        // while this session remained open".
        let mut emitter = emitter();
        let observations = [
            emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false),
            emitter.emit("2026-09-25T00:00:30Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, true),
            emitter.emit("2026-09-25T00:01:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, true),
        ];
        for observation in &observations {
            let DMonitorSubjectV1::DIoTRealization { runtime_binding_ref, dmap_ref, dforward_ref, .. } = &observation.subject else {
                panic!("wrong subject")
            };
            assert_eq!(runtime_binding_ref.as_ref().unwrap().binding_revision, binding().binding_revision);
            assert_eq!(runtime_binding_ref.as_ref().unwrap().binding_id, binding().binding_id);
            assert_eq!(dmap_ref.as_ref().unwrap().revision, binding().dmap_revision);
            assert_eq!(dforward_ref.as_ref().unwrap().revision, binding().dforward_revision);
        }
    }

    #[test]
    fn exact_dmap_ref_comes_from_binding() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let DMonitorSubjectV1::DIoTRealization { dmap_ref, .. } = &observation.subject else { panic!("wrong subject") };
        let dmap_ref = dmap_ref.as_ref().unwrap();
        assert_eq!(dmap_ref.id, "dmap:1");
        assert_eq!(dmap_ref.revision, 3);
        assert_eq!(dmap_ref.digest, format!("sha256:{}", "a".repeat(64)));
    }

    #[test]
    fn exact_dforward_ref_comes_from_binding() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let DMonitorSubjectV1::DIoTRealization { dforward_ref, .. } = &observation.subject else { panic!("wrong subject") };
        let dforward_ref = dforward_ref.as_ref().unwrap();
        assert_eq!(dforward_ref.id, "dforward:1");
        assert_eq!(dforward_ref.revision, 2);
        assert_eq!(dforward_ref.digest, format!("sha256:{}", "b".repeat(64)));
    }

    #[test]
    fn exact_runtime_binding_ref_comes_from_binding() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let DMonitorSubjectV1::DIoTRealization { runtime_binding_ref, .. } = &observation.subject else { panic!("wrong subject") };
        let runtime_binding_ref = runtime_binding_ref.as_ref().unwrap();
        assert_eq!(runtime_binding_ref.binding_id, binding().binding_id);
        assert_eq!(runtime_binding_ref.binding_revision, 1);
        assert_eq!(runtime_binding_ref.descriptor_digest, binding().descriptor_digest);
    }

    #[test]
    fn observation_id_is_deterministic_for_a_fixed_epoch_and_sequence() {
        let mut a = emitter();
        let mut b = emitter();
        let obs_a = a.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let obs_b = b.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        assert_eq!(obs_a.observation_id, obs_b.observation_id);
    }

    #[test]
    fn sequence_is_monotonic_across_repeated_emits() {
        let mut emitter = emitter();
        let first = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let second = emitter.emit("2026-09-25T00:00:10Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, true);
        assert_eq!(first.sequence, 1);
        assert_eq!(second.sequence, 2);
        assert_ne!(first.observation_id, second.observation_id);
    }

    #[test]
    fn new_listener_epoch_starts_sequence_at_one() {
        let context = DMonitorEmissionContext::new_process_epoch("fog-01");
        assert_eq!(context.next_sequence(), 1);
        let mut emitter = DiotRuntimeMonitorEmitter::new(context, binding());
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        assert_eq!(observation.sequence, 1);
    }

    #[test]
    fn no_broker_uri_or_secret_field_appears_anywhere_in_the_observation() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let json = serde_json::to_string(&observation).unwrap();
        for forbidden in ["broker_uri", "mqtt://", "password", "credential", "token"] {
            assert!(!json.to_lowercase().contains(forbidden), "observation leaked '{forbidden}': {json}");
        }
    }

    #[test]
    fn canonical_digest_recomputes_deterministically() {
        let mut emitter = emitter();
        let observation = emitter.emit("2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy, DiotRuntimeState::Subscribed, false);
        let a = crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(&observation).unwrap();
        let b = crate::agent::canonical_dmonitor::recompute_dmonitor_observation_digest(&observation).unwrap();
        assert_eq!(a, b);
    }
}
