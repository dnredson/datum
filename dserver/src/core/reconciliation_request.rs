//! Phase 138C: the control-plane trigger foundation for pull-based
//! reconciliation. A `ReconciliationRequestV1` is how an operator (via
//! Console, eventually) or any future non-human requester (a policy
//! engine, a failure-recovery loop) asks that reconciliation occur for one
//! node's operational D-Graph, WITHOUT `dserver` ever executing anything
//! itself. The node-resident DATUM agent remains the only thing that ever
//! touches a host; this module only persists and validates the REQUEST
//! and its CLAIM, never a mutation.
//!
//! # Six distinct objects -- never conflated
//!
//! - **Accepted authority** (`core::ddeploy`): what runtime SHOULD exist.
//!   Unmodified and unread-for-mutation by this module.
//! - **`ReconciliationRequestV1`** (this module): someone asks for
//!   reconciliation to occur, pinned to an exact operational D-Graph
//!   target.
//! - **`OperationalReconciliationAuthorization`** (`core::operational_dgraph`):
//!   permission to mutate a host for an exact scope. Claiming a request
//!   does NOT grant this -- it must independently exist and still be valid
//!   at attempt time (Phase 138A's unchanged, conjunctive gate).
//! - **`ReconciliationAttemptRecordV1`** (`core::reconciliation_attempt`):
//!   the agent actually began one reconciliation cycle. Linked to, never
//!   merged with, the request that caused it.
//! - **`OperationalEvidenceEvent`**: command-side occurrence/result
//!   evidence, unchanged.
//! - **DMonitor**: runtime observation authority, untouched by any of the
//!   above.
//!
//! # Scope: node, not application; operational D-Graph, not D-Map
//!
//! A request is `project_id` + `node_id` + `target_dgraph_id`/
//! `target_dgraph_revision` (the operational D-Graph identity, the SAME
//! identity Phase 138A's own live-revision mutation guard pins against) --
//! deliberately no singular `application_id` and no singular
//! `accepted_dmap_ref`, because one node's operational D-Graph can span
//! multiple applications' accepted placements (confirmed in the Phase
//! 138A/138B audits). Multi-application, multi-node "reconcile this
//! application" is expressed as a BATCH of node-scoped requests sharing one
//! `reconciliation_batch_id`, resolved server-side (`api::reconciliation_request::create_batch`)
//! -- never a single request naming an application.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub const RECONCILIATION_REQUEST_SCHEMA_VERSION: &str = "0.1.0";

pub const REQUEST_MODE_PREVIEW: &str = "preview";
pub const REQUEST_MODE_EXECUTE: &str = "execute";

/// Part R bounds -- mirrors Phase 138A's own `MAX_RECENT_ATTEMPTS_PER_PROJECT`
/// order of magnitude and bounded-list discipline. No unbounded queue.
pub const MAX_PENDING_REQUESTS_PER_NODE: usize = 20;
pub const MAX_RECENT_REQUESTS_PER_PROJECT: usize = 100;
pub const MAX_NODES_PER_BATCH: usize = 50;
/// A request an agent never claims is not left Pending forever.
pub const DEFAULT_REQUEST_TTL_SECONDS: i64 = 900;
/// A claim an agent never attempts against (crash between claim and
/// attempt) releases back to Pending after this bound -- a small multiple
/// of a realistic poll cycle, never so short a slow-but-alive agent loses
/// its own claim mid-attempt.
pub const DEFAULT_CLAIM_LEASE_SECONDS: i64 = 120;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationRequestStatus {
    Pending,
    Claimed,
    Attempted,
    Stale,
    Expired,
}

