//! Governed D-Code invocation DMonitor evidence builder (Phase 115B, ADR-0024,
//! Phase 115A audit). Consumes a real, already-produced
//! [`DCodeInvocationEvidenceV1`] (`dcode::invoke`'s own governed execution
//! evidence) and produces one [`CanonicalDMonitorObservationV1`] with
//! `subject = DServRealization` per honest invocation, plus the caller's
//! locally-verified `this_node_id`.
//!
//! # Why `DServRealization`, not `DCodeExecution` (Phase 115A audit finding)
//!
//! `DMonitorSubjectV1::DCodeExecution` looks like the more literal fit for a
//! single invocation event, and `DCodeInvocationEvidenceV1` already carries
//! everything that subject needs. But `dserver::core::dmonitor_interpretation`
//! (Phase 114C/114D1's D-Serv convergence/readiness logic) only ever reads
//! `DServRealization` evidence -- `DCodeExecution` observations would be
//! constructed correctly but never consumed by the existing, already-correct
//! readiness governance. `DServRealization`'s own documented meaning ("a
//! D-Serv placement's observed realization... may correlate exactly with the
//! D-Map that placed it") is satisfied honestly by a successful invocation: it
//! does not claim continuous uptime, only "this D-Serv placement was realized
//! AT `observed_at_utc`" -- exactly like every other DMonitor observation
//! (freshness/staleness, unchanged, already governs how long that claim stays
//! trustworthy). No new persistence semantics are invented here.
//!
//! # Identity, never re-derived (Phase 115A: "no new authority")
//!
//! Every identity field comes directly from `evidence`, which itself comes
//! directly from a governed, server-issued `DCodeInvocationAuthorizationV1`
//! (`dcode::invoke::invoke_dcode_impl`) -- this module invents nothing and
//! never falls back to `evidence.dserv_artifact_id` (the D-Code/artifact
//! identity) for `subject.service_id` (the D-Serv placement identity): those
//! are deliberately distinct fields in `DCodeInvocationEvidenceV1` and this
//! module preserves that distinction (Phase 115A Q8/ADR-0009).
//!
//! # Fail-closed node-identity guard (Phase 115A Q9)
//!
//! `dserver::core::canonical_dmonitor`'s cross-check that a subject's own
//! `node_id` must equal the observation's top-level `node_id` is currently
//! scoped only to `DIoTRealization` (see that type's doc comment). This
//! module enforces the equivalent rule itself, at construction time, for
//! `DServRealization`: if `evidence.node_id` (== the authorization's
//! `node_id`) does not match the caller-verified `this_node_id`, NO
//! observation is built at all (`build_dserv_realization_from_dcode_evidence`
//! returns `None`). `dcode::invoke::invoke_dcode_impl` only produces this
//! mismatch via `DCodeInvocationOutcome::StaleAuthority` ("authorization
//! targets node X, not this node"), so this guard and that outcome are
//! equivalent in practice -- this module checks the identity fields directly
//! rather than relying on the outcome tag, so it stays correct even if a
//! future outcome variant is added.
//!
//! # Health mapping (Phase 115A: "use existing contracts, do not invent")
//!
//! Directly from `DCodeInvocationOutcome`, never from `output_status` (the
//! GUEST's own business-level "ok"/"rejected" verdict, which is a different,
//! narrower question the invocation evidence's own doc comments already
//! separate from infrastructure-level success):
//!
//! - `Success` -> `Healthy`: the D-Code instance executed correctly under
//!   ADR-0008's fresh-instance-per-invocation model, regardless of the
//!   guest's own business verdict (`invoke.rs`'s own comment: "a well-formed
//!   envelope... is a successful INFRASTRUCTURE-level invocation").
//! - `ValidationFailed` -> `Degraded`: the D-Serv is reachable but something
//!   is genuinely wrong (an artifact/authorization/output-envelope mismatch)
//!   -- weaker than a full runtime failure, but never silently `Healthy`.
//! - `ResourceLimit` | `Trap` -> `Unhealthy`: a definite runtime execution
//!   failure.
//! - `StaleAuthority` -> unreachable here in practice (filtered by the
//!   node-identity guard above); mapped to `Unhealthy` only so the match
//!   stays exhaustive and a future new `DCodeInvocationOutcome` variant fails
//!   to compile here rather than being silently absorbed by a wildcard.

use serde_json::json;

