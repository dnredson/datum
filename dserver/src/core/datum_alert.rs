//! `datum.alert/1` (Phase 124) -- the canonical, typed, versioned
//! **operational condition** contract derived from one or more admitted
//! `datum.event/1` occurrences (`core::datum_event`, Phase 122).
//!
//! # Event != Alert != Incident
//!
//! - **`DatumEventV1`** (Phase 122): an immutable occurrence. Never
//!   mutated once admitted (Phase 123).
//! - **`DatumAlertV1`** (this module): a MUTABLE operational condition
//!   derived from one or more `DatumEventV1`s -- it owns a lifecycle
//!   (`DatumAlertStatusV1`), a stable correlation identity across
//!   repeated occurrences of the same condition, and provenance for who
//!   or what changed it.
//! - **Incident**: NOT implemented in Phase 124. A future phase may group
//!   multiple related alerts into one incident; nothing here anticipates
//!   or names that concept.
//!
//! This module defines ONLY the type, its structural validation, the
//! lifecycle transition graph (a pure function of two statuses), and
//! deterministic correlation-key derivation (a pure function of a
//! `DatumEventV1`). It performs no I/O and holds no persistence logic --
//! that is `storage::datum_alert_store`'s job, exactly mirroring the
//! `core::datum_event` / `storage::datum_event_store` split Phase 122/123
//! already established.
//!
//! # Authority posture
//!
//! `DatumAlertV1` is NOT a health/placement/convergence authority. It
//! says "a condition requiring attention exists or existed" -- it never
//! redefines DMonitor health, DMap placement, runtime observation, or
//! convergence. Alert status is lifecycle state, not infrastructure
//! state. See the Phase 124 architecture note for the full authority
//! boundary discussion.
//!
//! # "Seen" is deliberately not a lifecycle concept here
//!
//! Phase 124 does not track "an operator viewed this alert in a UI."
//! That is a Console/observability concern with no bearing on alert
//! IDENTITY, CORRELATION, or LIFECYCLE VALIDITY -- the three things this
//! phase is scoped to. `ACKNOWLEDGED` remains the first REAL, meaningful,
//! persisted lifecycle event: an actor explicitly accepting ownership,
//! never merely rendering the alert on a screen. See the architecture
//! note's "Seen vs. Acknowledged" section for the full reasoning behind
//! deferring view-tracking entirely to a future phase.
//!
//! # Hardening pass (post freeze-review)
//!
//! An independent adversarial review (documented in
//! `documentation/reference/phase124-alert-manager-lifecycle.md`) found
//! several gaps before this contract could be considered safe to freeze.
//! This revision fixes, in `core::datum_alert` specifically:
//! actor validation was dead code and is now wired into
//! `DatumAlertV1::validate()` (finding F1); status-specific required-field
//! checks were asymmetric between `Resolved`/`Closed` and
//! `Acknowledged`/`InProgress` and are now consistent, while still
//! respecting that `Acknowledged`/`InProgress` may legally be skipped en
//! route to `Resolved` (finding F4); `first_observed_at_utc <=
//! last_observed_at_utc` is now a checked invariant (finding F5, paired
//! with `storage::datum_alert_store`'s chronological aggregation fix for
//! F2); `DatumAlertActorV1` gained a structured `model_ref` field,
//! required for `Gnn`/`Llm` actors, mirroring Phase 122's own
//! `DatumEventV1::model_ref` rule exactly (finding F10); and
//! `resolution_reason`/`close_reason` now have an explicit maximum length
//! (finding F11). See each item's doc comment below for the specific
//! rule and rationale.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::core::canonical_dgraph::CanonicalValidationFinding;
use crate::core::datum_event::{
    DatumEventCategoryV1, DatumEventSeverityV1, DatumEventSubjectV1, DatumEventV1,
};
use crate::core::jcs;

/// `datum.alert/1` -- see module doc comment for the full contract.
pub const DATUM_ALERT_SCHEMA: &str = "datum.alert/1";

/// Maximum UTF-8 byte length for any free-form lifecycle reason/note text
/// (`resolution_reason`, `close_reason`, and the per-transition `reason`
/// stored by `storage::datum_alert_store`). Phase 124 freeze review,
/// finding F11: these fields are diagnostic audit metadata, not
/// unbounded documents -- nothing bounded them before this pass, a real
/// risk once an LLM actor can supply arbitrary reasoning text. `4096`
/// bytes (a few paragraphs) is generous for a genuine human or automated
/// summary while remaining a firm, explicit ceiling. Measured as UTF-8
/// byte length (`str::len()`), not character count, per the review's
/// explicit instruction.
pub const MAX_DATUM_ALERT_REASON_BYTES: usize = 4096;

fn finding(code: &'static str, message: impl Into<String>) -> CanonicalValidationFinding {
    CanonicalValidationFinding {
        code,
        message: message.into(),
    }
}

/// A mutable operational condition, derived from one or more
/// `DatumEventV1` occurrences sharing the same [`derive_correlation_key`]
/// result. See the module doc comment for the Event/Alert/Incident
/// distinction, and `storage::datum_alert_store` for how instances of
/// this type are created, updated, and persisted.
#[derive(Debug, Clone, PartialEq)]
pub struct DatumAlertV1 {
    /// Must equal [`DATUM_ALERT_SCHEMA`].
    pub schema: String,

    /// Identifies exactly ONE lifecycle episode. Deliberately NOT derived
    /// from `correlation_key` -- the same correlation key may map to many
    /// `alert_id`s over time (one CLOSED historical episode after
    /// another, plus at most one currently-active one). See
    /// `storage::datum_alert_store`'s module doc comment for the full
    /// recurrence-vs-reopen design this distinction exists to support.
    pub alert_id: String,

    /// Present whenever the underlying condition is project-scoped;
    /// absent for genuinely DServer-global conditions -- mirrors
    /// `DatumEventV1::project_id`'s own optionality exactly, since this
    /// is copied from the triggering event's own scope.
    pub project_id: Option<String>,
    pub application_id: Option<String>,

    /// The Alert-Manager-owned canonical condition identity -- see
    /// [`derive_correlation_key`]. NEVER equal to a producer's
    /// `DatumEventV1::correlation_hint` verbatim; that field is
    /// non-authoritative producer metadata, considered but not trusted
    /// (see `derive_correlation_key`'s own doc comment for why Phase 124
    /// does not fold it into the key at all yet).
    pub correlation_key: String,

    pub category: DatumEventCategoryV1,
    pub kind: String,

