//! Phase 123 -- the durable, append-only Event Store for the frozen
//! `datum.event/1` contract (`core::datum_event`).
//!
//! # Scope
//!
//! This module stores and retrieves `DatumEventV1` occurrences. It does
//! NOT implement `DatumAlert`, alert lifecycle, condition-level
//! deduplication, correlation, an active-monitoring scheduler, producer
//! wiring, or any HTTP API -- all of that is later-phase work. See
//! `documentation/reference/phase123-durable-datum-event-store.md` for the
//! full architectural rationale, including why a dedicated table (not the
//! generic `storage::control_state` JSON-blob domain model) was chosen,
//! and the Phase 123 hardening pass that added the numeric sortable
//! timestamp columns and denormalized-column consistency checking
//! described below.
//!
//! # Core semantic rule: append-only
//!
//! A `DatumEventV1` is immutable by contract (Phase 122). This store
//! enforces that at the persistence layer: there is no UPDATE path and no
//! DELETE path anywhere in this module -- `grep`-verifiable, and pinned by
//! `no_update_or_delete_api_exists_for_admitted_events` below. The only
//! mutation this module ever performs is a single `INSERT` per genuinely
//! new `event_id`.
//!
//! # Durability -- no volatile fallback
//!
//! Every function here takes an explicit `db_path: &Path`. There is no
//! `Option<PathBuf>` anywhere in this module's public surface, and no
//! in-memory fallback branch exists to fall back to -- unlike the
//! Phase 121-audited `OperationalEvidenceEvent` API path (which silently
//! degrades to an in-process `RwLock<BTreeMap>` when
//! `DATUM_CONTROL_STATE_DB_PATH` is unset), this module structurally
//! cannot do that: durability is guaranteed by the absence of any
//! non-durable code path, not by a runtime check. This is STORE-LEVEL
//! durability only -- see "Runtime integration durability" below for the
//! explicit, deliberately unresolved DSERVER-INTEGRATION question this
//! leaves for Phase 126.
//!
//! # Why `occurred_at_utc` is never sorted or range-compared as TEXT
//!
//! Phase 122's canonical timestamp form (`SecondsFormat::AutoSi`) omits
//! the fractional-second part entirely when it is exactly zero, rather
//! than padding to a fixed width. This means plain lexicographic TEXT
//! comparison of two canonical timestamps is **not** chronologically
//! correct in general: `"2026-09-29T12:00:00.100Z"` (100ms past the
//! second) sorts lexicographically *before* `"2026-09-29T12:00:00Z"`
//! (exactly the whole second, chronologically *earlier*), because the
//! byte `'.'` (0x2E) is less than `'Z'` (0x5A) at the first position the
//! two strings differ. This was found and proven (not assumed) during an
//! independent adversarial review of the initial Phase 123
//! implementation, which shipped with -- and never tested -- exactly this
//! defect in both its `ORDER BY` and its time-range filters.
//!
//! The fix: every admitted row also stores a numeric, chronologically
//! correct sortable representation, derived once (see
//! [`canonicalize_and_derive_sortable`]) from the same canonical
//! `occurred_at_utc` string Phase 122 already validated, and used for
//! every ordering and range comparison instead of the TEXT column. The
//! TEXT column remains -- unchanged, still the canonical Phase 122 field
//! -- purely as descriptive event content, never as an ordering
//! primitive.
//!
//! ## Why two columns, not one `epoch_nanos` column
//!
//! A single combined `i64` nanoseconds-since-epoch value was considered
//! and rejected after proving it unsafe, not assumed safe: `chrono`'s
//! `NaiveDate` (and therefore `DateTime<Utc>`) represents dates across a
//! range of roughly +-262,143 years (bounded by chrono's internal `i32`
//! day-number representation). `i64` nanoseconds since the Unix epoch, by
//! contrast, overflows for any instant outside roughly 1677-09-21 to
//! 2262-04-11 (`i64::MAX` nanoseconds is only ~292 years) -- a range
//! chrono itself can represent well beyond, and one `DatumEventV1::validate()`
//! does **not** restrict (Phase 122 validates canonical *form*, not
//! *year range*). A canonical, Phase-122-valid event dated e.g.
//! `"3000-01-01T00:00:00Z"` would silently overflow a combined
//! nanoseconds-since-epoch `i64`.
//!
//! The chosen representation instead splits into two ordinary SQLite
//! `INTEGER` (i64) columns: `occurred_at_epoch_seconds` (whole seconds
//! since the Unix epoch, via `chrono::DateTime::timestamp`, which can be
//! negative for pre-1970 instants) and `occurred_at_subsec_nanos` (the
//! non-negative forward sub-second offset, `0..=999_999_999`, via
//! `chrono::DateTime::timestamp_subsec_nanos`). Ordering/comparing the
//! `(seconds, nanos)` pair lexicographically is chronologically correct
//! and, since `i64` seconds-since-epoch covers roughly 292 BILLION years
//! in each direction -- about 1.1 million times chrono's own entire
//! representable range -- this has no realistic overflow exposure at all.
//! See the Phase 123 hardening report for the exact figures.

use std::{fs, path::Path, time::Duration};

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, ToSql, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::datum_event::{
    recompute_datum_event_digest, DatumEventCategoryV1, DatumEventSeverityV1, DatumEventV1,
};

/// Default page size when a caller does not specify one, and the ceiling
/// no query may exceed. Matches the values `api/operational_evidence.rs`
/// already established (`unwrap_or(100).clamp(1, 1000)`) for repository
/// consistency -- not chosen independently.
pub const DEFAULT_QUERY_LIMIT: u32 = 100;
pub const MAX_QUERY_LIMIT: u32 = 1000;

/// A durably stored `DatumEventV1`, alongside the storage metadata that is
/// deliberately NOT part of the canonical envelope itself (Phase 122 left
/// `admitted_at`/`stored_at` for this phase to own, precisely so it would
/// live here and not on `DatumEventV1`).
#[derive(Debug, Clone, PartialEq)]
pub struct StoredDatumEvent {
    pub event: DatumEventV1,
    /// The canonical Phase 122 digest (`recompute_datum_event_digest`),
    /// recomputed -- never trusted from a caller -- at admission time and
    /// re-verified against the stored `event_json` on every read.
    pub event_digest: String,
    /// When DServer durably admitted this occurrence -- distinct from
    /// `event.occurred_at_utc` ("when the occurrence happened"). Canonical
    /// UTC RFC 3339, using the same `chrono::SecondsFormat::AutoSi` + `Z`
    /// convention Phase 122 established for `occurred_at_utc`, so the two
    /// timestamps are textually comparable. Preserved exactly across
    /// idempotent replay -- never refreshed (see
    /// `idempotent_replay_preserves_the_original_admitted_at_utc` below).
    pub admitted_at_utc: String,
}

/// The outcome of [`admit_event`] -- deliberately NOT collapsed into one
/// opaque success, per the Phase 123 brief: a Phase 126 producer needs to
/// know whether its submission was newly recorded or was a replay of an
/// occurrence already durably admitted.
#[derive(Debug, Clone, PartialEq)]
pub enum DatumEventAdmission {
    /// A genuinely new `event_id` was durably inserted.
    Inserted(StoredDatumEvent),
    /// `event_id` was already admitted with an identical canonical digest
    /// -- idempotent replay of the SAME occurrence. No row was written;
    /// the already-stored record (including its ORIGINAL `admitted_at_utc`)
    /// is returned.
    IdempotentReplay(StoredDatumEvent),
}

/// Typed errors for the Event Store's public API. Deliberately not
/// `String` -- a caller (a future Phase 126 producer, or a future HTTP
/// layer) needs to branch on these, not parse error text.
#[derive(Debug, Clone, PartialEq)]
pub enum DatumEventStoreError {
    /// `DatumEventV1::validate()` returned non-empty findings. The event
    /// was never written.
    InvalidEvent(Vec<CanonicalValidationFinding>),
    /// A SQLite/IO-level failure unrelated to the event's own validity.
    Storage(String),
    /// `event_id` was already admitted with a DIFFERENT canonical digest
    /// than the one just submitted -- fails closed, per the Phase 123
    /// brief's Case B. The conflicting submission was never written; the
    /// already-stored occurrence is left untouched.
    Conflict {
        event_id: String,
        existing_digest: String,
        submitted_digest: String,
    },
    /// A stored row failed one of the corruption checks on read (JSON
    /// deserialization, Phase 122 validation, digest verification, or --
    /// added in the Phase 123 hardening pass -- a mismatch between the
    /// decoded event and one of the denormalized filter columns) -- see
    /// [`get_event`]/[`list_events`]'s doc comments.
    CorruptStoredEvent { event_id: String, reason: String },
    /// A malformed query (e.g. `limit` out of range, an empty explicit
    /// project filter, a query timestamp that is not valid or not in
    /// Phase 122 canonical form, or a reversed time range).
    InvalidQuery(String),
}

impl std::fmt::Display for DatumEventStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DatumEventStoreError::InvalidEvent(findings) => {
                write!(f, "invalid DatumEvent, not admitted: {findings:?}")
            }
            DatumEventStoreError::Storage(message) => {
                write!(f, "datum event store storage error: {message}")
            }
            DatumEventStoreError::Conflict {
                event_id,
                existing_digest,
                submitted_digest,
            } => write!(
                f,
                "datum event '{event_id}' conflict: already admitted with digest '{existing_digest}', submission carried digest '{submitted_digest}'"
            ),
            DatumEventStoreError::CorruptStoredEvent { event_id, reason } => {
                write!(f, "corrupt stored datum event '{event_id}': {reason}")
            }
            DatumEventStoreError::InvalidQuery(message) => {
                write!(f, "invalid datum event query: {message}")
            }
        }
    }
}