use crate::agent::canonical_dmonitor::{
    CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, DMonitorSubjectV1,
    CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
};
use crate::agent::dmonitor_emission::{observation_id, DMonitorEmissionContext};
use crate::dcode::evidence::{
    dcode_evidence_sha256, DCodeInvocationEvidenceV1, DCodeInvocationOutcome,
};

pub const DCODE_INVOKE_COLLECTOR: &str = "dcode_invoke";

fn health_for_outcome(outcome: &DCodeInvocationOutcome) -> DMonitorHealthV1 {
    match outcome {
        DCodeInvocationOutcome::Success => DMonitorHealthV1::Healthy,
        DCodeInvocationOutcome::ValidationFailed => DMonitorHealthV1::Degraded,
        DCodeInvocationOutcome::ResourceLimit | DCodeInvocationOutcome::Trap => {
            DMonitorHealthV1::Unhealthy
        }
        // Unreachable in practice: callers only reach this function after the
        // `evidence.node_id == this_node_id` guard, and `StaleAuthority` is
        // only ever produced by `dcode::invoke` on the opposite condition.
        // Mapped defensively rather than filtered by a wildcard so a future
        // new outcome variant does not silently fall through unclassified.
        DCodeInvocationOutcome::StaleAuthority => DMonitorHealthV1::Unhealthy,
    }
}

fn outcome_as_str(outcome: &DCodeInvocationOutcome) -> &'static str {
    match outcome {
        DCodeInvocationOutcome::Success => "success",
        DCodeInvocationOutcome::ValidationFailed => "validation_failed",
        DCodeInvocationOutcome::ResourceLimit => "resource_limit",
        DCodeInvocationOutcome::Trap => "trap",
        DCodeInvocationOutcome::StaleAuthority => "stale_authority",
    }
}

/// Builds one canonical `DServRealization` DMonitor observation from a real
/// `DCodeInvocationEvidenceV1`, or `None` if `evidence.node_id` does not
/// match `this_node_id` (fail-closed: never claim a local observation of a
/// realization this process did not itself execute -- Phase 115A Q9).
///
/// `sequence` must come from `context.allocate_sequence()` (called exactly
/// once by the caller, immediately before this), never invented
/// independently, mirroring `diot_runtime_dmonitor::build_diot_realization_observation`.
pub fn build_dserv_realization_from_dcode_evidence(
    context: &DMonitorEmissionContext,
    sequence: u64,
    this_node_id: &str,
    evidence: &DCodeInvocationEvidenceV1,
) -> Option<CanonicalDMonitorObservationV1> {
    if evidence.node_id != this_node_id {
        return None;
    }

    let health = health_for_outcome(&evidence.outcome);

    Some(CanonicalDMonitorObservationV1 {
        schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
        observation_id: observation_id(context.monitor_instance_id(), sequence),
        project_id: Some(evidence.project_id.clone()),
        node_id: this_node_id.to_string(),
        monitor_instance_id: context.monitor_instance_id().to_string(),
        sequence,
        observed_at_utc: evidence.received_at_utc.clone(),
        cycle_id: None,
        collector: DCODE_INVOKE_COLLECTOR.to_string(),
        subject: DMonitorSubjectV1::DServRealization {
            project_id: evidence.project_id.clone(),
            application_id: evidence.application_id.clone(),
            // The D-Serv PLACEMENT identity (`authorization.service_id`),
            // never `evidence.dserv_artifact_id` (the D-Code/artifact
            // identity) -- these are deliberately distinct fields (Phase
            // 115A Q8, ADR-0009).
            service_id: evidence.service_id.clone(),
            node_id: this_node_id.to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: evidence.dmap_id.clone(),
                revision: evidence.dmap_revision,
                digest: evidence.dmap_digest.clone(),
            }),
        },
        health,
        facts: json!({
            "outcome": outcome_as_str(&evidence.outcome),
            "duration_ms": evidence.duration_ms,
            "dcode_evidence_sha256": dcode_evidence_sha256(evidence),
        }),
    })
}

/// Convenience wrapper pairing a [`DMonitorEmissionContext`] with the
/// caller-verified local node identity, mirroring
/// `diot_runtime_dmonitor::DiotRuntimeMonitorEmitter`.
pub struct DCodeExecutionMonitorEmitter {
    context: DMonitorEmissionContext,
    this_node_id: String,
}

impl DCodeExecutionMonitorEmitter {
    pub fn new(context: DMonitorEmissionContext, this_node_id: String) -> Self {
        Self {
            context,
            this_node_id,
        }
    }

    pub fn monitor_instance_id(&self) -> &str {
        self.context.monitor_instance_id()
    }

