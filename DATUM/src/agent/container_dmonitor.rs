//! Governed container D-Serv realization DMonitor evidence builder (Phase
//! 139B, ADR-0024), mirroring `native_process_dmonitor.rs`'s exact
//! architecture and discipline for the container runtime kind -- this is
//! the one confirmed gap the Phase 139A audit found: container workloads
//! already reconcile and already produce command-side
//! `OperationalEvidenceEvent`s, but emitted no independent, governed DMonitor
//! *observation* of their own. No convergence/interpretation code changes
//! anywhere else -- `dserver::core::dmonitor_interpretation`'s existing
//! `evaluate_application_dserv_convergence` already consumes ANY
//! `DServRealization` evidence identically regardless of which runtime kind
//! produced it.
//!
//! # Identity vs. observation (mirrors native_process_dmonitor's own split)
//!
//! `application_id`/`service_id`/`dmap_ref` are IDENTITY -- carried verbatim
//! from the governed `OperationalArtifactAssignment` the agent's own
//! reconciliation cycle resolved THIS cycle (Phase 139B's additive
//! `OperationalServiceReconciliationReport.application_id`/`.dmap_ref`
//! fields, themselves threaded unchanged from that same assignment, the
//! exact same way `artifact_id`/`execution_generation` already are). This
//! module NEVER infers identity from the container's own Docker labels --
//! Docker inspection proves runtime identity/state; the governed context
//! proves DATUM logical identity (Phase 139B Part A2). A container whose
//! OWN labels disagree with the expected identity is reported `Unhealthy`
//! under the EXPECTED (governed) identity, never silently adopted as if it
//! were a different, matching realization.
//!
//! # Fail-closed on missing identity (mirrors native_process_dmonitor exactly)
//!
//! If `application_id` is empty, `dmap_ref` is `None` (a legacy/pre-
//! canonical placement, or a contract-error report with no assignment at
//! all), or no trusted container name/execution-identity fields resolved
//! this cycle (not a container service, or no local mapping) --
//! [`build_dserv_realization_from_container_state`] returns `None`. No
//! invalid observation is ever constructed.
//!
//! # Health mapping (from a fresh `docker inspect`, never invented)
//!
//! - [`ContainerIdentityCheck::Confirmed`] (container exists, `Running`,
//!   labels match the expected execution identity, and Docker's own
//!   `HEALTHCHECK` -- if configured -- does not report `"unhealthy"`) ->
//!   `Healthy`.
//! - [`ContainerIdentityCheck::HealthcheckUnhealthy`] (everything above
//!   holds, but Docker's own `HEALTHCHECK` explicitly reports
//!   `"unhealthy"`) -> `Degraded`: the PLACEMENT is still exactly right,
//!   something inside the container is not -- this is the one place this
//!   module uses a value `native_process_dmonitor` never needs (native has
//!   no comparable in-process healthcheck concept).
//! - [`ContainerIdentityCheck::ContainerAbsent`] |
//!   [`ContainerIdentityCheck::NotRunning`] |
//!   [`ContainerIdentityCheck::LabelMismatch`] -> `Unhealthy`: a definite,
//!   disproven or disqualified realization -- mirrors
//!   `native_process_dmonitor`'s `ProcessAbsent`/`PidReused`/
//!   `ExecutableMismatch`/`ArgsMismatch` bucket exactly (several distinct
//!   disproof reasons, one health verdict).
//! - [`ContainerIdentityCheck::InspectUnavailable`] (the `docker` command
//!   itself failed, or returned JSON missing the fields needed to even
//!   determine `Running`) -> `Unknown`: honest uncertainty, mirrors
//!   `IdentityCheck::Unavailable` -- never coerced to `Healthy` or
//!   `Unhealthy`.
//!
//! # No mutation, no second Docker-inspect parser (Part A4)
//!
//! Identity/state uses `operational_reconciliation::inspect_container_full`
//! verbatim -- never a second inspect-JSON parser -- and reuses
//! `operational_reconciliation::inspected_container_labels_match`/
//! `ExecutionIdentityLabels` for the identical label comparison M1/M3
//! convergence already performs. Resource telemetry adds one independent,
//! best-effort, read-only `docker stats --no-stream` sample; failure to read
//! stats never changes health or gates reconciliation. No `docker run`/
//! `docker stop`/`docker rm`/pull or any mutating command is invoked here.
//!
//! # Preview mode (Part B3)
//!
//! Consistent with `native_process_dmonitor`'s own existing behavior:
//! observation is NOT gated on `execute_requested`/preview-vs-execute mode
//! at all -- `operational_service_loop.rs` already calls the native
//! emission loop unconditionally once a reconciliation cycle produces a
//! report, regardless of mode, and the container emission loop is wired
//! into that exact same unconditional spot. `docker inspect` is read-only,
//! so this is safe during preview by construction, not by a mode check.
//!
//! # Runtime-instance identity (Part A11)
//!
//! The container's own id/name is never added to the canonical
//! `DServRealization` subject (that type has no such field at all, by
//! design -- see `canonical_dmonitor::DMonitorSubjectV1`). Logical service
//! identity (`service_id`/`application_id`/`dmap_ref`) is what the subject
//! carries, and stays stable across a container recreate with a new
//! container id -- exactly mirroring how native's subject never carries a
//! PID either.