impl std::error::Error for DatumEventStoreError {}

/// The Phase 123 brief's explicit instruction: "the store must support
/// `project_id = NULL` without treating NULL as invalid... queries must be
/// able to distinguish: all events / global-only events / specific
/// project events. Do not collapse global events into a fake project ID."
/// This three-way enum is what makes that a type-level guarantee rather
/// than an `Option<&str>` convention a caller could get wrong (e.g.
/// mistaking `None` for "global only" when it actually means "any
/// project").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProjectFilter<'a> {
    /// No project filter at all -- events from every project AND global
    /// events.
    #[default]
    Any,
    /// Only events with `project_id IS NULL`.
    GlobalOnly,
    /// Only events with `project_id = `the given value. An empty/blank
    /// string is rejected (`InvalidQuery`), not silently treated as
    /// `Any`/`GlobalOnly`.
    Project(&'a str),
}

/// A bounded, deterministically-ordered query against the Event Store.
/// Deliberately a small, fixed filter set (not a query DSL) -- exactly
/// enough to give Phase 127 a foundation, per the Phase 123 brief.
#[derive(Debug, Clone, PartialEq)]
pub struct DatumEventQuery<'a> {
    pub project_filter: ProjectFilter<'a>,
    /// `None` = do not filter by application. Does not distinguish
    /// "application IS NULL" the way `project_filter` does for projects --
    /// the Phase 123 brief did not ask for that symmetry, and
    /// `application_id` is not a top-level scope boundary the way
    /// `project_id` is.
    pub application_id: Option<&'a str>,
    pub category: Option<DatumEventCategoryV1>,
    pub kind: Option<&'a str>,
    pub severity: Option<DatumEventSeverityV1>,
    /// Matched against the exact wire-string form of `DatumEventSourceV1`
    /// (e.g. `"dmonitor"`, `"gnn"`, or an arbitrary forward-compatible
    /// value like `"future_subsystem"`) -- a plain string filter, not a
    /// typed `DatumEventSourceV1`, so a caller can filter by a source this
    /// binary doesn't have a dedicated enum variant for yet.
    pub source: Option<&'a str>,
    /// Half-open lower bound: `occurred_at_utc >= occurred_from_utc`.
    /// MUST be Phase 122's exact canonical UTC RFC 3339 form -- validated
    /// and converted to this store's internal numeric sortable
    /// representation before use (see the module doc comment for why);
    /// rejected as `InvalidQuery` if unparseable or not canonical, never
    /// silently compared as text.
    pub occurred_from_utc: Option<&'a str>,
    /// Half-open upper bound: `occurred_at_utc < occurred_to_utc`. Same
    /// validation rule as `occurred_from_utc`. If both bounds are
    /// present and `occurred_from_utc` denotes a strictly LATER instant
    /// than `occurred_to_utc`, the query is rejected as `InvalidQuery`
    /// (a reversed range very likely indicates caller error). Equal
    /// bounds are NOT an error -- they denote a valid, if degenerate,
    /// zero-width window, and correctly produce an empty result with no
    /// special-casing needed.
    pub occurred_to_utc: Option<&'a str>,
    /// Must be `1..=MAX_QUERY_LIMIT`; enforced by [`list_events`], not
    /// silently clamped -- see that function's doc comment for why a hard
    /// rejection was chosen over clamping.
    pub limit: u32,
}

impl<'a> Default for DatumEventQuery<'a> {
    fn default() -> Self {
        DatumEventQuery {
            project_filter: ProjectFilter::Any,
            application_id: None,
            category: None,
            kind: None,
            severity: None,
            source: None,
            occurred_from_utc: None,
            occurred_to_utc: None,
            limit: DEFAULT_QUERY_LIMIT,
        }
    }
}

/// Validates `event`, computes its canonical digest, and durably admits
/// it -- or reports why it could not be admitted. Idempotent for a
/// genuinely retried submission (same `event_id`, same content); fails
/// closed for a conflicting one (same `event_id`, different content).
///
/// # Algorithm (atomic; see the module's concurrency tests for proof)
///
/// 1. `event.validate()` -- reject before opening any transaction if
///    invalid (`InvalidEvent`). An invalid event is never written.
/// 2. Recompute the canonical digest via
///    `core::datum_event::recompute_datum_event_digest` -- the caller's
///    own notion of its content is never trusted for identity purposes.
/// 3. Derive the numeric sortable timestamp representation from the
///    already-canonical `event.occurred_at_utc` (see
///    [`canonicalize_and_derive_sortable`]).
/// 4. Open a SQLite transaction with `TransactionBehavior::Immediate`
///    (acquires the write lock at `BEGIN`, not deferred until the first
///    write) -- this is what makes the lookup-then-insert sequence below
///    safe under concurrent admission of the SAME `event_id` from
///    multiple connections: SQLite fully serializes `IMMEDIATE`
///    transactions against each other, so no second transaction can begin
///    its own lookup until the first has committed or rolled back. A
///    plain `SELECT` then unlocked `INSERT` (deferred transaction) would
///    not have this guarantee.
/// 5. Look up `event_id`. Absent -> insert and commit (`Inserted`).
///    Present with an equal digest -> commit/no-op and return the
///    already-stored record (`IdempotentReplay`, `admitted_at_utc`
///    UNCHANGED from the original insertion). Present with a
///    different digest -> roll back and return `Conflict`.
pub fn admit_event(
    db_path: &Path,
    event: &DatumEventV1,
) -> Result<DatumEventAdmission, DatumEventStoreError> {
    let findings = event.validate();
    if !findings.is_empty() {
        return Err(DatumEventStoreError::InvalidEvent(findings));
    }

    let event_digest =
        recompute_datum_event_digest(event).map_err(DatumEventStoreError::Storage)?;
    let event_json = serde_json::to_string(event).map_err(|error| {
        DatumEventStoreError::Storage(format!("serialize DatumEventV1: {error}"))
    })?;
    // `event.occurred_at_utc` was already required to be Phase 122
    // canonical by `event.validate()` above; a derivation failure here
    // would indicate an internal inconsistency between this module's
    // canonicalization logic and Phase 122's own, not a caller mistake --
    // hence `Storage`, not `InvalidQuery`.
    let (occurred_at_epoch_seconds, occurred_at_subsec_nanos) =
        canonicalize_and_derive_sortable(&event.occurred_at_utc)
            .map_err(DatumEventStoreError::Storage)?;

    let mut connection = open_database(db_path).map_err(DatumEventStoreError::Storage)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            DatumEventStoreError::Storage(format!("begin SQLite datum_events transaction: {error}"))
        })?;

    let existing_digest: Option<String> = transaction
        .query_row(
            "SELECT event_digest FROM datum_events WHERE event_id = ?1",
            params![&event.event_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| {
            DatumEventStoreError::Storage(format!("lookup datum_events event_id: {error}"))
        })?;

    match existing_digest {
        None => {
            let admitted_at_utc = canonical_utc_now();
            transaction
                .execute(
                    "INSERT INTO datum_events (
                        event_id, event_digest, schema, project_id, application_id,
                        category, kind, severity, source,
                        occurred_at_utc, occurred_at_epoch_seconds, occurred_at_subsec_nanos,
                        admitted_at_utc, event_json
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    params![
                        &event.event_id,
                        &event_digest,
                        &event.schema,
                        event.project_id.as_deref(),
                        event.application_id.as_deref(),
                        enum_wire_str(&event.category)?,
                        &event.kind,
                        enum_wire_str(&event.severity)?,
                        enum_wire_str(&event.source)?,
                        &event.occurred_at_utc,
                        occurred_at_epoch_seconds,
                        i64::from(occurred_at_subsec_nanos),
                        &admitted_at_utc,
                        &event_json,
                    ],
                )
                .map_err(|error| {
                    DatumEventStoreError::Storage(format!("insert datum_events row: {error}"))
                })?;
            transaction.commit().map_err(|error| {
                DatumEventStoreError::Storage(format!("commit datum_events insert: {error}"))
            })?;
            Ok(DatumEventAdmission::Inserted(StoredDatumEvent {
                event: event.clone(),
                event_digest,
                admitted_at_utc,
            }))
        }
        Some(stored_digest) if stored_digest == event_digest => {
            let stored = fetch_by_id(&transaction, &event.event_id)?.ok_or_else(|| {
                DatumEventStoreError::Storage(
                    "datum_events row vanished during idempotent-replay read".to_string(),
                )
            })?;
            transaction.commit().map_err(|error| {
                DatumEventStoreError::Storage(format!(
                    "commit datum_events idempotent-replay read: {error}"
                ))
            })?;
            Ok(DatumEventAdmission::IdempotentReplay(stored))
        }
        Some(stored_digest) => {
            transaction.rollback().map_err(|error| {
                DatumEventStoreError::Storage(format!("rollback datum_events conflict: {error}"))
            })?;
            Err(DatumEventStoreError::Conflict {
                event_id: event.event_id.clone(),
                existing_digest: stored_digest,
                submitted_digest: event_digest,
            })
        }
    }
}

/// Looks up exactly one event by its identity. Returns `Ok(None)` if
/// absent, `Err(CorruptStoredEvent)` if the stored row fails any
/// corruption check (see the module doc comment), never a silently
/// malformed result.
pub fn get_event(
    db_path: &Path,
    event_id: &str,
) -> Result<Option<StoredDatumEvent>, DatumEventStoreError> {
    let event_id = event_id.trim();
    if event_id.is_empty() {
        return Err(DatumEventStoreError::InvalidQuery(
            "event_id must be non-empty".to_string(),
        ));
    }
    let connection = open_database(db_path).map_err(DatumEventStoreError::Storage)?;
    fetch_by_id(&connection, event_id)
}