    /// Allocates the next sequence and builds one observation in one step
    /// (or `None` if `evidence` is not this node's own realization) -- the
    /// only way a caller can obtain a sequence from this emitter, so a
    /// sequence is never allocated without a corresponding attempted
    /// observation. On `None`, the allocated sequence is simply left
    /// unpublished (a permitted gap, per `dmonitor_emission`'s own rule),
    /// never reused.
    pub fn emit(
        &mut self,
        evidence: &DCodeInvocationEvidenceV1,
    ) -> Option<CanonicalDMonitorObservationV1> {
        let sequence = self.context.allocate_sequence();
        build_dserv_realization_from_dcode_evidence(
            &self.context,
            sequence,
            &self.this_node_id,
            evidence,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dcode::evidence::{DCodeAppliedLimits, DCODE_EVIDENCE_SCHEMA};

    const NODE: &str = "fog-01";

    fn base_evidence() -> DCodeInvocationEvidenceV1 {
        DCodeInvocationEvidenceV1 {
            schema: DCODE_EVIDENCE_SCHEMA.to_string(),
            authorization_id: "dcode-auth:proj:lab:app:pipeline:decode:fog-01:1".to_string(),
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
            service_id: "decode".to_string(),
            node_id: NODE.to_string(),
            dmap_id: "dmap:1:1:app:pipeline".to_string(),
            dmap_revision: 3,
            dmap_digest: format!("sha256:{}", "a".repeat(64)),
            dgraph_id: "dgraph:pipeline".to_string(),
            dgraph_revision: 2,
            dgraph_digest: format!("sha256:{}", "b".repeat(64)),
            dserv_artifact_id: "dserv:pipeline:decode".to_string(),
            dserv_artifact_digest: "c".repeat(64),
            dserv_artifact_descriptor_digest: format!("sha256:{}", "d".repeat(64)),
            module_size_bytes: Some(128),
            dnode_abi: "datum-dnode/0".to_string(),
            fresh_instance_per_invocation: true,
            input_port: "in".to_string(),
            message_id: "msg-1".to_string(),
            correlation_id: "corr-1".to_string(),
            input_sha256: "e".repeat(64),
            input_size: 10,
            input_envelope_sha256: "f".repeat(64),
            input_envelope_size: 64,
            received_at_utc: "2026-09-25T10:00:00Z".to_string(),
            output_sha256: None,
            output_size: None,
            output_status: Some("ok".to_string()),
            output_reason_present: false,
            output_reason_sha256: None,
            output_reason_size: None,
            emits: Vec::new(),
            limits_applied: DCodeAppliedLimits {
                fuel_per_invocation: 1_000_000,
                timeout_ms: 100,
                max_memory_pages: 4,
                max_message_bytes: 65536,
            },
            outcome: DCodeInvocationOutcome::Success,
            error_code: None,
            error_message: None,
            error_offending_value_sha256: None,
            error_offending_value_size: None,
            duration_ms: 5,
        }
    }

    fn context() -> DMonitorEmissionContext {
        DMonitorEmissionContext::from_parts(
            "dmonitor-instance:fog-01:fixed-for-test".to_string(),
            1,
        )
    }

    #[test]
    fn success_outcome_is_healthy_dserv_realization_with_exact_identity() {
        let evidence = base_evidence();
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence)
                .expect("matching node identity must produce an observation");
        assert_eq!(observation.health, DMonitorHealthV1::Healthy);
        assert_eq!(observation.project_id, Some("proj:lab".to_string()));
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
                assert_eq!(project_id, "proj:lab");
                assert_eq!(application_id, "app:pipeline");
                // Must be the D-Serv PLACEMENT id, never the D-Code artifact id.
                assert_eq!(service_id, "decode");
                assert_ne!(service_id, evidence.dserv_artifact_id);
                assert_eq!(node_id, NODE);
                let dmap_ref = dmap_ref.expect("dmap_ref must be populated from governed evidence");
                assert_eq!(dmap_ref.id, "dmap:1:1:app:pipeline");
                assert_eq!(dmap_ref.revision, 3);
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    #[test]
    fn service_id_is_never_derived_from_the_dcode_artifact_name() {
        let mut evidence = base_evidence();
        // Even if an artifact id happens to look like it could double as a
        // service id, the producer must still read the DEDICATED service_id
        // field, never derive/parse it out of the artifact identity.
        evidence.dserv_artifact_id =
            "dserv:pipeline:decode-totally-different-looking-artifact-name".to_string();
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence).unwrap();
        match observation.subject {
            DMonitorSubjectV1::DServRealization { service_id, .. } => {
                assert_eq!(service_id, "decode")
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    #[test]
    fn validation_failed_outcome_is_degraded_never_healthy() {
        let mut evidence = base_evidence();
        evidence.outcome = DCodeInvocationOutcome::ValidationFailed;
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Degraded);
    }

    #[test]
    fn resource_limit_outcome_is_unhealthy() {
        let mut evidence = base_evidence();
        evidence.outcome = DCodeInvocationOutcome::ResourceLimit;
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    #[test]
    fn trap_outcome_is_unhealthy() {
        let mut evidence = base_evidence();
        evidence.outcome = DCodeInvocationOutcome::Trap;
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    /// Phase 115A Q9 / 115B.3: node identity mismatch fails closed -- no
    /// observation is built at all, rather than guessing which of two
    /// disagreeing node values to trust.
    #[test]
    fn node_identity_mismatch_fails_closed_with_no_observation() {
        let mut evidence = base_evidence();
        evidence.node_id = "cloud-01".to_string();
        evidence.outcome = DCodeInvocationOutcome::StaleAuthority;
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence);
        assert!(observation.is_none());
    }

    /// Application identity is taken verbatim from governed evidence; a
    /// caller cannot make this producer emit a mismatched application_id --
    /// there is no code path that reads anything other than
    /// `evidence.application_id`.
    #[test]
    fn application_identity_is_exactly_the_governed_evidence_value() {
        let mut evidence = base_evidence();
        evidence.application_id = "app:other-pipeline".to_string();
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence).unwrap();
        match observation.subject {
            DMonitorSubjectV1::DServRealization { application_id, .. } => {
                assert_eq!(application_id, "app:other-pipeline")
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    /// Phase 115A Q11: no new authority is invented -- the dmap_ref emitted
    /// is EXACTLY the one carried by the governed evidence, byte for byte,
    /// never independently recomputed or defaulted.
    #[test]
    fn dmap_ref_is_exactly_the_governed_evidence_reference_never_recomputed() {
        let mut evidence = base_evidence();
        evidence.dmap_id = "dmap:9:9:app:other".to_string();
        evidence.dmap_revision = 42;
        evidence.dmap_digest = format!("sha256:{}", "9".repeat(64));
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &evidence).unwrap();
        match observation.subject {
            DMonitorSubjectV1::DServRealization { dmap_ref, .. } => {
                let dmap_ref = dmap_ref.unwrap();
                assert_eq!(dmap_ref.id, "dmap:9:9:app:other");
                assert_eq!(dmap_ref.revision, 42);
                assert_eq!(dmap_ref.digest, format!("sha256:{}", "9".repeat(64)));
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    #[test]
    fn observation_never_carries_a_desired_state_key() {
        let observation =
            build_dserv_realization_from_dcode_evidence(&context(), 1, NODE, &base_evidence())
                .unwrap();
        assert!(observation.facts.get("desired_state").is_none());
    }

    #[test]
    fn sequence_and_observation_id_follow_the_shared_emission_context_rules() {
        let mut context = context();
        let evidence = base_evidence();
        let first_sequence = context.allocate_sequence();
        let first =
            build_dserv_realization_from_dcode_evidence(&context, first_sequence, NODE, &evidence)
                .unwrap();
        assert_eq!(
            first.observation_id,
            "dmonitor-obs:dmonitor-instance:fog-01:fixed-for-test:1"
        );
        let second_sequence = context.allocate_sequence();
        let second =
            build_dserv_realization_from_dcode_evidence(&context, second_sequence, NODE, &evidence)
                .unwrap();
        assert_eq!(second.sequence, 2);
        assert_ne!(first.observation_id, second.observation_id);
    }

    #[test]
    fn emitter_allocates_a_fresh_sequence_per_call_and_never_reuses_a_skipped_one() {
        let mut emitter = DCodeExecutionMonitorEmitter::new(context(), NODE.to_string());
        let mut evidence = base_evidence();
        let first = emitter.emit(&evidence).unwrap();
        assert_eq!(first.sequence, 1);

        // A wrong-node evidence record consumes a sequence but yields no
        // observation -- the NEXT real observation must not reuse it.
        evidence.node_id = "cloud-01".to_string();
        assert!(emitter.emit(&evidence).is_none());

        evidence.node_id = NODE.to_string();
        let third = emitter.emit(&evidence).unwrap();
        assert_eq!(
            third.sequence, 3,
            "sequence 2 must remain a permitted, unreused gap"
        );
    }
}