impl ReconciliationRequestStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Claimed => "claimed",
            Self::Attempted => "attempted",
            Self::Stale => "stale",
            Self::Expired => "expired",
        }
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "pending" => Ok(Self::Pending),
            "claimed" => Ok(Self::Claimed),
            "attempted" => Ok(Self::Attempted),
            "stale" => Ok(Self::Stale),
            "expired" => Ok(Self::Expired),
            other => Err(format!("unknown reconciliation_request status '{other}'")),
        }
    }

    /// Only a request in one of these PERSISTED states may ever be
    /// returned to an agent as actionable (Part E2) or successfully
    /// claimed (Part D). `Attempted`/`Stale`/`Expired` are all terminal
    /// from the agent's perspective -- a request never un-terminates.
    pub fn is_actionable(self) -> bool {
        matches!(self, Self::Pending)
    }
}

pub fn validate_mode(mode: &str) -> Result<(), String> {
    if mode == REQUEST_MODE_PREVIEW || mode == REQUEST_MODE_EXECUTE {
        Ok(())
    } else {
        Err(format!(
            "unsupported reconciliation request mode '{mode}'; expected '{REQUEST_MODE_PREVIEW}' or '{REQUEST_MODE_EXECUTE}'"
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationRequestV1 {
    pub schema_version: String,
    pub reconciliation_request_id: String,
    pub client_request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reconciliation_batch_id: Option<String>,
    pub project_id: String,
    pub node_id: String,
    pub target_dgraph_id: String,
    pub target_dgraph_revision: u64,
    pub mode: String,
    /// Audit label only -- never authenticated identity (ADR-0013 §6,
    /// same discipline as `accepted_by`/Phase 138A's `requested_by`
    /// discussion). Never treat this as authorization.
    pub requested_by: String,
    pub created_at_utc: String,
    pub expires_at_utc: String,
    /// The PERSISTED lifecycle state -- callers needing the TRUE current
    /// state (accounting for request/claim TTL having lapsed without an
    /// explicit write) must call `effective_status`, never read this
    /// field directly for an actionability decision.
    pub status: ReconciliationRequestStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_expires_at_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_attempt_id: Option<String>,
    pub updated_at_utc: String,
}

/// Lazy, pure derivation of the TRUE current status (Phase 138A's own
/// `should_restore_authorization`/`LeaseGuard` precedent: expiry is
/// evaluated fresh against `now`, never baked into a value that can go
/// stale between writes). Never mutates the stored row -- callers that
/// need the persisted `status` column to reflect this transition do so
/// explicitly (`storage::reconciliation_request` lazily corrects it on
/// the next write path that touches the row).
pub fn effective_status(request: &ReconciliationRequestV1, now: DateTime<Utc>) -> ReconciliationRequestStatus {
    match request.status {
        ReconciliationRequestStatus::Attempted | ReconciliationRequestStatus::Stale => request.status,
        ReconciliationRequestStatus::Expired => ReconciliationRequestStatus::Expired,
        ReconciliationRequestStatus::Pending => {
            if parse_rfc3339(&request.expires_at_utc).is_some_and(|expires_at| now >= expires_at) {
                ReconciliationRequestStatus::Expired
            } else {
                ReconciliationRequestStatus::Pending
            }
        }
        ReconciliationRequestStatus::Claimed => {
            let claim_expired = request
                .claim_expires_at_utc
                .as_deref()
                .and_then(parse_rfc3339)
                .is_none_or(|claim_expires_at| now >= claim_expires_at);
            if claim_expired {
                // A crashed/never-attempting claimant releases the request
                // back to Pending (Part M), UNLESS the request's own TTL
                // has also lapsed in the meantime, in which case Expired
                // takes precedence -- a request is never actionable again
                // merely because its stale claim expired.
                if parse_rfc3339(&request.expires_at_utc).is_some_and(|expires_at| now >= expires_at) {
                    ReconciliationRequestStatus::Expired
                } else {
                    ReconciliationRequestStatus::Pending
                }
            } else {
                ReconciliationRequestStatus::Claimed
            }
        }
    }
}

fn parse_rfc3339(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim()).ok().map(|parsed| parsed.with_timezone(&Utc))
}

pub fn default_expires_at(now: DateTime<Utc>) -> String {
    (now + Duration::seconds(DEFAULT_REQUEST_TTL_SECONDS)).to_rfc3339()
}

pub fn default_claim_expires_at(now: DateTime<Utc>) -> String {
    (now + Duration::seconds(DEFAULT_CLAIM_LEASE_SECONDS)).to_rfc3339()
}

/// Part E: the MINIMAL shape threaded into the existing control-slice/head
/// response an agent already polls every cycle (Part E1: no new
/// steady-state poll). Deliberately carries only what the agent needs to
/// decide whether to call the claim endpoint -- never the full request
/// body (requested_by, timestamps, etc. stay server-side until read via
/// the explicit request-read API).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingRequestSignal {
    pub reconciliation_request_id: String,
    pub mode: String,
    pub target_dgraph_id: String,
    pub target_dgraph_revision: u64,
}

pub fn pending_request_signal(request: &ReconciliationRequestV1) -> PendingRequestSignal {
    PendingRequestSignal {
        reconciliation_request_id: request.reconciliation_request_id.clone(),
        mode: request.mode.clone(),
        target_dgraph_id: request.target_dgraph_id.clone(),
        target_dgraph_revision: request.target_dgraph_revision,
    }
}

/// Part E2: deterministic, oldest-valid-first selection among candidates
/// already filtered to `effective_status(..) == Pending` by the caller.
/// Pure -- the caller (storage layer) is responsible for only ever passing
/// already-filtered, already-live-target-revalidated candidates.
pub fn select_next_pending<'a>(candidates: &'a [ReconciliationRequestV1]) -> Option<&'a ReconciliationRequestV1> {
    candidates.iter().min_by(|a, b| a.created_at_utc.cmp(&b.created_at_utc))
}