    /// The severity of the CHRONOLOGICALLY LATEST associated occurrence
    /// (by `DatumEventV1::occurred_at_utc`, never by processing/arrival
    /// order -- Phase 124 freeze review, finding F2). Updates only when a
    /// newly-associated event becomes the new chronological latest (see
    /// `latest_event_id`); a late-arriving event that is chronologically
    /// OLDER than the current latest can never change this field. See
    /// `peak_severity` for why de-escalation history is not lost.
    pub severity: DatumEventSeverityV1,
    /// The highest severity ever observed across this alert's entire
    /// episode (order-independent -- computed over every associated
    /// event regardless of arrival order; never decreases while the
    /// episode is open; a NEW episode -- a new `alert_id` after `Closed`
    /// -- starts its own fresh peak). Exists specifically so a
    /// chronologically-later de-escalating event never silently erases
    /// the fact that the condition was, at some point, more severe --
    /// operationally important for post-episode triage/review, and
    /// cheaper than a full per-occurrence severity history (which
    /// remains fully recoverable anyway via
    /// `storage::datum_alert_store::list_alert_events`' join back to the
    /// Phase 123 Event Store, for anyone who needs the exact sequence).
    pub peak_severity: DatumEventSeverityV1,

    /// The operational entity this condition concerns. Copied from the
    /// FIRST triggering event and never updated afterward -- by
    /// construction, every event that maps to this alert's
    /// `correlation_key` is GUARANTEED to carry an identical `subject`
    /// value (subject is itself part of the correlation key's input), so
    /// there is nothing to reconcile on update.
    pub subject: DatumEventSubjectV1,

    pub status: DatumAlertStatusV1,

    /// The occurrence time (`DatumEventV1::occurred_at_utc`) of the
    /// CHRONOLOGICALLY EARLIEST associated event -- i.e. `MIN(occurred_at)`
    /// across every event ever associated with this `alert_id`, never
    /// "whichever event was processed first" (Phase 124 freeze review,
    /// findings F2/F5). A late-arriving event whose `occurred_at` is
    /// earlier than the current value updates this field. Deliberately
    /// named `_observed_`, not `_seen_`, to avoid exactly the ambiguity
    /// the Phase 124 brief warned about: this is when the CONDITION was
    /// first observed, never when a human viewed the alert. See
    /// `storage::datum_alert_store`'s chronological-aggregation
    /// documentation for the exact tie-break rule when two events share
    /// the same `occurred_at`.
    pub first_observed_at_utc: String,
    /// The occurrence time of the CHRONOLOGICALLY LATEST associated
    /// event -- `MAX(occurred_at)` across every associated event, never
    /// "whichever event was processed most recently." Always `>=
    /// first_observed_at_utc` -- checked by [`DatumAlertV1::validate`].
    pub last_observed_at_utc: String,

    /// The `event_id` of the chronologically earliest associated event
    /// (see `first_observed_at_utc`). Tie-broken deterministically by
    /// `event_id` lexical ordering when two associated events share the
    /// exact same `occurred_at` -- see
    /// `storage::datum_alert_store` for the
    /// single, stable rule.
    pub first_event_id: String,
    /// The `event_id` of the chronologically latest associated event
    /// (see `last_observed_at_utc`). Same tie-break rule as
    /// `first_event_id`.
    pub latest_event_id: String,

    /// Total occurrences associated with this `alert_id` across its
    /// entire episode, including any prior resolve/reopen cycle (a
    /// reopen does NOT reset this counter -- it is still the same
    /// episode). Always `>= 1`.
    pub occurrence_count: u64,

    pub acknowledged_by: Option<DatumAlertActorV1>,
    pub acknowledged_at_utc: Option<String>,

    pub in_progress_by: Option<DatumAlertActorV1>,
    pub in_progress_at_utc: Option<String>,

    /// Cleared (`None`) if this alert is later reopened (`Resolved` ->
    /// `Open`) -- see `storage::datum_alert_store`'s reopen logic. A
    /// fresh `resolve_alert` call is required to set these again before
    /// the alert can reach `Closed` a second time.
    pub resolved_by: Option<DatumAlertActorV1>,
    pub resolved_at_utc: Option<String>,
    /// Bounded to [`MAX_DATUM_ALERT_REASON_BYTES`] (Phase 124 freeze
    /// review, finding F11). `storage::datum_alert_store` normalizes an
    /// empty/whitespace-only caller-supplied reason to `None` before it
    /// ever reaches this field; `validate()` additionally rejects a
    /// stored `Some("")`/`Some("   ")` as defense in depth.
    pub resolution_reason: Option<String>,

    /// `Closed` is terminal -- see [`is_valid_transition`]. Once set,
    /// these lifecycle fields are never mutated again; late historical
    /// evidence may still update occurrence aggregation. A future
    /// recurrence of the same `correlation_key` creates a brand new
    /// `alert_id` instead of reopening this one.
    pub closed_by: Option<DatumAlertActorV1>,
    pub closed_at_utc: Option<String>,
    /// Same bound and normalization rule as `resolution_reason`.
    pub close_reason: Option<String>,
}

impl DatumAlertV1 {
    /// Fail-closed structural validation, in the same spirit as
    /// `DatumEventV1::validate` -- rejects empty required identifiers, an
    /// invalid `kind` format, a zero `occurrence_count`,
    /// `first_observed_at_utc > last_observed_at_utc` (finding F5), an
    /// actor/timestamp pair present on only one side, a malformed actor
    /// (finding F1 -- now actually enforced, see below), a status whose
    /// required fields are absent, and an over-long reason (finding F11).
    ///
    /// # Status-specific required fields (finding F4)
    ///
    /// - `Acknowledged`: requires `acknowledged_by`/`acknowledged_at_utc`.
    /// - `InProgress`: requires BOTH `acknowledged_by`/`acknowledged_at_utc`
    ///   AND `in_progress_by`/`in_progress_at_utc` -- safe to require the
    ///   acknowledged pair too because [`is_valid_transition`] only ever
    ///   allows `InProgress` to be reached via `Acknowledged` first, so
    ///   every legally-constructed `InProgress` alert has already passed
    ///   through it.
    /// - `Resolved`: requires `resolved_by`/`resolved_at_utc` ONLY.
    ///   Deliberately does NOT require `acknowledged_by`/`in_progress_by`
    ///   -- `Open -> Resolved` is a legal direct transition (a condition
    ///   can resolve itself before anyone acknowledges it), so requiring
    ///   a lifecycle checkpoint that may legally have been skipped would
    ///   incorrectly reject a valid `Open -> Resolved -> Closed` episode.
    /// - `Closed`: requires closure AND resolution metadata, for the
    ///   identical reason -- `Closed` is only reachable via `Resolved`,
    ///   but `Resolved` itself may have been reached directly from `Open`
    ///   or `Acknowledged`, skipping `InProgress` or both intermediate
    ///   stages entirely.
    ///
    /// Does NOT re-validate historical consistency across a reopen (e.g.
    /// it does not object to `status = Open` with `acknowledged_by` still
    /// set) -- see `acknowledged_by`'s doc comment for why that is the
    /// correct, not-a-bug, shape after a reopen. Cross-checking the
    /// current row against `datum_alert_transitions`/`datum_alert_events`
    /// history (findings F6/F7) is necessarily a
    /// `storage::datum_alert_store` concern, not this pure function's --
    /// see that module's `get_alert` documentation.
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = Vec::new();