/// A bounded, deterministically-ordered list query. Always ordered by the
/// numeric sortable timestamp derived from `occurred_at_utc`
/// (`occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC`),
/// then `event_id DESC` -- see the module documentation for why a
/// derived numeric representation is used instead of comparing
/// `occurred_at_utc` as TEXT (a real, proven chronological-ordering
/// defect for mixed fractional-precision timestamps), and for why
/// occurrence time (not admission time) was chosen as the primary sort
/// key. Known limitation, unchanged by this hardening pass: producer
/// clock skew means `occurred_at_utc` is not strictly monotonic with
/// admission order across different producers; a future cursor design
/// must account for this.
///
/// `limit` is REJECTED (`InvalidQuery`), not silently clamped, when it is
/// `0` or exceeds [`MAX_QUERY_LIMIT`] -- a hard ceiling is easier to
/// reason about and test than a caller discovering their request was
/// quietly truncated.
///
/// `occurred_from_utc`/`occurred_to_utc`, if present, must each be
/// Phase 122's exact canonical UTC form; malformed or non-canonical
/// values are rejected as `InvalidQuery` rather than silently producing
/// a meaningless text comparison. If both are present and denote a
/// reversed range (`from` strictly after `to`), the query is rejected as
/// `InvalidQuery`; equal bounds are accepted and correctly yield an
/// empty result.
///
/// If ANY row in the result set fails a corruption check, the entire call
/// fails (`CorruptStoredEvent`) rather than silently omitting that row --
/// "fail closed," per the Phase 123 brief, is interpreted here as never
/// returning a page that looks complete but silently dropped something.
pub fn list_events(
    db_path: &Path,
    query: &DatumEventQuery<'_>,
) -> Result<Vec<StoredDatumEvent>, DatumEventStoreError> {
    if query.limit == 0 || query.limit > MAX_QUERY_LIMIT {
        return Err(DatumEventStoreError::InvalidQuery(format!(
            "limit must be 1..={MAX_QUERY_LIMIT}, got {}",
            query.limit
        )));
    }
    if let ProjectFilter::Project(project_id) = query.project_filter {
        if project_id.trim().is_empty() {
            return Err(DatumEventStoreError::InvalidQuery(
                "project filter, if a specific project, must be non-empty".to_string(),
            ));
        }
    }

    let from_sortable = match query.occurred_from_utc {
        Some(raw) => Some((
            raw,
            canonicalize_and_derive_sortable(raw).map_err(DatumEventStoreError::InvalidQuery)?,
        )),
        None => None,
    };
    let to_sortable = match query.occurred_to_utc {
        Some(raw) => Some((
            raw,
            canonicalize_and_derive_sortable(raw).map_err(DatumEventStoreError::InvalidQuery)?,
        )),
        None => None,
    };
    if let (Some((from_raw, from_value)), Some((to_raw, to_value))) = (from_sortable, to_sortable) {
        if from_value > to_value {
            return Err(DatumEventStoreError::InvalidQuery(format!(
                "occurred_from_utc ('{from_raw}') must not be after occurred_to_utc ('{to_raw}')"
            )));
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
    if let Some(category) = query.category {
        clauses.push("category = ?".to_string());
        bindings.push(Box::new(enum_wire_str(&category)?));
    }
    if let Some(kind) = query.kind {
        clauses.push("kind = ?".to_string());
        bindings.push(Box::new(kind.to_string()));
    }
    if let Some(severity) = query.severity {
        clauses.push("severity = ?".to_string());
        bindings.push(Box::new(enum_wire_str(&severity)?));
    }
    if let Some(source) = query.source {
        clauses.push("source = ?".to_string());
        bindings.push(Box::new(source.to_string()));
    }
    // Range comparisons against the NUMERIC sortable columns, never the
    // TEXT `occurred_at_utc` column -- see the module doc comment.
    if let Some((_, (from_seconds, from_nanos))) = from_sortable {
        clauses.push(
            "(occurred_at_epoch_seconds > ? OR (occurred_at_epoch_seconds = ? AND occurred_at_subsec_nanos >= ?))"
                .to_string(),
        );
        bindings.push(Box::new(from_seconds));
        bindings.push(Box::new(from_seconds));
        bindings.push(Box::new(i64::from(from_nanos)));
    }
    if let Some((_, (to_seconds, to_nanos))) = to_sortable {
        clauses.push(
            "(occurred_at_epoch_seconds < ? OR (occurred_at_epoch_seconds = ? AND occurred_at_subsec_nanos < ?))"
                .to_string(),
        );
        bindings.push(Box::new(to_seconds));
        bindings.push(Box::new(to_seconds));
        bindings.push(Box::new(i64::from(to_nanos)));
    }

    let where_sql = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT event_id FROM datum_events
         {where_sql}
         ORDER BY occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC
         LIMIT ?"
    );
    bindings.push(Box::new(i64::from(query.limit)));

    let connection = open_database(db_path).map_err(DatumEventStoreError::Storage)?;
    let mut statement = connection.prepare(&sql).map_err(|error| {
        DatumEventStoreError::Storage(format!("prepare datum_events query: {error}"))
    })?;
    let param_refs: Vec<&dyn ToSql> = bindings.iter().map(|value| value.as_ref()).collect();
    let event_ids: Vec<String> = statement
        .query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|error| DatumEventStoreError::Storage(format!("query datum_events: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            DatumEventStoreError::Storage(format!("read datum_events rows: {error}"))
        })?;

    let mut events = Vec::with_capacity(event_ids.len());
    for event_id in event_ids {
        let stored = fetch_by_id(&connection, &event_id)?.ok_or_else(|| {
            DatumEventStoreError::Storage(format!(
                "datum_events row '{event_id}' vanished between query and read"
            ))
        })?;
        events.push(stored);
    }
    Ok(events)
}

/// The raw contents of one `datum_events` row, prior to decoding/
/// verification -- kept as a small private struct purely for readability
/// (13 raw columns is unwieldy as a bare tuple).
struct StoredRow {
    event_digest: String,
    admitted_at_utc: String,
    event_json: String,
    schema: String,
    project_id: Option<String>,
    application_id: Option<String>,
    category: String,
    kind: String,
    severity: String,
    source: String,
    occurred_at_utc: String,
    occurred_at_epoch_seconds: i64,
    occurred_at_subsec_nanos: i64,
}

/// Shared read-and-verify path for [`get_event`]/[`list_events`]/the
/// idempotent-replay branch of [`admit_event`]. Runs, in order, failing
/// closed on the first check that fails:
///
/// 1. JSON deserialization of the stored `event_json`.
/// 2. Phase 122 `DatumEventV1::validate()` on the decoded event.
/// 3. Stored-digest-vs-recomputed-digest equality.
/// 4. (Phase 123 hardening pass) Every denormalized/indexed column
///    (`schema`, `project_id`, `application_id`, `category`, `kind`,
///    `severity`, `source`, `occurred_at_utc`, and the derived
///    `occurred_at_epoch_seconds`/`occurred_at_subsec_nanos`) compared
///    against the corresponding field of the DECODED event -- closing
///    the gap the Phase 123 freeze review found: without this check, a
///    row whose `event_json` says project B but whose indexed
///    `project_id` column says project A would pass checks 1-3 (since
///    the digest is computed from `event_json`'s own internally
///    consistent content, independent of the separate columns) and could
///    be silently returned to a `ProjectFilter::Project("A")` query --
///    see `project_id_corruption_is_never_returned_under_the_wrong_project_filter`
///    below, which proves this exact scenario is now caught.
pub(crate) fn fetch_by_id(
    connection: &Connection,
    event_id: &str,
) -> Result<Option<StoredDatumEvent>, DatumEventStoreError> {
    let row: Option<StoredRow> = connection
        .query_row(
            "SELECT event_digest, admitted_at_utc, event_json,
                    schema, project_id, application_id, category, kind, severity, source,
                    occurred_at_utc, occurred_at_epoch_seconds, occurred_at_subsec_nanos
             FROM datum_events WHERE event_id = ?1",
            params![event_id],
            |row| {
                Ok(StoredRow {
                    event_digest: row.get(0)?,
                    admitted_at_utc: row.get(1)?,
                    event_json: row.get(2)?,
                    schema: row.get(3)?,
                    project_id: row.get(4)?,
                    application_id: row.get(5)?,
                    category: row.get(6)?,
                    kind: row.get(7)?,
                    severity: row.get(8)?,
                    source: row.get(9)?,
                    occurred_at_utc: row.get(10)?,
                    occurred_at_epoch_seconds: row.get(11)?,
                    occurred_at_subsec_nanos: row.get(12)?,
                })
            },
        )
        .optional()
        .map_err(|error| {
            DatumEventStoreError::Storage(format!("read datum_events row '{event_id}': {error}"))
        })?;

    let Some(row) = row else {
        return Ok(None);
    };

    let event: DatumEventV1 = serde_json::from_str(&row.event_json).map_err(|error| {
        DatumEventStoreError::CorruptStoredEvent {
            event_id: event_id.to_string(),
            reason: format!("stored event_json failed to deserialize: {error}"),
        }
    })?;

    let findings = event.validate();
    if !findings.is_empty() {
        return Err(DatumEventStoreError::CorruptStoredEvent {
            event_id: event_id.to_string(),
            reason: format!("stored event failed Phase 122 validation: {findings:?}"),
        });
    }

    let recomputed_digest = recompute_datum_event_digest(&event).map_err(|error| {
        DatumEventStoreError::CorruptStoredEvent {
            event_id: event_id.to_string(),
            reason: format!("could not recompute digest for stored event: {error}"),
        }
    })?;
    if recomputed_digest != row.event_digest {
        return Err(DatumEventStoreError::CorruptStoredEvent {
            event_id: event_id.to_string(),
            reason: format!(
                "stored digest '{}' does not match recomputed digest '{recomputed_digest}'",
                row.event_digest
            ),
        });
    }

    verify_denormalized_columns_match(event_id, &row, &event)?;

    Ok(Some(StoredDatumEvent {
        event,
        event_digest: row.event_digest,
        admitted_at_utc: row.admitted_at_utc,
    }))
}

/// The Phase 123 hardening pass's denormalized-column consistency check
/// -- see [`fetch_by_id`]'s doc comment. Collects every mismatch found
/// (rather than failing on the first) so a `CorruptStoredEvent`'s reason
/// names every divergent field, not just one.
fn verify_denormalized_columns_match(
    event_id: &str,
    row: &StoredRow,
    event: &DatumEventV1,
) -> Result<(), DatumEventStoreError> {
    let mut mismatches: Vec<String> = Vec::new();

    if row.schema != event.schema {
        mismatches.push(format!(
            "schema (column='{}' json='{}')",
            row.schema, event.schema
        ));
    }
    if row.project_id != event.project_id {
        mismatches.push(format!(
            "project_id (column={:?} json={:?})",
            row.project_id, event.project_id
        ));
    }
    if row.application_id != event.application_id {
        mismatches.push(format!(
            "application_id (column={:?} json={:?})",
            row.application_id, event.application_id
        ));
    }
    let category_wire = enum_wire_str(&event.category)?;
    if row.category != category_wire {
        mismatches.push(format!(
            "category (column='{}' json='{category_wire}')",
            row.category
        ));
    }
    if row.kind != event.kind {
        mismatches.push(format!(
            "kind (column='{}' json='{}')",
            row.kind, event.kind
        ));
    }
    let severity_wire = enum_wire_str(&event.severity)?;
    if row.severity != severity_wire {
        mismatches.push(format!(
            "severity (column='{}' json='{severity_wire}')",
            row.severity
        ));
    }
    let source_wire = enum_wire_str(&event.source)?;
    if row.source != source_wire {
        mismatches.push(format!(
            "source (column='{}' json='{source_wire}')",
            row.source
        ));
    }
    if row.occurred_at_utc != event.occurred_at_utc {
        mismatches.push(format!(
            "occurred_at_utc (column='{}' json='{}')",
            row.occurred_at_utc, event.occurred_at_utc
        ));
    }
    match canonicalize_and_derive_sortable(&event.occurred_at_utc) {
        Ok((seconds, nanos)) => {
            if row.occurred_at_epoch_seconds != seconds {
                mismatches.push(format!(
                    "occurred_at_epoch_seconds (column={} derived={seconds})",
                    row.occurred_at_epoch_seconds
                ));
            }
            if row.occurred_at_subsec_nanos != i64::from(nanos) {
                mismatches.push(format!(
                    "occurred_at_subsec_nanos (column={} derived={nanos})",
                    row.occurred_at_subsec_nanos
                ));
            }
        }
        Err(error) => mismatches.push(format!(
            "occurred_at sortable re-derivation failed: {error}"
        )),
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(DatumEventStoreError::CorruptStoredEvent {
            event_id: event_id.to_string(),
            reason: format!(
                "denormalized column(s) do not match decoded event: {}",
                mismatches.join("; ")
            ),
        })
    }
}

/// The wire-string form of a `Serialize`-via-bare-string type
/// (`DatumEventCategoryV1`, `DatumEventSeverityV1`, `DatumEventSourceV1`)
/// -- derived from the type's own `Serialize` impl (via a JSON round-trip)
/// rather than a hand-duplicated match, so this can never silently drift
/// from Phase 122's actual wire format.
fn enum_wire_str<T: Serialize>(value: &T) -> Result<String, DatumEventStoreError> {
    match serde_json::to_value(value)
        .map_err(|error| DatumEventStoreError::Storage(format!("serialize enum value: {error}")))?
    {
        Value::String(s) => Ok(s),
        other => Err(DatumEventStoreError::Storage(format!(
            "expected enum value to serialize to a JSON string, got {other:?}"
        ))),
    }
}

/// `chrono::Utc::now()` rendered in the exact same canonical form Phase
/// 122 defined for `occurred_at_utc` (`SecondsFormat::AutoSi` + literal
/// `Z`) -- not a new canonicalization scheme, the same one, applied to
/// this store's own `admitted_at_utc` metadata so the two timestamps stay
/// textually comparable. Distinct from [`canonicalize_and_derive_sortable`]:
/// this function generates a fresh "now" value (never parses external
/// input), so it has nothing to validate or reject.
fn canonical_utc_now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// The single place this module ever parses an RFC 3339 string -- per
/// the Phase 123 hardening brief's explicit instruction not to parse
/// timestamps differently in several places. Used for BOTH purposes that
/// require it: deriving `occurred_at_epoch_seconds`/`occurred_at_subsec_nanos`
/// from an event's already-Phase-122-validated `occurred_at_utc` at
/// admission time, AND validating/deriving a caller-supplied
/// `occurred_from_utc`/`occurred_to_utc` query bound (which has NOT been
/// through `DatumEventV1::validate()`, so it genuinely needs the full
/// check here).
///
/// Mirrors Phase 122's own (private, unreachable from this module)
/// `canonicalize_rfc3339_utc` exactly: parse RFC 3339, convert to UTC,
/// re-render via `SecondsFormat::AutoSi` + literal `Z`, and require the
/// input to already equal that canonical form byte-for-byte -- never
/// silently accepting a non-UTC offset or a non-minimal-tier fractional
/// representation. On success, returns the same `(epoch_seconds,
/// subsec_nanos)` pair [`admit_event`]/[`list_events`] use for ordering
/// and range comparisons -- see the module doc comment for why this pair,
/// not a combined nanoseconds value, is the sortable representation.
pub(crate) fn canonicalize_and_derive_sortable(raw: &str) -> Result<(i64, u32), String> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .map_err(|error| format!("'{raw}' is not a valid RFC 3339 timestamp: {error}"))?;
    let utc = parsed.with_timezone(&Utc);
    let canonical = utc.to_rfc3339_opts(SecondsFormat::AutoSi, true);
    if canonical != raw {
        return Err(format!(
            "'{raw}' is not in Phase 122 canonical UTC form (expected '{canonical}')"
        ));
    }
    Ok((utc.timestamp(), utc.timestamp_subsec_nanos()))
}

