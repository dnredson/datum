//! Phase 124 -- the Alert Manager: durable persistence, condition-level
//! deduplication, and lifecycle management for `datum.alert/1`
//! (`core::datum_alert`), derived from `datum.event/1` occurrences
//! already durably admitted by Phase 123 (`storage::datum_event_store`).
//!
//! # Scope
//!
//! This module implements exactly five things: (1) a canonical,
//! deterministic correlation-key-based mapping from a stored
//! `DatumEventV1` to at most one ACTIVE `DatumAlertV1`; (2) durable,
//! normalized SQLite persistence for alerts, their event associations,
//! and their lifecycle transition history; (3) condition-level
//! deduplication (`occurrence_count`, distinct from Phase 123's own
//! event-identity replay/idempotency); (4) the lifecycle command API
//! (acknowledge/start-work/resolve/close); (5) a small, bounded internal
//! query API. It does NOT implement an Incident Manager, an
//! active-monitoring scheduler, event producers, DDeploy/DMonitor wiring,
//! a notification system, a Console, an operations dashboard, AI/GNN
//! action execution, remediation, or any HTTP API -- see
//! `documentation/reference/phase124-alert-manager-lifecycle.md` for the
//! full architectural rationale and explicit proof these were not
//! introduced.
//!
//! # Phase boundary: only stored events, never bypassing the Event Store
//!
//! `process_event` accepts a StoredDatumEvent and verifies it against the
//! admitted immutable row in the same transaction before using its contents.
//!
//! # Durability -- same pattern as Phase 123, same file
//!
//! Every function here takes an explicit `db_path: &Path`; there is no
//! `Option<PathBuf>` and no in-memory fallback anywhere in this module,
//! mirroring `storage::datum_event_store` exactly. This module's tables
//! (`datum_alerts`, `datum_alert_events`, `datum_alert_transitions`) are
//! created in the SAME physical SQLite file the caller points `db_path`
//! at -- the same file `datum_events` lives in, by convention. This lets
//! `datum_alert_events.event_id`/`datum_alerts.first_event_id`/
//! `datum_alerts.latest_event_id`/`datum_alert_transitions.related_event_id`
//! use REAL SQLite `FOREIGN KEY REFERENCES datum_events(event_id)`
//! constraints, giving genuine database-level referential integrity.
//! [`open_database`] now explicitly calls
//! `datum_event_store::ensure_datum_events_schema` BEFORE creating its
//! own FK-referencing tables (Phase 124 freeze review, finding F9) --
//! this module no longer silently assumes Phase 123 already ran first
//! against a given file; it makes that true itself, reusing Phase 123's
//! own schema definition rather than duplicating it.
//!
//! # Hardening pass (post freeze-review)
//!
//! An independent adversarial review found several gaps in the original
//! implementation. This revision fixes, in this module specifically:
//!
//! - **F2/F5 (out-of-order events):** aggregation (`first_observed_at_utc`/
//!   `last_observed_at_utc`/`first_event_id`/`latest_event_id`/
//!   `severity`/`peak_severity`) is now computed by CHRONOLOGICAL
//!   occurrence order (`DatumEventV1::occurred_at_utc`, parsed and
//!   compared as instants, never as TEXT), not processing/arrival order.
//!   See [`apply_chronological_aggregation`] for the exact algorithm and
//!   tie-break rule.
//! - **F3 (schema-init lock):** this module now shares
//!   `storage::schema_init::SCHEMA_INIT_LOCK` with
//!   `storage::datum_event_store` instead of maintaining an independent,
//!   non-coordinating `Mutex` of the same name.
//! - **F8 (event-association cardinality):** `datum_alert_events` is now
//!   unique on `(alert_id, event_id)`, not globally on `event_id` alone
//!   -- one `DatumEventV1` may in principle support more than one
//!   `DatumAlertV1` in a future phase, without a schema migration. Phase
//!   124's own `process_event` still deterministically produces exactly
//!   one association per event (see "Idempotency" below for how this is
//!   preserved under the new cardinality).
//! - **F6 (status vs. transition-history consistency)**, **F7
//!   (occurrence/observed/severity vs. event-association consistency)**,
//!   and the pragmatic lifecycle-checkpoint cross-check requested by the
//!   review: see [`get_alert`]'s documentation for exactly what is
//!   verified in the same snapshot before returning or mutating alerts.
//! - **F1 (actor validation)**, **F11 (reason length)**: every lifecycle
//!   command validates its caller-supplied actor and normalizes/bounds
//!   its reason text BEFORE opening any transaction -- see
//!   [`apply_lifecycle_command`].
//!
//! # Resolved/Closed late-event semantics (new policy, needed to make
//! F2/F5 coherent, not itself one of the review's numbered findings)
//!
//! An event's `occurred_at_utc` can arrive "late" relative to an alert's
//! own lifecycle timeline (distributed producers, clock skew, retries).
//! Two boundaries needed an explicit, deterministic policy:
//!
//! - **Resolved, event occurs AFTER `resolved_at_utc`:** genuine
//!   recurrence -- REOPEN (`Resolved -> Open`), exactly as before.
//! - **Resolved, event occurs AT OR BEFORE `resolved_at_utc`:** this is
//!   late historical evidence for the episode that was already resolved,
//!   not a new occurrence of the condition after the fix -- associate it,
//!   update aggregation (it can still, for instance, become the new
//!   `first_observed_at_utc`), but do NOT reopen, do NOT touch
//!   `resolved_by`/`resolved_at_utc`/`resolution_reason`, do NOT insert a
//!   transition. Returned as [`AlertProcessingOutcome::LateAssociated`].
//! - **Closed evidence:** choose the earliest closure boundary at or after
//!   event occurrence time (numeric seconds/nanos, then row id for ties),
//!   even if a newer active episode exists. Associate late evidence with
//!   that Closed episode and recompute aggregation; lifecycle status,
//!   actor checkpoints, reasons and transitions remain unchanged.
//! - **After every closure boundary:** use the active episode, or create
//!   a new Open episode if none exists. Equality belongs to the Closed
//!   historical episode. Lifecycle immutability does not freeze evidence.
//!
//! # Idempotency under composite association uniqueness
//!
//! The event-identity idempotency check (finding a previous
//! `AlreadyProcessed`) is scoped to "has this exact `event_id` been
//! associated with an alert sharing this event's OWN correlation key,"
//! not "has this `event_id` been associated with ANY alert at all." This
//! is deliberately NOT a global check: with composite
//! `(alert_id, event_id)` uniqueness, a future rule-engine phase may
//! legitimately associate the same immutable event with a second,
//! independently-derived alert under a DIFFERENT correlation key, and a
//! global check would incorrectly treat that as "already processed"
//! rather than a genuinely new, different association. Scoping by
//! correlation key also FIXES a regression a naive
//! "check-only-the-currently-active-alert" redesign would otherwise
//! introduce: a genuine retry of an event whose original alert has since
//! been `Closed` still correctly resolves to `AlreadyProcessed`
//! referencing that closed alert, rather than spawning a spurious new
//! episode.

use std::{path::Path, time::Duration};

use chrono::{SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, ToSql, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::datum_alert::{
    derive_correlation_key, is_valid_transition, DatumAlertActorTypeV1, DatumAlertActorV1,
    DatumAlertStatusV1, DatumAlertTransitionV1, DatumAlertV1, CLOSED_STATUS_WIRE_TOKEN,
    DATUM_ALERT_SCHEMA, MAX_DATUM_ALERT_REASON_BYTES,
};
use crate::core::datum_event::{DatumEventCategoryV1, DatumEventSeverityV1, DatumEventSubjectV1};
use crate::storage::datum_event_store::{
    self, DatumEventStoreError, ProjectFilter, StoredDatumEvent, DEFAULT_QUERY_LIMIT,
    MAX_QUERY_LIMIT,
};

/// The outcome of [`process_event`]. Five variants (one more than the
/// pre-hardening design, see module doc comment's "Resolved/Closed
/// late-event semantics") -- deliberately never collapsed into one
/// opaque success, mirroring Phase 123's own `Inserted`/`IdempotentReplay`
/// design philosophy.
#[derive(Debug, Clone, PartialEq)]
pub enum AlertProcessingOutcome {
    /// No active alert existed for this event's correlation key -- a
    /// brand new alert episode (`status = Open`) was created.
    Created(DatumAlertV1),
    /// An active, non-`Resolved`, non-`Closed` alert already existed for
    /// this correlation key; occurrence tracking was updated
    /// chronologically, the alert's lifecycle `status` was NOT changed.
    Updated(DatumAlertV1),
    /// A `Resolved` alert existed for this correlation key, and the
    /// incoming event's `occurred_at_utc` is chronologically AFTER the
    /// alert's own `resolved_at_utc` -- genuine recurrence. REOPENED
    /// (`Resolved -> Open`) and occurrence tracking updated.
    Reopened(DatumAlertV1),
    /// A historical Closed episode applies, or a `Resolved` alert exists and the
    /// incoming event's `occurred_at_utc` is AT OR BEFORE the alert's own
    /// `resolved_at_utc` -- late historical evidence for the
    /// already-resolved episode, not a new recurrence. Associated and
    /// aggregated chronologically; `status`/`resolved_by`/
    /// `resolved_at_utc`/`resolution_reason` all left untouched; no
    /// transition recorded.
    LateAssociated(DatumAlertV1),
    /// This exact `event_id` was already associated with an alert sharing
    /// this event's own correlation key by an earlier call (a genuine
    /// retry/reprocess) -- condition-level idempotency, distinct from
    /// Phase 123's event-identity replay idempotency. No occurrence_count
    /// increment, no new association row, nothing changed; the
    /// already-current alert is returned unchanged.
    AlreadyProcessed(DatumAlertV1),
}

impl AlertProcessingOutcome {
    pub fn alert(&self) -> &DatumAlertV1 {
        match self {
            AlertProcessingOutcome::Created(alert)
            | AlertProcessingOutcome::Updated(alert)
            | AlertProcessingOutcome::Reopened(alert)
            | AlertProcessingOutcome::LateAssociated(alert)
            | AlertProcessingOutcome::AlreadyProcessed(alert) => alert,
        }
    }
}

/// Typed errors for the Alert Manager's public API -- mirrors Phase 123's
/// `DatumEventStoreError` design (typed, not `String`, so a caller can
/// branch on the exact outcome).
#[derive(Debug, Clone, PartialEq)]
pub enum DatumAlertStoreError {
    /// `DatumAlertV1::validate()` returned non-empty findings for a
    /// freshly-constructed alert (should not occur under normal
    /// operation -- this module only ever constructs alerts from
    /// already-validated event fields; a firing here would indicate an
    /// internal inconsistency, not caller misuse).
    InvalidAlert(Vec<CanonicalValidationFinding>),
    /// A caller-supplied `DatumAlertActorV1` failed
    /// `DatumAlertActorV1::validate()` -- checked BEFORE any transaction
    /// is opened (Phase 124 freeze review, finding F1). Nothing is ever
    /// persisted for an invalid actor.
    InvalidActor(Vec<CanonicalValidationFinding>),
    /// A caller-supplied reason/note exceeds
    /// `core::datum_alert::MAX_DATUM_ALERT_REASON_BYTES` (finding F11).
    ReasonTooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    /// A SQLite/IO-level failure, or a foreign-key reference to an
    /// `event_id` that does not exist in `datum_events` (Phase 123) --
    /// see [`DatumAlertStoreError::EventNotFound`] for the latter's
    /// dedicated, more specific variant; this catches the residual case
    /// where the underlying SQLite error could not be classified more
    /// precisely.
    Storage(String),
    /// `alert_id` does not exist.
    AlertNotFound(String),
    /// The requested lifecycle command would perform a transition
    /// `core::datum_alert::is_valid_transition` rejects.
    InvalidTransition {
        alert_id: String,
        from: DatumAlertStatusV1,
        to: DatumAlertStatusV1,
    },
    /// A stored `datum_alerts`/`datum_alert_transitions`/
    /// `datum_alert_events` row (or the relationship between them) failed
    /// reconstruction, `DatumAlertV1::validate()`, or one of the
    /// cross-table consistency checks documented on [`get_alert`] --
    /// fails closed, never silently returns a malformed or internally
    /// inconsistent alert.
    CorruptStoredAlert { alert_id: String, reason: String },
    /// An event association was attempted against an `event_id` that
    /// does not exist in Phase 123's `datum_events` table -- surfaced via
    /// the real `FOREIGN KEY` constraint on `datum_alert_events.event_id`
    /// (see the module doc comment), translated into this specific,
    /// clean, typed error rather than a generic `Storage` one.
    EventNotFound(String),
    /// A malformed query (e.g. `limit` out of range, an empty explicit
    /// project filter).
    InvalidQuery(String),
}

impl std::fmt::Display for DatumAlertStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatumAlertStoreError::InvalidAlert(findings) => {
                write!(f, "invalid DatumAlert, not admitted: {findings:?}")
            }
            DatumAlertStoreError::InvalidActor(findings) => {
                write!(f, "invalid actor, not admitted: {findings:?}")
            }
            DatumAlertStoreError::ReasonTooLong {
                max_bytes,
                actual_bytes,
            } => write!(
                f,
                "reason is {actual_bytes} bytes, exceeding the {max_bytes}-byte limit"
            ),
            DatumAlertStoreError::Storage(message) => {
                write!(f, "datum alert store storage error: {message}")
            }
            DatumAlertStoreError::AlertNotFound(alert_id) => {
                write!(f, "datum alert '{alert_id}' not found")
            }
            DatumAlertStoreError::InvalidTransition { alert_id, from, to } => write!(
                f,
                "datum alert '{alert_id}' cannot transition from {from:?} to {to:?}"
            ),
            DatumAlertStoreError::CorruptStoredAlert { alert_id, reason } => {
                write!(f, "corrupt stored datum alert '{alert_id}': {reason}")
            }
            DatumAlertStoreError::EventNotFound(event_id) => {
                write!(f, "datum event '{event_id}' not found; cannot associate")
            }
            DatumAlertStoreError::InvalidQuery(message) => {
                write!(f, "invalid datum alert query: {message}")
            }
        }
    }
}