        if self.schema != DATUM_ALERT_SCHEMA {
            findings.push(finding(
                "schema_invalid",
                format!(
                    "schema must be '{DATUM_ALERT_SCHEMA}', got '{}'",
                    self.schema
                ),
            ));
        }
        if self.alert_id.trim().is_empty() {
            findings.push(finding("missing_field", "alert_id must be non-empty"));
        }
        if let Some(project_id) = &self.project_id {
            if project_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "project_id, if present, must be non-empty",
                ));
            }
        }
        if let Some(application_id) = &self.application_id {
            if application_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "application_id, if present, must be non-empty",
                ));
            }
        }
        if self.correlation_key.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "correlation_key must be non-empty",
            ));
        }
        if self.kind.trim().is_empty() {
            findings.push(finding("missing_field", "kind must be non-empty"));
        } else if !is_lower_snake_case(&self.kind) {
            findings.push(finding(
                "kind_invalid_format",
                format!(
                    "kind must be lower_snake_case (ascii a-z, 0-9, '_'), got '{}'",
                    self.kind
                ),
            ));
        }

        let first_observed_parsed = parse_and_check_timestamp(
            &self.first_observed_at_utc,
            "first_observed_at_utc",
            &mut findings,
        );
        let last_observed_parsed = parse_and_check_timestamp(
            &self.last_observed_at_utc,
            "last_observed_at_utc",
            &mut findings,
        );
        if let (Some(first), Some(last)) = (first_observed_parsed, last_observed_parsed) {
            if first > last {
                findings.push(finding(
                    "observed_window_inverted",
                    format!(
                        "first_observed_at_utc ('{}') must not be after last_observed_at_utc ('{}')",
                        self.first_observed_at_utc, self.last_observed_at_utc
                    ),
                ));
            }
        }

        if self.first_event_id.trim().is_empty() {
            findings.push(finding("missing_field", "first_event_id must be non-empty"));
        }
        if self.latest_event_id.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                "latest_event_id must be non-empty",
            ));
        }
        if self.occurrence_count == 0 {
            findings.push(finding(
                "occurrence_count_invalid",
                "occurrence_count must be >= 1",
            ));
        }

        check_actor(
            "acknowledged",
            &self.acknowledged_by,
            &self.acknowledged_at_utc,
            &mut findings,
        );
        check_actor(
            "in_progress",
            &self.in_progress_by,
            &self.in_progress_at_utc,
            &mut findings,
        );
        check_actor(
            "resolved",
            &self.resolved_by,
            &self.resolved_at_utc,
            &mut findings,
        );
        check_actor(
            "closed",
            &self.closed_by,
            &self.closed_at_utc,
            &mut findings,
        );

        check_reason("resolution_reason", &self.resolution_reason, &mut findings);
        check_reason("close_reason", &self.close_reason, &mut findings);

        match self.status {
            DatumAlertStatusV1::Acknowledged => {
                require_pair_present(
                    "acknowledged",
                    &self.acknowledged_by,
                    &self.acknowledged_at_utc,
                    "acknowledged_fields_required",
                    "status = Acknowledged requires acknowledged_by and acknowledged_at_utc to be present",
                    &mut findings,
                );
            }
            DatumAlertStatusV1::InProgress => {
                require_pair_present(
                    "acknowledged",
                    &self.acknowledged_by,
                    &self.acknowledged_at_utc,
                    "acknowledged_fields_required",
                    "status = InProgress requires acknowledged_by and acknowledged_at_utc to be present (InProgress is only reachable via Acknowledged)",
                    &mut findings,
                );
                require_pair_present(
                    "in_progress",
                    &self.in_progress_by,
                    &self.in_progress_at_utc,
                    "in_progress_fields_required",
                    "status = InProgress requires in_progress_by and in_progress_at_utc to be present",
                    &mut findings,
                );
            }
            DatumAlertStatusV1::Resolved => {
                require_pair_present(
                    "resolved",
                    &self.resolved_by,
                    &self.resolved_at_utc,
                    "resolved_fields_required",
                    "status = Resolved requires resolved_by and resolved_at_utc to be present",
                    &mut findings,
                );
            }
            DatumAlertStatusV1::Closed => {
                require_pair_present(
                    "resolved",
                    &self.resolved_by,
                    &self.resolved_at_utc,
                    "resolved_fields_required",
                    "Closed requires resolution metadata",
                    &mut findings,
                );
                require_pair_present(
                    "closed",
                    &self.closed_by,
                    &self.closed_at_utc,
                    "closed_fields_required",
                    "status = Closed requires closed_by and closed_at_utc to be present",
                    &mut findings,
                );
            }
            DatumAlertStatusV1::Open => {}
        }

        findings
    }
}

/// Parses an RFC 3339 timestamp field for `validate()`, pushing a finding
/// (empty or unparseable) and returning `None` on failure so the caller
/// can skip any comparison that depends on it, or `Some(parsed)` on
/// success -- shared by the `first_observed_at_utc <= last_observed_at_utc`
/// check (finding F5), which needs the PARSED instant, never a TEXT
/// comparison (the exact TEXT-ordering pitfall
/// `storage::datum_event_store` already proved unsafe in Phase 123).
fn parse_and_check_timestamp(
    value: &str,
    field_name: &str,
    findings: &mut Vec<CanonicalValidationFinding>,
) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    if value.trim().is_empty() {
        findings.push(finding(
            "missing_field",
            format!("{field_name} must be non-empty"),
        ));
        return None;
    }
    match chrono::DateTime::parse_from_rfc3339(value) {
        Ok(parsed) => Some(parsed),
        Err(_) => {
            findings.push(finding(
                "invalid_timestamp",
                format!("{field_name} is not a valid RFC 3339 timestamp: '{value}'"),
            ));
            None
        }
    }
}

fn check_actor(
    label: &str,
    actor: &Option<DatumAlertActorV1>,
    at_utc: &Option<String>,
    findings: &mut Vec<CanonicalValidationFinding>,
) {
    if actor.is_some() != at_utc.is_some() {
        findings.push(finding(
            "actor_timestamp_pair_inconsistent",
            format!(
                "{label}_by and {label}_at_utc must be present or absent together (by={:?}, at={:?})",
                actor.is_some(),
                at_utc.is_some()
            ),
        ));
    }
    // Phase 124 freeze review, finding F1: `DatumAlertActorV1::validate()`
    // existed but was never actually called from here -- a malformed
    // actor (an invalid `Other(...)` token, or a whitespace-only
    // `actor_id`) was silently accepted. Now genuinely wired in for every
    // actor field, not just the ones a status requires.
    if let Some(actor) = actor {
        findings.extend(actor.validate());
    }
    if let Some(at) = at_utc {
        parse_and_check_timestamp(at, &format!("{label}_at_utc"), findings);
    }
}

fn require_pair_present(
    label: &str,
    actor: &Option<DatumAlertActorV1>,
    at_utc: &Option<String>,
    code: &'static str,
    message: &'static str,
    findings: &mut Vec<CanonicalValidationFinding>,
) {
    let _ = label;
    if actor.is_none() || at_utc.is_none() {
        findings.push(finding(code, message));
    }
}