use std::process::Command;

use serde_json::json;

use crate::agent::canonical_dmonitor::{
    CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, DMonitorSubjectV1,
    CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
};
use crate::agent::dmonitor_emission::{observation_id, DMonitorEmissionContext};
use crate::agent::operational_reconciliation::{
    inspect_container_full, inspected_container_labels_match, ExecutionIdentityLabels,
    OperationalServiceReconciliationReport,
};

pub const CONTAINER_OBSERVE_COLLECTOR: &str = "container_observe";

/// Mirrors `native_process_state::IdentityCheck`'s own shape and discipline,
/// for the container runtime kind. See this module's own doc comment for
/// the exact health mapping each variant produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerIdentityCheck {
    Confirmed,
    HealthcheckUnhealthy,
    ContainerAbsent,
    NotRunning,
    LabelMismatch,
    InspectUnavailable,
}

fn health_for_identity(identity: ContainerIdentityCheck) -> DMonitorHealthV1 {
    match identity {
        ContainerIdentityCheck::Confirmed => DMonitorHealthV1::Healthy,
        ContainerIdentityCheck::HealthcheckUnhealthy => DMonitorHealthV1::Degraded,
        ContainerIdentityCheck::ContainerAbsent
        | ContainerIdentityCheck::NotRunning
        | ContainerIdentityCheck::LabelMismatch => DMonitorHealthV1::Unhealthy,
        ContainerIdentityCheck::InspectUnavailable => DMonitorHealthV1::Unknown,
    }
}

fn identity_as_str(identity: ContainerIdentityCheck) -> &'static str {
    match identity {
        ContainerIdentityCheck::Confirmed => "confirmed",
        ContainerIdentityCheck::HealthcheckUnhealthy => "healthcheck_unhealthy",
        ContainerIdentityCheck::ContainerAbsent => "container_absent",
        ContainerIdentityCheck::NotRunning => "not_running",
        ContainerIdentityCheck::LabelMismatch => "label_mismatch",
        ContainerIdentityCheck::InspectUnavailable => "inspect_unavailable",
    }
}

/// Pure classification of an ALREADY-FETCHED `docker inspect` result (or
/// `None`, for "no such container") against the expected execution
/// identity. No I/O -- directly unit-testable with handcrafted JSON,
/// exactly like `operational_reconciliation`'s own comparison functions
/// this reuses.
///
/// `Running` is checked for READABILITY before anything else: a
/// structurally insufficient inspect result (missing/wrongly-typed
/// `State.Running`) can never prove running OR stopped, so it is always
/// `InspectUnavailable` regardless of whether labels happen to match --
/// never a guessed `NotRunning`/`Confirmed`.
pub(crate) fn classify_inspected_container(
    inspected: Option<&serde_json::Value>,
    expected: &ExecutionIdentityLabels,
) -> ContainerIdentityCheck {
    let Some(inspected) = inspected else {
        return ContainerIdentityCheck::ContainerAbsent;
    };
    let running = inspected
        .get("State")
        .and_then(|state| state.get("Running"))
        .and_then(serde_json::Value::as_bool);
    let Some(running) = running else {
        return ContainerIdentityCheck::InspectUnavailable;
    };
    if !inspected_container_labels_match(inspected, expected) {
        return ContainerIdentityCheck::LabelMismatch;
    }
    if !running {
        return ContainerIdentityCheck::NotRunning;
    }
    let healthcheck_status = inspected
        .get("State")
        .and_then(|state| state.get("Health"))
        .and_then(|health| health.get("Status"))
        .and_then(serde_json::Value::as_str);
    if healthcheck_status == Some("unhealthy") {
        return ContainerIdentityCheck::HealthcheckUnhealthy;
    }
    ContainerIdentityCheck::Confirmed
}