impl std::error::Error for DatumAlertStoreError {}

impl From<DatumEventStoreError> for DatumAlertStoreError {
    fn from(error: DatumEventStoreError) -> Self {
        DatumAlertStoreError::Storage(format!("underlying event store error: {error}"))
    }
}

/// A bounded, deterministically-ordered alert query. Small, fixed filter
/// set, not a query DSL -- mirrors `storage::datum_event_store::DatumEventQuery`.
///
/// **Ordering (documented explicitly per Phase 124 freeze review, finding
/// F12):** [`list_alerts`] orders by row-creation order (`id DESC`), NOT
/// by last-observed/last-updated activity. This is NOT the intended
/// future operations-dashboard ranking -- a future Phase 127 read model
/// must implement activity/severity-oriented ordering, and per the same
/// review, must reuse Phase 123's numeric-sortable-timestamp derivation
/// pattern for it rather than raw variable-precision RFC 3339 TEXT
/// ordering (proven unsafe by Phase 123's own F1 finding).
#[derive(Debug, Clone, PartialEq)]
pub struct AlertQuery<'a> {
    pub project_filter: ProjectFilter<'a>,
    pub application_id: Option<&'a str>,
    pub status: Option<DatumAlertStatusV1>,
    pub severity: Option<DatumEventSeverityV1>,
    pub kind: Option<&'a str>,
    pub correlation_key: Option<&'a str>,
    /// Shorthand for "only non-`Closed` alerts" -- see
    /// `core::datum_alert::is_active`. Combines with `status` (if both
    /// are set, both must hold).
    pub active_only: bool,
    pub limit: u32,
}

impl<'a> Default for AlertQuery<'a> {
    fn default() -> Self {
        AlertQuery {
            project_filter: ProjectFilter::Any,
            application_id: None,
            status: None,
            severity: None,
            kind: None,
            correlation_key: None,
            active_only: false,
            limit: DEFAULT_QUERY_LIMIT,
        }
    }
}

/// One event's association with an alert -- returned by
/// [`list_alert_events`]. Carries the FULL, corruption-checked
/// `StoredDatumEvent` (obtained via
/// `storage::datum_event_store::get_event`), never a raw re-read of
/// `datum_events`.
///
/// **Ordering (documented explicitly, finding F13):**
/// [`list_alert_events`] returns rows in ASSOCIATION order (the order
/// evidence was durably recorded, `id ASC`), NOT in event-occurrence
/// chronological order. Do not mislabel this as a time series -- a
/// future Phase 127 AI/time-series read model must offer a separate,
/// explicitly chronological ordering based on each event's own
/// `occurred_at_utc` if that is what it needs.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertEventAssociation {
    pub associated_at_utc: String,
    pub event: StoredDatumEvent,
}

// ------------------------------------------------ CHRONOLOGICAL AGGREGATION

/// Parses `DatumEventV1::occurred_at_utc` (already Phase-122-canonical by
/// the time any `StoredDatumEvent` reaches this module) into a comparable
/// instant. Every chronological comparison in this module goes through
/// this function -- never a TEXT comparison, the exact pitfall Phase
/// 123's own F1 finding proved unsafe for variable fractional-second
/// precision.
fn parse_occurred_at(value: &str) -> Result<(i64, u32), DatumAlertStoreError> {
    datum_event_store::canonicalize_and_derive_sortable(value)
        .map_err(DatumAlertStoreError::Storage)
}

/// Applies one newly-associated event's chronological contribution to
/// `alert`'s aggregation fields, in place, and increments
/// `occurrence_count` -- the single, shared implementation of finding
/// F2/F5's fix, used identically whether the event is a normal update, a
/// reopen, or a late-historical association to a still-`Resolved` alert
/// (see the module doc comment's "Resolved/Closed late-event semantics").
///
/// # Algorithm
///
/// - `first_observed_at_utc`/`first_event_id` (the running
///   `MIN(occurred_at)`): updated when `event.occurred_at_utc` is
///   chronologically EARLIER than the current value, OR exactly equal
///   AND `event.event_id` sorts lexicographically BEFORE the current
///   `first_event_id` (the deterministic tie-break, chosen and documented
///   once here -- see `core::datum_alert::DatumAlertV1::first_event_id`'s
///   doc comment).
/// - `last_observed_at_utc`/`latest_event_id`/`severity` (the running
///   `MAX(occurred_at)`): updated when `event.occurred_at_utc` is
///   chronologically LATER, or equal AND `event.event_id` sorts
///   lexicographically AFTER the current `latest_event_id`. `severity`
///   is updated ONLY when this MAX comparison actually changes
///   `latest_event_id` -- i.e. `severity` always equals the severity of
///   whichever event `latest_event_id` currently names, never simply
///   "whichever event was processed most recently." A chronologically
///   OLDER, lower-severity event arriving late can therefore never
///   downgrade the alert's current severity.
/// - `peak_severity`: updated whenever `event.severity` outranks the
///   current peak, UNCONDITIONALLY of chronological order -- the peak is
///   defined as the maximum severity ever observed across the episode,
///   order-independent by design.
/// - `occurrence_count`: always incremented by exactly 1.
fn apply_chronological_aggregation(
    alert: &mut DatumAlertV1,
    event: &crate::core::datum_event::DatumEventV1,
) -> Result<(), DatumAlertStoreError> {
    let occurred = parse_occurred_at(&event.occurred_at_utc)?;
    let current_first = parse_occurred_at(&alert.first_observed_at_utc)?;
    let current_last = parse_occurred_at(&alert.last_observed_at_utc)?;

    let is_new_first = occurred < current_first
        || (occurred == current_first && event.event_id < alert.first_event_id);
    if is_new_first {
        alert.first_observed_at_utc = event.occurred_at_utc.clone();
        alert.first_event_id = event.event_id.clone();
    }

    let is_new_latest = occurred > current_last
        || (occurred == current_last && event.event_id > alert.latest_event_id);
    if is_new_latest {
        alert.last_observed_at_utc = event.occurred_at_utc.clone();
        alert.latest_event_id = event.event_id.clone();
        alert.severity = event.severity;
    }

    if severity_rank(event.severity) > severity_rank(alert.peak_severity) {
        alert.peak_severity = event.severity;
    }

    alert.occurrence_count += 1;
    Ok(())
}

fn severity_rank(severity: DatumEventSeverityV1) -> u8 {
    match severity {
        DatumEventSeverityV1::Info => 0,
        DatumEventSeverityV1::Warning => 1,
        DatumEventSeverityV1::High => 2,
        DatumEventSeverityV1::Critical => 3,
    }
}

// -------------------------------------------------------------- COMMANDS