/// Part I2: the minimal, honestly-derivable batch aggregate. No rollback
/// semantics, no state the members themselves cannot prove.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationBatchStatus {
    Pending,
    Executing,
    Succeeded,
    Partial,
    Failed,
    Unknown,
    Stale,
}

/// Derives the batch-level status from member requests' own
/// `effective_status` plus (when `Attempted`) their linked attempt's
/// status, which the caller supplies via `attempt_statuses` (keyed by
/// `reconciliation_attempt_id`) -- this module never reaches into
/// `core::reconciliation_attempt` itself, keeping the two modules
/// independently testable and avoiding a circular dependency.
pub fn derive_batch_status(
    members: &[ReconciliationRequestV1],
    attempt_statuses: &BTreeMap<String, crate::core::reconciliation_attempt::ReconciliationAttemptStatus>,
    now: DateTime<Utc>,
) -> ReconciliationBatchStatus {
    use crate::core::reconciliation_attempt::ReconciliationAttemptStatus as AttemptStatus;

    if members.is_empty() {
        return ReconciliationBatchStatus::Unknown;
    }

    let mut any_succeeded = false;
    let mut any_failed_or_blocked = false;
    let mut any_unknown = false;
    let mut any_in_flight = false;
    let mut any_stale = false;

    for member in members {
        match effective_status(member, now) {
            ReconciliationRequestStatus::Pending | ReconciliationRequestStatus::Claimed => any_in_flight = true,
            ReconciliationRequestStatus::Stale => any_stale = true,
            ReconciliationRequestStatus::Expired => any_unknown = true,
            ReconciliationRequestStatus::Attempted => {
                let attempt_status = member
                    .linked_attempt_id
                    .as_deref()
                    .and_then(|id| attempt_statuses.get(id));
                match attempt_status {
                    Some(AttemptStatus::Succeeded) => any_succeeded = true,
                    Some(AttemptStatus::Partial) => {
                        any_succeeded = true;
                        any_failed_or_blocked = true;
                    }
                    Some(AttemptStatus::Failed) => any_failed_or_blocked = true,
                    Some(AttemptStatus::Unknown) | None => any_unknown = true,
                }
            }
        }
    }

    if any_in_flight {
        return ReconciliationBatchStatus::Executing;
    }
    if any_stale {
        return ReconciliationBatchStatus::Stale;
    }
    match (any_succeeded, any_failed_or_blocked, any_unknown) {
        (true, false, false) => ReconciliationBatchStatus::Succeeded,
        (true, true, _) | (true, false, true) => ReconciliationBatchStatus::Partial,
        (false, true, _) => ReconciliationBatchStatus::Failed,
        (false, false, _) => ReconciliationBatchStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::reconciliation_attempt::ReconciliationAttemptStatus as AttemptStatus;

    fn request(status: ReconciliationRequestStatus) -> ReconciliationRequestV1 {
        ReconciliationRequestV1 {
            schema_version: RECONCILIATION_REQUEST_SCHEMA_VERSION.to_string(),
            reconciliation_request_id: "reconciliation-request:1".to_string(),
            client_request_id: "client:1".to_string(),
            reconciliation_batch_id: None,
            project_id: "project-a".to_string(),
            node_id: "node-a".to_string(),
            target_dgraph_id: "dgraph:app-1".to_string(),
            target_dgraph_revision: 4,
            mode: REQUEST_MODE_EXECUTE.to_string(),
            requested_by: "operator".to_string(),
            created_at_utc: "2026-10-05T12:00:00Z".to_string(),
            expires_at_utc: "2026-10-05T12:15:00Z".to_string(),
            status,
            claim_token: None,
            claimed_at_utc: None,
            claim_expires_at_utc: None,
            linked_attempt_id: None,
            updated_at_utc: "2026-10-05T12:00:00Z".to_string(),
        }
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn pending_becomes_expired_once_its_own_ttl_lapses() {
        let r = request(ReconciliationRequestStatus::Pending);
        assert_eq!(effective_status(&r, at("2026-10-05T12:10:00Z")), ReconciliationRequestStatus::Pending);
        assert_eq!(effective_status(&r, at("2026-10-05T12:15:00Z")), ReconciliationRequestStatus::Expired);
        assert_eq!(effective_status(&r, at("2026-10-05T13:00:00Z")), ReconciliationRequestStatus::Expired);
    }

    #[test]
    fn claimed_reverts_to_pending_once_the_claim_lease_lapses_if_request_itself_still_valid() {
        let mut r = request(ReconciliationRequestStatus::Claimed);
        r.claim_token = Some("token-1".to_string());
        r.claimed_at_utc = Some("2026-10-05T12:00:00Z".to_string());
        r.claim_expires_at_utc = Some("2026-10-05T12:02:00Z".to_string());
        assert_eq!(effective_status(&r, at("2026-10-05T12:01:00Z")), ReconciliationRequestStatus::Claimed);
        assert_eq!(effective_status(&r, at("2026-10-05T12:03:00Z")), ReconciliationRequestStatus::Pending, "a crashed claimant must release the request, never leave it stuck Claimed forever");
    }

    #[test]
    fn claimed_becomes_expired_not_pending_once_claim_lapses_past_the_requests_own_ttl() {
        let mut r = request(ReconciliationRequestStatus::Claimed);
        r.claim_token = Some("token-1".to_string());
        r.claimed_at_utc = Some("2026-10-05T12:00:00Z".to_string());
        r.claim_expires_at_utc = Some("2026-10-05T12:02:00Z".to_string());
        assert_eq!(effective_status(&r, at("2026-10-05T13:00:00Z")), ReconciliationRequestStatus::Expired);
    }

    #[test]
    fn terminal_states_never_change() {
        for status in [ReconciliationRequestStatus::Attempted, ReconciliationRequestStatus::Stale, ReconciliationRequestStatus::Expired] {
            let r = request(status);
            assert_eq!(effective_status(&r, at("2099-01-01T00:00:00Z")), status);
        }
    }

    #[test]
    fn select_next_pending_is_oldest_first() {
        let mut older = request(ReconciliationRequestStatus::Pending);
        older.reconciliation_request_id = "reconciliation-request:older".to_string();
        older.created_at_utc = "2026-10-05T11:00:00Z".to_string();
        let mut newer = request(ReconciliationRequestStatus::Pending);
        newer.reconciliation_request_id = "reconciliation-request:newer".to_string();
        newer.created_at_utc = "2026-10-05T12:00:00Z".to_string();
        let candidates = [newer, older];
        let chosen = select_next_pending(&candidates).unwrap();
        assert_eq!(chosen.reconciliation_request_id, "reconciliation-request:older");
    }

    #[test]
    fn select_next_pending_is_none_for_empty_candidates() {
        assert!(select_next_pending(&[]).is_none());
    }

    #[test]
    fn validate_mode_accepts_exactly_preview_and_execute() {
        assert!(validate_mode(REQUEST_MODE_PREVIEW).is_ok());
        assert!(validate_mode(REQUEST_MODE_EXECUTE).is_ok());
        assert!(validate_mode("deploy").is_err());
        assert!(validate_mode("").is_err());
    }

    #[test]
    fn batch_status_all_succeeded_is_succeeded() {
        let mut a = request(ReconciliationRequestStatus::Attempted);
        a.linked_attempt_id = Some("attempt:a".to_string());
        let mut b = request(ReconciliationRequestStatus::Attempted);
        b.linked_attempt_id = Some("attempt:b".to_string());
        let statuses = BTreeMap::from([
            ("attempt:a".to_string(), AttemptStatus::Succeeded),
            ("attempt:b".to_string(), AttemptStatus::Succeeded),
        ]);
        assert_eq!(derive_batch_status(&[a, b], &statuses, at("2026-10-05T12:30:00Z")), ReconciliationBatchStatus::Succeeded);
    }

    #[test]
    fn batch_status_mixed_success_and_failure_is_partial() {
        let mut a = request(ReconciliationRequestStatus::Attempted);
        a.linked_attempt_id = Some("attempt:a".to_string());
        let mut b = request(ReconciliationRequestStatus::Attempted);
        b.linked_attempt_id = Some("attempt:b".to_string());
        let statuses = BTreeMap::from([
            ("attempt:a".to_string(), AttemptStatus::Succeeded),
            ("attempt:b".to_string(), AttemptStatus::Failed),
        ]);
        assert_eq!(derive_batch_status(&[a, b], &statuses, at("2026-10-05T12:30:00Z")), ReconciliationBatchStatus::Partial);
    }

    #[test]
    fn batch_status_is_executing_while_any_member_still_pending_or_claimed() {
        let a = request(ReconciliationRequestStatus::Pending);
        let mut b = request(ReconciliationRequestStatus::Attempted);
        b.linked_attempt_id = Some("attempt:b".to_string());
        let statuses = BTreeMap::from([("attempt:b".to_string(), AttemptStatus::Succeeded)]);
        assert_eq!(derive_batch_status(&[a, b], &statuses, at("2026-10-05T12:05:00Z")), ReconciliationBatchStatus::Executing);
    }

    #[test]
    fn batch_status_never_claims_success_while_any_member_is_stale() {
        let mut a = request(ReconciliationRequestStatus::Attempted);
        a.linked_attempt_id = Some("attempt:a".to_string());
        let b = request(ReconciliationRequestStatus::Stale);
        let statuses = BTreeMap::from([("attempt:a".to_string(), AttemptStatus::Succeeded)]);
        assert_eq!(derive_batch_status(&[a, b], &statuses, at("2026-10-05T12:30:00Z")), ReconciliationBatchStatus::Stale);
    }
}
