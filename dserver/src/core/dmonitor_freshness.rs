//! Server-derived DMonitor freshness evaluation (Phase 114B1, ADR-0024).
//!
//! Freshness is NOT part of `CanonicalDMonitorObservationV1` -- it is a
//! POLICY judgment the server computes, kept structurally separate from
//! three other concepts that must never collapse into it:
//!
//! - **health** (`DMonitorHealthV1`): what the collector believed AT
//!   `observed_at_utc`. An observation can be `Healthy` and evaluate as
//!   [`DMonitorFreshnessV1::Stale`] with no contradiction -- this module
//!   never rewrites or reads health to decide freshness, and never lets
//!   freshness imply a health verdict.
//! - **current/head**: an ORDERING concept (`DMonitorStateV1::stream_head`
//!   simply returns whatever the newest accepted sequence is). The newest
//!   accepted observation in a stream is not automatically fresh -- a
//!   monitor that stopped reporting hours ago still has a "current" head,
//!   and evaluating it against a real policy correctly returns `Stale`.
//! - **convergence** (does this observation agree with active DMap/DForward
//!   authority): out of scope for Phase 114B1/114C's job, not this module's.
//!
//! [`evaluate_freshness`] is a pure function: it takes the evaluation time
//! as an explicit parameter rather than calling `Utc::now()` itself, so it
//! is deterministically unit-testable and so a caller (a future batch
//! freshness sweep, say) can evaluate many records against one consistent
//! "now" rather than a slightly different one per record.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::dmonitor_state::DMonitorObservationRecordV1;

/// All three bounds are expressed in seconds and must be non-negative by
/// construction (`u64`) -- there is no way to construct a policy that
/// permits negative age, and no field here can be influenced by anything in
/// a request body (it is server configuration, not caller input).
///
/// `Serialize`/`Deserialize` (Phase 114C): the convergence diagnostic API
/// echoes the exact policy it evaluated against back in its response, and
/// accepts one as an explicit per-request DIAGNOSTIC input (never a
/// server-owned governance policy -- see `dmonitor_interpretation`'s module
/// doc). Widening this type's derive list is purely additive; B1's own
/// evaluation contract is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DMonitorFreshnessPolicyV1 {
    /// Maximum acceptable `evaluation_time - observed_at_utc`.
    pub max_observed_age_seconds: u64,
    /// Maximum acceptable `evaluation_time - received_at_utc`.
    pub max_received_age_seconds: u64,
    /// Maximum acceptable `observed_at_utc - received_at_utc` (a node
    /// reporting an observation time AHEAD of when the server received it).
    /// A small positive allowance absorbs ordinary clock skew; anything
    /// beyond it is treated as untrustworthy rather than "very fresh" --
    /// see the module doc and section 15's explicit anti-gaming rule: a node
    /// must never be able to manufacture permanent freshness by claiming a
    /// future `observed_at_utc`.
    pub max_future_clock_skew_seconds: u64,
}