/// Processes one already-admitted `StoredDatumEvent` into the alert
/// layer: derives its canonical correlation key, finds, creates, or
/// (late-arriving evidence) associates without reopening the
/// corresponding alert, and durably records the association -- all
/// atomically. See the module doc comment for the phase-boundary
/// rationale and the full Resolved/Closed late-event policy.
///
/// # Algorithm (atomic; see the module's concurrency tests for proof)
///
/// 1. Derive `correlation_key`.
/// 2. Open a `TransactionBehavior::Immediate` transaction.
/// 3. If `event_id` is already associated with an alert sharing this
///    `correlation_key` (see the module doc comment's "Idempotency"
///    section for why this is scoped, not global) -> commit read-only,
///    return `AlreadyProcessed`.
/// 4. Route historical evidence to the earliest qualifying Closed boundary.
///    Otherwise use the active episode: Resolved reopens only for occurrence
///    time strictly after resolution; equality and older evidence merely
///    augment it. If neither historical nor active episode applies, create.
///    All aggregation uses numeric occurrence time and event-id tie-breaking.
/// 5. Insert the `datum_alert_events` association row.
/// 6. Commit.
pub fn process_event(
    db_path: &Path,
    stored_event: &StoredDatumEvent,
) -> Result<AlertProcessingOutcome, DatumAlertStoreError> {
    let correlation_key =
        derive_correlation_key(&stored_event.event).map_err(DatumAlertStoreError::Storage)?;
    let event = &stored_event.event;

    let mut connection = open_database(db_path).map_err(DatumAlertStoreError::Storage)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("begin datum_alerts transaction: {error}"))
        })?;

    // StoredDatumEvent has public fields: verify the supplied value against
    // the admitted immutable row rather than trusting the wrapper alone.
    let admitted = datum_event_store::fetch_by_id(&transaction, &event.event_id)?
        .ok_or_else(|| DatumAlertStoreError::EventNotFound(event.event_id.clone()))?;
    if admitted != *stored_event {
        return Err(DatumAlertStoreError::Storage(
            "supplied event differs from admitted event".into(),
        ));
    }

    let already_associated_alert_id: Option<String> = transaction
        .query_row(
            "SELECT a.alert_id FROM datum_alert_events e
             JOIN datum_alerts a ON a.alert_id = e.alert_id
             WHERE e.event_id = ?1 AND a.correlation_key = ?2 ORDER BY a.id ASC LIMIT 1",
            params![&event.event_id, &correlation_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("lookup datum_alert_events: {error}"))
        })?;
    if let Some(existing_alert_id) = already_associated_alert_id {
        let alert = fetch_alert_by_id(&transaction, &existing_alert_id)?.ok_or_else(|| {
            DatumAlertStoreError::Storage(
                "datum_alerts row vanished for an already-associated event".to_string(),
            )
        })?;
        transaction.commit().map_err(|error| {
            DatumAlertStoreError::Storage(format!(
                "commit datum_alerts already-processed read: {error}"
            ))
        })?;
        return Ok(AlertProcessingOutcome::AlreadyProcessed(alert));
    }

    let active_alert_id: Option<String> = transaction
        .query_row(
            &format!(
                "SELECT alert_id FROM datum_alerts WHERE correlation_key = ?1 AND status != '{CLOSED_STATUS_WIRE_TOKEN}'"
            ),
            params![&correlation_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| DatumAlertStoreError::Storage(format!("lookup datum_alerts: {error}")))?;

    // Closed boundaries partition occurrence time: select the earliest
    // closure >= occurrence time, with row id breaking equal-boundary ties.
    // This also routes old evidence correctly when a newer active episode exists.
    let occurred = parse_occurred_at(&event.occurred_at_utc)?;
    let mut closed_statement = transaction
        .prepare(
            "SELECT alert_id, closed_at_utc, id FROM datum_alerts
         WHERE correlation_key = ?1 AND status = 'closed' ORDER BY id",
        )
        .map_err(storage_error)?;
    let closed_rows = closed_statement
        .query_map(params![correlation_key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    drop(closed_statement);
    let mut historical: Option<((i64, u32), i64, String)> = None;
    for (id, at, row_id) in closed_rows {
        let boundary =
            parse_occurred_at(&at).map_err(|e| DatumAlertStoreError::CorruptStoredAlert {
                alert_id: id.clone(),
                reason: e.to_string(),
            })?;
        if occurred <= boundary
            && historical
                .as_ref()
                .is_none_or(|h| (boundary, row_id) < (h.0, h.1))
        {
            historical = Some((boundary, row_id, id));
        }
    }
    let target_id = historical.map(|h| h.2).or(active_alert_id);
    let now = canonical_utc_now();
    let (alert, outcome_kind) = match target_id {
        Some(existing_alert_id) => {
            let mut alert =
                fetch_alert_by_id(&transaction, &existing_alert_id)?.ok_or_else(|| {
                    DatumAlertStoreError::Storage(
                        "datum_alerts row vanished between lookup and read".to_string(),
                    )
                })?;

            if alert.status == DatumAlertStatusV1::Closed {
                apply_chronological_aggregation(&mut alert, event)?;
                update_alert_row(&transaction, &alert)?;
                (alert, OutcomeKind::LateAssociated)
            } else if alert.status == DatumAlertStatusV1::Resolved {
                let occurred = parse_occurred_at(&event.occurred_at_utc)?;
                let resolved_at =
                    parse_occurred_at(alert.resolved_at_utc.as_deref().ok_or_else(|| {
                        DatumAlertStoreError::Storage(
                            "Resolved alert missing resolved_at_utc".to_string(),
                        )
                    })?)?;

                if occurred > resolved_at {
                    // Genuine recurrence: reopen.
                    insert_transition(
                        &transaction,
                        &alert.alert_id,
                        Some(DatumAlertStatusV1::Resolved),
                        DatumAlertStatusV1::Open,
                        &DatumAlertActorV1::system(),
                        Some("condition recurred after resolution"),
                        &now,
                        Some(&event.event_id),
                    )?;
                    alert.status = DatumAlertStatusV1::Open;
                    alert.resolved_by = None;
                    alert.resolved_at_utc = None;
                    alert.resolution_reason = None;
                    apply_chronological_aggregation(&mut alert, event)?;
                    update_alert_row(&transaction, &alert)?;
                    (alert, OutcomeKind::Reopened)
                } else {
                    // Late historical evidence for the already-resolved
                    // episode -- associate and aggregate, do not reopen.
                    apply_chronological_aggregation(&mut alert, event)?;
                    update_alert_row(&transaction, &alert)?;
                    (alert, OutcomeKind::LateAssociated)
                }
            } else {
                apply_chronological_aggregation(&mut alert, event)?;
                update_alert_row(&transaction, &alert)?;
                (alert, OutcomeKind::Updated)
            }
        }
        None => {
            let alert = DatumAlertV1 {
                schema: DATUM_ALERT_SCHEMA.to_string(),
                alert_id: format!("datumalert:{}", Uuid::new_v4()),
                project_id: event.project_id.clone(),
                application_id: event.application_id.clone(),
                correlation_key: correlation_key.clone(),
                category: event.category,
                kind: event.kind.clone(),
                severity: event.severity,
                peak_severity: event.severity,
                subject: event.subject.clone(),
                status: DatumAlertStatusV1::Open,
                first_observed_at_utc: event.occurred_at_utc.clone(),
                last_observed_at_utc: event.occurred_at_utc.clone(),
                first_event_id: event.event_id.clone(),
                latest_event_id: event.event_id.clone(),
                occurrence_count: 1,
                acknowledged_by: None,
                acknowledged_at_utc: None,
                in_progress_by: None,
                in_progress_at_utc: None,
                resolved_by: None,
                resolved_at_utc: None,
                resolution_reason: None,
                closed_by: None,
                closed_at_utc: None,
                close_reason: None,
            };
            let findings = alert.validate();
            if !findings.is_empty() {
                transaction.rollback().ok();
                return Err(DatumAlertStoreError::InvalidAlert(findings));
            }
            insert_new_alert_row(&transaction, &alert)?;
            insert_transition(
                &transaction,
                &alert.alert_id,
                None,
                DatumAlertStatusV1::Open,
                &DatumAlertActorV1::system(),
                Some("alert created"),
                &now,
                Some(&event.event_id),
            )?;
            (alert, OutcomeKind::Created)
        }
    };

    insert_association(&transaction, &alert.alert_id, &event.event_id, &now)?;

    transaction.commit().map_err(|error| {
        DatumAlertStoreError::Storage(format!("commit datum_alerts process_event: {error}"))
    })?;

    Ok(match outcome_kind {
        OutcomeKind::Created => AlertProcessingOutcome::Created(alert),
        OutcomeKind::Updated => AlertProcessingOutcome::Updated(alert),
        OutcomeKind::Reopened => AlertProcessingOutcome::Reopened(alert),
        OutcomeKind::LateAssociated => AlertProcessingOutcome::LateAssociated(alert),
    })
}

enum OutcomeKind {
    Created,
    Updated,
    Reopened,
    LateAssociated,
}

/// Normalizes a caller-supplied optional reason/note: trims whitespace,
/// treats an empty/whitespace-only result as `None` (finding F11's
/// explicit "normalize, don't reject" choice for the empty case), and
/// rejects (`ReasonTooLong`) anything exceeding
/// `MAX_DATUM_ALERT_REASON_BYTES` measured as UTF-8 byte length.
fn normalize_reason(reason: Option<&str>) -> Result<Option<String>, DatumAlertStoreError> {
    if let Some(text) = reason {
        if text.len() > MAX_DATUM_ALERT_REASON_BYTES {
            return Err(DatumAlertStoreError::ReasonTooLong {
                max_bytes: MAX_DATUM_ALERT_REASON_BYTES,
                actual_bytes: text.len(),
            });
        }
    }
    let trimmed = reason.map(str::trim).filter(|s| !s.is_empty());
    match trimmed {
        None => Ok(None),
        Some(text) if text.len() > MAX_DATUM_ALERT_REASON_BYTES => {
            Err(DatumAlertStoreError::ReasonTooLong {
                max_bytes: MAX_DATUM_ALERT_REASON_BYTES,
                actual_bytes: text.len(),
            })
        }
        Some(text) => Ok(Some(text.to_string())),
    }
}

/// Explicitly accepts ownership of `alert_id` (`Open -> Acknowledged`;
/// calling this when the alert is not `Open` returns `InvalidTransition`).
pub fn acknowledge_alert(
    db_path: &Path,
    alert_id: &str,
    actor: DatumAlertActorV1,
    reason: Option<&str>,
) -> Result<DatumAlertV1, DatumAlertStoreError> {
    apply_lifecycle_command(
        db_path,
        alert_id,
        DatumAlertStatusV1::Acknowledged,
        actor,
        reason,
        |alert, actor, now, _reason| {
            alert.acknowledged_by = Some(actor.clone());
            alert.acknowledged_at_utc = Some(now.to_string());
        },
    )
}

/// Marks `alert_id` as actively being investigated/treated
/// (`Acknowledged -> InProgress`).
pub fn start_alert_work(
    db_path: &Path,
    alert_id: &str,
    actor: DatumAlertActorV1,
    reason: Option<&str>,
) -> Result<DatumAlertV1, DatumAlertStoreError> {
    apply_lifecycle_command(
        db_path,
        alert_id,
        DatumAlertStatusV1::InProgress,
        actor,
        reason,
        |alert, actor, now, _reason| {
            alert.in_progress_by = Some(actor.clone());
            alert.in_progress_at_utc = Some(now.to_string());
        },
    )
}

/// Marks the underlying condition as no longer active
/// (`Open`/`Acknowledged`/`InProgress -> Resolved`). `resolution_reason`
/// is recommended but not required.
pub fn resolve_alert(
    db_path: &Path,
    alert_id: &str,
    actor: DatumAlertActorV1,
    resolution_reason: Option<&str>,
) -> Result<DatumAlertV1, DatumAlertStoreError> {
    apply_lifecycle_command(
        db_path,
        alert_id,
        DatumAlertStatusV1::Resolved,
        actor,
        resolution_reason,
        |alert, actor, now, reason| {
            alert.resolved_by = Some(actor.clone());
            alert.resolved_at_utc = Some(now.to_string());
            alert.resolution_reason = reason.map(str::to_string);
        },
    )
}

/// Administratively completes `alert_id`'s lifecycle episode
/// (`Resolved -> Closed`, terminal). `close_reason` is optional.
pub fn close_alert(
    db_path: &Path,
    alert_id: &str,
    actor: DatumAlertActorV1,
    close_reason: Option<&str>,
) -> Result<DatumAlertV1, DatumAlertStoreError> {
    apply_lifecycle_command(
        db_path,
        alert_id,
        DatumAlertStatusV1::Closed,
        actor,
        close_reason,
        |alert, actor, now, reason| {
            alert.closed_by = Some(actor.clone());
            alert.closed_at_utc = Some(now.to_string());
            alert.close_reason = reason.map(str::to_string);
        },
    )
}

/// Shared transactional shape for the four lifecycle commands.
///
/// **Finding F1:** `actor.validate()` runs FIRST, before
/// `open_database`/any transaction -- a malformed actor is rejected
/// (`InvalidActor`) with nothing touched at all, not even a connection
/// opened. **Finding F11:** `reason` is normalized/length-checked next,
/// also before any transaction.
///
/// Then: fetch the current alert, reject an invalid transition
/// (`InvalidTransition`, never silently coerced), otherwise apply the
/// target-specific fields, persist the row update AND append one
/// transition record atomically, then return the updated alert. The
/// caller can never set `status` arbitrarily -- only the four named
/// public entry points exist, each fixed to exactly one target status.
fn apply_lifecycle_command(
    db_path: &Path,
    alert_id: &str,
    target_status: DatumAlertStatusV1,
    actor: DatumAlertActorV1,
    reason: Option<&str>,
    apply_fields: impl FnOnce(&mut DatumAlertV1, &DatumAlertActorV1, &str, Option<&str>),
) -> Result<DatumAlertV1, DatumAlertStoreError> {
    let alert_id_trimmed = alert_id.trim();
    if alert_id_trimmed.is_empty() {
        return Err(DatumAlertStoreError::InvalidQuery(
            "alert_id must be non-empty".to_string(),
        ));
    }

    let actor_findings = actor.validate();
    if !actor_findings.is_empty() {
        return Err(DatumAlertStoreError::InvalidActor(actor_findings));
    }
    let reason = normalize_reason(reason)?;

    let mut connection = open_database(db_path).map_err(DatumAlertStoreError::Storage)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!(
                "begin datum_alerts lifecycle-command transaction: {error}"
            ))
        })?;

    let mut alert = fetch_alert_by_id(&transaction, alert_id_trimmed)?
        .ok_or_else(|| DatumAlertStoreError::AlertNotFound(alert_id_trimmed.to_string()))?;

    if !is_valid_transition(alert.status, target_status) {
        transaction.rollback().ok();
        return Err(DatumAlertStoreError::InvalidTransition {
            alert_id: alert_id_trimmed.to_string(),
            from: alert.status,
            to: target_status,
        });
    }

    let now = canonical_utc_now();
    let from_status = alert.status;
    apply_fields(&mut alert, &actor, &now, reason.as_deref());
    alert.status = target_status;

    let findings = alert.validate();
    if !findings.is_empty() {
        transaction.rollback().ok();
        return Err(DatumAlertStoreError::InvalidAlert(findings));
    }

    update_alert_row(&transaction, &alert)?;
    insert_transition(
        &transaction,
        alert_id_trimmed,
        Some(from_status),
        target_status,
        &actor,
        reason.as_deref(),
        &now,
        None,
    )?;

    transaction.commit().map_err(|error| {
        DatumAlertStoreError::Storage(format!("commit datum_alerts lifecycle command: {error}"))
    })?;

    Ok(alert)
}