fn check_reason(
    field_name: &str,
    reason: &Option<String>,
    findings: &mut Vec<CanonicalValidationFinding>,
) {
    if let Some(reason) = reason {
        if reason.trim().is_empty() {
            findings.push(finding(
                "missing_field",
                format!("{field_name}, if present, must be non-empty (normalize empty reasons to None instead)"),
            ));
        } else if reason.len() > MAX_DATUM_ALERT_REASON_BYTES {
            findings.push(finding(
                "reason_too_long",
                format!(
                    "{field_name} is {} bytes, exceeding the {MAX_DATUM_ALERT_REASON_BYTES}-byte limit",
                    reason.len()
                ),
            ));
        }
    }
}

/// The alert lifecycle. See the Phase 124 architecture note for the full
/// per-state semantic description; briefly:
///
/// - `Open`: requires attention, nobody has accepted ownership yet.
/// - `Acknowledged`: an actor explicitly accepted awareness/ownership.
///   Does NOT mean remediation has started.
/// - `InProgress`: actively being investigated or treated.
/// - `Resolved`: the underlying condition is believed no longer active.
///   NOT the same as `Closed` -- a resolved alert may still be reopened.
/// - `Closed`: the lifecycle episode is administratively complete and
///   terminal. See [`is_valid_transition`] -- no transition out of
///   `Closed` exists. A future recurrence of the same condition creates a
///   NEW `alert_id`, never reopens a closed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatumAlertStatusV1 {
    Open,
    Acknowledged,
    InProgress,
    Resolved,
    Closed,
}

/// The exact wire token [`DatumAlertStatusV1::Closed`] serializes to.
/// Phase 124 freeze review, finding F14: the literal `"closed"` was
/// previously duplicated across three separate SQL locations in
/// `storage::datum_alert_store` with nothing tying them together. This
/// constant is the single source of truth every one of those call sites
/// now uses (via `to_string()`), and a dedicated test
/// (`closed_wire_token_matches_partial_index_literal`) pins the DDL's own
/// hardcoded copy (which cannot itself reference a Rust constant) against
/// this value.
pub const CLOSED_STATUS_WIRE_TOKEN: &str = "closed";

/// `status != Closed` -- OPEN/ACKNOWLEDGED/IN_PROGRESS/RESOLVED all count
/// as "active" for the purposes of the at-most-one-active-alert-per-
/// correlation-key uniqueness rule (`storage::datum_alert_store`). This
/// is intentionally a BROADER definition than [`is_operationally_active`]
/// -- a `Resolved` alert still occupies the "active" slot for its
/// correlation key (blocking a new episode from being created) precisely
/// because it can still be reopened.
pub fn is_active(status: DatumAlertStatusV1) -> bool {
    status != DatumAlertStatusV1::Closed
}

/// `Open`/`Acknowledged`/`InProgress` -- the condition genuinely still
/// needs work. Deliberately does NOT include `Resolved`: per the Phase
/// 124 brief's own three-way distinction (operationally active / pending
/// closure / historical), a resolved-but-not-yet-closed alert is no
/// longer something anyone needs to actively WORK on, even though it is
/// still "active" in the narrower `is_active` sense above.
pub fn is_operationally_active(status: DatumAlertStatusV1) -> bool {
    matches!(
        status,
        DatumAlertStatusV1::Open
            | DatumAlertStatusV1::Acknowledged
            | DatumAlertStatusV1::InProgress
    )
}

/// `Resolved` only -- believed fixed, awaiting administrative closure or
/// validation.
pub fn is_pending_closure(status: DatumAlertStatusV1) -> bool {
    status == DatumAlertStatusV1::Resolved
}

/// `Closed` only -- a completed, terminal historical lifecycle episode.
pub fn is_historical(status: DatumAlertStatusV1) -> bool {
    status == DatumAlertStatusV1::Closed
}

/// The Phase 124 lifecycle transition graph -- a pure function of two
/// statuses, deliberately kept here (not in the storage layer) so it can
/// be unit-tested and reasoned about independent of any database.
/// **Unchanged by the hardening pass** -- the freeze review examined the
/// `Open -> InProgress` question explicitly and the brief's own
/// instruction was to keep the graph as-is (a future Console may compose
/// "Start handling" as Acknowledge-then-StartWork if a single-click UX is
/// wanted; that is a Console-layer concern, not a graph change).
///
/// # Chosen edges, and why
///
/// - `Open -> Acknowledged`: the baseline "someone took ownership" step.
/// - `Open -> Resolved`: allowed. A condition can disappear on its own
///   before anyone gets to acknowledge it (e.g. a transient blip);
///   forcing a meaningless acknowledge/in-progress detour first would
///   only produce a misleading provenance trail.
/// - `Acknowledged -> InProgress`: the baseline "work has started" step.
/// - `Acknowledged -> Resolved`: allowed. An immediately-fixed condition
///   does not always warrant a distinct "in progress" phase.
/// - `InProgress -> Resolved`: the baseline "work concluded" step.
/// - `Resolved -> Closed`: the baseline "administratively complete" step.
///   `Closed` can ONLY be reached via `Resolved` -- there is no
///   `Open/Acknowledged/InProgress -> Closed` shortcut, because `Closed`
///   is meant to certify the condition was actually resolved first, even
///   briefly.
/// - `Resolved -> Open`: a REOPEN. The condition recurred (chronologically
///   AFTER the resolution -- see `storage::datum_alert_store`'s
///   reopen-boundary documentation for the out-of-order-event-aware rule
///   added in this hardening pass) before the episode was administratively
///   closed -- still the same episode, continuing.
///
/// # Deliberately NOT allowed
///
/// - `Open -> InProgress` directly: requires going through `Acknowledged`
///   first, so there is always a recorded "who accepted this" moment
///   before "who is working on it."
/// - Any transition OUT of `Closed`: **deliberately, carefully decided
///   terminal.** A recurrence of the same `correlation_key` after closure
///   creates a NEW `alert_id` (a new episode) rather than reopening the
///   old one. The correlation key may therefore map to zero-or-one ACTIVE
///   alert (see [`is_active`]) plus arbitrarily many CLOSED historical
///   ones.
/// - Any other combination not listed above: not evidenced by the brief,
///   not invented here.
pub fn is_valid_transition(from: DatumAlertStatusV1, to: DatumAlertStatusV1) -> bool {
    use DatumAlertStatusV1::{Acknowledged, Closed as ClosedStatus, InProgress, Open, Resolved};
    matches!(
        (from, to),
        (Open, Acknowledged)
            | (Open, Resolved)
            | (Acknowledged, InProgress)
            | (Acknowledged, Resolved)
            | (InProgress, Resolved)
            | (Resolved, ClosedStatus)
            | (Resolved, Open)
    )
}