/// A conservative default: five minutes of observed/received staleness
/// tolerance, thirty seconds of clock-skew tolerance. Not wired into any
/// production code path automatically -- a caller must choose (or override)
/// a policy explicitly; this exists only so tests and future callers are not
/// forced to hand-roll the same numbers repeatedly.
impl Default for DMonitorFreshnessPolicyV1 {
    fn default() -> Self {
        Self {
            max_observed_age_seconds: 300,
            max_received_age_seconds: 300,
            max_future_clock_skew_seconds: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DMonitorFreshnessV1 {
    Fresh,
    Stale,
    /// `observed_at_utc` is ahead of `received_at_utc` by more than
    /// `max_future_clock_skew_seconds`. Takes precedence over the plain
    /// age comparisons -- an untrustworthy clock is reported as such, never
    /// silently classified as merely "stale" or, worse, "fresh".
    ClockSkew,
}

/// Fail-closed input validation for a CALLER-SUPPLIED freshness policy
/// (Phase 114C, section 32). `evaluate_freshness` itself casts each bound
/// to `i64` (`policy.max_future_clock_skew_seconds as i64`, etc.); an
/// internal caller always supplies a small, sane `u64` (seconds), but an
/// HTTP caller could supply a value exceeding `i64::MAX`, which an `as i64`
/// cast silently reinterprets as a NEGATIVE number (two's-complement
/// truncation) rather than erroring -- turning, for example,
/// `max_future_clock_skew_seconds: u64::MAX` into a skew bound of `-1`,
/// which would then spuriously classify nearly every real evaluation as
/// `ClockSkew`. This validator rejects any bound whose `u64` value exceeds
/// `i64::MAX` BEFORE it ever reaches `evaluate_freshness`, closing that gap
/// at the trust boundary rather than changing `evaluate_freshness`'s own
/// established contract (which stays a pure, infallible-on-policy-shape
/// function for its existing internal callers).
pub fn validate_freshness_policy(policy: &DMonitorFreshnessPolicyV1) -> Result<(), String> {
    let ceiling = i64::MAX as u64;
    if policy.max_observed_age_seconds > ceiling {
        return Err(format!(
            "max_observed_age_seconds {} exceeds the maximum representable bound {ceiling}",
            policy.max_observed_age_seconds
        ));
    }
    if policy.max_received_age_seconds > ceiling {
        return Err(format!(
            "max_received_age_seconds {} exceeds the maximum representable bound {ceiling}",
            policy.max_received_age_seconds
        ));
    }
    if policy.max_future_clock_skew_seconds > ceiling {
        return Err(format!(
            "max_future_clock_skew_seconds {} exceeds the maximum representable bound {ceiling}",
            policy.max_future_clock_skew_seconds
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DMonitorFreshnessEvaluationV1 {
    /// Independent age predicate, including when ClockSkew takes display priority.
    /// Exposed for condition-scoped recovery; uses the exact existing age rules.
    pub age_dimensions_satisfied: bool,
    pub freshness: DMonitorFreshnessV1,
    /// `evaluation_time - observed_at_utc`, in seconds. May be negative if
    /// `evaluation_time` somehow precedes the observation (e.g. a caller
    /// evaluating against a fixed past instant in a test) -- never clamped,
    /// so the exact signed value is always available to a caller that wants
    /// it.
    pub observed_age_seconds: i64,
    /// `evaluation_time - received_at_utc`, in seconds. Same non-clamping
    /// rule as `observed_age_seconds`.
    pub received_age_seconds: i64,
}

/// Evaluates one record's freshness against `policy` as of
/// `evaluation_time_utc` (RFC 3339). Never reads the system clock itself.
///
/// Fails closed (Phase 114B1.1 closure, section 13) when `evaluation_time`
/// precedes `received_at`: `received_at` is the server's own authoritative
/// anchor for when this record came to exist in durable storage, so
/// evaluating freshness "as of" a point in time before that is an
/// impossible clock ordering, not merely an unusual one -- returning a
/// (necessarily negative) age and silently calling that `Fresh` would be
/// more misleading than refusing to answer. `observed_at` exceeding
/// `evaluation_time` is NOT rejected here (a small positive skew between
/// observation and evaluation is ordinary and already governed by
/// `max_future_clock_skew_seconds` against `received_at`, not against
/// `evaluation_time`).
pub fn evaluate_freshness(
    record: &DMonitorObservationRecordV1,
    policy: &DMonitorFreshnessPolicyV1,
    evaluation_time_utc: &str,
) -> Result<DMonitorFreshnessEvaluationV1, String> {
    let observed_at = parse_timestamp(&record.observation.observed_at_utc, "observed_at_utc")?;
    let received_at = parse_timestamp(&record.received_at_utc, "received_at_utc")?;
    let evaluated_at = parse_timestamp(evaluation_time_utc, "evaluation_time_utc")?;

    if evaluated_at < received_at {
        return Err(format!(
            "evaluation_time_utc '{evaluation_time_utc}' precedes received_at_utc '{}' -- freshness cannot be evaluated for a point in time before the record existed",
            record.received_at_utc
        ));
    }

    let observed_age_seconds = (evaluated_at - observed_at).num_seconds();
    let received_age_seconds = (evaluated_at - received_at).num_seconds();
    let observed_ahead_of_received_seconds = (observed_at - received_at).num_seconds();
    let within_observed_age = observed_age_seconds <= policy.max_observed_age_seconds as i64;
    let within_received_age = received_age_seconds <= policy.max_received_age_seconds as i64;
    let age_dimensions_satisfied = within_observed_age && within_received_age;

    if observed_ahead_of_received_seconds > policy.max_future_clock_skew_seconds as i64 {
        return Ok(DMonitorFreshnessEvaluationV1 {
            age_dimensions_satisfied,
            freshness: DMonitorFreshnessV1::ClockSkew,
            observed_age_seconds,
            received_age_seconds,
        });
    }

    let freshness = if within_observed_age && within_received_age {
        DMonitorFreshnessV1::Fresh
    } else {
        DMonitorFreshnessV1::Stale
    };

    Ok(DMonitorFreshnessEvaluationV1 {
        age_dimensions_satisfied,
        freshness,
        observed_age_seconds,
        received_age_seconds,
    })
}

pub(crate) fn parse_timestamp(value: &str, label: &str) -> Result<DateTime<Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|error| format!("{label} is not a valid RFC 3339 timestamp: '{value}': {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorHealthV1, DMonitorSubjectV1,
        CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use serde_json::json;

    // ---------------------------------------------- PHASE 114C CLOSURE
    // (freshness policy validation)

    #[test]
    fn a_sane_policy_validates() {
        assert!(validate_freshness_policy(&DMonitorFreshnessPolicyV1::default()).is_ok());
    }

    #[test]
    fn zero_bounds_are_valid() {
        let policy = DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 0,
            max_received_age_seconds: 0,
            max_future_clock_skew_seconds: 0,
        };
        assert!(validate_freshness_policy(&policy).is_ok());
    }

    #[test]
    fn i64_max_bound_is_valid() {
        let ceiling = i64::MAX as u64;
        let policy = DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: ceiling,
            max_received_age_seconds: ceiling,
            max_future_clock_skew_seconds: ceiling,
        };
        assert!(validate_freshness_policy(&policy).is_ok());
    }

    #[test]
    fn one_past_i64_max_is_rejected_for_each_field() {
        let ceiling = i64::MAX as u64;
        let base = DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 1,
            max_received_age_seconds: 1,
            max_future_clock_skew_seconds: 1,
        };

        let mut observed_overflow = base;
        observed_overflow.max_observed_age_seconds = ceiling + 1;
        assert!(validate_freshness_policy(&observed_overflow).is_err());

        let mut received_overflow = base;
        received_overflow.max_received_age_seconds = ceiling + 1;
        assert!(validate_freshness_policy(&received_overflow).is_err());

        let mut skew_overflow = base;
        skew_overflow.max_future_clock_skew_seconds = ceiling + 1;
        assert!(validate_freshness_policy(&skew_overflow).is_err());
    }

    #[test]
    fn u64_max_is_rejected_and_would_otherwise_silently_invert_to_negative() {
        // Documents the exact defect class this validator closes: casting
        // u64::MAX to i64 wraps to -1, which (if this validator did not
        // exist) would make `evaluate_freshness`'s clock-skew comparison
        // spuriously fire for nearly every real evaluation.
        assert_eq!(u64::MAX as i64, -1);
        let policy = DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 60,
            max_received_age_seconds: 60,
            max_future_clock_skew_seconds: u64::MAX,
        };
        assert!(validate_freshness_policy(&policy).is_err());
    }

    fn record(
        observed_at_utc: &str,
        received_at_utc: &str,
        health: DMonitorHealthV1,
    ) -> DMonitorObservationRecordV1 {
        let observation = CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: "dmonitor-obs:11111111-1111-1111-1111-111111111111".to_string(),
            project_id: None,
            node_id: "fog-01".to_string(),
            monitor_instance_id: "fog-01-monitor-1".to_string(),
            sequence: 1,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "host_telemetry_linux".to_string(),
            subject: DMonitorSubjectV1::DNode {
                node_id: "fog-01".to_string(),
            },
            health,
            facts: json!({"cpu_usage_permille": 100}),
        };
        DMonitorObservationRecordV1 {
            observation_digest:
                crate::core::canonical_dmonitor::recompute_dmonitor_observation_digest(&observation)
                    .unwrap(),
            observation,
            server_project_scope: "proj:lab".to_string(),
            received_at_utc: received_at_utc.to_string(),
        }
    }

    fn policy() -> DMonitorFreshnessPolicyV1 {
        DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 60,
            max_received_age_seconds: 60,
            max_future_clock_skew_seconds: 10,
        }
    }

    #[test]
    fn fresh_observation_is_fresh() {
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:00:10Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Fresh);
        assert_eq!(result.observed_age_seconds, 10);
        assert_eq!(result.received_age_seconds, 10);
    }