// ----------------------------------------------------------------- QUERIES

/// Reads a complete alert in one SQLite snapshot. All materialized fields
/// are verified against the entire association and transition histories.
/// The public query page limit never limits integrity verification.
pub fn get_alert(
    db_path: &Path,
    alert_id: &str,
) -> Result<Option<DatumAlertV1>, DatumAlertStoreError> {
    if alert_id.trim().is_empty() {
        return Err(DatumAlertStoreError::InvalidQuery(
            "alert_id must be non-empty".into(),
        ));
    }
    let mut connection = open_database(db_path).map_err(DatumAlertStoreError::Storage)?;
    let transaction = connection.transaction().map_err(storage_error)?;
    fetch_alert_by_id(&transaction, alert_id.trim())
}

fn storage_error(error: rusqlite::Error) -> DatumAlertStoreError {
    DatumAlertStoreError::Storage(error.to_string())
}

/// Uses the caller's snapshot, including during writes, so corruption is
/// rejected before it can be propagated into a newly materialized row.
fn verify_deep_consistency(
    connection: &Connection,
    alert: &DatumAlertV1,
) -> Result<(), DatumAlertStoreError> {
    let corrupt = |reason: String| DatumAlertStoreError::CorruptStoredAlert {
        alert_id: alert.alert_id.clone(),
        reason,
    };
    let mut statement = connection
        .prepare("SELECT event_id FROM datum_alert_events WHERE alert_id = ?1 ORDER BY id")
        .map_err(storage_error)?;
    let ids = statement
        .query_map(params![alert.alert_id], |r| r.get::<_, String>(0))
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;
    if ids.len() as u64 != alert.occurrence_count || ids.is_empty() {
        return Err(corrupt(
            "occurrence_count disagrees with association count".into(),
        ));
    }
    let mut expected = alert.clone();
    for (index, id) in ids.iter().enumerate() {
        let stored = datum_event_store::fetch_by_id(connection, id)
            .map_err(|e| corrupt(format!("associated event: {e}")))?
            .ok_or_else(|| corrupt(format!("missing associated event {id}")))?;
        if stored.event.event_id != *id {
            return Err(corrupt("associated event identity mismatch".into()));
        }
        let event = &stored.event;
        if index == 0 {
            expected.occurrence_count = 1;
            expected.first_event_id = event.event_id.clone();
            expected.latest_event_id = event.event_id.clone();
            expected.first_observed_at_utc = event.occurred_at_utc.clone();
            expected.last_observed_at_utc = event.occurred_at_utc.clone();
            expected.severity = event.severity;
            expected.peak_severity = event.severity;
        } else {
            apply_chronological_aggregation(&mut expected, event)?;
        }
    }
    if expected.first_event_id != alert.first_event_id
        || expected.latest_event_id != alert.latest_event_id
        || expected.first_observed_at_utc != alert.first_observed_at_utc
        || expected.last_observed_at_utc != alert.last_observed_at_utc
        || expected.severity != alert.severity
        || expected.peak_severity != alert.peak_severity
    {
        return Err(corrupt(
            "materialized occurrence aggregation disagrees with association history".into(),
        ));
    }

    let transitions = read_transitions(connection, &alert.alert_id, i64::MAX)?;
    let mut previous = None;
    let mut acknowledged = None;
    let mut in_progress = None;
    let mut resolved = None;
    let mut closed = None;
    for (index, t) in transitions.iter().enumerate() {
        if index == 0 {
            if t.from_status.is_some() || t.to_status != DatumAlertStatusV1::Open {
                return Err(corrupt("first transition must be None -> Open".into()));
            }
        } else if t.from_status != previous || !is_valid_transition(previous.unwrap(), t.to_status)
        {
            return Err(corrupt("broken or illegal transition chain".into()));
        }
        let checkpoint = Some((t.actor.clone(), t.transitioned_at_utc.clone()));
        match t.to_status {
            DatumAlertStatusV1::Acknowledged => acknowledged = checkpoint,
            DatumAlertStatusV1::InProgress => in_progress = checkpoint,
            DatumAlertStatusV1::Resolved => {
                resolved = Some((checkpoint.unwrap(), t.reason.clone()))
            }
            DatumAlertStatusV1::Closed => closed = Some((checkpoint.unwrap(), t.reason.clone())),
            DatumAlertStatusV1::Open => resolved = None,
        }
        previous = Some(t.to_status);
    }
    if previous != Some(alert.status) {
        return Err(corrupt(
            "current status disagrees with transition history or creation is missing".into(),
        ));
    }
    if acknowledged
        != alert
            .acknowledged_by
            .clone()
            .zip(alert.acknowledged_at_utc.clone())
        || in_progress
            != alert
                .in_progress_by
                .clone()
                .zip(alert.in_progress_at_utc.clone())
        || resolved
            != alert
                .resolved_by
                .clone()
                .zip(alert.resolved_at_utc.clone())
                .map(|p| (p, alert.resolution_reason.clone()))
        || closed
            != alert
                .closed_by
                .clone()
                .zip(alert.closed_at_utc.clone())
                .map(|p| (p, alert.close_reason.clone()))
    {
        return Err(corrupt(
            "lifecycle checkpoints disagree with transition history".into(),
        ));
    }
    Ok(())
}

/// Bounded list ordered by row creation (`id DESC`), not dashboard rank.
/// Every returned alert is fully verified within one read snapshot.
pub fn list_alerts(
    db_path: &Path,
    query: &AlertQuery<'_>,
) -> Result<Vec<DatumAlertV1>, DatumAlertStoreError> {
    if query.limit == 0 || query.limit > MAX_QUERY_LIMIT {
        return Err(DatumAlertStoreError::InvalidQuery(format!(
            "limit must be 1..={MAX_QUERY_LIMIT}, got {}",
            query.limit
        )));
    }
    if let ProjectFilter::Project(project_id) = query.project_filter {
        if project_id.trim().is_empty() {
            return Err(DatumAlertStoreError::InvalidQuery(
                "project filter, if a specific project, must be non-empty".to_string(),
            ));
        }
    }

    let mut clauses: Vec<String> = Vec::new();
    let mut bindings: Vec<Box<dyn ToSql>> = Vec::new();

    match query.project_filter {
        ProjectFilter::Any => {}
        ProjectFilter::GlobalOnly => clauses.push("project_id IS NULL".to_string()),
        ProjectFilter::Project(project_id) => {
            clauses.push("project_id = ?".to_string());
            bindings.push(Box::new(project_id.to_string()));
        }
    }
    if let Some(application_id) = query.application_id {
        clauses.push("application_id = ?".to_string());
        bindings.push(Box::new(application_id.to_string()));
    }
    if let Some(status) = query.status {
        clauses.push("status = ?".to_string());
        bindings.push(Box::new(enum_wire_str(&status)?));
    }
    if let Some(severity) = query.severity {
        clauses.push("severity = ?".to_string());
        bindings.push(Box::new(enum_wire_str(&severity)?));
    }
    if let Some(kind) = query.kind {
        clauses.push("kind = ?".to_string());
        bindings.push(Box::new(kind.to_string()));
    }
    if let Some(correlation_key) = query.correlation_key {
        clauses.push("correlation_key = ?".to_string());
        bindings.push(Box::new(correlation_key.to_string()));
    }
    if query.active_only {
        clauses.push(format!("status != '{CLOSED_STATUS_WIRE_TOKEN}'"));
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!("SELECT alert_id FROM datum_alerts {where_sql} ORDER BY id DESC LIMIT ?");
    bindings.push(Box::new(i64::from(query.limit)));

    let mut database = open_database(db_path).map_err(DatumAlertStoreError::Storage)?;
    let connection = database.transaction().map_err(storage_error)?;
    let mut statement = connection.prepare(&sql).map_err(|error| {
        DatumAlertStoreError::Storage(format!("prepare datum_alerts query: {error}"))
    })?;
    let param_refs: Vec<&dyn ToSql> = bindings.iter().map(|value| value.as_ref()).collect();
    let alert_ids: Vec<String> = statement
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|error| DatumAlertStoreError::Storage(format!("query datum_alerts: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("read datum_alerts rows: {error}"))
        })?;

    let mut alerts = Vec::with_capacity(alert_ids.len());
    for alert_id in alert_ids {
        let alert = fetch_alert_by_id(&connection, &alert_id)?.ok_or_else(|| {
            DatumAlertStoreError::Storage(format!(
                "datum_alerts row '{alert_id}' vanished between query and read"
            ))
        })?;
        alerts.push(alert);
    }
    Ok(alerts)
}

/// Every `DatumEventV1` associated with `alert_id`, in ASSOCIATION order
/// (see [`AlertEventAssociation`]'s doc comment), each obtained via Phase
/// 123's own corruption-checked `storage::datum_event_store::get_event`.
/// Bounded by `limit`.
pub fn list_alert_events(
    db_path: &Path,
    alert_id: &str,
    limit: u32,
) -> Result<Vec<AlertEventAssociation>, DatumAlertStoreError> {
    let alert_id = alert_id.trim();
    if alert_id.is_empty() {
        return Err(DatumAlertStoreError::InvalidQuery(
            "alert_id must be non-empty".to_string(),
        ));
    }
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        return Err(DatumAlertStoreError::InvalidQuery(format!(
            "limit must be 1..={MAX_QUERY_LIMIT}, got {limit}"
        )));
    }

    let connection = open_database(db_path).map_err(DatumAlertStoreError::Storage)?;
    let mut statement = connection
        .prepare(
            "SELECT event_id, associated_at_utc FROM datum_alert_events
             WHERE alert_id = ?1 ORDER BY id ASC LIMIT ?2",
        )
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("prepare datum_alert_events query: {error}"))
        })?;
    let rows: Vec<(String, String)> = statement
        .query_map(params![alert_id, i64::from(limit)], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("query datum_alert_events: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("read datum_alert_events rows: {error}"))
        })?;

    let mut associations = Vec::with_capacity(rows.len());
    for (event_id, associated_at_utc) in rows {
        let event = datum_event_store::get_event(db_path, &event_id)?.ok_or_else(|| {
            DatumAlertStoreError::Storage(format!(
                "datum_alert_events references event_id '{event_id}' that Phase 123's store no longer returns"
            ))
        })?;
        associations.push(AlertEventAssociation {
            associated_at_utc,
            event,
        });
    }
    Ok(associations)
}

/// Every lifecycle transition recorded for `alert_id`, oldest first,
/// bounded by `limit`.
pub fn list_alert_transitions(
    db_path: &Path,
    alert_id: &str,
    limit: u32,
) -> Result<Vec<DatumAlertTransitionV1>, DatumAlertStoreError> {
    let alert_id = alert_id.trim();
    if alert_id.is_empty() {
        return Err(DatumAlertStoreError::InvalidQuery(
            "alert_id must be non-empty".to_string(),
        ));
    }
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        return Err(DatumAlertStoreError::InvalidQuery(format!(
            "limit must be 1..={MAX_QUERY_LIMIT}, got {limit}"
        )));
    }

    let connection = open_database(db_path).map_err(DatumAlertStoreError::Storage)?;
    read_transitions(&connection, alert_id, i64::from(limit))
}