/// Provenance for who or what performed a lifecycle action or produced a
/// transition. This is AUDIT METADATA ONLY -- it grants no authority by
/// itself. `Gnn`/`Llm`/`RuleEngine`/`Automation` acting on an alert is
/// recorded exactly the same way `Operator` is; nothing in this module or
/// `storage::datum_alert_store` treats any actor type as pre-authorized
/// to perform any action. Phase 124 makes the DATA MODEL capable of
/// recording AI/GNN provenance; it implements no authorization policy for
/// when that provenance is legitimate -- that remains explicit future
/// work.
///
/// Mirrors `DatumEventSourceV1`'s hand-written wire representation
/// exactly (bare lowercase string, `Other(String)` forward-compatible
/// catch-all) -- the same justified local pattern, reused a second time
/// within this crate rather than shared across the module boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum DatumAlertActorTypeV1 {
    Operator,
    System,
    RuleEngine,
    Gnn,
    Llm,
    Automation,
    /// A provenance category not yet given a dedicated variant. See
    /// `DatumEventSourceV1::Other`'s doc comment for the identical
    /// forward-compatibility contract this mirrors.
    Other(String),
}

impl DatumAlertActorTypeV1 {
    fn wire_str(&self) -> &str {
        match self {
            DatumAlertActorTypeV1::Operator => "operator",
            DatumAlertActorTypeV1::System => "system",
            DatumAlertActorTypeV1::RuleEngine => "rule_engine",
            DatumAlertActorTypeV1::Gnn => "gnn",
            DatumAlertActorTypeV1::Llm => "llm",
            DatumAlertActorTypeV1::Automation => "automation",
            DatumAlertActorTypeV1::Other(value) => value.as_str(),
        }
    }

    fn from_wire_token(token: &str) -> DatumAlertActorTypeV1 {
        match token {
            "operator" => DatumAlertActorTypeV1::Operator,
            "system" => DatumAlertActorTypeV1::System,
            "rule_engine" => DatumAlertActorTypeV1::RuleEngine,
            "gnn" => DatumAlertActorTypeV1::Gnn,
            "llm" => DatumAlertActorTypeV1::Llm,
            "automation" => DatumAlertActorTypeV1::Automation,
            other => DatumAlertActorTypeV1::Other(other.to_string()),
        }
    }

    /// `Gnn`/`Llm` actors must carry [`DatumAlertActorV1::model_ref`] --
    /// Phase 124 freeze review, finding F10, mirroring
    /// `DatumEventSourceV1::requires_model_provenance` (Phase 122)
    /// exactly. Every other actor type (including a forward-compatible
    /// `Other(...)`) leaves `model_ref` optional: a `RuleEngine` or
    /// `Automation` actor MAY want to record a model/version reference in
    /// the future, and nothing today justifies forbidding it outright for
    /// those types.
    fn requires_model_provenance(&self) -> bool {
        matches!(
            self,
            DatumAlertActorTypeV1::Gnn | DatumAlertActorTypeV1::Llm
        )
    }

    fn validate(&self) -> Vec<CanonicalValidationFinding> {
        if let DatumAlertActorTypeV1::Other(value) = self {
            if !is_lower_snake_case(value) {
                return vec![finding(
                    "actor_type_other_invalid_format",
                    format!("actor_type (other) must be non-empty lower_snake_case, got '{value}'"),
                )];
            }
            if !matches!(
                DatumAlertActorTypeV1::from_wire_token(value),
                DatumAlertActorTypeV1::Other(_)
            ) {
                return vec![finding(
                    "actor_type_other_collides_with_known_value",
                    format!(
                        "actor_type (other) value '{value}' collides with a known actor type -- construct the typed variant instead"
                    ),
                )];
            }
        }
        Vec::new()
    }
}

impl Serialize for DatumAlertActorTypeV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.wire_str())
    }
}

impl<'de> Deserialize<'de> for DatumAlertActorTypeV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let token = String::deserialize(deserializer)?;
        Ok(DatumAlertActorTypeV1::from_wire_token(&token))
    }
}

/// Full actor identity: a type, an optional caller-supplied identifier,
/// and (new in the hardening pass, finding F10) an optional structured
/// model-provenance reference. `actor_id` identifies the ACTOR/PRINCIPAL
/// (an operator username, an automation job id, a rule engine's rule id)
/// -- `model_ref` identifies the MODEL/VERSION that produced the action,
/// exactly mirroring the distinction Phase 122's `DatumEventV1` already
/// draws between its `source` and its own `model_ref`. The two are
/// deliberately never conflated into one overloaded string field.
#[derive(Debug, Clone, PartialEq)]
pub struct DatumAlertActorV1 {
    pub actor_type: DatumAlertActorTypeV1,
    pub actor_id: Option<String>,
    /// Required (non-empty) when `actor_type` is `Gnn` or `Llm` --
    /// enforced by [`DatumAlertActorV1::validate`]. Optional for every
    /// other actor type. Provenance only -- see the module doc comment's
    /// authority-posture note; this field grants zero authority by
    /// itself.
    pub model_ref: Option<String>,
}

impl DatumAlertActorV1 {
    pub fn system() -> DatumAlertActorV1 {
        DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::System,
            actor_id: None,
            model_ref: None,
        }
    }

    /// Fail-closed structural validation for one actor: `actor_type`'s
    /// own rules (forward-compatible `Other(...)` token format, no
    /// collision with a known token), `actor_id` non-empty if present,
    /// and (finding F10) `model_ref` required and non-empty for
    /// `Gnn`/`Llm`, non-empty if present for every other type. **Actually
    /// invoked from [`DatumAlertV1::validate`] as of this hardening pass
    /// (finding F1) -- previously written but never called from
    /// anywhere, a genuine dead-code validation gap the freeze review
    /// caught.**
    pub fn validate(&self) -> Vec<CanonicalValidationFinding> {
        let mut findings = self.actor_type.validate();
        if let Some(actor_id) = &self.actor_id {
            if actor_id.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "actor_id, if present, must be non-empty",
                ));
            }
        }
        let model_ref_present_and_nonempty = self
            .model_ref
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty());
        if self.actor_type.requires_model_provenance() && !model_ref_present_and_nonempty {
            findings.push(finding(
                "model_ref_required_for_actor_type",
                format!(
                    "model_ref is required and must be non-empty when actor_type is '{}'",
                    self.actor_type.wire_str()
                ),
            ));
        }
        if let Some(model_ref) = &self.model_ref {
            if model_ref.trim().is_empty() {
                findings.push(finding(
                    "missing_field",
                    "model_ref, if present, must be non-empty",
                ));
            }
        }
        findings
    }
}