/// Real I/O: one read-only `docker inspect` (`inspect_container_full`,
/// reused verbatim, never a second parser) plus the pure classification
/// above. `Err` from `inspect_container_full` (the `docker` command itself
/// could not be run, or its output could not be decoded as JSON at all) is
/// `InspectUnavailable` -- honest uncertainty, never silently treated as
/// "absent".
pub(crate) fn observe_container_identity(container_name: &str, expected: &ExecutionIdentityLabels) -> ContainerIdentityCheck {
    match inspect_container_full(container_name) {
        Ok(inspected) => classify_inspected_container(inspected.as_ref(), expected),
        Err(_) => ContainerIdentityCheck::InspectUnavailable,
    }
}

/// Read-only, best-effort runtime resource observation for a governed
/// container. This is telemetry only: inability to obtain stats never
/// changes placement/identity health and never blocks reconciliation.
#[derive(Debug, Clone, PartialEq)]
pub struct ContainerRuntimeUsage {
    pub cpu_percent: f64,
    pub memory_used_bytes: u64,
    pub memory_limit_bytes: u64,
    pub pids: u64,
}

fn parse_percent(value: &str) -> Option<f64> {
    let parsed = value.trim().trim_end_matches('%').trim().parse::<f64>().ok()?;
    parsed.is_finite().then_some(parsed)
}

fn parse_docker_bytes(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let split = value
        .find(|ch: char| !ch.is_ascii_digit() && ch != '.')
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split);
    let number = number.parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1_f64,
        "kb" => 1_000_f64,
        "mb" => 1_000_000_f64,
        "gb" => 1_000_000_000_f64,
        "tb" => 1_000_000_000_000_f64,
        "kib" => 1_024_f64,
        "mib" => 1_048_576_f64,
        "gib" => 1_073_741_824_f64,
        "tib" => 1_099_511_627_776_f64,
        _ => return None,
    };
    let bytes = number * multiplier;
    if !bytes.is_finite() || bytes > u64::MAX as f64 {
        return None;
    }
    Some(bytes.round() as u64)
}

fn parse_docker_stats_line(line: &str) -> Option<ContainerRuntimeUsage> {
    let mut fields = line.trim().split('\t');
    let cpu_percent = parse_percent(fields.next()?)?;
    let memory = fields.next()?.trim();
    let pids = fields.next()?.trim().parse::<u64>().ok()?;
    if fields.next().is_some() {
        return None;
    }
    let (used, limit) = memory.split_once('/')?;
    Some(ContainerRuntimeUsage {
        cpu_percent,
        memory_used_bytes: parse_docker_bytes(used)?,
        memory_limit_bytes: parse_docker_bytes(limit)?,
        pids,
    })
}