fn read_transitions(
    connection: &Connection,
    alert_id: &str,
    limit: i64,
) -> Result<Vec<DatumAlertTransitionV1>, DatumAlertStoreError> {
    let mut statement = connection
        .prepare(
            "SELECT transition_id, from_status, to_status, actor_type, actor_id, model_ref, reason,
                    transitioned_at_utc, related_event_id
             FROM datum_alert_transitions WHERE alert_id = ?1 ORDER BY id ASC LIMIT ?2",
        )
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("prepare datum_alert_transitions query: {error}"))
        })?;
    let rows = statement
        .query_map(params![alert_id, i64::from(limit)], |row| {
            Ok(TransitionRow {
                transition_id: row.get(0)?,
                from_status: row.get(1)?,
                to_status: row.get(2)?,
                actor_type: row.get(3)?,
                actor_id: row.get(4)?,
                model_ref: row.get(5)?,
                reason: row.get(6)?,
                transitioned_at_utc: row.get(7)?,
                related_event_id: row.get(8)?,
            })
        })
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("query datum_alert_transitions: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("read datum_alert_transitions rows: {error}"))
        })?;

    rows.into_iter()
        .map(|row| row_to_transition(alert_id, row))
        .collect()
}

// ------------------------------------------------------------- INTERNALS

struct TransitionRow {
    transition_id: String,
    from_status: Option<String>,
    to_status: String,
    actor_type: String,
    actor_id: Option<String>,
    model_ref: Option<String>,
    reason: Option<String>,
    transitioned_at_utc: String,
    related_event_id: Option<String>,
}

fn row_to_transition(
    alert_id: &str,
    row: TransitionRow,
) -> Result<DatumAlertTransitionV1, DatumAlertStoreError> {
    let from_status = row
        .from_status
        .map(|token| parse_wire_token::<DatumAlertStatusV1>(&token))
        .transpose()
        .map_err(|reason| DatumAlertStoreError::CorruptStoredAlert {
            alert_id: alert_id.to_string(),
            reason: format!("transition '{}' from_status: {reason}", row.transition_id),
        })?;
    let to_status = parse_wire_token::<DatumAlertStatusV1>(&row.to_status).map_err(|reason| {
        DatumAlertStoreError::CorruptStoredAlert {
            alert_id: alert_id.to_string(),
            reason: format!("transition '{}' to_status: {reason}", row.transition_id),
        }
    })?;
    let actor_type =
        parse_wire_token::<DatumAlertActorTypeV1>(&row.actor_type).map_err(|reason| {
            DatumAlertStoreError::CorruptStoredAlert {
                alert_id: alert_id.to_string(),
                reason: format!("transition '{}' actor_type: {reason}", row.transition_id),
            }
        })?;
    let transition = DatumAlertTransitionV1 {
        transition_id: row.transition_id,
        alert_id: alert_id.to_string(),
        from_status,
        to_status,
        actor: DatumAlertActorV1 {
            actor_type,
            actor_id: row.actor_id,
            model_ref: row.model_ref,
        },
        reason: row.reason,
        transitioned_at_utc: row.transitioned_at_utc,
        related_event_id: row.related_event_id,
    };
    if !transition.actor.validate().is_empty()
        || parse_occurred_at(&transition.transitioned_at_utc).is_err()
        || normalize_reason(transition.reason.as_deref()).ok().as_ref() != Some(&transition.reason)
        || transition.transition_id.trim().is_empty()
    {
        return Err(DatumAlertStoreError::CorruptStoredAlert {
            alert_id: alert_id.into(),
            reason: "malformed transition provenance, timestamp or reason".into(),
        });
    }
    Ok(transition)
}

struct AlertRow {
    alert_id: String,
    schema: String,
    project_id: Option<String>,
    application_id: Option<String>,
    correlation_key: String,
    category: String,
    kind: String,
    severity: String,
    peak_severity: String,
    subject_json: String,
    status: String,
    first_observed_at_utc: String,
    last_observed_at_utc: String,
    first_event_id: String,
    latest_event_id: String,
    occurrence_count: i64,
    acknowledged_by_type: Option<String>,
    acknowledged_by_id: Option<String>,
    acknowledged_by_model_ref: Option<String>,
    acknowledged_at_utc: Option<String>,
    in_progress_by_type: Option<String>,
    in_progress_by_id: Option<String>,
    in_progress_by_model_ref: Option<String>,
    in_progress_at_utc: Option<String>,
    resolved_by_type: Option<String>,
    resolved_by_id: Option<String>,
    resolved_by_model_ref: Option<String>,
    resolved_at_utc: Option<String>,
    resolution_reason: Option<String>,
    closed_by_type: Option<String>,
    closed_by_id: Option<String>,
    closed_by_model_ref: Option<String>,
    closed_at_utc: Option<String>,
    close_reason: Option<String>,
}

const ALERT_COLUMNS: &str = "alert_id, schema, project_id, application_id, correlation_key,
    category, kind, severity, peak_severity, subject_json, status,
    first_observed_at_utc, last_observed_at_utc, first_event_id, latest_event_id,
    occurrence_count,
    acknowledged_by_type, acknowledged_by_id, acknowledged_by_model_ref, acknowledged_at_utc,
    in_progress_by_type, in_progress_by_id, in_progress_by_model_ref, in_progress_at_utc,
    resolved_by_type, resolved_by_id, resolved_by_model_ref, resolved_at_utc, resolution_reason,
    closed_by_type, closed_by_id, closed_by_model_ref, closed_at_utc, close_reason";

fn map_alert_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AlertRow> {
    Ok(AlertRow {
        alert_id: row.get(0)?,
        schema: row.get(1)?,
        project_id: row.get(2)?,
        application_id: row.get(3)?,
        correlation_key: row.get(4)?,
        category: row.get(5)?,
        kind: row.get(6)?,
        severity: row.get(7)?,
        peak_severity: row.get(8)?,
        subject_json: row.get(9)?,
        status: row.get(10)?,
        first_observed_at_utc: row.get(11)?,
        last_observed_at_utc: row.get(12)?,
        first_event_id: row.get(13)?,
        latest_event_id: row.get(14)?,
        occurrence_count: row.get(15)?,
        acknowledged_by_type: row.get(16)?,
        acknowledged_by_id: row.get(17)?,
        acknowledged_by_model_ref: row.get(18)?,
        acknowledged_at_utc: row.get(19)?,
        in_progress_by_type: row.get(20)?,
        in_progress_by_id: row.get(21)?,
        in_progress_by_model_ref: row.get(22)?,
        in_progress_at_utc: row.get(23)?,
        resolved_by_type: row.get(24)?,
        resolved_by_id: row.get(25)?,
        resolved_by_model_ref: row.get(26)?,
        resolved_at_utc: row.get(27)?,
        resolution_reason: row.get(28)?,
        closed_by_type: row.get(29)?,
        closed_by_id: row.get(30)?,
        closed_by_model_ref: row.get(31)?,
        closed_at_utc: row.get(32)?,
        close_reason: row.get(33)?,
    })
}

/// Reconstructs and structurally validates the row, then verifies all
/// association aggregation and lifecycle history in the caller's snapshot.
fn fetch_alert_by_id(
    connection: &Connection,
    alert_id: &str,
) -> Result<Option<DatumAlertV1>, DatumAlertStoreError> {
    let row: Option<AlertRow> = connection
        .query_row(
            &format!("SELECT {ALERT_COLUMNS} FROM datum_alerts WHERE alert_id = ?1"),
            params![alert_id],
            map_alert_row,
        )
        .optional()
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("read datum_alerts row '{alert_id}': {error}"))
        })?;

    let Some(row) = row else {
        return Ok(None);
    };

    let corrupt = |reason: String| DatumAlertStoreError::CorruptStoredAlert {
        alert_id: alert_id.to_string(),
        reason,
    };

    let subject: DatumEventSubjectV1 = serde_json::from_str(&row.subject_json)
        .map_err(|error| corrupt(format!("subject_json failed to deserialize: {error}")))?;
    let category = parse_wire_token::<DatumEventCategoryV1>(&row.category)
        .map_err(|error| corrupt(format!("category: {error}")))?;
    let severity = parse_wire_token::<DatumEventSeverityV1>(&row.severity)
        .map_err(|error| corrupt(format!("severity: {error}")))?;
    let peak_severity = parse_wire_token::<DatumEventSeverityV1>(&row.peak_severity)
        .map_err(|error| corrupt(format!("peak_severity: {error}")))?;
    let status = parse_wire_token::<DatumAlertStatusV1>(&row.status)
        .map_err(|error| corrupt(format!("status: {error}")))?;
    let occurrence_count = u64::try_from(row.occurrence_count)
        .map_err(|_| corrupt("occurrence_count is negative".to_string()))?;

    let actor = |actor_type: Option<String>,
                 actor_id: Option<String>,
                 model_ref: Option<String>,
                 label: &str| {
        if actor_type.is_none() && (actor_id.is_some() || model_ref.is_some()) {
            return Err(corrupt(format!("{label}: orphaned actor metadata")));
        }
        actor_type
            .map(|token| {
                parse_wire_token::<DatumAlertActorTypeV1>(&token)
                    .map(|actor_type| DatumAlertActorV1 {
                        actor_type,
                        actor_id,
                        model_ref,
                    })
                    .map_err(|error| corrupt(format!("{label}_by_type: {error}")))
            })
            .transpose()
    };

    let alert = DatumAlertV1 {
        schema: row.schema,
        alert_id: row.alert_id,
        project_id: row.project_id,
        application_id: row.application_id,
        correlation_key: row.correlation_key,
        category,
        kind: row.kind,
        severity,
        peak_severity,
        subject,
        status,
        first_observed_at_utc: row.first_observed_at_utc,
        last_observed_at_utc: row.last_observed_at_utc,
        first_event_id: row.first_event_id,
        latest_event_id: row.latest_event_id,
        occurrence_count,
        acknowledged_by: actor(
            row.acknowledged_by_type,
            row.acknowledged_by_id,
            row.acknowledged_by_model_ref,
            "acknowledged",
        )?,
        acknowledged_at_utc: row.acknowledged_at_utc,
        in_progress_by: actor(
            row.in_progress_by_type,
            row.in_progress_by_id,
            row.in_progress_by_model_ref,
            "in_progress",
        )?,
        in_progress_at_utc: row.in_progress_at_utc,
        resolved_by: actor(
            row.resolved_by_type,
            row.resolved_by_id,
            row.resolved_by_model_ref,
            "resolved",
        )?,
        resolved_at_utc: row.resolved_at_utc,
        resolution_reason: row.resolution_reason,
        closed_by: actor(
            row.closed_by_type,
            row.closed_by_id,
            row.closed_by_model_ref,
            "closed",
        )?,
        closed_at_utc: row.closed_at_utc,
        close_reason: row.close_reason,
    };

    let findings = alert.validate();
    if !findings.is_empty() {
        return Err(corrupt(format!(
            "stored alert failed validation: {findings:?}"
        )));
    }

    let latest_to_status: Option<String> = connection
        .query_row(
            "SELECT to_status FROM datum_alert_transitions WHERE alert_id = ?1 ORDER BY id DESC LIMIT 1",
            params![alert_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            DatumAlertStoreError::Storage(format!("read latest transition for '{alert_id}': {error}"))
        })?;
    match latest_to_status {
        None => {
            return Err(corrupt(
                "no transition history exists for this alert (the creation transition is missing)"
                    .to_string(),
            ));
        }
        Some(token) => {
            let latest_status = parse_wire_token::<DatumAlertStatusV1>(&token)
                .map_err(|error| corrupt(format!("latest transition to_status: {error}")))?;
            if latest_status != alert.status {
                return Err(corrupt(format!(
                    "alert.status ({:?}) disagrees with the latest transition's to_status ({latest_status:?})",
                    alert.status
                )));
            }
        }
    }

    verify_deep_consistency(connection, &alert)?;
    Ok(Some(alert))
}

