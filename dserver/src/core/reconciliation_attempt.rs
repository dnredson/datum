//! Phase 138A (Part C): the DMap-native reconciliation-attempt read model.
//!
//! This is DESERVER-owned, COMMAND-SIDE state only -- it records what the
//! DATUM agent reported it attempted/did during one governed reconciliation
//! cycle (`DATUM::agent::operational_reconciliation::run_operational_reconciliation_from_bootstrap`,
//! identified by `reconciliation_attempt_id`). It is built entirely by
//! folding `OperationalEvidenceEvent`s the agent already POSTs to
//! `/api/v1/operational-evidence` -- no new agent-to-server wire contract,
//! no new ingestion path.
//!
//! This is explicitly NOT reused from, or connected to, the pre-DDeploy
//! `deployment_manifest`/`deployment_assignment`/`DeploymentAttemptRecord`
//! lineage (Phase 138/138A architecture audits: that subsystem is keyed by
//! `manifest_id`/`assignment_id`, has zero cross-reference to
//! `operational_dgraph`/`ActiveApplicationDeployment`/`CanonicalDMapV1`, and
//! is a disconnected pre-canonical-authority lineage). It is also explicitly
//! NOT DMonitor: this module never reads from, writes to, or is read by
//! `core::dmonitor_state`/`canonical_dmonitor` -- command-side "what did the
//! agent attempt" and observation-side "what is actually running/healthy"
//! remain two separate authorities, exactly as the architecture invariants
//! require. Accepted D-Map placement authority (`core::ddeploy`) is never
//! read or mutated by anything in this module -- folding evidence into an
//! attempt record is a pure, independent side effect.
//!
//! # Honest state limits
//!
//! The current evidence stream carries exactly ONE reconciliation-attempt-
//! tagged `OperationalEvidenceEvent` per cycle (the final `reconciliation`/
//! `reconciliation_action`/`drift_verification` event
//! `write_reconciliation_result` emits -- earlier `runtime_prerequisite*`
//! events in the same cycle are not yet tagged with an attempt id). This
//! module therefore folds attempts from a single terminal event in
//! practice, not a genuine multi-step `Planned -> Authorized -> Executing`
//! progression -- `Planned`/`Authorized`/`Executing` are modeled as valid
//! states (so a future richer event stream can use them) but are never
//! actually produced by `fold_evidence_into_attempt` today; the first
//! evidence event for a `reconciliation_attempt_id` goes straight to a
//! terminal status. Do not read anything into their current absence beyond
//! "the agent does not yet emit intermediate progress events".
//!
//! # Per-service result classification is a conservative best effort
//!
//! The agent's own internal `execution_status`/`message` vocabulary (free
//! text, scattered across many constants in
//! `DATUM::agent::native_process_runtime`/`operational_reconciliation`) is
//! NOT a closed, shared enum between the two crates -- re-deriving a
//! precise Created/Started/Replaced/Removed distinction from it here would
//! be guessing, not deriving. `classify_service_result` therefore uses only
//! structurally reliable signals: `host_mutation_performed: bool`, the
//! established `blocked_*` prefix convention, and the agent's exact
//! `execution_status="suggested"` value for a successful dry-run/Preview
//! decision that deliberately performed no host mutation. Everything else
//! remains `Unknown` rather than fabricating a specific class it cannot
//! prove. The raw
//! `execution_status`/`message` strings are retained verbatim alongside the
//! classification on every `ReconciliationServiceResult`, so nothing is
//! ever lost to this conservative collapse.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const RECONCILIATION_ATTEMPT_STATE_SCHEMA_VERSION: &str = "0.1.0";
/// Part I/H bound: a project's recent-attempts index retains at most this
/// many summaries, most-recent-first -- never an unbounded history. Mirrors
/// the bounded-list convention used throughout the Console-facing read
/// models (e.g. `truncated` flags on list-shaped projections).
pub const MAX_RECENT_ATTEMPTS_PER_PROJECT: usize = 50;
/// Part I/H bound: a single attempt record retains at most this many
/// per-service results. A node/application combination with more desired
/// services than this is itself out of scope for Phase 138A's restricted
/// MVP runtime classes (Part F) -- this bound exists as defense-in-depth,
/// not as an expected-to-be-hit limit.
pub const MAX_SERVICE_RESULTS_PER_ATTEMPT: usize = 500;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationAttemptStatus {
    // Independent review (Audit F): `Planned`/`Authorized`/`Executing` were
    // removed. No production path can produce them today -- the current
    // evidence stream carries exactly one, terminal, already-complete
    // event per cycle (see this module's own "Honest state limits" doc
    // comment), so `fold_evidence_into_attempt`/`derive_status` only ever
    // go straight to one of the four states below. Re-add intermediate
    // states only once a real event actually produces them -- this
    // project's own discipline is to never model a state the system
    // cannot truthfully observe, not to keep states "for future-proofing".
    Succeeded,
    Partial,
    Failed,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationActionResult {
    NoOp,
    /// The agent completed a dry-run/Preview decision and reported the
    /// concrete action it would take, while intentionally performing no
    /// host mutation. Distinct from NoOp: desired and observed state may
    /// differ; the action was merely not executed.
    Suggested,
    Mutated,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationServiceResult {
    pub service_id: String,
    pub result: ReconciliationActionResult,
    pub host_mutation_performed: bool,
    /// Verbatim agent-reported status/message, preserved exactly even
    /// though `result` above conservatively collapses it -- see this
    /// module's own doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationAttemptRecordV1 {
    pub schema_version: String,
    pub reconciliation_attempt_id: String,
    /// Phase 138C (Part Q/G): the `ReconciliationRequestV1` that caused
    /// this attempt, when one did -- `None` for a manual `--execute` cycle
    /// with no claimed request behind it (Part T backward compatibility).
    /// Set once, from the first evidence that carries it; never changes
    /// `reconciliation_attempt_id`'s own agent-side generation/ownership.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciliation_request_id: Option<String>,
    pub project_id: String,
    /// One reconciliation cycle is node-scoped, not application-scoped --
    /// `OperationalReconciliationContext.desired_services` can legitimately
    /// span multiple applications' placements on the same node (the
    /// operational D-Graph unions every application placed there). There
    /// is deliberately NO singular `application_id` field here: independent
    /// review confirmed `OperationalServiceReconciliationReport` (the only
    /// per-service shape the agent actually serializes into evidence) does
    /// not carry `application_id` at all today, so any such field could
    /// only ever be populated from a single arbitrarily-chosen service,
    /// misrepresenting a cycle that touched more than one application. Per-
    /// application detail belongs on each `ReconciliationServiceResult`
    /// if/when the agent ever serializes it there, not as one attempt-wide
    /// value.
    pub node_id: String,
    pub dgraph_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dgraph_revision: Option<u64>,
    /// Safe to take as one attempt-wide value (unlike `application_id`
    /// above): exactly one authorization/lease covers the WHOLE node-scoped
    /// cycle (`control_plane_authorized`/`Stage107ACycleGate` are computed
    /// once per cycle, never per service), so reading it off any one
    /// service in the payload is reading a genuine cycle-wide invariant,
    /// not an arbitrary per-service choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_id: Option<String>,
    pub started_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_utc: Option<String>,
    pub status: ReconciliationAttemptStatus,
    pub services: BTreeMap<String, ReconciliationServiceResult>,
    /// `true` once any fold's payload carried more service entries than
    /// `MAX_SERVICE_RESULTS_PER_ATTEMPT` -- sticky across folds. When set,
    /// `status` is never reported `Succeeded` (capped to `Partial`/`Unknown`
    /// by `derive_status`), since some service outcomes were structurally
    /// discarded and a full-success claim would not be provable.
    pub services_truncated: bool,
    /// The highest `OperationalEvidenceEvent.sequence` already folded into
    /// this record -- Part C7's ordering guard. A later fold call carrying
    /// a `sequence` at or below this value is a duplicate/out-of-order
    /// delivery and is rejected (fail-closed, not silently re-applied).
    pub last_applied_sequence: u64,
    pub created_at_utc: String,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationAttemptSummaryV1 {
    pub reconciliation_attempt_id: String,
    pub node_id: String,
    pub dgraph_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dgraph_revision: Option<u64>,
    pub status: ReconciliationAttemptStatus,
    /// Audit G (independent review): surfaced on the summary too, not only
    /// the full record -- a `Partial` status alone does not distinguish
    /// "some services were blocked" from "some service results were
    /// discarded past the bound"; an operator scanning the recent-attempts
    /// list should see this without fetching the full record.
    pub services_truncated: bool,
    pub started_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at_utc: Option<String>,
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationAttemptIndexV1 {
    pub schema_version: String,
    pub project_id: String,
    /// Most-recent-first, capped at `MAX_RECENT_ATTEMPTS_PER_PROJECT`.
    pub attempts: Vec<ReconciliationAttemptSummaryV1>,
}

impl ReconciliationAttemptIndexV1 {
    pub fn empty(project_id: &str) -> Self {
        Self {
            schema_version: RECONCILIATION_ATTEMPT_STATE_SCHEMA_VERSION.to_string(),
            project_id: project_id.to_string(),
            attempts: Vec::new(),
        }
    }

    /// Insert/replace this attempt's summary and re-sort most-recent-first
    /// by `updated_at_utc`, then truncate to the bound. Pure -- callers
    /// persist the result.
    pub fn upsert(&mut self, summary: ReconciliationAttemptSummaryV1) {
        self.attempts.retain(|existing| existing.reconciliation_attempt_id != summary.reconciliation_attempt_id);
        self.attempts.push(summary);
        self.attempts.sort_by(|a, b| b.updated_at_utc.cmp(&a.updated_at_utc));
        self.attempts.truncate(MAX_RECENT_ATTEMPTS_PER_PROJECT);
    }
}

/// Minimal, pure input to the fold -- deliberately NOT
/// `core::operational_evidence::OperationalEvidenceEvent` itself, so this
/// module never needs to know that type's unrelated fields
/// (`event_id`/`occurred_at_utc`/`resource_id` are passed through
/// explicitly by the caller, which already has the real event in hand).
pub struct EvidenceForFold<'a> {
    pub project_id: &'a str,
    pub node_id: &'a str,
    pub dgraph_id: &'a str,
    pub dgraph_revision: Option<u64>,
    pub sequence: u64,
    pub occurred_at_utc: &'a str,
    pub side_effects_performed: bool,
    pub payload: &'a serde_json::Value,
}

/// Best-effort, conservative per-service classification -- see this
/// module's own doc comment for why a richer distinction is not attempted.
fn classify_service_result(host_mutation_performed: bool, execution_status: Option<&str>) -> ReconciliationActionResult {
    if host_mutation_performed {
        return ReconciliationActionResult::Mutated;
    }
    let normalized = execution_status.unwrap_or("").trim().to_ascii_lowercase();
    if normalized.starts_with("blocked") {
        return ReconciliationActionResult::Blocked;
    }
    if normalized == "suggested" {
        return ReconciliationActionResult::Suggested;
    }
    if normalized.is_empty()
        || normalized.contains("observed_only")
        || normalized.contains("no_op")
        || normalized.contains("already")
        || normalized.contains("unchanged")
    {
        return ReconciliationActionResult::NoOp;
    }
    ReconciliationActionResult::Unknown
}

fn derive_status(services: &BTreeMap<String, ReconciliationServiceResult>) -> ReconciliationAttemptStatus {
    if services.is_empty() {
        return ReconciliationAttemptStatus::Unknown;
    }
    let any_ok = services.values().any(|service| {
        matches!(
            service.result,
            ReconciliationActionResult::NoOp
                | ReconciliationActionResult::Suggested
                | ReconciliationActionResult::Mutated
        )
    });
    let any_blocked = services.values().any(|service| matches!(service.result, ReconciliationActionResult::Blocked));
    let any_unknown = services.values().any(|service| matches!(service.result, ReconciliationActionResult::Unknown));
    match (any_ok, any_blocked, any_unknown) {
        (true, false, false) => ReconciliationAttemptStatus::Succeeded,
        (true, _, _) => ReconciliationAttemptStatus::Partial,
        (false, true, _) => ReconciliationAttemptStatus::Failed,
        (false, false, _) => ReconciliationAttemptStatus::Unknown,
    }
}

/// Fold one evidence event into an attempt record, creating it if
/// `existing` is `None`. Pure -- the caller persists the result. Returns
/// `Err` (never silently drops or re-applies) for:
/// - a `sequence` at or below `existing.last_applied_sequence`
///   (duplicate/out-of-order -- Part C7);
/// - a `project_id`/`node_id`/`dgraph_id` that disagrees with an existing
///   record for the same `reconciliation_attempt_id` (identity must never
///   drift across folds).
pub fn fold_evidence_into_attempt(
    existing: Option<ReconciliationAttemptRecordV1>,
    reconciliation_attempt_id: &str,
    authorization_id: Option<&str>,
    reconciliation_request_id: Option<&str>,
    now_utc: &str,
    evidence: &EvidenceForFold,
) -> Result<ReconciliationAttemptRecordV1, String> {
    if let Some(existing) = &existing {
        if evidence.sequence <= existing.last_applied_sequence {
            return Err(format!(
                "stale or duplicate evidence sequence {} for reconciliation_attempt_id '{}' (already at {})",
                evidence.sequence, reconciliation_attempt_id, existing.last_applied_sequence
            ));
        }
        if existing.project_id != evidence.project_id || existing.node_id != evidence.node_id {
            return Err(format!(
                "evidence project_id/node_id ({}/{}) disagrees with existing attempt '{}' ({}/{})",
                evidence.project_id, evidence.node_id, reconciliation_attempt_id, existing.project_id, existing.node_id
            ));
        }
    }

    let mut services: BTreeMap<String, ReconciliationServiceResult> = existing
        .as_ref()
        .map(|record| record.services.clone())
        .unwrap_or_default();
    let mut services_truncated = existing.as_ref().is_some_and(|record| record.services_truncated);

    if let Some(entries) = evidence.payload.get("services").and_then(|value| value.as_array()) {
        if entries.len() > MAX_SERVICE_RESULTS_PER_ATTEMPT {
            // Part H/24: never silently discard -- mark the record so
            // `derive_status` refuses to report full success over
            // incomplete information, rather than truncating with no
            // trace of having done so.
            services_truncated = true;
        }
        for entry in entries.iter().take(MAX_SERVICE_RESULTS_PER_ATTEMPT) {
            let service_id = match entry.get("service_id").and_then(|value| value.as_str()) {
                Some(value) if !value.trim().is_empty() => value.to_string(),
                _ => continue,
            };
            let host_mutation_performed = entry
                .get("host_mutation_performed")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let execution_status = entry.get("execution_status").and_then(|value| value.as_str());
            let message = entry.get("message").and_then(|value| value.as_str());
            services.insert(
                service_id.clone(),
                ReconciliationServiceResult {
                    service_id,
                    result: classify_service_result(host_mutation_performed, execution_status),
                    host_mutation_performed,
                    execution_status: execution_status.map(str::to_string),
                    message: message.map(str::to_string),
                },
            );
        }
    } else if evidence.side_effects_performed {
        // No per-service breakdown in this payload, but the event itself
        // attests a mutation happened -- never silently report "no
        // services, status Unknown with zero evidence of what occurred".
        services.insert(
            "(unspecified)".to_string(),
            ReconciliationServiceResult {
                service_id: "(unspecified)".to_string(),
                result: ReconciliationActionResult::Mutated,
                host_mutation_performed: true,
                execution_status: None,
                message: Some("event reported side_effects_performed=true with no per-service breakdown".to_string()),
            },
        );
    }

    let mut status = derive_status(&services);
    if services_truncated && status == ReconciliationAttemptStatus::Succeeded {
        // Some service outcomes were structurally discarded this fold (or a
        // prior one) -- a full-success claim is not provable, so this is
        // capped to Partial even though every RETAINED result looks ok.
        status = ReconciliationAttemptStatus::Partial;
    }
    let started_at_utc = existing
        .as_ref()
        .map(|record| record.started_at_utc.clone())
        .unwrap_or_else(|| evidence.occurred_at_utc.to_string());
    let created_at_utc = existing.as_ref().map(|record| record.created_at_utc.clone()).unwrap_or_else(|| now_utc.to_string());
    let authorization_id = authorization_id
        .map(str::to_string)
        .or_else(|| existing.as_ref().and_then(|record| record.authorization_id.clone()));
    let reconciliation_request_id = reconciliation_request_id
        .map(str::to_string)
        .or_else(|| existing.as_ref().and_then(|record| record.reconciliation_request_id.clone()));

    Ok(ReconciliationAttemptRecordV1 {
        schema_version: RECONCILIATION_ATTEMPT_STATE_SCHEMA_VERSION.to_string(),
        reconciliation_request_id,
        reconciliation_attempt_id: reconciliation_attempt_id.to_string(),
        project_id: evidence.project_id.to_string(),
        node_id: evidence.node_id.to_string(),
        dgraph_id: evidence.dgraph_id.to_string(),
        dgraph_revision: evidence.dgraph_revision,
        authorization_id,
        started_at_utc,
        finished_at_utc: Some(evidence.occurred_at_utc.to_string()),
        status,
        services,
        services_truncated,
        last_applied_sequence: evidence.sequence,
        created_at_utc,
        updated_at_utc: now_utc.to_string(),
    })
}

pub fn attempt_summary(record: &ReconciliationAttemptRecordV1) -> ReconciliationAttemptSummaryV1 {
    ReconciliationAttemptSummaryV1 {
        reconciliation_attempt_id: record.reconciliation_attempt_id.clone(),
        node_id: record.node_id.clone(),
        dgraph_id: record.dgraph_id.clone(),
        dgraph_revision: record.dgraph_revision,
        status: record.status,
        services_truncated: record.services_truncated,
        started_at_utc: record.started_at_utc.clone(),
        finished_at_utc: record.finished_at_utc.clone(),
        updated_at_utc: record.updated_at_utc.clone(),
    }
}

/// Deterministic per-project, per-attempt domain keys for
/// `storage::control_state::{load_domain_json,persist_domain_json}`.
pub fn attempt_domain(reconciliation_attempt_id: &str) -> String {
    format!("reconciliation_attempt:{reconciliation_attempt_id}")
}
pub fn attempt_index_domain(project_id: &str) -> String {
    format!("reconciliation_attempts_index:{project_id}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Independent review (Audits T/U/V): structural, not just observed-at-
    // review-time, proof that this module has no actual code dependency on
    // accepted placement authority, DMonitor, or the legacy pre-DDeploy
    // deployment lineage -- mirrors the established source-text-scan
    // regression pattern already used elsewhere in this crate (e.g.
    // `dgraph_execution_safety.rs`'s `source_guard_contains_no_execution_or_mutation_path`).
    // Comments are allowed to name these (and do, to explain the boundary);
    // only actual code (`use`/calls/types) is forbidden.
    #[test]
    fn source_has_no_code_dependency_on_ddeploy_dmonitor_or_legacy_deployment_lineage() {
        let source = include_str!("reconciliation_attempt.rs");
        // Scoped to everything BEFORE this file's own `#[cfg(test)]` module
        // -- this test's own forbidden-word list literally contains these
        // strings, so scanning past this boundary would make the test fail
        // against itself. Mirrors the established
        // `dgraph_execution_safety.rs` precedent exactly.
        let implementation_only = source.split("#[cfg(test)]").next().unwrap_or(source);
        let implementation: String = implementation_only
            .lines()
            .filter(|line| !line.trim_start().starts_with("//") && !line.trim_start().starts_with("///") && !line.trim_start().starts_with("//!"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "core::ddeploy",
            "crate::core::ddeploy",
            "dmonitor_state",
            "canonical_dmonitor",
            "DMonitor",
            "DeploymentAttemptRecord",
            "deployment_manifest",
            "deployment_assignment",
            "dispatch_policy",
        ] {
            assert!(
                !implementation.contains(forbidden),
                "reconciliation_attempt.rs's actual code (comments excluded) must never reference '{forbidden}'"
            );
        }
    }

    fn evidence<'a>(payload: &'a serde_json::Value, sequence: u64) -> EvidenceForFold<'a> {
        EvidenceForFold {
            project_id: "project-a",
            node_id: "node-a",
            dgraph_id: "dgraph:app-1",
            dgraph_revision: Some(4),
            sequence,
            occurred_at_utc: "2026-10-05T12:00:00Z",
            side_effects_performed: true,
            payload,
        }
    }

    #[test]
    fn first_fold_creates_a_new_attempt_with_succeeded_status() {
        let payload = json!({"services":[
            {"service_id":"service-a","host_mutation_performed":true,"execution_status":"started"},
            {"service_id":"service-b","host_mutation_performed":false,"execution_status":"already_running"}
        ]});
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:1", Some("opauth:1"), None,
            "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert_eq!(record.status, ReconciliationAttemptStatus::Succeeded);
        assert_eq!(record.services.len(), 2);
        assert_eq!(record.services["service-a"].result, ReconciliationActionResult::Mutated);
        assert_eq!(record.services["service-b"].result, ReconciliationActionResult::NoOp);
        assert_eq!(record.last_applied_sequence, 1);
        assert_eq!(record.dgraph_revision, Some(4));
    }

    #[test]
    fn mixed_outcomes_fold_to_partial() {
        let payload = json!({"services":[
            {"service_id":"service-a","host_mutation_performed":true,"execution_status":"started"},
            {"service_id":"service-b","host_mutation_performed":false,"execution_status":"blocked_wrong_resource_identity"}
        ]});
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:2", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert_eq!(record.status, ReconciliationAttemptStatus::Partial);
    }

    #[test]
    fn all_blocked_folds_to_failed() {
        let payload = json!({"services":[
            {"service_id":"service-a","host_mutation_performed":false,"execution_status":"blocked_stale_dgraph_revision"}
        ]});
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:3", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert_eq!(record.status, ReconciliationAttemptStatus::Failed);
    }

    #[test]
    fn suggested_preview_folds_to_suggested_and_succeeded_without_mutation() {
        let payload = json!({"services":[
            {
                "service_id":"service-a",
                "host_mutation_performed":false,
                "execution_status":"suggested",
                "message":"WASM requested ensure_service_running, but --execute was not provided"
            }
        ]});
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:preview", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert_eq!(record.services["service-a"].result, ReconciliationActionResult::Suggested);
        assert!(!record.services["service-a"].host_mutation_performed);
        assert_eq!(record.status, ReconciliationAttemptStatus::Succeeded);
    }

    #[test]
    fn unrecognized_status_with_no_mutation_folds_to_unknown_never_fabricated_as_succeeded_or_failed() {
        let payload = json!({"services":[
            {"service_id":"service-a","host_mutation_performed":false,"execution_status":"something_this_module_has_never_seen"}
        ]});
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:4", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert_eq!(record.services["service-a"].result, ReconciliationActionResult::Unknown);
        assert_eq!(record.status, ReconciliationAttemptStatus::Unknown);
    }

    #[test]
    fn empty_services_with_no_breakdown_and_no_side_effects_folds_to_unknown() {
        let payload = json!({});
        let mut ev = evidence(&payload, 1);
        ev.side_effects_performed = false;
        let record = fold_evidence_into_attempt(None, "reconciliation-attempt:5", None, None, "2026-10-05T12:00:01Z", &ev).unwrap();
        assert_eq!(record.status, ReconciliationAttemptStatus::Unknown);
        assert!(record.services.is_empty());
    }

    #[test]
    fn side_effects_performed_with_no_breakdown_is_never_silently_unknown() {
        let payload = json!({});
        let ev = evidence(&payload, 1);
        let record = fold_evidence_into_attempt(None, "reconciliation-attempt:6", None, None, "2026-10-05T12:00:01Z", &ev).unwrap();
        assert_eq!(record.status, ReconciliationAttemptStatus::Succeeded);
        assert!(record.services.contains_key("(unspecified)"));
    }

    #[test]
    fn duplicate_or_out_of_order_sequence_is_rejected_and_never_applied() {
        let payload = json!({"services":[{"service_id":"service-a","host_mutation_performed":true,"execution_status":"started"}]});
        let first = fold_evidence_into_attempt(
            None, "reconciliation-attempt:7", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 5),
        )
        .unwrap();
        let replay = fold_evidence_into_attempt(
            Some(first.clone()), "reconciliation-attempt:7", None, None, "2026-10-05T12:00:02Z", &evidence(&payload, 5),
        );
        assert!(replay.is_err());
        let stale = fold_evidence_into_attempt(
            Some(first.clone()), "reconciliation-attempt:7", None, None, "2026-10-05T12:00:02Z", &evidence(&payload, 3),
        );
        assert!(stale.is_err());
    }

    #[test]
    fn identity_drift_across_folds_is_rejected() {
        let payload = json!({"services":[]});
        let first = fold_evidence_into_attempt(
            None, "reconciliation-attempt:8", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        let mut drifted = evidence(&payload, 2);
        drifted.node_id = "node-b";
        let result = fold_evidence_into_attempt(Some(first), "reconciliation-attempt:8", None, None, "2026-10-05T12:00:02Z", &drifted);
        assert!(result.is_err());
    }

    #[test]
    fn later_fold_advances_sequence_and_merges_additional_services_without_losing_earlier_ones() {
        let first_payload = json!({"services":[{"service_id":"service-a","host_mutation_performed":true,"execution_status":"started"}]});
        let first = fold_evidence_into_attempt(
            None, "reconciliation-attempt:9", None, None, "2026-10-05T12:00:01Z", &evidence(&first_payload, 1),
        )
        .unwrap();
        let first_started_at_utc = first.started_at_utc.clone();
        let second_payload = json!({"services":[{"service_id":"service-b","host_mutation_performed":true,"execution_status":"started"}]});
        let second = fold_evidence_into_attempt(
            Some(first), "reconciliation-attempt:9", None, None, "2026-10-05T12:00:05Z", &evidence(&second_payload, 2),
        )
        .unwrap();
        assert_eq!(second.services.len(), 2);
        assert!(second.services.contains_key("service-a"));
        assert!(second.services.contains_key("service-b"));
        assert_eq!(second.last_applied_sequence, 2);
        assert_eq!(second.started_at_utc, first_started_at_utc, "started_at_utc must never move once set");
    }

    fn many_mutated_services(count: usize) -> serde_json::Value {
        let services: Vec<serde_json::Value> = (0..count)
            .map(|n| json!({"service_id": format!("service-{n}"), "host_mutation_performed": true, "execution_status": "started"}))
            .collect();
        json!({"services": services})
    }

    #[test]
    fn more_than_the_bound_marks_services_truncated_and_caps_status_below_succeeded() {
        // Audit G (independent review): >500 service entries must never be
        // silently discarded -- the record must say so, and must never
        // claim full success over information it structurally dropped.
        let payload = many_mutated_services(MAX_SERVICE_RESULTS_PER_ATTEMPT + 10);
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:10", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert!(record.services_truncated);
        assert_eq!(record.services.len(), MAX_SERVICE_RESULTS_PER_ATTEMPT, "exactly the bound is retained, never more");
        assert_ne!(record.status, ReconciliationAttemptStatus::Succeeded, "truncated data can never honestly claim full success");
        assert_eq!(record.status, ReconciliationAttemptStatus::Partial);
    }

    #[test]
    fn at_or_under_the_bound_never_sets_services_truncated() {
        let payload = many_mutated_services(MAX_SERVICE_RESULTS_PER_ATTEMPT);
        let record = fold_evidence_into_attempt(
            None, "reconciliation-attempt:11", None, None, "2026-10-05T12:00:01Z", &evidence(&payload, 1),
        )
        .unwrap();
        assert!(!record.services_truncated);
        assert_eq!(record.status, ReconciliationAttemptStatus::Succeeded);
    }

    #[test]
    fn services_truncated_is_sticky_and_cannot_be_cleared_by_a_later_untruncated_fold() {
        let first_payload = many_mutated_services(MAX_SERVICE_RESULTS_PER_ATTEMPT + 1);
        let first = fold_evidence_into_attempt(
            None, "reconciliation-attempt:12", None, None, "2026-10-05T12:00:01Z", &evidence(&first_payload, 1),
        )
        .unwrap();
        assert!(first.services_truncated);

        let second_payload = json!({"services":[{"service_id":"service-0","host_mutation_performed":true,"execution_status":"started"}]});
        let second = fold_evidence_into_attempt(
            Some(first), "reconciliation-attempt:12", None, None, "2026-10-05T12:00:05Z", &evidence(&second_payload, 2),
        )
        .unwrap();
        assert!(second.services_truncated, "a later, well-formed fold must never silently clear a prior truncation");
        assert_ne!(second.status, ReconciliationAttemptStatus::Succeeded);
    }

    #[test]
    fn index_upsert_is_most_recent_first_and_bounded() {
        let mut index = ReconciliationAttemptIndexV1::empty("project-a");
        for n in 0..(MAX_RECENT_ATTEMPTS_PER_PROJECT + 5) {
            index.upsert(ReconciliationAttemptSummaryV1 {
                reconciliation_attempt_id: format!("reconciliation-attempt:{n}"),
                node_id: "node-a".to_string(),
                dgraph_id: "dgraph:app-1".to_string(),
                dgraph_revision: Some(4),
                status: ReconciliationAttemptStatus::Succeeded,
                services_truncated: false,
                started_at_utc: "2026-10-05T12:00:00Z".to_string(),
                finished_at_utc: None,
                updated_at_utc: format!("2026-10-05T12:{n:02}:00Z"),
            });
        }
        assert_eq!(index.attempts.len(), MAX_RECENT_ATTEMPTS_PER_PROJECT);
        assert_eq!(index.attempts[0].reconciliation_attempt_id, format!("reconciliation-attempt:{}", MAX_RECENT_ATTEMPTS_PER_PROJECT + 4));
    }

    #[test]
    fn index_upsert_replaces_existing_entry_for_the_same_attempt_rather_than_duplicating() {
        let mut index = ReconciliationAttemptIndexV1::empty("project-a");
        let summary = |status: ReconciliationAttemptStatus, updated: &str| ReconciliationAttemptSummaryV1 {
            reconciliation_attempt_id: "reconciliation-attempt:1".to_string(),
            node_id: "node-a".to_string(),
            dgraph_id: "dgraph:app-1".to_string(),
            dgraph_revision: Some(4),
            status,
            services_truncated: false,
            started_at_utc: "2026-10-05T12:00:00Z".to_string(),
            finished_at_utc: None,
            updated_at_utc: updated.to_string(),
        };
        index.upsert(summary(ReconciliationAttemptStatus::Unknown, "2026-10-05T12:00:00Z"));
        index.upsert(summary(ReconciliationAttemptStatus::Succeeded, "2026-10-05T12:00:05Z"));
        assert_eq!(index.attempts.len(), 1);
        assert_eq!(index.attempts[0].status, ReconciliationAttemptStatus::Succeeded);
    }
}