    #[test]
    fn stale_by_observed_age() {
        // observed long ago, but received (and thus evaluated) recently --
        // isolates the observed-age half of the policy from the received-age
        // half.
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:05:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:05:05Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Stale);
        assert!(result.observed_age_seconds > policy().max_observed_age_seconds as i64);
    }

    #[test]
    fn stale_by_received_age() {
        // observed recently relative to evaluation time is impossible if
        // received is old and evaluation is even later, so isolate purely:
        // received a long time before evaluation time, observed at the same
        // instant as received (no clock skew), but evaluation happens long
        // after both.
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:05:00Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Stale);
        assert!(result.received_age_seconds > policy().max_received_age_seconds as i64);
    }

    #[test]
    fn future_timestamp_within_allowed_skew_is_not_clock_skew() {
        // observed_at 5s ahead of received_at, policy allows up to 10s.
        let r = record(
            "2026-09-25T00:00:05Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:00:06Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Fresh);
    }

    #[test]
    fn future_timestamp_beyond_allowed_skew_fails_closed() {
        // observed_at 1 hour ahead of received_at -- an attempt to
        // manufacture permanent freshness by lying about observed_at must
        // fail closed as ClockSkew, never be accepted as Fresh.
        let r = record(
            "2026-09-25T01:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:00:01Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::ClockSkew);
    }

    #[test]
    fn healthy_and_stale_remains_possible() {
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T01:00:00Z").unwrap();
        assert_eq!(r.observation.health, DMonitorHealthV1::Healthy);
        assert_eq!(result.freshness, DMonitorFreshnessV1::Stale);
    }

    #[test]
    fn unhealthy_and_fresh_remains_possible() {
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Unhealthy,
        );
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:00:05Z").unwrap();
        assert_eq!(r.observation.health, DMonitorHealthV1::Unhealthy);
        assert_eq!(result.freshness, DMonitorFreshnessV1::Fresh);
    }

    #[test]
    fn current_head_does_not_imply_fresh() {
        // A stream's head is whatever the newest accepted sequence is,
        // regardless of how old it now is -- ordering (`stream_head`) and
        // freshness (`evaluate_freshness`) are independently computed and
        // must not be conflated.
        use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};
        let mut state = DMonitorStateV1::empty();
        let observation = CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: "dmonitor-obs:22222222-1111-1111-1111-111111111111".to_string(),
            project_id: None,
            node_id: "fog-01".to_string(),
            monitor_instance_id: "fog-01-monitor-1".to_string(),
            sequence: 1,
            observed_at_utc: "2026-09-25T00:00:00Z".to_string(),
            cycle_id: None,
            collector: "host_telemetry_linux".to_string(),
            subject: DMonitorSubjectV1::DNode {
                node_id: "fog-01".to_string(),
            },
            health: DMonitorHealthV1::Healthy,
            facts: json!({}),
        };
        ingest_observation(&mut state, "proj:lab", observation, "2026-09-25T00:00:00Z").unwrap();
        let head = state.stream_head("proj:lab", "fog-01", "fog-01-monitor-1").unwrap();
        // The head exists (it IS current) but evaluating it a full day later
        // is Stale under any reasonable policy.
        let result = evaluate_freshness(head, &policy(), "2026-09-26T00:00:00Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Stale);
    }

    #[test]
    fn invalid_timestamp_fails_closed() {
        let r = record("not-a-timestamp", "2026-09-25T00:00:00Z", DMonitorHealthV1::Healthy);
        assert!(evaluate_freshness(&r, &policy(), "2026-09-25T00:00:05Z").is_err());
    }

    // ------------------------------------------------- PHASE 114B1.1 CLOSURE

    /// Section 13: evaluating "as of" a point in time BEFORE the server even
    /// received the record is an impossible clock ordering and must fail
    /// closed, not silently return a negative age classified as Fresh.
    #[test]
    fn evaluation_time_before_received_at_fails_closed() {
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:10Z",
            DMonitorHealthV1::Healthy,
        );
        let error = evaluate_freshness(&r, &policy(), "2026-09-25T00:00:05Z").unwrap_err();
        assert!(error.contains("precedes received_at_utc"), "{error}");
    }

    /// `observed_at` slightly ahead of `evaluation_time` (but still within
    /// the allowed future-skew tolerance measured against `received_at`) is
    /// ordinary and must not be rejected.
    #[test]
    fn evaluation_time_before_observed_at_within_allowed_skew_is_not_rejected() {
        let r = record(
            "2026-09-25T00:00:08Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        // evaluated_at (00:00:05) is BEFORE observed_at (00:00:08) but AFTER
        // received_at (00:00:00); observed_at is 8s ahead of received_at,
        // within the 10s skew allowance.
        let result = evaluate_freshness(&r, &policy(), "2026-09-25T00:00:05Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Fresh);
    }

    /// Zero max ages: freshness must be exact, not "approximately now".
    #[test]
    fn zero_max_ages_reject_any_nonzero_age_but_accept_exact_zero() {
        let zero_policy = DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 0,
            max_received_age_seconds: 0,
            max_future_clock_skew_seconds: 0,
        };
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let exact = evaluate_freshness(&r, &zero_policy, "2026-09-25T00:00:00Z").unwrap();
        assert_eq!(exact.freshness, DMonitorFreshnessV1::Fresh);

        let one_second_later = evaluate_freshness(&r, &zero_policy, "2026-09-25T00:00:01Z").unwrap();
        assert_eq!(one_second_later.freshness, DMonitorFreshnessV1::Stale);
    }

    /// The observed-age boundary is INCLUSIVE: exactly at
    /// `max_observed_age_seconds` is still Fresh, one second beyond is Stale.
    #[test]
    fn observed_age_exact_boundary_is_fresh_one_second_beyond_is_stale() {
        let r = record(
            "2026-09-25T00:00:00Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let at_boundary = evaluate_freshness(&r, &policy(), "2026-09-25T00:01:00Z").unwrap();
        assert_eq!(at_boundary.observed_age_seconds, 60);
        assert_eq!(at_boundary.freshness, DMonitorFreshnessV1::Fresh);

        let beyond_boundary = evaluate_freshness(&r, &policy(), "2026-09-25T00:01:01Z").unwrap();
        assert_eq!(beyond_boundary.freshness, DMonitorFreshnessV1::Stale);
    }

    /// The future-clock-skew boundary is also INCLUSIVE: exactly at
    /// `max_future_clock_skew_seconds` is allowed (not ClockSkew), one
    /// second beyond fails closed.
    #[test]
    fn future_skew_exact_boundary_is_allowed_one_second_beyond_is_clock_skew() {
        let at_boundary = record(
            "2026-09-25T00:00:10Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&at_boundary, &policy(), "2026-09-25T00:00:10Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::Fresh);

        let beyond_boundary = record(
            "2026-09-25T00:00:11Z",
            "2026-09-25T00:00:00Z",
            DMonitorHealthV1::Healthy,
        );
        let result = evaluate_freshness(&beyond_boundary, &policy(), "2026-09-25T00:00:11Z").unwrap();
        assert_eq!(result.freshness, DMonitorFreshnessV1::ClockSkew);
    }

    /// Section 24 restart proof: ingest an observation, persist it, drop the
    /// in-memory state entirely and reload from disk, then confirm every
    /// identity fact survived the round trip byte-for-byte (digest,
    /// `received_at_utc`, stream sequence, typed subject identity) via BOTH
    /// read paths (`stream_head`, `by_observation_id`), and that freshness
    /// can still be evaluated correctly against the reloaded record.
    #[test]
    fn restart_proof_reload_preserves_identity_and_evaluates_freshness() {
        use crate::core::dmonitor_state::{ingest_observation, load_state, persist_state, DMonitorStateV1};

        let path = std::env::temp_dir().join(format!(
            "datum-dmonitor-restart-proof-{}.db",
            uuid::Uuid::new_v4()
        ));

        let observation = CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: "dmonitor-obs:33333333-1111-1111-1111-111111111111".to_string(),
            project_id: None,
            node_id: "fog-01".to_string(),
            monitor_instance_id: "fog-01-monitor-1".to_string(),
            sequence: 7,
            observed_at_utc: "2026-09-25T00:00:00Z".to_string(),
            cycle_id: None,
            collector: "host_telemetry_linux".to_string(),
            subject: DMonitorSubjectV1::NetworkReachability {
                node_id: "fog-01".to_string(),
                target: "cloud-01:1883".to_string(),
            },
            health: DMonitorHealthV1::Degraded,
            facts: json!({"latency_ms": 42}),
        };

        // 1. ingest.
        let mut state = DMonitorStateV1::empty();
        let ingested = ingest_observation(&mut state, "proj:lab", observation, "2026-09-25T00:00:01Z").unwrap();
        let original_digest = ingested.record.observation_digest.clone();

        // 2. persist state.
        persist_state(&path, &state).unwrap();

        // 3. reconstruct/reload DServer state from a fresh, empty in-memory
        // value -- proving nothing survives except what was actually
        // persisted to disk.
        drop(state);
        let reloaded = load_state(&path).unwrap();

        // 4/5. query via both read paths and verify every identity fact.
        let via_head = reloaded.stream_head("proj:lab", "fog-01", "fog-01-monitor-1").unwrap();
        let via_id = reloaded
            .by_observation_id("dmonitor-obs:33333333-1111-1111-1111-111111111111")
            .unwrap();
        assert_eq!(via_head, via_id, "both read paths must resolve to the identical record");
        assert_eq!(via_head.observation_digest, original_digest, "canonical digest must survive reload unchanged");
        assert_eq!(via_head.received_at_utc, "2026-09-25T00:00:01Z", "received_at must survive reload unchanged");
        assert_eq!(via_head.observation.sequence, 7, "stream sequence must survive reload unchanged");
        assert_eq!(
            via_head.observation.subject,
            DMonitorSubjectV1::NetworkReachability {
                node_id: "fog-01".to_string(),
                target: "cloud-01:1883".to_string(),
            },
            "typed subject identity must survive reload unchanged"
        );

        // 6. evaluate freshness after reload.
        let evaluation = evaluate_freshness(via_head, &policy(), "2026-09-25T00:00:30Z").unwrap();
        assert_eq!(evaluation.freshness, DMonitorFreshnessV1::Fresh);

        std::fs::remove_file(&path).ok();
        std::fs::remove_file(path.with_extension("db-wal")).ok();
        std::fs::remove_file(path.with_extension("db-shm")).ok();
    }
}