fn enum_wire_str<T: Serialize>(value: &T) -> Result<String, DatumAlertStoreError> {
    match serde_json::to_value(value).map_err(|e| DatumAlertStoreError::Storage(e.to_string()))? {
        Value::String(token) => Ok(token),
        _ => Err(DatumAlertStoreError::Storage("expected string enum".into())),
    }
}

fn parse_wire_token<T: DeserializeOwned>(value: &str) -> Result<T, String> {
    serde_json::from_value(Value::String(value.into())).map_err(|e| e.to_string())
}

fn canonical_utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn alert_values(alert: &DatumAlertV1) -> Result<Vec<rusqlite::types::Value>, DatumAlertStoreError> {
    Ok(vec![
        alert.alert_id.clone().into(),
        alert.schema.clone().into(),
        alert.project_id.clone().into(),
        alert.application_id.clone().into(),
        alert.correlation_key.clone().into(),
        enum_wire_str(&alert.category)?.into(),
        alert.kind.clone().into(),
        enum_wire_str(&alert.severity)?.into(),
        enum_wire_str(&alert.peak_severity)?.into(),
        serde_json::to_string(&alert.subject)
            .map_err(|e| DatumAlertStoreError::Storage(e.to_string()))?
            .into(),
        enum_wire_str(&alert.status)?.into(),
        alert.first_observed_at_utc.clone().into(),
        alert.last_observed_at_utc.clone().into(),
        alert.first_event_id.clone().into(),
        alert.latest_event_id.clone().into(),
        i64::try_from(alert.occurrence_count)
            .map_err(|e| DatumAlertStoreError::Storage(e.to_string()))?
            .into(),
        alert
            .acknowledged_by
            .as_ref()
            .map(|a| enum_wire_str(&a.actor_type))
            .transpose()?
            .into(),
        alert
            .acknowledged_by
            .as_ref()
            .and_then(|a| a.actor_id.clone())
            .into(),
        alert
            .acknowledged_by
            .as_ref()
            .and_then(|a| a.model_ref.clone())
            .into(),
        alert.acknowledged_at_utc.clone().into(),
        alert
            .in_progress_by
            .as_ref()
            .map(|a| enum_wire_str(&a.actor_type))
            .transpose()?
            .into(),
        alert
            .in_progress_by
            .as_ref()
            .and_then(|a| a.actor_id.clone())
            .into(),
        alert
            .in_progress_by
            .as_ref()
            .and_then(|a| a.model_ref.clone())
            .into(),
        alert.in_progress_at_utc.clone().into(),
        alert
            .resolved_by
            .as_ref()
            .map(|a| enum_wire_str(&a.actor_type))
            .transpose()?
            .into(),
        alert
            .resolved_by
            .as_ref()
            .and_then(|a| a.actor_id.clone())
            .into(),
        alert
            .resolved_by
            .as_ref()
            .and_then(|a| a.model_ref.clone())
            .into(),
        alert.resolved_at_utc.clone().into(),
        alert.resolution_reason.clone().into(),
        alert
            .closed_by
            .as_ref()
            .map(|a| enum_wire_str(&a.actor_type))
            .transpose()?
            .into(),
        alert
            .closed_by
            .as_ref()
            .and_then(|a| a.actor_id.clone())
            .into(),
        alert
            .closed_by
            .as_ref()
            .and_then(|a| a.model_ref.clone())
            .into(),
        alert.closed_at_utc.clone().into(),
        alert.close_reason.clone().into(),
    ])
}

fn insert_new_alert_row(
    connection: &Connection,
    alert: &DatumAlertV1,
) -> Result<(), DatumAlertStoreError> {
    let values = alert_values(alert)?;
    connection.execute(
        &format!("INSERT INTO datum_alerts ({ALERT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34)"),
        rusqlite::params_from_iter(values),
    ).map_err(|e| DatumAlertStoreError::Storage(format!("insert alert: {e}")))?;
    Ok(())
}

fn update_alert_row(
    connection: &Connection,
    alert: &DatumAlertV1,
) -> Result<(), DatumAlertStoreError> {
    let values = alert_values(alert)?;
    connection.execute(
        "UPDATE datum_alerts SET schema = ?2, project_id = ?3, application_id = ?4, correlation_key = ?5, category = ?6, kind = ?7, severity = ?8, peak_severity = ?9, subject_json = ?10, status = ?11, first_observed_at_utc = ?12, last_observed_at_utc = ?13, first_event_id = ?14, latest_event_id = ?15, occurrence_count = ?16, acknowledged_by_type = ?17, acknowledged_by_id = ?18, acknowledged_by_model_ref = ?19, acknowledged_at_utc = ?20, in_progress_by_type = ?21, in_progress_by_id = ?22, in_progress_by_model_ref = ?23, in_progress_at_utc = ?24, resolved_by_type = ?25, resolved_by_id = ?26, resolved_by_model_ref = ?27, resolved_at_utc = ?28, resolution_reason = ?29, closed_by_type = ?30, closed_by_id = ?31, closed_by_model_ref = ?32, closed_at_utc = ?33, close_reason = ?34 WHERE alert_id = ?1",
        rusqlite::params_from_iter(values),
    ).map_err(|e| DatumAlertStoreError::Storage(format!("update alert: {e}")))?;
    Ok(())
}

fn insert_association(
    connection: &Connection,
    alert_id: &str,
    event_id: &str,
    now: &str,
) -> Result<(), DatumAlertStoreError> {
    connection.execute("INSERT INTO datum_alert_events (alert_id, event_id, associated_at_utc) VALUES (?1, ?2, ?3)",
        params![alert_id, event_id, now]).map_err(|e| DatumAlertStoreError::Storage(format!("associate event: {e}")))?;
    Ok(())
}