/// One immutable, append-only lifecycle transition record -- the read
/// model `storage::datum_alert_store::list_alert_transitions` returns.
/// `from_status: None` denotes the alert's CREATION (no prior status);
/// every other transition carries `Some(...)`.
#[derive(Debug, Clone, PartialEq)]
pub struct DatumAlertTransitionV1 {
    pub transition_id: String,
    pub alert_id: String,
    pub from_status: Option<DatumAlertStatusV1>,
    pub to_status: DatumAlertStatusV1,
    pub actor: DatumAlertActorV1,
    pub reason: Option<String>,
    pub transitioned_at_utc: String,
    /// The `DatumEventV1` that triggered this transition, if any (e.g.
    /// the event that caused a reopen). Lifecycle commands invoked
    /// directly by an actor (acknowledge/start-work/resolve/close)
    /// typically leave this `None`.
    pub related_event_id: Option<String>,
}

/// Derives the Alert-Manager-owned canonical correlation key for `event`
/// -- the heart of Phase 124's condition-level deduplication.
/// **Unchanged by the hardening pass** -- the freeze review examined this
/// algorithm in depth (source exclusion, correlation_hint exclusion,
/// category+kind+subject participation) and raised no finding against
/// it.
///
/// # What participates in the key, and why
///
/// - **`category`**, **`kind`**: the "what kind of thing happened" axis.
///   Both are included, even though `kind` alone is usually specific
///   enough, as a deliberate collision guard: `kind` is an OPEN,
///   producer-chosen string (Phase 122 validates its *format*, not
///   cross-producer uniqueness), so two unrelated producers could
///   plausibly reuse the same `kind` string for semantically different
///   conditions in different categories; including `category` too closes
///   that gap at negligible cost.
/// - **`subject`**: the "what operational entity" axis, included in
///   full. `DatumEventSubjectV1` already IS Phase 122's own definition of
///   "what operational entity this occurrence concerns," at exactly the
///   granularity Phase 122 designed (e.g. `DServ`'s optional `node_id`:
///   if two events differ only in `node_id`, they concern different
///   concrete runtime instances and correctly produce different keys).
///   Using the subject verbatim, rather than hand-picking sub-fields per
///   variant, is both simpler and more correct than any manual
///   alternative.
///
/// `project_id`/`application_id` are deliberately NOT separately
/// included: wherever they are semantically relevant to a given subject
/// variant, `DatumEventSubjectV1` already carries them (and Phase 122's
/// own `validate()` guarantees the event's top-level fields are
/// consistent with the subject's), so re-including them would be
/// redundant, not more precise.
///
/// # What never participates, and why
///
/// - **`severity`**: an escalating/de-escalating condition (`warning` ->
///   `high` -> `critical`) is still the SAME condition -- explicitly
///   required by the Phase 124 brief.
/// - **`event_id`**, **`occurred_at_utc`**, **`confidence_permille`**,
///   **`attributes`**: pure occurrence metadata, never condition
///   identity.
/// - **`source`**: deliberately excluded, and this is a real design
///   decision, not an oversight. Two different sources independently
///   observing the SAME real-world fact (e.g. DMonitor and
///   SentinelTelemetry both reporting "fog-01 unreachable") describe ONE
///   condition, not two -- correlating them into a single alert is more
///   correct (and, later, higher-confidence) than fragmenting one real
///   condition into a per-source alert. `source` remains available as
///   occurrence-level provenance on each individually-associated event
///   (via `storage::datum_alert_store::list_alert_events`), never
///   promoted into alert identity.
/// - **`event.correlation_hint`**: Phase 122 defines this as
///   producer-supplied, non-authoritative metadata, and the Phase 124
///   brief is explicit that "the Alert Manager owns the canonical
///   correlation key" and must not blindly trust it. Phase 124's chosen
///   policy: the canonical key is derived ENTIRELY from the
///   Alert-Manager-owned dimensions above; `correlation_hint` is treated
///   as inert, stored-but-unused metadata for this phase.
///
/// # Mechanism
///
/// A JCS + SHA-256 digest (ADR-0006) over `{category, kind, subject}`,
/// reusing Phase 122's own `core::jcs` infrastructure directly -- not a
/// new canonicalization scheme.
pub fn derive_correlation_key(event: &DatumEventV1) -> Result<String, String> {
    let category_value = serde_json::to_value(event.category)
        .map_err(|error| format!("serialize category for correlation key: {error}"))?;
    let subject_value = serde_json::to_value(&event.subject)
        .map_err(|error| format!("serialize subject for correlation key: {error}"))?;
    let identity: Value = serde_json::json!({
        "category": category_value,
        "kind": event.kind,
        "subject": subject_value,
    });
    jcs::canonical_digest_sha256_hex(&identity).map_err(|error| error.to_string())
}