/// One read-only Docker stats sample. Failure is represented as None so
/// resource telemetry can never turn a confirmed realization unhealthy
/// merely because stats were temporarily unavailable.
pub(crate) fn observe_container_runtime_usage(container_name: &str) -> Option<ContainerRuntimeUsage> {
    let output = Command::new("docker")
        .args([
            "stats",
            "--no-stream",
            "--format",
            "{{.CPUPerc}}\t{{.MemUsage}}\t{{.PIDs}}",
            container_name,
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_docker_stats_line(stdout.lines().next()?)
}

/// Builds one canonical `DServRealization` DMonitor observation from a
/// governed per-cycle service report + an already-computed
/// [`ContainerIdentityCheck`], or `None` if `project_id`/`application_id`
/// are empty or `dmap_ref` is absent (fail-closed, never guessed -- mirrors
/// `native_process_dmonitor::build_dserv_realization_from_native_state`
/// exactly).
///
/// `sequence` must come from `context.allocate_sequence()`, called exactly
/// once by the caller immediately before this -- mirrors
/// `native_process_dmonitor`/`diot_runtime_dmonitor`/
/// `dcode_execution_dmonitor`'s own identical convention.
pub fn build_dserv_realization_from_container_state(
    context: &DMonitorEmissionContext,
    sequence: u64,
    this_node_id: &str,
    project_id: &str,
    observed_at_utc: &str,
    report: &OperationalServiceReconciliationReport,
    identity: ContainerIdentityCheck,
) -> Option<CanonicalDMonitorObservationV1> {
    if project_id.trim().is_empty() {
        return None;
    }
    let application_id = report.application_id.as_deref().filter(|value| !value.trim().is_empty())?;
    let dmap_ref = report.dmap_ref.as_ref()?;

    Some(CanonicalDMonitorObservationV1 {
        schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
        observation_id: observation_id(context.monitor_instance_id(), sequence),
        project_id: Some(project_id.to_string()),
        node_id: this_node_id.to_string(),
        monitor_instance_id: context.monitor_instance_id().to_string(),
        sequence,
        observed_at_utc: observed_at_utc.to_string(),
        cycle_id: None,
        collector: CONTAINER_OBSERVE_COLLECTOR.to_string(),
        subject: DMonitorSubjectV1::DServRealization {
            project_id: project_id.to_string(),
            application_id: application_id.to_string(),
            service_id: report.service_id.clone(),
            node_id: this_node_id.to_string(),
            dmap_ref: Some(DMonitorContractRefV1 {
                id: dmap_ref.id.clone(),
                revision: dmap_ref.revision,
                digest: dmap_ref.digest.clone(),
            }),
        },
        health: health_for_identity(identity),
        facts: json!({
            "container_identity_check": identity_as_str(identity),
        }),
    })
}

/// Thin wrapper around one shared [`DMonitorEmissionContext`] (one process
/// epoch, shared across every container service this loop observes --
/// never one context per service), mirroring
/// `NativeProcessMonitorEmitter` exactly. Each call independently resolves
/// whether `report` even names a governed container service with enough
/// identity to observe (a native-process service, a dispatch-error report,
/// or a container with no resolved local mapping all silently produce
/// `None` here, same as native's own emitter silently skips non-native
/// services) -- identity is supplied fresh each time from `report`, never
/// cached across calls.
pub struct ContainerMonitorEmitter {
    context: DMonitorEmissionContext,
}

impl ContainerMonitorEmitter {
    pub fn new(context: DMonitorEmissionContext) -> Self {
        Self { context }
    }

    pub fn monitor_instance_id(&self) -> &str {
        self.context.monitor_instance_id()
    }

    /// Allocates the next sequence and builds one observation for ONE
    /// service in one step (or `None`, per the same fail-closed/skip rules
    /// as `build_dserv_realization_from_container_state` and the missing-
    /// container-identity checks below) -- the only way a caller can obtain
    /// a sequence from this emitter, so a sequence is never allocated
    /// without a corresponding attempted observation. On `None`, the
    /// allocated sequence is simply left unpublished (a permitted gap,
    /// mirroring `NativeProcessMonitorEmitter::emit`'s own documented
    /// behavior), never reused.
    ///
    /// Independent of the cycle's own command-side `execution_status`
    /// (Part B2): a real, fresh `docker inspect` is always attempted
    /// whenever `report` names a container with enough identity to observe,
    /// regardless of whether this cycle's reconciliation attempt Succeeded,
    /// Failed, Partial, or did not run a mutation at all this cycle.
    pub fn emit(
        &mut self,
        this_node_id: &str,
        project_id: &str,
        observed_at_utc: &str,
        report: &OperationalServiceReconciliationReport,
    ) -> Option<CanonicalDMonitorObservationV1> {
        let sequence = self.context.allocate_sequence();
        let container_name = report.trusted_container_name.as_deref().filter(|value| !value.trim().is_empty())?;
        let artifact_id = report.artifact_id.as_deref().filter(|value| !value.trim().is_empty())?;
        let execution_generation = report.execution_generation?;
        let execution_projection_digest = report
            .expected_execution_projection_digest
            .as_deref()
            .filter(|value| !value.trim().is_empty())?;
        let expected = ExecutionIdentityLabels {
            artifact_id,
            execution_generation,
            execution_projection_digest,
            dmap_id: None,
            dmap_revision: None,
            dmap_digest: None,
        };
        let identity = observe_container_identity(container_name, &expected);
        let mut observation = build_dserv_realization_from_container_state(
            &self.context,
            sequence,
            this_node_id,
            project_id,
            observed_at_utc,
            report,
            identity,
        )?;
        if let Some(usage) = observe_container_runtime_usage(container_name) {
            observation.facts["resource_usage"] = json!({
                "cpu_percent": usage.cpu_percent,
                "memory_used_bytes": usage.memory_used_bytes,
                "memory_limit_bytes": usage.memory_limit_bytes,
                "pids": usage.pids,
            });
        }
        Some(observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::operational_reconciliation::no_wasm_decision;
    use crate::dcode::authorization::CanonicalContractReferenceV1;
    use serde_json::json;

    const NODE: &str = "fog-01";
    const PROJECT: &str = "proj:lab";

    fn context() -> DMonitorEmissionContext {
        DMonitorEmissionContext::from_parts("dmonitor-instance:fog-01:fixed-for-test".to_string(), 1)
    }

    fn report(container_name: &str) -> OperationalServiceReconciliationReport {
        OperationalServiceReconciliationReport {
            service_id: "decode".to_string(),
            artifact_id: Some("decode-artifact".to_string()),
            artifact_version: Some("1".to_string()),
            desired_state: "running".to_string(),
            trusted_container_name: Some(container_name.to_string()),
            trusted_image: Some("registry.example/decode:1".to_string()),
            observed_before_running: false,
            observation_error: None,
            secret_resolution_status: "not_checked".to_string(),
            wasm: no_wasm_decision("not_applicable"),
            execution_status: "deployed".to_string(),
            message: String::new(),
            observed_after_running: true,
            host_mutation_performed: false,
            execution_semantics: None,
            authorization_id: None,
            authorization_expires_at_utc: None,
            execution_generation: Some(1),
            expected_execution_projection_digest: Some(format!("sha256:{}", "a".repeat(64))),
            recomputed_execution_projection_digest: None,
            execution_pinning_mode: None,
            f2_status: None,
            local_policy_digest: None,
            unpinned_inputs: Vec::new(),
            authorization_clock_skew_seconds: None,
            verified_external_inputs: Vec::new(),
            local_substitutions: Vec::new(),
            mutation_boundaries_crossed: Vec::new(),
            verified_image_identity: None,
            application_id: Some("app:pipeline".to_string()),
            dmap_ref: Some(CanonicalContractReferenceV1 {
                id: "dmap:1".to_string(),
                revision: 3,
                digest: format!("sha256:{}", "b".repeat(64)),
            }),
        }
    }

    fn expected_labels(report: &OperationalServiceReconciliationReport) -> ExecutionIdentityLabels<'_> {
        ExecutionIdentityLabels {
            artifact_id: report.artifact_id.as_deref().unwrap(),
            execution_generation: report.execution_generation.unwrap(),
            execution_projection_digest: report.expected_execution_projection_digest.as_deref().unwrap(),
            dmap_id: None,
            dmap_revision: None,
            dmap_digest: None,
        }
    }

    fn inspected_running_with_matching_labels() -> serde_json::Value {
        json!({
            "State": {"Running": true, "Status": "running", "ExitCode": 0, "StartedAt": "2026-01-01T00:00:05Z"},
            "Config": {
                "Labels": {
                    "datum.artifact_id": "decode-artifact",
                    "datum.execution_generation": "1",
                    "datum.execution_projection_digest": format!("sha256:{}", "a".repeat(64)),
                }
            },
        })
    }

    #[test]
    fn docker_stats_parser_accepts_binary_memory_units() {
        let usage = parse_docker_stats_line("1.25%\t12.5MiB / 3.75GiB\t7").unwrap();
        assert_eq!(usage.cpu_percent, 1.25);
        assert_eq!(usage.memory_used_bytes, 13_107_200);
        assert_eq!(usage.memory_limit_bytes, 4_026_531_840);
        assert_eq!(usage.pids, 7);
    }

    #[test]
    fn docker_stats_parser_rejects_malformed_or_non_finite_values() {
        assert!(parse_docker_stats_line("nan%\t12MiB / 1GiB\t2").is_none());
        assert!(parse_docker_stats_line("1%\tbogus / 1GiB\t2").is_none());
        assert!(parse_docker_stats_line("1%\t12MiB / 1GiB\tnot-a-number").is_none());
    }
    // 1. exact governed container running -> valid DServRealization, Healthy.
    #[test]
    fn exact_governed_container_running_is_healthy_dserv_realization() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let identity = classify_inspected_container(Some(&inspected_running_with_matching_labels()), &expected);
        assert_eq!(identity, ContainerIdentityCheck::Confirmed);

        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, identity)
            .expect("full identity must produce an observation");
        assert_eq!(observation.health, DMonitorHealthV1::Healthy);
        assert!(observation.validate().is_empty(), "{:?}", observation.validate());
        match observation.subject {
            DMonitorSubjectV1::DServRealization { project_id, application_id, service_id, node_id, dmap_ref } => {
                assert_eq!(project_id, PROJECT);
                assert_eq!(application_id, "app:pipeline");
                assert_eq!(service_id, "decode");
                assert_eq!(node_id, NODE);
                assert_eq!(dmap_ref.unwrap().id, "dmap:1");
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    // 2. stopped container -> honest Unhealthy, never fabricated Healthy.
    #[test]
    fn stopped_container_is_unhealthy() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let mut inspected = inspected_running_with_matching_labels();
        inspected["State"]["Running"] = json!(false);
        let identity = classify_inspected_container(Some(&inspected), &expected);
        assert_eq!(identity, ContainerIdentityCheck::NotRunning);
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, identity).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    // 3. missing dmap_ref -> fail-closed, no observation at all.
    #[test]
    fn missing_dmap_ref_fails_closed_with_no_observation() {
        let mut r = report("decode-container");
        r.dmap_ref = None;
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed);
        assert!(observation.is_none());
    }

    #[test]
    fn missing_application_id_fails_closed_with_no_observation() {
        let mut r = report("decode-container");
        r.application_id = None;
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed);
        assert!(observation.is_none());
    }

    // 4. wrong labels -> never falsely Healthy (Unhealthy, same bucket as
    // ProcessAbsent/PidReused/ExecutableMismatch on the native side).
    #[test]
    fn wrong_labels_never_falsely_healthy() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let mut inspected = inspected_running_with_matching_labels();
        inspected["Config"]["Labels"]["datum.artifact_id"] = json!("some-other-artifact");
        let identity = classify_inspected_container(Some(&inspected), &expected);
        assert_eq!(identity, ContainerIdentityCheck::LabelMismatch);
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, identity).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
        // The SUBJECT's own dmap_ref is still the EXPECTED (governed) one --
        // never derived from the mismatched container's own labels (Part A2).
        match observation.subject {
            DMonitorSubjectV1::DServRealization { dmap_ref, .. } => assert_eq!(dmap_ref.unwrap().id, "dmap:1"),
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    // 6/7. Docker inspect unavailable (the `docker` command itself failed)
    // and malformed inspect (structurally insufficient JSON) both -> Unknown,
    // never fabricated Healthy or Unhealthy.
    #[test]
    fn docker_inspect_unavailable_is_unknown() {
        // `observe_container_identity` maps an `Err` from
        // `inspect_container_full` to `InspectUnavailable` directly; this
        // test exercises the pure classification half with a malformed (no
        // `State.Running`) document, which the real function would also
        // reach whenever `docker inspect`'s JSON lacks that field.
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let malformed = json!({"Config": {"Labels": {}}});
        let identity = classify_inspected_container(Some(&malformed), &expected);
        assert_eq!(identity, ContainerIdentityCheck::InspectUnavailable);
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, identity).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unknown);
    }

    #[test]
    fn container_absent_is_unhealthy_never_silently_skipped() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let identity = classify_inspected_container(None, &expected);
        assert_eq!(identity, ContainerIdentityCheck::ContainerAbsent);
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, identity).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Unhealthy);
    }

    // 8/9/10. Docker HEALTHCHECK, when configured, refines Healthy ->
    // Degraded (never full Unhealthy -- the PLACEMENT is still exactly
    // right); absent healthcheck is handled honestly as plain Healthy.
    #[test]
    fn healthcheck_healthy_or_absent_stays_healthy() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let mut inspected = inspected_running_with_matching_labels();
        inspected["State"]["Health"] = json!({"Status": "healthy"});
        assert_eq!(classify_inspected_container(Some(&inspected), &expected), ContainerIdentityCheck::Confirmed);

        let no_healthcheck = inspected_running_with_matching_labels();
        assert_eq!(classify_inspected_container(Some(&no_healthcheck), &expected), ContainerIdentityCheck::Confirmed);
    }

    #[test]
    fn healthcheck_unhealthy_degrades_without_denying_placement() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        let mut inspected = inspected_running_with_matching_labels();
        inspected["State"]["Health"] = json!({"Status": "unhealthy"});
        let identity = classify_inspected_container(Some(&inspected), &expected);
        assert_eq!(identity, ContainerIdentityCheck::HealthcheckUnhealthy);
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, identity).unwrap();
        assert_eq!(observation.health, DMonitorHealthV1::Degraded);
    }

    // 11. already covered inline above (`observation.validate().is_empty()`).

    // 12. project/application/service/node identity preserved exactly.
    #[test]
    fn identity_fields_preserved_exactly() {
        let r = report("decode-container");
        let observation = build_dserv_realization_from_container_state(&context(), 7, "cloud-02", "proj:other", "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed).unwrap();
        assert_eq!(observation.node_id, "cloud-02");
        assert_eq!(observation.project_id, Some("proj:other".to_string()));
        match observation.subject {
            DMonitorSubjectV1::DServRealization { project_id, node_id, service_id, .. } => {
                assert_eq!(project_id, "proj:other");
                assert_eq!(node_id, "cloud-02");
                assert_eq!(service_id, "decode");
            }
            other => panic!("expected DServRealization, got {other:?}"),
        }
    }

    // 13/14. No desired-state authority mutation, no Docker mutation command.
    // Identity still reuses inspect_container_full; the ONLY direct process
    // invocation allowed here is the observational docker-stats sample.
    #[test]
    fn source_allows_only_read_only_docker_stats_and_no_mutating_subcommand() {
        let source = include_str!("container_dmonitor.rs");
        let before_tests = source.split("#[cfg(test)]").next().unwrap_or(source);
        // Scoped to actual CODE (comments excluded) -- this module's own doc
        // comment legitimately names mutating subcommands in prose.
        let implementation_only: String = before_tests
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            implementation_only.matches("Command::new(\"docker\")").count(),
            1,
            "container monitoring may spawn only one direct docker command site"
        );
        assert!(
            implementation_only.contains("\"stats\"")
                && implementation_only.contains("\"--no-stream\""),
            "the one direct Docker invocation must be the read-only stats sample"
        );
        for forbidden in [
            "\"run\"", "\"stop\"", "\"rm\"", "\"start\"", "\"kill\"",
            "\"restart\"", "\"pull\"", "\"create\"", "docker run", "docker stop",
        ] {
            assert!(
                !implementation_only.contains(forbidden),
                "container_dmonitor.rs must never reference a mutating docker subcommand: '{forbidden}'"
            );
        }
    }

    // 15. restart/reinspection: a fresh inspect is re-evaluated each call,
    // never cached -- two different inspect results for the SAME expected
    // identity produce different, honest classifications.
    #[test]
    fn reinspection_is_never_cached_across_calls() {
        let r = report("decode-container");
        let expected = expected_labels(&r);
        assert_eq!(classify_inspected_container(Some(&inspected_running_with_matching_labels()), &expected), ContainerIdentityCheck::Confirmed);
        let mut stopped = inspected_running_with_matching_labels();
        stopped["State"]["Running"] = json!(false);
        assert_eq!(classify_inspected_container(Some(&stopped), &expected), ContainerIdentityCheck::NotRunning);
    }

    // 16. same logical service with a new container id remains the SAME
    // subject -- the subject never carries a container id at all, so a
    // restart/recreate (represented here only by a different inspected
    // state, since the canonical subject has no instance-id field to even
    // vary) never changes subject identity.
    #[test]
    fn restart_with_a_new_container_id_never_changes_logical_subject_identity() {
        let r = report("decode-container");
        let before = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed).unwrap();
        // A recreate gets a brand-new Docker container id, but `report` (the
        // governed per-cycle context) and therefore the observation's own
        // subject are keyed purely on logical identity -- re-running the
        // SAME builder against the SAME `report` after a hypothetical
        // recreate (a new container id the subject never even sees) must
        // produce byte-identical subject identity.
        let after = build_dserv_realization_from_container_state(&context(), 2, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed).unwrap();
        assert_eq!(before.subject, after.subject);
    }

    #[test]
    fn no_container_name_this_cycle_is_silently_skipped_never_an_error() {
        let mut r = report("decode-container");
        r.trusted_container_name = None;
        let mut emitter = ContainerMonitorEmitter::new(context());
        assert!(emitter.emit(NODE, PROJECT, "2026-01-01T00:00:05Z", &r).is_none());
    }

    #[test]
    fn empty_project_id_fails_closed_with_no_observation() {
        let r = report("decode-container");
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, "", "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed);
        assert!(observation.is_none());
    }

    #[test]
    fn observation_never_carries_a_desired_state_key() {
        let r = report("decode-container");
        let observation = build_dserv_realization_from_container_state(&context(), 1, NODE, PROJECT, "2026-01-01T00:00:05Z", &r, ContainerIdentityCheck::Confirmed).unwrap();
        assert!(observation.facts.get("desired_state").is_none());
    }

    // Phase 139 closure review (Audit E) -- a mixed node (one native-process
    // service, one container service) must produce independent canonical
    // observations, never cross-emit or double-count. `observe_current_native_state`
    // (native_process_runtime.rs) depends on a persisted native runtime-state
    // FILE keyed by (project_id, node_id, service_id) that nothing ever
    // writes for a container service -- so the complementary native-side
    // skip is structurally guaranteed by that precondition, not duplicated
    // here. This test proves the container side's own half of that
    // independence directly: in ONE emitter (ONE cycle), a native-shaped
    // report (no `trusted_container_name`, as every native service's report
    // actually looks) is silently skipped while a container-shaped report in
    // the SAME cycle is still correctly emitted, with its own independently
    // allocated sequence (the skipped native-shaped report consumes a
    // sequence too -- a permitted gap, never reused, matching this emitter's
    // own documented contract).
    #[test]
    fn mixed_node_skips_native_shaped_report_and_still_emits_for_the_container_shaped_one() {
        let mut native_shaped = report("irrelevant");
        native_shaped.service_id = "native-service".to_string();
        native_shaped.trusted_container_name = None;
        let container_shaped = report("decode-container");

        let mut emitter = ContainerMonitorEmitter::new(context());
        let native_result = emitter.emit(NODE, PROJECT, "2026-01-01T00:00:05Z", &native_shaped);
        let container_result = emitter.emit(NODE, PROJECT, "2026-01-01T00:00:05Z", &container_shaped);

        assert!(native_result.is_none(), "a native-shaped report must never be emitted by the container producer");
        let observation = container_result.expect("the container-shaped report in the same cycle must still be emitted");
        match observation.subject {
            DMonitorSubjectV1::DServRealization { service_id, .. } => assert_eq!(service_id, "decode"),
            other => panic!("expected DServRealization, got {other:?}"),
        }
        // Sequence 1 was allocated (and left unpublished) for the skipped
        // native-shaped report; the container-shaped one correctly got the
        // NEXT sequence, never reusing 1.
        assert_eq!(observation.sequence, 2);
    }
}