fn insert_transition(
    connection: &Connection,
    alert_id: &str,
    from: Option<DatumAlertStatusV1>,
    to: DatumAlertStatusV1,
    actor: &DatumAlertActorV1,
    reason: Option<&str>,
    now: &str,
    related_event_id: Option<&str>,
) -> Result<(), DatumAlertStoreError> {
    let findings = actor.validate();
    if !findings.is_empty() {
        return Err(DatumAlertStoreError::InvalidActor(findings));
    }
    let reason = normalize_reason(reason)?;
    connection.execute("INSERT INTO datum_alert_transitions
        (transition_id, alert_id, from_status, to_status, actor_type, actor_id, model_ref, reason, transitioned_at_utc, related_event_id)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![format!("alerttransition:{}", Uuid::new_v4()), alert_id, from.as_ref().map(enum_wire_str).transpose()?,
            enum_wire_str(&to)?, enum_wire_str(&actor.actor_type)?, actor.actor_id, actor.model_ref, reason, now, related_event_id])
        .map_err(|e| DatumAlertStoreError::Storage(format!("insert transition: {e}")))?;
    Ok(())
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let connection = Connection::open(path).map_err(|e| e.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(30))
        .map_err(|e| e.to_string())?;
    let _guard = crate::storage::schema_init::SCHEMA_INIT_LOCK
        .lock()
        .map_err(|_| "SQLite schema-init lock poisoned".to_string())?;
    connection
        .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;")
        .map_err(|e| e.to_string())?;
    datum_event_store::ensure_datum_events_schema(&connection)?;
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS datum_alerts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            alert_id TEXT NOT NULL UNIQUE, schema TEXT NOT NULL,
            project_id TEXT, application_id TEXT, correlation_key TEXT NOT NULL,
            category TEXT NOT NULL, kind TEXT NOT NULL, severity TEXT NOT NULL,
            peak_severity TEXT NOT NULL, subject_json TEXT NOT NULL, status TEXT NOT NULL,
            first_observed_at_utc TEXT NOT NULL, last_observed_at_utc TEXT NOT NULL,
            first_event_id TEXT NOT NULL REFERENCES datum_events(event_id),
            latest_event_id TEXT NOT NULL REFERENCES datum_events(event_id),
            occurrence_count INTEGER NOT NULL CHECK(occurrence_count > 0),
            acknowledged_by_type TEXT, acknowledged_by_id TEXT, acknowledged_by_model_ref TEXT, acknowledged_at_utc TEXT,
            in_progress_by_type TEXT, in_progress_by_id TEXT, in_progress_by_model_ref TEXT, in_progress_at_utc TEXT,
            resolved_by_type TEXT, resolved_by_id TEXT, resolved_by_model_ref TEXT, resolved_at_utc TEXT, resolution_reason TEXT,
            closed_by_type TEXT, closed_by_id TEXT, closed_by_model_ref TEXT, closed_at_utc TEXT, close_reason TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_datum_alerts_active_correlation_key
            ON datum_alerts(correlation_key) WHERE status != 'closed';
        CREATE INDEX IF NOT EXISTS idx_datum_alerts_project_status ON datum_alerts(project_id, status, id DESC);
        CREATE INDEX IF NOT EXISTS idx_datum_alerts_correlation_key ON datum_alerts(correlation_key, id DESC);
        CREATE TABLE IF NOT EXISTS datum_alert_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            alert_id TEXT NOT NULL REFERENCES datum_alerts(alert_id),
            event_id TEXT NOT NULL REFERENCES datum_events(event_id),
            associated_at_utc TEXT NOT NULL, UNIQUE(alert_id, event_id)
        );
        CREATE INDEX IF NOT EXISTS idx_datum_alert_events_alert ON datum_alert_events(alert_id, id);
        CREATE TABLE IF NOT EXISTS datum_alert_transitions (
            id INTEGER PRIMARY KEY AUTOINCREMENT, transition_id TEXT NOT NULL UNIQUE,
            alert_id TEXT NOT NULL REFERENCES datum_alerts(alert_id),
            from_status TEXT, to_status TEXT NOT NULL, actor_type TEXT NOT NULL,
            actor_id TEXT, model_ref TEXT, reason TEXT,
            transitioned_at_utc TEXT NOT NULL,
            related_event_id TEXT REFERENCES datum_events(event_id)
        );
        CREATE INDEX IF NOT EXISTS idx_datum_alert_transitions_alert ON datum_alert_transitions(alert_id, id);"
    ).map_err(|e| format!("initialize alert schema: {e}"))?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::datum_event::{DatumEventSourceV1, DatumEventV1, DATUM_EVENT_SCHEMA};
    use crate::storage::datum_event_store::{admit_event, DatumEventAdmission};
    use std::sync::{Arc, Barrier};
    use DatumEventSeverityV1::{Critical, Warning};

    struct TestDb(std::path::PathBuf);
    impl TestDb {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("phase124-{}.db", Uuid::new_v4())))
        }
    }
    impl Drop for TestDb {
        fn drop(&mut self) {
            for path in [
                self.0.clone(),
                self.0.with_extension("db-wal"),
                self.0.with_extension("db-shm"),
            ] {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    fn event(id: &str, time: &str, severity: DatumEventSeverityV1) -> DatumEventV1 {
        DatumEventV1 {
            schema: DATUM_EVENT_SCHEMA.into(),
            event_id: id.into(),
            project_id: Some("proj:test".into()),
            application_id: None,
            category: DatumEventCategoryV1::Observation,
            kind: "telemetry_stale".into(),
            severity,
            source: DatumEventSourceV1::SentinelTelemetry,
            subject: DatumEventSubjectV1::Project {
                project_id: "proj:test".into(),
            },
            occurred_at_utc: time.into(),
            evidence_refs: vec![],
            correlation_hint: None,
            confidence_permille: None,
            model_ref: None,
            attributes: serde_json::json!({}),
        }
    }
    fn admit(path: &Path, e: &DatumEventV1) -> StoredDatumEvent {
        match admit_event(path, e).unwrap() {
            DatumEventAdmission::Inserted(e) | DatumEventAdmission::IdempotentReplay(e) => e,
        }
    }
    fn process(
        path: &Path,
        id: &str,
        time: &str,
        severity: DatumEventSeverityV1,
    ) -> AlertProcessingOutcome {
        process_event(path, &admit(path, &event(id, time, severity))).unwrap()
    }
    fn initial(path: &Path) -> DatumAlertV1 {
        process(path, "first", "2020-01-01T12:00:00Z", Warning)
            .alert()
            .clone()
    }
    fn shift(time: &str, seconds: i64) -> String {
        (chrono::DateTime::parse_from_rfc3339(time).unwrap() + chrono::Duration::seconds(seconds))
            .with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::AutoSi, true)
    }
    fn corrupt(result: Result<Option<DatumAlertV1>, DatumAlertStoreError>) {
        assert!(
            matches!(result, Err(DatumAlertStoreError::CorruptStoredAlert { .. })),
            "{result:?}"
        );
    }

    #[test]
    fn out_of_order_critical_then_warning_and_reverse_have_identical_aggregation() {
        let mut results = vec![];
        for reverse in [false, true] {
            let db = TestDb::new();
            let events = [
                event("latest", "2020-01-01T12:02:00Z", Critical),
                event("earliest", "2020-01-01T12:00:00Z", Warning),
            ];
            for i in if reverse { [1, 0] } else { [0, 1] } {
                process_event(&db.0, &admit(&db.0, &events[i])).unwrap();
            }
            let a = list_alerts(&db.0, &AlertQuery::default())
                .unwrap()
                .remove(0);
            assert_eq!((a.severity, a.peak_severity), (Critical, Critical));
            assert_eq!(a.first_event_id, "earliest");
            assert_eq!(a.latest_event_id, "latest");
            assert_eq!(a.first_observed_at_utc, events[1].occurred_at_utc);
            assert_eq!(a.last_observed_at_utc, events[0].occurred_at_utc);
            results.push((
                a.severity,
                a.peak_severity,
                a.first_event_id,
                a.latest_event_id,
                a.first_observed_at_utc,
                a.last_observed_at_utc,
                a.occurrence_count,
            ));
        }
        assert_eq!(results[0], results[1]);
    }

    #[test]
    fn late_critical_changes_peak_but_not_current_warning() {
        let db = TestDb::new();
        process(&db.0, "latest", "2020-01-01T12:02:00Z", Warning);
        let a = process(&db.0, "earliest", "2020-01-01T12:00:00Z", Critical)
            .alert()
            .clone();
        assert_eq!((a.severity, a.peak_severity), (Warning, Critical));
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(a));
    }

    #[test]
    fn fractional_seconds_and_event_id_ties_are_chronological() {
        for reverse in [false, true] {
            let db = TestDb::new();
            let ids = if reverse { ["z", "a"] } else { ["a", "z"] };
            for id in ids {
                process(&db.0, id, "3000-01-01T12:00:00Z", Warning);
            }
            let a = process(&db.0, "fraction", "3000-01-01T12:00:00.001Z", Critical)
                .alert()
                .clone();
            assert_eq!(a.first_event_id, "a");
            assert_eq!(a.latest_event_id, "fraction");
            assert_eq!(a.severity, Critical);
            assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(a));
        }
    }

    #[test]
    fn resolved_pre_boundary_and_equal_events_do_not_reopen_post_boundary_does() {
        let db = TestDb::new();
        let a = initial(&db.0);
        let resolved = resolve_alert(
            &db.0,
            &a.alert_id,
            DatumAlertActorV1::system(),
            Some("fixed"),
        )
        .unwrap();
        let at = resolved.resolved_at_utc.as_ref().unwrap();
        for (id, time) in [("late", shift(at, -1)), ("equal", at.clone())] {
            let outcome = process(&db.0, id, &time, Critical);
            assert!(matches!(outcome, AlertProcessingOutcome::LateAssociated(_)));
            assert_eq!(outcome.alert().resolved_by, resolved.resolved_by);
            assert_eq!(
                outcome.alert().resolution_reason,
                resolved.resolution_reason
            );
        }
        assert_eq!(
            list_alert_transitions(&db.0, &a.alert_id, 100)
                .unwrap()
                .len(),
            2
        );
        let recurrence = process(&db.0, "recurrence", &shift(at, 1), Warning);
        assert!(matches!(recurrence, AlertProcessingOutcome::Reopened(_)));
        assert_eq!(recurrence.alert().alert_id, a.alert_id);
        assert_eq!(recurrence.alert().resolved_by, None);
        assert_eq!(recurrence.alert().resolved_at_utc, None);
        assert_eq!(recurrence.alert().resolution_reason, None);
        assert_eq!(
            get_alert(&db.0, &a.alert_id).unwrap().unwrap().status,
            DatumAlertStatusV1::Open
        );
    }

    #[test]
    fn closed_late_evidence_preserves_lifecycle_and_post_close_creates_new_episode() {
        let db = TestDb::new();
        let a = initial(&db.0);
        resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        let closed = close_alert(
            &db.0,
            &a.alert_id,
            DatumAlertActorV1::system(),
            Some("done"),
        )
        .unwrap();
        let at = closed.closed_at_utc.as_ref().unwrap();
        let history = list_alert_transitions(&db.0, &a.alert_id, 100).unwrap();
        for (id, time) in [("late", shift(at, -1)), ("equal", at.clone())] {
            let outcome = process(&db.0, id, &time, Critical);
            assert!(matches!(outcome, AlertProcessingOutcome::LateAssociated(_)));
            assert_eq!(outcome.alert().alert_id, a.alert_id);
            assert_eq!(outcome.alert().status, DatumAlertStatusV1::Closed);
            assert_eq!(outcome.alert().closed_by, closed.closed_by);
            assert_eq!(outcome.alert().closed_at_utc, closed.closed_at_utc);
        }
        let new = process(&db.0, "new", &shift(at, 1), Warning);
        assert!(matches!(new, AlertProcessingOutcome::Created(_)));
        assert_ne!(new.alert().alert_id, a.alert_id);
        assert_eq!(new.alert().occurrence_count, 1);
        // Historical routing takes precedence over the newer active episode.
        let old = process(&db.0, "older", "2019-01-01T00:00:00Z", Warning);
        assert_eq!(old.alert().alert_id, a.alert_id);
        assert_eq!(old.alert().occurrence_count, 4);
        assert_eq!(
            list_alert_transitions(&db.0, &a.alert_id, 100).unwrap(),
            history
        );
        assert_eq!(
            get_alert(&db.0, &a.alert_id).unwrap().unwrap().status,
            DatumAlertStatusV1::Closed
        );
    }

    #[test]
    fn same_event_same_alert_is_idempotent_even_after_closure() {
        let db = TestDb::new();
        let a = initial(&db.0);
        resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        let closed = close_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        let retry = process(&db.0, "first", "2020-01-01T12:00:00Z", Warning);
        assert_eq!(retry, AlertProcessingOutcome::AlreadyProcessed(closed));
        assert_eq!(list_alert_events(&db.0, &a.alert_id, 100).unwrap().len(), 1);
    }

    #[test]
    fn same_event_can_support_another_alert_but_pair_is_unique() {
        let db = TestDb::new();
        let mut a = initial(&db.0);
        let connection = open_database(&db.0).unwrap();
        a.alert_id = "second-condition".into();
        a.correlation_key = "independent-condition".into();
        insert_new_alert_row(&connection, &a).unwrap();
        insert_transition(
            &connection,
            &a.alert_id,
            None,
            DatumAlertStatusV1::Open,
            &DatumAlertActorV1::system(),
            None,
            &canonical_utc_now(),
            Some("first"),
        )
        .unwrap();
        insert_association(&connection, &a.alert_id, "first", &canonical_utc_now()).unwrap();
        assert!(
            insert_association(&connection, &a.alert_id, "first", &canonical_utc_now()).is_err()
        );
        drop(connection);
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(a));
    }

    macro_rules! corrupt_column_test {
        ($name:ident, $sql:expr) => {
            #[test]
            fn $name() {
                let db = TestDb::new();
                let a = initial(&db.0);
                process(&db.0, "last", "2020-01-01T12:02:00Z", Critical);
                Connection::open(&db.0)
                    .unwrap()
                    .execute($sql, params![a.alert_id])
                    .unwrap();
                corrupt(get_alert(&db.0, &a.alert_id));
                assert!(matches!(
                    list_alerts(&db.0, &AlertQuery::default()),
                    Err(DatumAlertStoreError::CorruptStoredAlert { .. })
                ));
                assert!(matches!(
                    resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None),
                    Err(DatumAlertStoreError::CorruptStoredAlert { .. })
                ));
            }
        };
    }
    corrupt_column_test!(
        corrupt_occurrence_count_detected,
        "UPDATE datum_alerts SET occurrence_count=1001 WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_first_event_id_detected,
        "UPDATE datum_alerts SET first_event_id='last' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_latest_event_id_detected,
        "UPDATE datum_alerts SET latest_event_id='first' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_first_observed_detected,
        "UPDATE datum_alerts SET first_observed_at_utc='2019-01-01T00:00:00Z' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_last_observed_detected,
        "UPDATE datum_alerts SET last_observed_at_utc='2021-01-01T00:00:00Z' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_current_severity_detected,
        "UPDATE datum_alerts SET severity='warning' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_peak_severity_detected,
        "UPDATE datum_alerts SET peak_severity='warning' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_status_history_detected,
        "UPDATE datum_alert_transitions SET to_status='resolved' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        missing_creation_detected,
        "DELETE FROM datum_alert_transitions WHERE alert_id=?1"
    );
    corrupt_column_test!(
        non_creation_first_transition_detected,
        "UPDATE datum_alert_transitions SET from_status='resolved' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        corrupt_transition_actor_detected,
        "UPDATE datum_alert_transitions SET actor_id='   ' WHERE alert_id=?1"
    );
    corrupt_column_test!(
        orphaned_actor_model_detected,
        "UPDATE datum_alerts SET acknowledged_by_model_ref='model:v1' WHERE alert_id=?1"
    );

    #[test]
    fn actor_validation_prevents_persistence_and_model_provenance_round_trips() {
        let db = TestDb::new();
        let a = initial(&db.0);
        let invalid = [
            DatumAlertActorV1 {
                actor_type: DatumAlertActorTypeV1::Operator,
                actor_id: Some("  ".into()),
                model_ref: None,
            },
            DatumAlertActorV1 {
                actor_type: DatumAlertActorTypeV1::Other(" bad ".into()),
                actor_id: None,
                model_ref: None,
            },
            DatumAlertActorV1 {
                actor_type: DatumAlertActorTypeV1::Gnn,
                actor_id: None,
                model_ref: None,
            },
            DatumAlertActorV1 {
                actor_type: DatumAlertActorTypeV1::Llm,
                actor_id: None,
                model_ref: None,
            },
            DatumAlertActorV1 {
                actor_type: DatumAlertActorTypeV1::Llm,
                actor_id: None,
                model_ref: Some(" ".into()),
            },
        ];
        for actor in invalid {
            assert!(matches!(
                acknowledge_alert(&db.0, &a.alert_id, actor, None),
                Err(DatumAlertStoreError::InvalidActor(_))
            ));
        }
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(a.clone()));
        let gnn = DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Gnn,
            actor_id: Some("worker".into()),
            model_ref: Some("gnn:v1".into()),
        };
        let llm = DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Llm,
            actor_id: Some("agent".into()),
            model_ref: Some("llm:v2".into()),
        };
        acknowledge_alert(&db.0, &a.alert_id, gnn.clone(), None).unwrap();
        start_alert_work(&db.0, &a.alert_id, llm.clone(), None).unwrap();
        resolve_alert(&db.0, &a.alert_id, gnn.clone(), Some("resolved")).unwrap();
        let closed = close_alert(&db.0, &a.alert_id, llm.clone(), Some("closed")).unwrap();
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(closed));
        let history = list_alert_transitions(&db.0, &a.alert_id, 100).unwrap();
        assert_eq!(
            history
                .iter()
                .skip(1)
                .map(|t| t.actor.clone())
                .collect::<Vec<_>>(),
            vec![gnn.clone(), llm.clone(), gnn, llm]
        );
    }

    #[test]
    fn lifecycle_retries_and_skipped_start_are_invalid_closed_is_terminal() {
        let db = TestDb::new();
        let a = initial(&db.0);
        assert!(matches!(
            start_alert_work(&db.0, &a.alert_id, DatumAlertActorV1::system(), None),
            Err(DatumAlertStoreError::InvalidTransition { .. })
        ));
        acknowledge_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        assert!(matches!(
            acknowledge_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None),
            Err(DatumAlertStoreError::InvalidTransition { .. })
        ));
        start_alert_work(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        assert!(matches!(
            start_alert_work(&db.0, &a.alert_id, DatumAlertActorV1::system(), None),
            Err(DatumAlertStoreError::InvalidTransition { .. })
        ));
        resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        assert!(matches!(
            resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None),
            Err(DatumAlertStoreError::InvalidTransition { .. })
        ));
        let closed = close_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
        for command in [
            acknowledge_alert,
            start_alert_work,
            resolve_alert,
            close_alert,
        ] {
            assert!(matches!(
                command(&db.0, &a.alert_id, DatumAlertActorV1::system(), None),
                Err(DatumAlertStoreError::InvalidTransition { .. })
            ));
        }
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(closed));
    }

    #[test]
    fn reasons_are_bounded_by_utf8_bytes_for_every_command() {
        let db = TestDb::new();
        let a = initial(&db.0);
        let oversized = "é".repeat(2049);
        for command in [
            acknowledge_alert,
            start_alert_work,
            resolve_alert,
            close_alert,
        ] {
            assert!(matches!(
                command(
                    &db.0,
                    &a.alert_id,
                    DatumAlertActorV1::system(),
                    Some(&oversized)
                ),
                Err(DatumAlertStoreError::ReasonTooLong { .. })
            ));
        }
        let exact = "é".repeat(2048);
        acknowledge_alert(
            &db.0,
            &a.alert_id,
            DatumAlertActorV1::system(),
            Some(&exact),
        )
        .unwrap();
        assert_eq!(
            list_alert_transitions(&db.0, &a.alert_id, 100).unwrap()[1]
                .reason
                .as_deref(),
            Some(exact.as_str())
        );
        start_alert_work(&db.0, &a.alert_id, DatumAlertActorV1::system(), Some("   ")).unwrap();
        assert_eq!(
            list_alert_transitions(&db.0, &a.alert_id, 100).unwrap()[2].reason,
            None
        );
    }

    #[test]
    fn restart_durability_all_three_tables_and_direct_resolve_close() {
        let db = TestDb::new();
        let a = initial(&db.0);
        resolve_alert(
            &db.0,
            &a.alert_id,
            DatumAlertActorV1::system(),
            Some("fixed"),
        )
        .unwrap();
        let closed = close_alert(
            &db.0,
            &a.alert_id,
            DatumAlertActorV1::system(),
            Some("done"),
        )
        .unwrap();
        // Each public call drops its connection. Nothing is cached across calls.
        let transitions = list_alert_transitions(&db.0, &a.alert_id, 100).unwrap();
        let associations = list_alert_events(&db.0, &a.alert_id, 100).unwrap();
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(closed));
        assert_eq!(
            list_alert_transitions(&db.0, &a.alert_id, 100).unwrap(),
            transitions
        );
        assert_eq!(
            list_alert_events(&db.0, &a.alert_id, 100).unwrap(),
            associations
        );
        assert_eq!(transitions.len(), 3);
        assert_eq!(associations.len(), 1);
    }

    #[test]
    fn concurrent_event_and_alert_first_touch_fresh_database() {
        for _ in 0..5 {
            let db = TestDb::new();
            assert!(!db.0.exists());
            let barrier = Arc::new(Barrier::new(8));
            let handles: Vec<_> = (0..8)
                .map(|i| {
                    let path = db.0.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        if i % 2 == 0 {
                            datum_event_store::get_event(&path, "absent").unwrap();
                        } else {
                            get_alert(&path, "absent").unwrap();
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
            let a = initial(&db.0);
            assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(a));
        }
    }

    #[test]
    fn concurrent_occurrences_and_replays_are_serialized() {
        let db = TestDb::new();
        let events: Vec<_> = (0..8)
            .map(|i| {
                admit(
                    &db.0,
                    &event(&format!("event-{i}"), "2020-01-01T12:00:00Z", Warning),
                )
            })
            .collect();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = events
            .into_iter()
            .map(|e| {
                let path = db.0.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = process_event(&path, &e).unwrap();
                    assert!(matches!(
                        process_event(&path, &e).unwrap(),
                        AlertProcessingOutcome::AlreadyProcessed(_)
                    ));
                    result
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, AlertProcessingOutcome::Created(_)))
                .count(),
            1
        );
        let alerts = list_alerts(&db.0, &AlertQuery::default()).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].occurrence_count, 8);
    }

    #[test]
    fn concurrent_conflicting_lifecycle_commands_have_one_winner() {
        let db = TestDb::new();
        let a = initial(&db.0);
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = db.0.clone();
                let id = a.alert_id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    acknowledge_alert(&path, &id, DatumAlertActorV1::system(), None)
                })
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|r| matches!(r, Err(DatumAlertStoreError::InvalidTransition { .. })))
                .count(),
            7
        );
        assert_eq!(
            list_alert_transitions(&db.0, &a.alert_id, 100)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn integrity_verification_is_not_limited_to_public_query_page_size() {
        let db = TestDb::new();
        let mut a = initial(&db.0);
        let mut events = Vec::new();
        for i in 0..1001 {
            events.push(admit(
                &db.0,
                &event(&format!("bulk-{i:04}"), "2020-01-01T12:01:00Z", Critical),
            ));
        }
        let mut connection = open_database(&db.0).unwrap();
        let tx = connection.transaction().unwrap();
        for e in &events {
            apply_chronological_aggregation(&mut a, &e.event).unwrap();
            insert_association(&tx, &a.alert_id, &e.event.event_id, &canonical_utc_now()).unwrap();
        }
        // Valid history beyond 1,000 records, ending Open with resolution cleared.
        for _ in 0..501 {
            insert_transition(
                &tx,
                &a.alert_id,
                Some(DatumAlertStatusV1::Open),
                DatumAlertStatusV1::Resolved,
                &DatumAlertActorV1::system(),
                None,
                &canonical_utc_now(),
                None,
            )
            .unwrap();
            insert_transition(
                &tx,
                &a.alert_id,
                Some(DatumAlertStatusV1::Resolved),
                DatumAlertStatusV1::Open,
                &DatumAlertActorV1::system(),
                None,
                &canonical_utc_now(),
                Some("first"),
            )
            .unwrap();
        }
        update_alert_row(&tx, &a).unwrap();
        tx.commit().unwrap();
        drop(connection);
        assert_eq!(get_alert(&db.0, &a.alert_id).unwrap(), Some(a.clone()));
        Connection::open(&db.0).unwrap().execute(
            "UPDATE datum_alert_transitions SET actor_id=' ' WHERE id=(SELECT MAX(id) FROM datum_alert_transitions)", []
        ).unwrap();
        corrupt(get_alert(&db.0, &a.alert_id));
    }

    #[test]
    fn equal_timestamp_latest_id_determines_current_severity() {
        for reverse in [false, true] {
            let db = TestDb::new();
            let events = [
                event("a", "2020-01-01T00:00:00Z", Critical),
                event("z", "2020-01-01T00:00:00Z", Warning),
            ];
            for i in if reverse { [1, 0] } else { [0, 1] } {
                process_event(&db.0, &admit(&db.0, &events[i])).unwrap();
            }
            let a = list_alerts(&db.0, &AlertQuery::default())
                .unwrap()
                .remove(0);
            assert_eq!(
                (a.first_event_id.as_str(), a.latest_event_id.as_str()),
                ("a", "z")
            );
            assert_eq!((a.severity, a.peak_severity), (Warning, Critical));
        }
    }

    #[test]
    fn database_enforces_active_uniqueness_and_foreign_keys() {
        let db = TestDb::new();
        let mut a = initial(&db.0);
        let connection = open_database(&db.0).unwrap();
        a.alert_id = "duplicate".into();
        assert!(insert_new_alert_row(&connection, &a).is_err());
        assert!(insert_association(&connection, "absent", "first", &canonical_utc_now()).is_err());
        assert!(
            insert_association(&connection, &a.alert_id, "absent", &canonical_utc_now()).is_err()
        );
        assert!(insert_transition(
            &connection,
            "absent",
            None,
            DatumAlertStatusV1::Open,
            &DatumAlertActorV1::system(),
            None,
            &canonical_utc_now(),
            None
        )
        .is_err());
        let index: String = connection.query_row(
            "SELECT sql FROM sqlite_master WHERE name='idx_datum_alerts_active_correlation_key'",
            [], |r| r.get(0)).unwrap();
        assert!(index.contains(&format!("status != '{CLOSED_STATUS_WIRE_TOKEN}'")));
    }

    #[test]
    fn query_filters_limits_and_association_order_remain_explicit() {
        let db = TestDb::new();
        let a = process(&db.0, "later", "2020-01-01T12:02:00Z", Critical)
            .alert()
            .clone();
        process(&db.0, "earlier", "2020-01-01T12:00:00Z", Warning);
        let evidence = list_alert_events(&db.0, &a.alert_id, 100).unwrap();
        assert_eq!(
            evidence
                .iter()
                .map(|e| e.event.event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["later", "earlier"]
        );
        assert_eq!(
            list_alerts(
                &db.0,
                &AlertQuery {
                    project_filter: ProjectFilter::Project("proj:test"),
                    severity: Some(Critical),
                    status: Some(DatumAlertStatusV1::Open),
                    correlation_key: Some(&a.correlation_key),
                    kind: Some("telemetry_stale"),
                    active_only: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .len(),
            1
        );
        assert!(list_alerts(
            &db.0,
            &AlertQuery {
                project_filter: ProjectFilter::GlobalOnly,
                ..Default::default()
            }
        )
        .unwrap()
        .is_empty());
        for limit in [0, MAX_QUERY_LIMIT + 1] {
            assert!(matches!(
                list_alerts(
                    &db.0,
                    &AlertQuery {
                        limit,
                        ..Default::default()
                    }
                ),
                Err(DatumAlertStoreError::InvalidQuery(_))
            ));
            assert!(matches!(
                list_alert_events(&db.0, &a.alert_id, limit),
                Err(DatumAlertStoreError::InvalidQuery(_))
            ));
            assert!(matches!(
                list_alert_transitions(&db.0, &a.alert_id, limit),
                Err(DatumAlertStoreError::InvalidQuery(_))
            ));
        }
    }

    #[test]
    fn forged_stored_event_is_not_accepted() {
        let db = TestDb::new();
        let mut stored = admit(&db.0, &event("forged", "2020-01-01T12:00:00Z", Warning));
        stored.event.severity = Critical;
        assert!(process_event(&db.0, &stored).is_err());
        assert!(list_alerts(&db.0, &AlertQuery::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn missing_checkpoint_and_broken_transition_chain_fail_closed() {
        for sql in [
            "UPDATE datum_alerts SET acknowledged_by_type=NULL, acknowledged_at_utc=NULL WHERE alert_id=?1",
            "UPDATE datum_alert_transitions SET from_status='in_progress' WHERE alert_id=?1 AND to_status='resolved'",
            "UPDATE datum_alerts SET resolution_reason='changed' WHERE alert_id=?1",
        ] {
            let db = TestDb::new(); let a = initial(&db.0);
            acknowledge_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
            resolve_alert(&db.0, &a.alert_id, DatumAlertActorV1::system(), None).unwrap();
            Connection::open(&db.0).unwrap().execute(sql, params![a.alert_id]).unwrap();
            corrupt(get_alert(&db.0, &a.alert_id));
        }
    }
}