fn is_lower_snake_case(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed == value
        && trimmed
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !trimmed.starts_with('_')
        && !trimmed.ends_with('_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::datum_event::{DatumEventSourceV1, DATUM_EVENT_SCHEMA};
    use serde_json::json;

    fn sample_event(kind: &str, project_id: &str, node_id: &str) -> DatumEventV1 {
        DatumEventV1 {
            schema: DATUM_EVENT_SCHEMA.to_string(),
            event_id: "datumevent:1".to_string(),
            project_id: Some(project_id.to_string()),
            application_id: None,
            category: DatumEventCategoryV1::Observation,
            kind: kind.to_string(),
            severity: DatumEventSeverityV1::Warning,
            source: DatumEventSourceV1::SentinelTelemetry,
            subject: DatumEventSubjectV1::DNode {
                node_id: node_id.to_string(),
            },
            occurred_at_utc: "2026-09-29T12:00:00Z".to_string(),
            evidence_refs: Vec::new(),
            correlation_hint: None,
            confidence_permille: None,
            model_ref: None,
            attributes: json!({}),
        }
    }

    fn operator_actor() -> DatumAlertActorV1 {
        DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Operator,
            actor_id: Some("alice".to_string()),
            model_ref: None,
        }
    }

    fn valid_alert() -> DatumAlertV1 {
        DatumAlertV1 {
            schema: DATUM_ALERT_SCHEMA.to_string(),
            alert_id: "datumalert:1".to_string(),
            project_id: Some("proj:lab".to_string()),
            application_id: None,
            correlation_key: "sha256:abc".to_string(),
            category: DatumEventCategoryV1::Observation,
            kind: "peer_unreachable".to_string(),
            severity: DatumEventSeverityV1::Warning,
            peak_severity: DatumEventSeverityV1::Warning,
            subject: DatumEventSubjectV1::DNode {
                node_id: "fog-01".to_string(),
            },
            status: DatumAlertStatusV1::Open,
            first_observed_at_utc: "2026-09-29T12:00:00Z".to_string(),
            last_observed_at_utc: "2026-09-29T12:00:00Z".to_string(),
            first_event_id: "datumevent:1".to_string(),
            latest_event_id: "datumevent:1".to_string(),
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
        }
    }

    // ------------------------------------------------------ CORRELATION KEY

    #[test]
    fn correlation_key_is_deterministic() {
        let event = sample_event("peer_unreachable", "proj:lab", "fog-01");
        let a = derive_correlation_key(&event).unwrap();
        let b = derive_correlation_key(&event).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
    }

    #[test]
    fn correlation_key_ignores_severity_event_id_and_occurred_at() {
        let mut a = sample_event("peer_unreachable", "proj:lab", "fog-01");
        let mut b = a.clone();
        a.severity = DatumEventSeverityV1::Warning;
        b.severity = DatumEventSeverityV1::Critical;
        a.event_id = "datumevent:aaa".to_string();
        b.event_id = "datumevent:bbb".to_string();
        a.occurred_at_utc = "2026-09-29T12:00:00Z".to_string();
        b.occurred_at_utc = "2026-09-29T12:05:00Z".to_string();
        assert_eq!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap(),
            "severity escalation, event_id, and occurred_at must not affect the correlation key"
        );
    }

    #[test]
    fn correlation_key_ignores_correlation_hint_and_source() {
        let mut a = sample_event("peer_unreachable", "proj:lab", "fog-01");
        let mut b = a.clone();
        a.correlation_hint = None;
        b.correlation_hint = Some("producer-said-group-42".to_string());
        a.source = DatumEventSourceV1::SentinelTelemetry;
        b.source = DatumEventSourceV1::DMonitor;
        assert_eq!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap()
        );
    }

    #[test]
    fn correlation_key_differs_by_kind() {
        let a = sample_event("peer_unreachable", "proj:lab", "fog-01");
        let b = sample_event("disk_full", "proj:lab", "fog-01");
        assert_ne!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap()
        );
    }

    #[test]
    fn correlation_key_differs_by_subject() {
        let a = sample_event("peer_unreachable", "proj:lab", "fog-01");
        let b = sample_event("peer_unreachable", "proj:lab", "fog-02");
        assert_ne!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap()
        );
    }

    #[test]
    fn correlation_key_differs_by_project() {
        let mut a = sample_event("peer_unreachable", "proj:a", "fog-01");
        a.subject = DatumEventSubjectV1::Project {
            project_id: "proj:a".to_string(),
        };
        let mut b = sample_event("peer_unreachable", "proj:b", "fog-01");
        b.subject = DatumEventSubjectV1::Project {
            project_id: "proj:b".to_string(),
        };
        assert_ne!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap(),
            "different project must yield different key (carried via subject)"
        );
    }

    #[test]
    fn correlation_key_for_project_agnostic_dnode_subject_ignores_top_level_project_id() {
        let a = sample_event("peer_unreachable", "proj:a", "fog-01");
        let b = sample_event("peer_unreachable", "proj:b", "fog-01");
        assert_eq!(a.subject, b.subject);
        assert_eq!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap()
        );
    }

    #[test]
    fn correlation_key_differs_by_category_even_with_same_kind() {
        let mut a = sample_event("degraded", "proj:lab", "fog-01");
        let mut b = a.clone();
        a.category = DatumEventCategoryV1::Runtime;
        b.category = DatumEventCategoryV1::Health;
        assert_ne!(
            derive_correlation_key(&a).unwrap(),
            derive_correlation_key(&b).unwrap()
        );
    }

    // ---------------------------------------------------- TRANSITION GRAPH

    #[test]
    fn baseline_forward_path_is_valid() {
        use DatumAlertStatusV1::*;
        assert!(is_valid_transition(Open, Acknowledged));
        assert!(is_valid_transition(Acknowledged, InProgress));
        assert!(is_valid_transition(InProgress, Resolved));
        assert!(is_valid_transition(Resolved, Closed));
    }

    #[test]
    fn documented_shortcuts_are_valid() {
        use DatumAlertStatusV1::*;
        assert!(is_valid_transition(Open, Resolved));
        assert!(is_valid_transition(Acknowledged, Resolved));
        assert!(is_valid_transition(Resolved, Open));
    }

    #[test]
    fn closed_is_terminal() {
        use DatumAlertStatusV1::*;
        for target in [Open, Acknowledged, InProgress, Resolved, Closed] {
            assert!(
                !is_valid_transition(Closed, target),
                "Closed -> {target:?} must be rejected"
            );
        }
    }

    #[test]
    fn open_cannot_skip_directly_to_in_progress() {
        assert!(!is_valid_transition(
            DatumAlertStatusV1::Open,
            DatumAlertStatusV1::InProgress
        ));
    }

    #[test]
    fn no_transition_to_or_from_itself_is_valid() {
        use DatumAlertStatusV1::*;
        for status in [Open, Acknowledged, InProgress, Resolved, Closed] {
            assert!(
                !is_valid_transition(status, status),
                "{status:?} -> itself must be rejected"
            );
        }
    }

    #[test]
    fn active_pending_historical_classification() {
        use DatumAlertStatusV1::*;
        assert!(is_operationally_active(Open));
        assert!(is_operationally_active(Acknowledged));
        assert!(is_operationally_active(InProgress));
        assert!(!is_operationally_active(Resolved));
        assert!(!is_operationally_active(Closed));

        assert!(is_pending_closure(Resolved));
        assert!(!is_pending_closure(Open));
        assert!(!is_pending_closure(Closed));

        assert!(is_historical(Closed));
        assert!(!is_historical(Resolved));

        assert!(is_active(Open));
        assert!(is_active(Resolved));
        assert!(!is_active(Closed));
    }

    #[test]
    fn closed_wire_token_matches_partial_index_literal() {
        assert_eq!(
            serde_json::to_value(DatumAlertStatusV1::Closed).unwrap(),
            json!(CLOSED_STATUS_WIRE_TOKEN)
        );
    }

    // ------------------------------------------------------------ VALIDATE

    #[test]
    fn minimal_valid_alert_validates_cleanly() {
        assert!(
            valid_alert().validate().is_empty(),
            "{:?}",
            valid_alert().validate()
        );
    }

    #[test]
    fn zero_occurrence_count_is_rejected() {
        let mut alert = valid_alert();
        alert.occurrence_count = 0;
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "occurrence_count_invalid"));
    }

    #[test]
    fn inverted_observed_window_is_rejected() {
        let mut alert = valid_alert();
        alert.first_observed_at_utc = "2026-09-29T13:00:00Z".to_string();
        alert.last_observed_at_utc = "2026-09-29T12:00:00Z".to_string();
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "observed_window_inverted"));
    }

    #[test]
    fn equal_observed_window_bounds_are_valid() {
        let alert = valid_alert(); // first == last already
        assert!(alert.validate().is_empty(), "{:?}", alert.validate());
    }

    #[test]
    fn resolved_status_without_resolved_fields_is_rejected() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Resolved;
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "resolved_fields_required"));
    }

    #[test]
    fn closed_status_without_closed_fields_is_rejected() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Closed;
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "closed_fields_required"));
    }

    #[test]
    fn acknowledged_status_without_acknowledged_fields_is_rejected() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Acknowledged;
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "acknowledged_fields_required"));
    }

    #[test]
    fn in_progress_status_without_acknowledged_or_in_progress_fields_is_rejected() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::InProgress;
        let findings = alert.validate();
        assert!(findings
            .iter()
            .any(|f| f.code == "acknowledged_fields_required"));
        assert!(findings
            .iter()
            .any(|f| f.code == "in_progress_fields_required"));
    }

    #[test]
    fn in_progress_status_with_both_field_pairs_is_valid() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::InProgress;
        alert.acknowledged_by = Some(operator_actor());
        alert.acknowledged_at_utc = Some("2026-09-29T11:00:00Z".to_string());
        alert.in_progress_by = Some(operator_actor());
        alert.in_progress_at_utc = Some("2026-09-29T11:30:00Z".to_string());
        assert!(alert.validate().is_empty(), "{:?}", alert.validate());
    }

    #[test]
    fn open_to_resolved_to_closed_without_ever_being_acknowledged_is_valid() {
        // The brief's own explicit example: a legal Open -> Resolved ->
        // Closed episode must not be rejected merely for never having
        // passed through Acknowledged/InProgress.
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Closed;
        alert.resolved_by = Some(DatumAlertActorV1::system());
        alert.resolved_at_utc = Some("2026-09-29T12:05:00Z".to_string());
        alert.closed_by = Some(operator_actor());
        alert.closed_at_utc = Some("2026-09-29T12:10:00Z".to_string());
        assert!(alert.validate().is_empty(), "{:?}", alert.validate());
    }

    #[test]
    fn one_sided_actor_timestamp_pair_is_rejected() {
        let mut alert = valid_alert();
        alert.acknowledged_by = Some(operator_actor());
        // acknowledged_at_utc left None -- inconsistent pair.
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "actor_timestamp_pair_inconsistent"));
    }

    #[test]
    fn open_status_with_stale_acknowledged_fields_after_reopen_is_still_valid() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Open;
        alert.acknowledged_by = Some(operator_actor());
        alert.acknowledged_at_utc = Some("2026-09-29T11:00:00Z".to_string());
        assert!(alert.validate().is_empty(), "{:?}", alert.validate());
    }

    // ---------------------------------------------------- ACTOR VALIDATION

    #[test]
    fn malformed_actor_id_is_rejected_via_alert_validate() {
        // Proves finding F1 is actually fixed: DatumAlertV1::validate()
        // must now surface a bad actor, not silently accept it.
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Acknowledged;
        alert.acknowledged_by = Some(DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Operator,
            actor_id: Some("   ".to_string()),
            model_ref: None,
        });
        alert.acknowledged_at_utc = Some("2026-09-29T11:00:00Z".to_string());
        assert!(alert.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn malformed_other_actor_type_is_rejected_via_alert_validate() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Acknowledged;
        alert.acknowledged_by = Some(DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Other("Not Lower Snake".to_string()),
            actor_id: None,
            model_ref: None,
        });
        alert.acknowledged_at_utc = Some("2026-09-29T11:00:00Z".to_string());
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "actor_type_other_invalid_format"));
    }

    #[test]
    fn gnn_actor_without_model_ref_is_rejected() {
        let actor = DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Gnn,
            actor_id: None,
            model_ref: None,
        };
        assert!(actor
            .validate()
            .iter()
            .any(|f| f.code == "model_ref_required_for_actor_type"));
    }

    #[test]
    fn llm_actor_without_model_ref_is_rejected() {
        let actor = DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Llm,
            actor_id: None,
            model_ref: None,
        };
        assert!(actor
            .validate()
            .iter()
            .any(|f| f.code == "model_ref_required_for_actor_type"));
    }

    #[test]
    fn gnn_actor_with_model_ref_is_valid() {
        let actor = DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Gnn,
            actor_id: Some("gnn-worker-1".to_string()),
            model_ref: Some("gnn:deterministic_one_layer_message_passing".to_string()),
        };
        assert!(actor.validate().is_empty(), "{:?}", actor.validate());
    }

    #[test]
    fn operator_actor_never_requires_model_ref() {
        let actor = operator_actor();
        assert!(actor.validate().is_empty(), "{:?}", actor.validate());
    }

    #[test]
    fn gnn_actor_without_model_ref_is_rejected_via_alert_validate() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Resolved;
        alert.resolved_by = Some(DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Gnn,
            actor_id: None,
            model_ref: None,
        });
        alert.resolved_at_utc = Some("2026-09-29T12:05:00Z".to_string());
        assert!(alert
            .validate()
            .iter()
            .any(|f| f.code == "model_ref_required_for_actor_type"));
    }

    // ---------------------------------------------------------- REASON LEN

    #[test]
    fn empty_resolution_reason_is_rejected() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Resolved;
        alert.resolved_by = Some(DatumAlertActorV1::system());
        alert.resolved_at_utc = Some("2026-09-29T12:05:00Z".to_string());
        alert.resolution_reason = Some("   ".to_string());
        assert!(alert.validate().iter().any(|f| f.code == "missing_field"));
    }

    #[test]
    fn over_long_resolution_reason_is_rejected() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Resolved;
        alert.resolved_by = Some(DatumAlertActorV1::system());
        alert.resolved_at_utc = Some("2026-09-29T12:05:00Z".to_string());
        alert.resolution_reason = Some("x".repeat(MAX_DATUM_ALERT_REASON_BYTES + 1));
        assert!(alert.validate().iter().any(|f| f.code == "reason_too_long"));
    }

    #[test]
    fn resolution_reason_at_exactly_the_limit_is_valid() {
        let mut alert = valid_alert();
        alert.status = DatumAlertStatusV1::Resolved;
        alert.resolved_by = Some(DatumAlertActorV1::system());
        alert.resolved_at_utc = Some("2026-09-29T12:05:00Z".to_string());
        alert.resolution_reason = Some("x".repeat(MAX_DATUM_ALERT_REASON_BYTES));
        assert!(alert.validate().is_empty(), "{:?}", alert.validate());
    }

    // -------------------------------------------------------------- ACTORS

    #[test]
    fn unknown_actor_type_round_trips_and_preserves_identity() {
        let actor = DatumAlertActorV1 {
            actor_type: DatumAlertActorTypeV1::Other("external_webhook".to_string()),
            actor_id: Some("hook-1".to_string()),
            model_ref: None,
        };
        let value = serde_json::to_value(&actor.actor_type).unwrap();
        assert_eq!(value, json!("external_webhook"));
        let restored: DatumAlertActorTypeV1 = serde_json::from_value(value).unwrap();
        assert_eq!(
            restored,
            DatumAlertActorTypeV1::Other("external_webhook".to_string())
        );
    }

    #[test]
    fn known_actor_types_serialize_to_expected_tokens() {
        let expectations = [
            (DatumAlertActorTypeV1::Operator, "operator"),
            (DatumAlertActorTypeV1::System, "system"),
            (DatumAlertActorTypeV1::RuleEngine, "rule_engine"),
            (DatumAlertActorTypeV1::Gnn, "gnn"),
            (DatumAlertActorTypeV1::Llm, "llm"),
            (DatumAlertActorTypeV1::Automation, "automation"),
        ];
        for (actor_type, token) in expectations {
            assert_eq!(serde_json::to_value(&actor_type).unwrap(), json!(token));
        }
    }
}