/// Creates the `datum_events` table and its indexes if absent. Callable
/// by any `storage::` module that shares this database file and needs
/// `datum_events` to exist before creating its own FK-referencing tables
/// (currently `storage::datum_alert_store`) -- see that module's
/// "Foreign Keys"/"Database Colocation" documentation for why it calls
/// this rather than duplicating the DDL.
///
/// Deliberately does NOT acquire `schema_init::SCHEMA_INIT_LOCK` or set
/// any connection-level `PRAGMA` itself -- the caller is required to hold
/// the shared lock and have already configured its own connection's
/// pragmas (`journal_mode`/`synchronous`/`foreign_keys`) before calling
/// this, exactly as [`open_database`] does for its own connection. This
/// keeps pragma configuration (connection-scoped, must be set on every
/// connection SQLite hands out, even to the same file) cleanly separate
/// from schema DDL (file-scoped, genuinely idempotent once run by any
/// connection).
pub(crate) fn ensure_datum_events_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS datum_events (
                 event_id TEXT PRIMARY KEY,
                 event_digest TEXT NOT NULL,
                 schema TEXT NOT NULL,
                 project_id TEXT,
                 application_id TEXT,
                 category TEXT NOT NULL,
                 kind TEXT NOT NULL,
                 severity TEXT NOT NULL,
                 source TEXT NOT NULL,
                 occurred_at_utc TEXT NOT NULL,
                 occurred_at_epoch_seconds INTEGER NOT NULL,
                 occurred_at_subsec_nanos INTEGER NOT NULL,
                 admitted_at_utc TEXT NOT NULL,
                 event_json TEXT NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_datum_events_project_occurred
                 ON datum_events(project_id, occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC);
             CREATE INDEX IF NOT EXISTS idx_datum_events_project_severity_occurred
                 ON datum_events(project_id, severity, occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC);
             CREATE INDEX IF NOT EXISTS idx_datum_events_project_kind_occurred
                 ON datum_events(project_id, kind, occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC);
             CREATE INDEX IF NOT EXISTS idx_datum_events_occurred
                 ON datum_events(occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC);",
        )
        .map_err(|error| format!("initialize SQLite datum_events schema: {error}"))
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create SQLite datum_events directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let connection = Connection::open(path).map_err(|error| {
        format!(
            "open SQLite datum_events database '{}': {error}",
            path.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("configure SQLite datum_events busy timeout: {error}"))?;

    // No centralized SQLite migration framework exists in this repository
    // (confirmed by the Phase 121 audit and by inspecting
    // `storage::control_state`/`storage::operational_evidence`, neither of
    // which use one either) -- this follows the strongest existing
    // pattern instead: one idempotent `CREATE TABLE IF NOT EXISTS` plus
    // `CREATE INDEX IF NOT EXISTS` per module, run on every connection
    // open (see `SCHEMA_INIT_LOCK`'s doc comment for the concurrency
    // caveat this needed beyond what the existing modules' pattern
    // provides as-is). Acknowledged limitations, both deliberately
    // deferred rather than fixed here (Phase 123 freeze review, F5/F22):
    // (1) no mechanism for a future ALTER TABLE-style column migration --
    // adding a column later would need a dedicated migration step this
    // module does not yet provide, exactly the same limitation
    // `operational_evidence`'s table already has today; (2) this DDL
    // (and the `SCHEMA_INIT_LOCK` acquisition) re-runs on EVERY call, not
    // once at process startup -- real, avoidable per-call overhead at
    // the volume this store is designed for. Phase 126 integration
    // should initialize the database/schema once at DServer startup and
    // avoid this per-operation DDL check -- an explicit Phase 126
    // acceptance criterion, not a Phase 123 change (this schema
    // addition did not, by itself, make deferring this unsafe).
    {
        let _schema_init_guard = crate::storage::schema_init::SCHEMA_INIT_LOCK
            .lock()
            .map_err(|_| "SQLite schema-init lock poisoned".to_string())?;
        connection
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 PRAGMA foreign_keys=ON;",
            )
            .map_err(|error| format!("configure SQLite datum_events connection: {error}"))?;
        ensure_datum_events_schema(&connection)?;
    }

    Ok(connection)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::core::datum_event::{
        DatumEventEvidenceRefV1, DatumEventSourceV1, DatumEventSubjectV1, DATUM_EVENT_SCHEMA,
    };

    fn temp_db_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-events-test-{}.db", Uuid::new_v4()))
    }

    fn cleanup(path: &Path) {
        let _ = fs::remove_file(path);
        let _ = fs::remove_file(path.with_extension("db-wal"));
        let _ = fs::remove_file(path.with_extension("db-shm"));
    }

    fn sample_event(event_id: &str, project_id: &str, occurred_at_utc: &str) -> DatumEventV1 {
        DatumEventV1 {
            schema: DATUM_EVENT_SCHEMA.to_string(),
            event_id: event_id.to_string(),
            project_id: Some(project_id.to_string()),
            application_id: None,
            category: DatumEventCategoryV1::Observation,
            kind: "telemetry_stale".to_string(),
            severity: DatumEventSeverityV1::Warning,
            source: DatumEventSourceV1::SentinelTelemetry,
            subject: DatumEventSubjectV1::Project {
                project_id: project_id.to_string(),
            },
            occurred_at_utc: occurred_at_utc.to_string(),
            evidence_refs: Vec::new(),
            correlation_hint: None,
            confidence_permille: None,
            model_ref: None,
            attributes: json!({}),
        }
    }

    fn invalid_event() -> DatumEventV1 {
        let mut event = sample_event("datumevent:invalid", "proj:lab", "2026-09-29T00:00:00Z");
        event.kind = String::new(); // fails Phase 122 validation
        event
    }

    /// Directly tampers ONE raw column of an already-admitted row via a
    /// bypass connection, leaving every other column (crucially
    /// `event_json` and `event_digest`) untouched -- simulates the exact
    /// denormalized-column-divergence scenario the Phase 123 hardening
    /// pass closes.
    fn tamper_column(path: &Path, event_id: &str, column: &str, new_value: &dyn ToSql) {
        let connection = Connection::open(path).unwrap();
        connection
            .execute(
                &format!("UPDATE datum_events SET {column} = ?1 WHERE event_id = ?2"),
                params![new_value, event_id],
            )
            .unwrap();
    }

    // -------------------------------------------------------------- 1, 2, 3
    #[test]
    fn valid_event_is_admitted_and_round_trips_exactly() {
        let path = temp_db_path();
        let event = sample_event("datumevent:1", "proj:lab", "2026-09-29T00:00:00Z");
        let admission = admit_event(&path, &event).unwrap();
        let DatumEventAdmission::Inserted(stored) = admission else {
            panic!("expected Inserted, got {admission:?}");
        };
        assert_eq!(stored.event, event);
        assert!(jcs_digest_shape_ok(&stored.event_digest));

        let fetched = get_event(&path, "datumevent:1")
            .unwrap()
            .expect("event exists");
        assert_eq!(fetched.event, event);
        assert_eq!(fetched.event_digest, stored.event_digest);
        assert_eq!(fetched.admitted_at_utc, stored.admitted_at_utc);
        cleanup(&path);
    }

    #[test]
    fn invalid_event_is_rejected_and_never_stored() {
        let path = temp_db_path();
        let result = admit_event(&path, &invalid_event());
        assert!(matches!(result, Err(DatumEventStoreError::InvalidEvent(_))));
        assert_eq!(get_event(&path, "datumevent:invalid").unwrap(), None);
        cleanup(&path);
    }

    // ------------------------------------------------------------------ 4
    #[test]
    fn admitted_at_is_distinct_from_occurred_at() {
        let path = temp_db_path();
        let event = sample_event("datumevent:4", "proj:lab", "2020-01-01T00:00:00Z");
        let DatumEventAdmission::Inserted(stored) = admit_event(&path, &event).unwrap() else {
            panic!("expected Inserted");
        };
        assert_eq!(stored.event.occurred_at_utc, "2020-01-01T00:00:00Z");
        assert_ne!(stored.admitted_at_utc, stored.event.occurred_at_utc);
        // admitted_at_utc must itself be a canonical Phase-122-style UTC
        // timestamp (parseable, not the sentinel we asserted against).
        assert!(stored.admitted_at_utc.ends_with('Z'));
        cleanup(&path);
    }

    // ------------------------------------------------------------------ 5
    #[test]
    fn same_event_replay_returns_idempotent_replay_not_a_duplicate() {
        let path = temp_db_path();
        let event = sample_event("datumevent:5", "proj:lab", "2026-09-29T00:00:00Z");
        let first = admit_event(&path, &event).unwrap();
        assert!(matches!(first, DatumEventAdmission::Inserted(_)));

        let second = admit_event(&path, &event).unwrap();
        let DatumEventAdmission::IdempotentReplay(stored) = second else {
            panic!("expected IdempotentReplay, got {second:?}");
        };
        assert_eq!(stored.event, event);

        let mut query = DatumEventQuery::default();
        query.project_filter = ProjectFilter::Project("proj:lab");
        assert_eq!(
            list_events(&path, &query).unwrap().len(),
            1,
            "no duplicate row"
        );
        cleanup(&path);
    }

    // -------------------------------------------------------- F7 / item 17
    #[test]
    fn idempotent_replay_preserves_the_original_admitted_at_utc() {
        let path = temp_db_path();
        let event = sample_event("datumevent:5b", "proj:lab", "2026-09-29T00:00:00Z");

        let DatumEventAdmission::Inserted(first) = admit_event(&path, &event).unwrap() else {
            panic!("expected Inserted");
        };

        // Ensure real wall-clock time actually advances, so a buggy
        // "refresh admitted_at on replay" implementation would be caught
        // rather than accidentally producing the same value anyway.
        std::thread::sleep(std::time::Duration::from_millis(20));

        let DatumEventAdmission::IdempotentReplay(second) = admit_event(&path, &event).unwrap()
        else {
            panic!("expected IdempotentReplay");
        };
        assert_eq!(
            second.admitted_at_utc, first.admitted_at_utc,
            "replay must NOT refresh admitted_at_utc"
        );

        // And a fresh read agrees.
        let fetched = get_event(&path, "datumevent:5b").unwrap().unwrap();
        assert_eq!(fetched.admitted_at_utc, first.admitted_at_utc);
        cleanup(&path);
    }

    // ------------------------------------------------------------------ 6
    #[test]
    fn same_event_id_different_digest_returns_conflict() {
        let path = temp_db_path();
        let first = sample_event("datumevent:6", "proj:lab", "2026-09-29T00:00:00Z");
        let mut second = first.clone();
        second.severity = DatumEventSeverityV1::Critical; // different content, same event_id

        admit_event(&path, &first).unwrap();
        let result = admit_event(&path, &second);
        match result {
            Err(DatumEventStoreError::Conflict {
                event_id,
                existing_digest,
                submitted_digest,
            }) => {
                assert_eq!(event_id, "datumevent:6");
                assert_ne!(existing_digest, submitted_digest);
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
        // the original, conflicting-free content must remain untouched
        let stored = get_event(&path, "datumevent:6").unwrap().unwrap();
        assert_eq!(stored.event, first);
        cleanup(&path);
    }

    // ------------------------------------------------------------------ 7
    #[test]
    fn different_ids_semantically_identical_events_both_stored() {
        let path = temp_db_path();
        let first = sample_event("datumevent:7a", "proj:lab", "2026-09-29T00:00:00Z");
        let mut second = first.clone();
        second.event_id = "datumevent:7b".to_string();

        assert!(matches!(
            admit_event(&path, &first).unwrap(),
            DatumEventAdmission::Inserted(_)
        ));
        assert!(matches!(
            admit_event(&path, &second).unwrap(),
            DatumEventAdmission::Inserted(_)
        ));

        let mut query = DatumEventQuery::default();
        query.project_filter = ProjectFilter::Project("proj:lab");
        assert_eq!(
            list_events(&path, &query).unwrap().len(),
            2,
            "Phase 123 does not dedup conditions"
        );
        cleanup(&path);
    }

    // ------------------------------------------------------------------ 8
    #[test]
    fn global_event_with_null_project_is_stored_and_queryable() {
        let path = temp_db_path();
        let mut event = sample_event("datumevent:8", "unused", "2026-09-29T00:00:00Z");
        event.project_id = None;
        event.subject = DatumEventSubjectV1::DServerGlobal;

        admit_event(&path, &event).unwrap();
        let fetched = get_event(&path, "datumevent:8").unwrap().unwrap();
        assert_eq!(fetched.event.project_id, None);

        let global_only = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::GlobalOnly,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(global_only.len(), 1);
        assert_eq!(global_only[0].event.event_id, "datumevent:8");
        cleanup(&path);
    }

    // ------------------------------------------------------------------ 9
    #[test]
    fn project_filter_distinguishes_any_global_only_and_specific_project() {
        let path = temp_db_path();
        let mut global_event = sample_event("datumevent:9g", "unused", "2026-09-29T00:00:00Z");
        global_event.project_id = None;
        global_event.subject = DatumEventSubjectV1::DServerGlobal;
        let project_a = sample_event("datumevent:9a", "proj:a", "2026-09-29T00:01:00Z");
        let project_b = sample_event("datumevent:9b", "proj:b", "2026-09-29T00:02:00Z");

        admit_event(&path, &global_event).unwrap();
        admit_event(&path, &project_a).unwrap();
        admit_event(&path, &project_b).unwrap();

        let all = list_events(&path, &DatumEventQuery::default()).unwrap();
        assert_eq!(
            all.len(),
            3,
            "ProjectFilter::Any must return every project plus global"
        );

        let global_only = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::GlobalOnly,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(global_only.len(), 1);
        assert_eq!(global_only[0].event.event_id, "datumevent:9g");

        let project_a_only = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:a"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(project_a_only.len(), 1);
        assert_eq!(project_a_only[0].event.event_id, "datumevent:9a");
        cleanup(&path);
    }

    // ----------------------------------------------------------- F8 / item 18
    #[test]
    fn empty_specific_project_filter_is_rejected() {
        let path = temp_db_path();
        let result = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project(""),
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(DatumEventStoreError::InvalidQuery(_))));

        let whitespace_result = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("   "),
                ..Default::default()
            },
        );
        assert!(matches!(
            whitespace_result,
            Err(DatumEventStoreError::InvalidQuery(_))
        ));
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 10
    #[test]
    fn application_filter_handles_null_application_id_cleanly() {
        let path = temp_db_path();
        let mut with_app = sample_event("datumevent:10a", "proj:lab", "2026-09-29T00:00:00Z");
        with_app.application_id = Some("app:pipeline".to_string());
        with_app.subject = DatumEventSubjectV1::Application {
            project_id: "proj:lab".to_string(),
            application_id: "app:pipeline".to_string(),
        };
        let without_app = sample_event("datumevent:10b", "proj:lab", "2026-09-29T00:01:00Z");
        assert_eq!(without_app.application_id, None);

        admit_event(&path, &with_app).unwrap();
        admit_event(&path, &without_app).unwrap();

        let filtered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                application_id: Some("app:pipeline"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event.event_id, "datumevent:10a");

        let unfiltered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            unfiltered.len(),
            2,
            "no application filter must not exclude NULL rows"
        );
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 11
    #[test]
    fn category_filter_works() {
        let path = temp_db_path();
        let mut runtime_event = sample_event("datumevent:11a", "proj:lab", "2026-09-29T00:00:00Z");
        runtime_event.category = DatumEventCategoryV1::Runtime;
        let observation_event = sample_event("datumevent:11b", "proj:lab", "2026-09-29T00:01:00Z");

        admit_event(&path, &runtime_event).unwrap();
        admit_event(&path, &observation_event).unwrap();

        let filtered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                category: Some(DatumEventCategoryV1::Runtime),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event.event_id, "datumevent:11a");
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 12
    #[test]
    fn kind_filter_works() {
        let path = temp_db_path();
        let mut failed = sample_event("datumevent:12a", "proj:lab", "2026-09-29T00:00:00Z");
        failed.kind = "service_failed".to_string();
        let stale = sample_event("datumevent:12b", "proj:lab", "2026-09-29T00:01:00Z");

        admit_event(&path, &failed).unwrap();
        admit_event(&path, &stale).unwrap();

        let filtered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                kind: Some("service_failed"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event.event_id, "datumevent:12a");
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 13
    #[test]
    fn severity_filter_works() {
        let path = temp_db_path();
        let mut critical = sample_event("datumevent:13a", "proj:lab", "2026-09-29T00:00:00Z");
        critical.severity = DatumEventSeverityV1::Critical;
        let warning = sample_event("datumevent:13b", "proj:lab", "2026-09-29T00:01:00Z");

        admit_event(&path, &critical).unwrap();
        admit_event(&path, &warning).unwrap();

        let filtered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                severity: Some(DatumEventSeverityV1::Critical),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event.event_id, "datumevent:13a");
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 14
    #[test]
    fn source_filter_works_including_forward_compatible_other_source() {
        let path = temp_db_path();
        let mut future_sourced = sample_event("datumevent:14a", "proj:lab", "2026-09-29T00:00:00Z");
        future_sourced.source = DatumEventSourceV1::Other("future_subsystem".to_string());
        let known_sourced = sample_event("datumevent:14b", "proj:lab", "2026-09-29T00:01:00Z");

        admit_event(&path, &future_sourced).unwrap();
        admit_event(&path, &known_sourced).unwrap();

        let filtered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                source: Some("future_subsystem"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event.event_id, "datumevent:14a");
        assert_eq!(
            filtered[0].event.source,
            DatumEventSourceV1::Other("future_subsystem".to_string())
        );
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 15
    #[test]
    fn time_range_filter_works() {
        let path = temp_db_path();
        let early = sample_event("datumevent:15a", "proj:lab", "2026-01-01T00:00:00Z");
        let mid = sample_event("datumevent:15b", "proj:lab", "2026-06-01T00:00:00Z");
        let late = sample_event("datumevent:15c", "proj:lab", "2026-12-01T00:00:00Z");
        for event in [&early, &mid, &late] {
            admit_event(&path, event).unwrap();
        }

        let filtered = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                occurred_from_utc: Some("2026-03-01T00:00:00Z"),
                occurred_to_utc: Some("2026-09-01T00:00:00Z"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].event.event_id, "datumevent:15b");
        cleanup(&path);
    }

    // ---------------------------------------------- F1 / items 1, 2 (CRITICAL)
    #[test]
    fn whole_second_and_fractional_events_in_the_same_second_sort_chronologically() {
        let path = temp_db_path();
        // Exactly the brief's own worked example. Under the OLD TEXT-based
        // ordering, "12:00:00Z" would have sorted AFTER both fractional
        // values (since '.' < 'Z' in ASCII), which is chronologically
        // backwards -- this test proves the numeric sortable columns fix
        // that defect.
        let zero = sample_event("datumevent:ord-0", "proj:lab", "2026-09-29T12:00:00Z");
        let hundred_ms = sample_event(
            "datumevent:ord-100ms",
            "proj:lab",
            "2026-09-29T12:00:00.100Z",
        );
        let nine_hundred_ms = sample_event(
            "datumevent:ord-900ms",
            "proj:lab",
            "2026-09-29T12:00:00.900Z",
        );
        for event in [&zero, &hundred_ms, &nine_hundred_ms] {
            admit_event(&path, event).unwrap();
        }

        let results = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = results.iter().map(|s| s.event.event_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["datumevent:ord-900ms", "datumevent:ord-100ms", "datumevent:ord-0"],
            "DESC order must be chronological (900ms, then 100ms, then the whole second), not lexicographic"
        );
        cleanup(&path);
    }

    #[test]
    fn mixed_fractional_precision_tiers_sort_chronologically() {
        let path = temp_db_path();
        // Millisecond, microsecond, and nanosecond tiers, deliberately
        // interleaved chronologically within the same second, to prove
        // the numeric derivation (not the TEXT tier width) governs order.
        let a = sample_event("datumevent:tier-a", "proj:lab", "2026-09-29T12:00:00.001Z"); // 1ms
        let b = sample_event(
            "datumevent:tier-b",
            "proj:lab",
            "2026-09-29T12:00:00.001002Z",
        ); // 1.002ms
        let c = sample_event(
            "datumevent:tier-c",
            "proj:lab",
            "2026-09-29T12:00:00.001002003Z",
        ); // 1.002003ms
        for event in [&a, &b, &c] {
            admit_event(&path, event).unwrap();
        }

        let results = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = results.iter().map(|s| s.event.event_id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "datumevent:tier-c",
                "datumevent:tier-b",
                "datumevent:tier-a"
            ],
            "chronological DESC across mixed millisecond/microsecond/nanosecond tiers"
        );
        cleanup(&path);
    }

    // ------------------------------------------------------------ F1 / item 3
    #[test]
    fn time_range_filter_is_correct_across_same_second_fractional_boundary() {
        let path = temp_db_path();
        let zero = sample_event("datumevent:range-0", "proj:lab", "2026-09-29T12:00:00Z");
        let half = sample_event(
            "datumevent:range-500ms",
            "proj:lab",
            "2026-09-29T12:00:00.500Z",
        );
        for event in [&zero, &half] {
            admit_event(&path, event).unwrap();
        }

        // [12:00:00.250Z, ...) must include the 500ms event but NOT the
        // whole-second (earlier) event -- a TEXT comparison would get
        // this backwards.
        let from_mid_second = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                occurred_from_utc: Some("2026-09-29T12:00:00.250Z"),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = from_mid_second
            .iter()
            .map(|s| s.event.event_id.as_str())
            .collect();
        assert_eq!(ids, vec!["datumevent:range-500ms"]);

        // (..., 12:00:00.250Z) must include the whole-second event but
        // NOT the 500ms one.
        let to_mid_second = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                occurred_to_utc: Some("2026-09-29T12:00:00.250Z"),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = to_mid_second
            .iter()
            .map(|s| s.event.event_id.as_str())
            .collect();
        assert_eq!(ids, vec!["datumevent:range-0"]);
        cleanup(&path);
    }

    // ------------------------------------------------------------ F4 / item 4
    #[test]
    fn malformed_query_timestamp_is_rejected() {
        let path = temp_db_path();
        let result = list_events(
            &path,
            &DatumEventQuery {
                occurred_from_utc: Some("not-a-timestamp"),
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(DatumEventStoreError::InvalidQuery(_))));
        cleanup(&path);
    }

    // ------------------------------------------------------------ F4 / item 5
    #[test]
    fn noncanonical_query_timestamp_is_rejected() {
        let path = temp_db_path();
        // Valid RFC 3339, but a non-UTC offset -- not Phase 122 canonical
        // form. Must be rejected, never silently text-compared.
        let offset_result = list_events(
            &path,
            &DatumEventQuery {
                occurred_from_utc: Some("2026-09-29T09:00:00-03:00"),
                ..Default::default()
            },
        );
        assert!(matches!(
            offset_result,
            Err(DatumEventStoreError::InvalidQuery(_))
        ));

        // Valid instant, canonical offset, but non-minimal-tier
        // fractional representation.
        let non_canonical_fraction_result = list_events(
            &path,
            &DatumEventQuery {
                occurred_to_utc: Some("2026-09-29T12:00:00.500000000Z"),
                ..Default::default()
            },
        );
        assert!(matches!(
            non_canonical_fraction_result,
            Err(DatumEventStoreError::InvalidQuery(_))
        ));
        cleanup(&path);
    }

    // ------------------------------------------------------------ F7? item 6
    #[test]
    fn reversed_time_range_is_rejected() {
        let path = temp_db_path();
        let result = list_events(
            &path,
            &DatumEventQuery {
                occurred_from_utc: Some("2026-09-29T12:00:00Z"),
                occurred_to_utc: Some("2026-09-29T00:00:00Z"),
                ..Default::default()
            },
        );
        assert!(matches!(result, Err(DatumEventStoreError::InvalidQuery(_))));
        cleanup(&path);
    }

    // ------------------------------------------------------------------ item 7
    #[test]
    fn equal_time_range_bounds_are_valid_and_produce_an_empty_result() {
        let path = temp_db_path();
        let event = sample_event("datumevent:equalrange", "proj:lab", "2026-09-29T12:00:00Z");
        admit_event(&path, &event).unwrap();

        let result = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                occurred_from_utc: Some("2026-09-29T12:00:00Z"),
                occurred_to_utc: Some("2026-09-29T12:00:00Z"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            result.len(),
            0,
            "equal [from, to) bounds must be a valid, zero-width, empty result -- not an error"
        );
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 16
    #[test]
    fn ordering_is_deterministic_occurred_at_desc_then_event_id_desc() {
        let path = temp_db_path();
        // Two events sharing the SAME occurred_at_utc, to prove the
        // event_id tiebreaker actually governs ordering.
        let a = sample_event("datumevent:16a", "proj:lab", "2026-09-29T00:00:00Z");
        let b = sample_event("datumevent:16b", "proj:lab", "2026-09-29T00:00:00Z");
        let c = sample_event("datumevent:16c", "proj:lab", "2026-09-29T01:00:00Z");
        for event in [&a, &b, &c] {
            admit_event(&path, event).unwrap();
        }

        let results = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                ..Default::default()
            },
        )
        .unwrap();
        let ids: Vec<&str> = results.iter().map(|s| s.event.event_id.as_str()).collect();
        // c (latest occurred_at) first, then b before a (event_id DESC
        // tiebreaker: "datumevent:16b" > "datumevent:16a" lexically).
        assert_eq!(
            ids,
            vec!["datumevent:16c", "datumevent:16b", "datumevent:16a"]
        );
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 17
    #[test]
    fn query_limit_is_enforced() {
        let path = temp_db_path();
        for i in 0..5 {
            let event = sample_event(
                &format!("datumevent:17-{i}"),
                "proj:lab",
                &format!("2026-09-29T00:0{i}:00Z"),
            );
            admit_event(&path, &event).unwrap();
        }
        let limited = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                limit: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(limited.len(), 2);
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 18
    #[test]
    fn maximum_query_limit_is_enforced() {
        let path = temp_db_path();
        let zero = list_events(
            &path,
            &DatumEventQuery {
                limit: 0,
                ..Default::default()
            },
        );
        assert!(matches!(zero, Err(DatumEventStoreError::InvalidQuery(_))));

        let too_large = list_events(
            &path,
            &DatumEventQuery {
                limit: MAX_QUERY_LIMIT + 1,
                ..Default::default()
            },
        );
        assert!(matches!(
            too_large,
            Err(DatumEventStoreError::InvalidQuery(_))
        ));

        let at_max = list_events(
            &path,
            &DatumEventQuery {
                limit: MAX_QUERY_LIMIT,
                ..Default::default()
            },
        );
        assert!(at_max.is_ok());
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 19
    #[test]
    fn events_survive_a_full_close_and_reopen_of_the_database() {
        let path = temp_db_path();
        {
            let event_a = sample_event("datumevent:19a", "proj:lab", "2026-09-29T00:00:00Z");
            let event_b = sample_event("datumevent:19b", "proj:lab", "2026-09-29T00:01:00Z");
            admit_event(&path, &event_a).unwrap();
            admit_event(&path, &event_b).unwrap();
            // Every `open_database` call in this module opens and drops
            // its own `rusqlite::Connection` -- there is no persistent
            // handle held across this block to "close": the connections
            // used above are already fully closed by the time this block
            // ends, which is itself the proof that nothing but the SQLite
            // file on disk carries the data forward.
        }

        // Reopen against the SAME path, in what is, from this process's
        // perspective, an entirely fresh set of connections.
        let reopened = get_event(&path, "datumevent:19a").unwrap();
        assert!(
            reopened.is_some(),
            "event must survive reopening the database"
        );
        let reopened = reopened.unwrap();
        assert_eq!(reopened.event.event_id, "datumevent:19a");

        let all = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.len(), 2);
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 20
    #[test]
    fn concurrent_admission_of_distinct_event_ids_all_survive() {
        let path = temp_db_path();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let event = sample_event(
                        &format!("datumevent:20-{i}"),
                        "proj:lab",
                        &format!("2026-09-29T00:{i:02}:00Z"),
                    );
                    barrier.wait();
                    admit_event(&path, &event)
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        for result in &results {
            assert!(
                matches!(result, Ok(DatumEventAdmission::Inserted(_))),
                "expected every distinct concurrent admission to succeed: {result:?}"
            );
        }

        let all = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:lab"),
                limit: MAX_QUERY_LIMIT,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.len(), 8);
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 21
    #[test]
    fn concurrent_replay_of_the_same_event_is_idempotent() {
        let path = temp_db_path();
        let event = sample_event("datumevent:21", "proj:lab", "2026-09-29T00:00:00Z");
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let event = event.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    admit_event(&path, &event)
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let inserted_count = results
            .iter()
            .filter(|r| matches!(r, Ok(DatumEventAdmission::Inserted(_))))
            .count();
        let replay_count = results
            .iter()
            .filter(|r| matches!(r, Ok(DatumEventAdmission::IdempotentReplay(_))))
            .count();
        assert_eq!(
            inserted_count, 1,
            "exactly one concurrent admission must insert: {results:?}"
        );
        assert_eq!(
            replay_count, 7,
            "the rest must be idempotent replays: {results:?}"
        );

        let mut query = DatumEventQuery::default();
        query.project_filter = ProjectFilter::Project("proj:lab");
        assert_eq!(
            list_events(&path, &query).unwrap().len(),
            1,
            "no duplicate row"
        );
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 22
    #[test]
    fn concurrent_conflicting_admissions_leave_exactly_one_canonical_occurrence() {
        let path = temp_db_path();
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    // Same event_id, DIFFERENT severity per thread -- a
                    // genuine content conflict, not a replay.
                    let mut event =
                        sample_event("datumevent:22", "proj:lab", "2026-09-29T00:00:00Z");
                    event.severity = if i % 2 == 0 {
                        DatumEventSeverityV1::Warning
                    } else {
                        DatumEventSeverityV1::Critical
                    };
                    barrier.wait();
                    admit_event(&path, &event)
                })
            })
            .collect();

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let winners = results
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    Ok(DatumEventAdmission::Inserted(_))
                        | Ok(DatumEventAdmission::IdempotentReplay(_))
                )
            })
            .count();
        let conflicts = results
            .iter()
            .filter(|r| matches!(r, Err(DatumEventStoreError::Conflict { .. })))
            .count();
        // Every thread with the SAME severity as whichever inserted first
        // resolves as IdempotentReplay; every thread with the OTHER
        // severity resolves as Conflict. Both groups are non-empty given
        // 8 threads split 4/4, so at least one Conflict must occur, and
        // together winners+conflicts must account for every thread.
        assert_eq!(winners + conflicts, 8, "{results:?}");
        assert!(
            conflicts > 0,
            "expected at least one conflicting admission to be rejected: {results:?}"
        );

        // Exactly one row exists, and its content is one of the two
        // candidate severities -- never a corrupted mix of both.
        let stored = get_event(&path, "datumevent:22").unwrap().unwrap();
        assert!(matches!(
            stored.event.severity,
            DatumEventSeverityV1::Warning | DatumEventSeverityV1::Critical
        ));
        let mut query = DatumEventQuery::default();
        query.project_filter = ProjectFilter::Project("proj:lab");
        assert_eq!(list_events(&path, &query).unwrap().len(), 1);
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 23
    #[test]
    fn corrupted_json_is_detected_on_read() {
        let path = temp_db_path();
        let event = sample_event("datumevent:23", "proj:lab", "2026-09-29T00:00:00Z");
        admit_event(&path, &event).unwrap();

        tamper_column(&path, "datumevent:23", "event_json", &"not valid json");

        let result = get_event(&path, "datumevent:23");
        match result {
            Err(DatumEventStoreError::CorruptStoredEvent { event_id, .. }) => {
                assert_eq!(event_id, "datumevent:23");
            }
            other => panic!("expected CorruptStoredEvent, got {other:?}"),
        }
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 24
    #[test]
    fn corrupted_stored_digest_is_detected_on_read() {
        let path = temp_db_path();
        let event = sample_event("datumevent:24", "proj:lab", "2026-09-29T00:00:00Z");
        admit_event(&path, &event).unwrap();

        tamper_column(
            &path,
            "datumevent:24",
            "event_digest",
            &format!("sha256:{}", "0".repeat(64)),
        );

        let result = get_event(&path, "datumevent:24");
        match result {
            Err(DatumEventStoreError::CorruptStoredEvent { event_id, reason }) => {
                assert_eq!(event_id, "datumevent:24");
                assert!(reason.contains("digest"));
            }
            other => panic!("expected CorruptStoredEvent, got {other:?}"),
        }
        cleanup(&path);
    }

    // ---------------------------------------- F2 / items 8-16 (HIGH-VALUE)

    #[test]
    fn project_id_corruption_is_never_returned_under_the_wrong_project_filter() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-project",
            "proj:a",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        // Tamper ONLY the indexed project_id column to "proj:b", leaving
        // event_json (which still says "proj:a") and event_digest intact
        // -- exactly the divergence scenario the freeze review's Q9/Q10
        // hypothesized.
        tamper_column(&path, "datumevent:corrupt-project", "project_id", &"proj:b");

        // Must NOT silently appear under the tampered column's project.
        let under_b = list_events(
            &path,
            &DatumEventQuery {
                project_filter: ProjectFilter::Project("proj:b"),
                ..Default::default()
            },
        );
        assert!(
            matches!(under_b, Err(DatumEventStoreError::CorruptStoredEvent { .. })),
            "a project_id-corrupted row matching the filter's COLUMN value must fail closed, not be silently returned: {under_b:?}"
        );

        // A direct get_event must also fail closed, not return the event
        // with either project_id silently.
        let direct = get_event(&path, "datumevent:corrupt-project");
        assert!(matches!(
            direct,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn application_id_corruption_is_detected() {
        let path = temp_db_path();
        let mut event = sample_event("datumevent:corrupt-app", "proj:lab", "2026-09-29T00:00:00Z");
        event.application_id = Some("app:real".to_string());
        event.subject = DatumEventSubjectV1::Application {
            project_id: "proj:lab".to_string(),
            application_id: "app:real".to_string(),
        };
        admit_event(&path, &event).unwrap();

        tamper_column(
            &path,
            "datumevent:corrupt-app",
            "application_id",
            &"app:fake",
        );

        let result = get_event(&path, "datumevent:corrupt-app");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn category_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-category",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        tamper_column(&path, "datumevent:corrupt-category", "category", &"runtime");

        let result = get_event(&path, "datumevent:corrupt-category");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn kind_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-kind",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        tamper_column(&path, "datumevent:corrupt-kind", "kind", &"service_failed");

        let result = get_event(&path, "datumevent:corrupt-kind");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn severity_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-severity",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        tamper_column(
            &path,
            "datumevent:corrupt-severity",
            "severity",
            &"critical",
        );

        let result = get_event(&path, "datumevent:corrupt-severity");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn source_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-source",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        tamper_column(&path, "datumevent:corrupt-source", "source", &"gnn");

        let result = get_event(&path, "datumevent:corrupt-source");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn occurred_at_utc_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-occurred",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        tamper_column(
            &path,
            "datumevent:corrupt-occurred",
            "occurred_at_utc",
            &"2030-01-01T00:00:00Z",
        );

        let result = get_event(&path, "datumevent:corrupt-occurred");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    #[test]
    fn sortable_timestamp_column_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-sort",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        // Tamper the DERIVED numeric column specifically, leaving the
        // canonical TEXT `occurred_at_utc` (and everything else) intact
        // -- proves the sortable columns are independently verified, not
        // merely assumed consistent because the TEXT matches.
        tamper_column(
            &path,
            "datumevent:corrupt-sort",
            "occurred_at_epoch_seconds",
            &0i64,
        );

        let result = get_event(&path, "datumevent:corrupt-sort");
        match result {
            Err(DatumEventStoreError::CorruptStoredEvent { reason, .. }) => {
                assert!(reason.contains("occurred_at_epoch_seconds"), "{reason}");
            }
            other => panic!("expected CorruptStoredEvent, got {other:?}"),
        }
        cleanup(&path);
    }

    #[test]
    fn schema_column_corruption_is_detected() {
        let path = temp_db_path();
        let event = sample_event(
            "datumevent:corrupt-schema",
            "proj:lab",
            "2026-09-29T00:00:00Z",
        );
        admit_event(&path, &event).unwrap();

        tamper_column(
            &path,
            "datumevent:corrupt-schema",
            "schema",
            &"datum.event/2",
        );

        let result = get_event(&path, "datumevent:corrupt-schema");
        assert!(matches!(
            result,
            Err(DatumEventStoreError::CorruptStoredEvent { .. })
        ));
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 25
    #[test]
    fn canonical_phase_122_validation_still_governs_admission() {
        let path = temp_db_path();
        // A GNN-sourced event with no model_ref -- valid Rust value, but
        // Phase 122's own validate() must still reject it (F2's rule),
        // proving Phase 123 does not bypass or re-implement validation.
        let mut event = sample_event("datumevent:25", "proj:lab", "2026-09-29T00:00:00Z");
        event.source = DatumEventSourceV1::Gnn;
        event.model_ref = None;
        event.confidence_permille = None;

        let result = admit_event(&path, &event);
        match result {
            Err(DatumEventStoreError::InvalidEvent(findings)) => {
                assert!(findings
                    .iter()
                    .any(|f| f.code == "model_ref_required_for_model_source"));
            }
            other => panic!("expected InvalidEvent, got {other:?}"),
        }
        cleanup(&path);
    }

    #[test]
    fn evidence_refs_and_attributes_round_trip_through_storage_losslessly() {
        let path = temp_db_path();
        let mut event = sample_event("datumevent:25b", "proj:lab", "2026-09-29T00:00:00Z");
        event.evidence_refs = vec![DatumEventEvidenceRefV1::DMonitorObservation {
            observation_id: "dmonitor-obs:1".to_string(),
        }];
        event.attributes = json!({"nested": {"diagnostic": true, "count": 3}});
        event.correlation_hint = Some("reconcile-cycle:1".to_string());

        admit_event(&path, &event).unwrap();
        let fetched = get_event(&path, "datumevent:25b").unwrap().unwrap();
        assert_eq!(
            fetched.event, event,
            "full canonical event must be losslessly recoverable"
        );
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 26
    #[test]
    fn no_alert_lifecycle_columns_or_state_exist() {
        let path = temp_db_path();
        let event = sample_event("datumevent:26", "proj:lab", "2026-09-29T00:00:00Z");
        admit_event(&path, &event).unwrap();

        let connection = Connection::open(&path).unwrap();
        let mut statement = connection
            .prepare("PRAGMA table_info(datum_events)")
            .unwrap();
        let columns: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();

        let allowed: std::collections::BTreeSet<&str> = [
            "event_id",
            "event_digest",
            "schema",
            "project_id",
            "application_id",
            "category",
            "kind",
            "severity",
            "source",
            "occurred_at_utc",
            "occurred_at_epoch_seconds",
            "occurred_at_subsec_nanos",
            "admitted_at_utc",
            "event_json",
        ]
        .into_iter()
        .collect();
        let actual: std::collections::BTreeSet<&str> = columns.iter().map(String::as_str).collect();
        assert_eq!(
            actual, allowed,
            "unexpected datum_events column set: {actual:?}"
        );

        let forbidden = [
            "status",
            "open",
            "acknowledged",
            "acknowledged_by",
            "acknowledged_at",
            "resolved",
            "resolved_at",
            "suppressed",
            "first_seen",
            "last_seen",
            "occurrence_count",
            "correlation_key",
        ];
        for column in forbidden {
            assert!(
                !actual.contains(column),
                "datum_events must never carry alert-lifecycle column '{column}'"
            );
        }
        cleanup(&path);
    }

    // ----------------------------------------------------------------- 27
    #[test]
    fn no_update_or_delete_api_exists_for_admitted_events() {
        // Architectural/regression checkpoint (grep-verifiable, not just
        // this assertion): this module's PRODUCTION code exposes exactly
        // three public functions that touch `datum_events` content --
        // `admit_event` (insert-only-on-new-event_id), `get_event`, and
        // `list_events`. No `update_event`/`delete_event`/`remove_event`
        // function, and no raw `UPDATE`/`DELETE` SQL, exists in the
        // production portion of this file. Scoped to the code BEFORE
        // `#[cfg(test)]` deliberately: the corruption tests below use raw
        // `UPDATE` SQL (via `tamper_column`) to simulate on-disk
        // corruption, which is expected and must not trip this check.
        let source = include_str!("datum_event_store.rs");
        let production_code = source
            .split("#[cfg(test)]")
            .next()
            .expect("this file always contains a #[cfg(test)] module");
        for forbidden in [
            "fn update_event",
            "fn delete_event",
            "fn remove_event",
            "UPDATE datum_events",
            "DELETE FROM datum_events",
        ] {
            assert!(
                !production_code.contains(forbidden),
                "found forbidden mutation/deletion path '{forbidden}' in datum_event_store.rs production code"
            );
        }
    }

    fn jcs_digest_shape_ok(digest: &str) -> bool {
        digest.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
    }
}
