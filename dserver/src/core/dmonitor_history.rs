//! Canonical historical read model over persisted DMonitor evidence (Phase
//! 140A). A VIEW over `DMonitorStateV1` -- not a second monitoring model,
//! not a new event store, and not a retention/pruning fix (Phase 139A's own
//! finding that DMonitor history retention is currently unbounded remains a
//! documented, deferred P2; this module queries whatever is already
//! persisted, exactly as persisted, bounded and ordered).
//!
//! # Honest historical semantics (no retroactive authority)
//!
//! This module never reads `core::ddeploy::active_placement_authority` or
//! any other authority source, past or present, and never computes
//! `DMonitorConvergenceV1`/`AuthorityCorrelationV1` for a historical row. A
//! past observation is exposed exactly as it was persisted (subject,
//! health, both timestamps, raw `facts`) plus ONE honestly-derivable,
//! authority-independent judgment: freshness (`DMonitorFreshnessV1`),
//! computed via the EXISTING, unmodified `dmonitor_freshness::evaluate_freshness`
//! against the query's own `queried_at_utc` -- never a new freshness
//! concept. `ClockSkew` in particular is a pure function of the two
//! timestamps already persisted on the record (`observed_at_utc` vs.
//! `received_at_utc`), independent of "now", so it remains exactly as
//! trustworthy for a historical row as for a live one. `Stale` is the
//! expected, honest answer for most historical rows (they are, definitionally,
//! old) -- this is not fabrication, it is the same canonical rule applied
//! consistently.
//!
//! # Identity: logical, never ephemeral-runtime-instance
//!
//! Filtering is limited to the SAME logical identity Phase 139 already
//! established: project (the server's own enforced scope, from
//! `DMonitorStreamV1::project_scope` -- never re-trusted from the
//! observation's own `project_id` a second way), `node_id` (observation
//! top-level, universal across every subject variant), `application_id`,
//! and a generic logical "service-like" id extracted per subject variant
//! (`service_id` for `DServRealization`/`DCodeExecution`,
//! `diot_placement_id` for `DIoTRealization`, `call_id` for
//! `DCallDelivery`; `None` for `DNode`/`RuntimeProcess`/
//! `NetworkReachability`/`Unbound`, which carry no application-scoped
//! logical subject at all). PID/container-id/`monitor_instance_id` are
//! never exposed as a filterable identity dimension here.
//!
//! # Bounded by construction
//!
//! A query always has an explicit `[start_utc, end_utc]` window (defaulted
//! to the most recent [`DEFAULT_HISTORY_WINDOW_SECONDS`] when the caller
//! omits either bound) capped at [`MAX_HISTORY_WINDOW_SECONDS`], and a
//! `limit` clamped to [`MAX_HISTORY_RESULTS`]. There is no unbounded
//! "return everything" mode. Ordering is primary-chronological ascending by
//! `received_at_utc` (the one timestamp only the server can honestly
//! assert -- never the node-authored `observed_at_utc`, which is exactly
//! the field ClockSkew exists to distrust for time reasoning), tied-broken
//! by `observation_id` (stable, unique, the same tie-break convention
//! `dmonitor_interpretation`'s own evidence ordering already uses) so
//! repeated identical queries over unchanged persistence are always
//! byte-identical in row order.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::core::canonical_dmonitor::DMonitorSubjectV1;
use crate::core::dmonitor_freshness::{evaluate_freshness, parse_timestamp, DMonitorFreshnessPolicyV1, DMonitorFreshnessV1};
use crate::core::dmonitor_state::{DMonitorObservationRecordV1, DMonitorStateV1};

/// Never widen past this regardless of what a caller requests (Part C).
pub const MAX_HISTORY_RESULTS: usize = 500;
/// Default window end-to-start when a caller supplies neither bound.
pub const DEFAULT_HISTORY_WINDOW_SECONDS: i64 = 86_400;
/// Reject (never silently clamp) a requested window wider than this --
/// unlike `limit`, a time window being silently narrowed would make the
/// response quietly incomplete in a way a caller has no way to detect from
/// the response shape alone, so an oversized window is a clear error
/// instead (Part C: "reject or safely constrain unreasonable requests").
pub const MAX_HISTORY_WINDOW_SECONDS: i64 = 7 * 86_400;

#[derive(Debug, Clone, Default)]
pub struct DMonitorHistoryFilterV1 {
    pub application_id: Option<String>,
    pub node_id: Option<String>,
    pub service_id: Option<String>,
}

/// One persisted observation, exposed honestly: everything that was
/// actually recorded, plus the one authority-independent judgment
/// (`freshness`) this module is allowed to derive. No convergence, no
/// authority correlation.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct HistoricalDMonitorObservationV1 {
    pub observation_id: String,
    pub node_id: String,
    pub monitor_instance_id: String,
    pub sequence: u64,
    pub collector: String,
    pub subject: DMonitorSubjectV1,
    pub health: crate::core::canonical_dmonitor::DMonitorHealthV1,
    pub observed_at_utc: String,
    pub received_at_utc: String,
    pub freshness: DMonitorFreshnessV1,
    pub facts: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct DMonitorHistoryFiltersEchoV1 {
    pub application_id: Option<String>,
    pub node_id: Option<String>,
    pub service_id: Option<String>,
}

pub const DMONITOR_HISTORY_SCHEMA: &str = "datum.dmonitor-history/1";

#[derive(Debug, Clone, Serialize)]
pub struct DMonitorHistoryQueryResponseV1 {
    pub schema: String,
    pub project_id: String,
    pub start_utc: String,
    pub end_utc: String,
    pub queried_at_utc: String,
    pub filters: DMonitorHistoryFiltersEchoV1,
    pub limit: usize,
    pub count: usize,
    /// True when more rows matched the filters/window than `limit` allows
    /// -- never a silently-incomplete response with no indication of it.
    pub truncated: bool,
    pub observations: Vec<HistoricalDMonitorObservationV1>,
}

/// Generic extraction of the two application-scoped logical identity
/// fields a subject MAY carry. `node_id` is deliberately NOT extracted here
/// -- every subject variant observation also carries a top-level,
/// universally-present `node_id` on `CanonicalDMonitorObservationV1` itself
/// (not every subject variant has its own), so node filtering always uses
/// that top-level field instead of this per-variant extraction.
fn subject_logical_identity(subject: &DMonitorSubjectV1) -> (Option<&str>, Option<&str>) {
    match subject {
        DMonitorSubjectV1::DNode { .. }
        | DMonitorSubjectV1::RuntimeProcess { .. }
        | DMonitorSubjectV1::NetworkReachability { .. }
        | DMonitorSubjectV1::Unbound { .. } => (None, None),
        DMonitorSubjectV1::DServRealization { application_id, service_id, .. } => (Some(application_id.as_str()), Some(service_id.as_str())),
        DMonitorSubjectV1::DCodeExecution { application_id, service_id, .. } => (Some(application_id.as_str()), Some(service_id.as_str())),
        DMonitorSubjectV1::DCallDelivery { application_id, call_id, .. } => (Some(application_id.as_str()), Some(call_id.as_str())),
        DMonitorSubjectV1::DIoTRealization { application_id, diot_placement_id, .. } => (Some(application_id.as_str()), Some(diot_placement_id.as_str())),
    }
}

fn matches_optional(filter: Option<&str>, actual: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(wanted) => actual == Some(wanted),
    }
}

/// Validates and resolves the caller-supplied (or defaulted) window,
/// rejecting anything unreasonable rather than silently narrowing it.
fn resolve_window(start_utc: Option<&str>, end_utc: Option<&str>, queried_at_utc: &str) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    let now = parse_timestamp(queried_at_utc, "queried_at_utc")?;
    let end = match end_utc {
        Some(value) => parse_timestamp(value, "end_utc")?,
        None => now,
    };
    let start = match start_utc {
        Some(value) => parse_timestamp(value, "start_utc")?,
        None => end - chrono::Duration::seconds(DEFAULT_HISTORY_WINDOW_SECONDS),
    };
    if start > end {
        return Err(format!("start_utc '{start}' must not be after end_utc '{end}'"));
    }
    let window_seconds = (end - start).num_seconds();
    if window_seconds > MAX_HISTORY_WINDOW_SECONDS {
        return Err(format!(
            "requested window of {window_seconds}s exceeds the maximum of {MAX_HISTORY_WINDOW_SECONDS}s ({} days) -- narrow start_utc/end_utc",
            MAX_HISTORY_WINDOW_SECONDS / 86_400
        ));
    }
    Ok((start, end))
}

/// Pure (apart from parsing the current-time-relative freshness judgment):
/// no I/O, no mutation, reads only the `&DMonitorStateV1` already loaded by
/// the caller. Returns `Err` only for a malformed/unreasonable request
/// (bad timestamp, start after end, oversized window) -- never for "no
/// matching rows", which is an honest empty, `Ok` result (Part: "empty
/// range returns an honest empty result").
#[allow(clippy::too_many_arguments)]
pub fn query_dmonitor_history(
    state: &DMonitorStateV1,
    project_scope: &str,
    filter: &DMonitorHistoryFilterV1,
    start_utc: Option<&str>,
    end_utc: Option<&str>,
    limit: usize,
    queried_at_utc: &str,
    freshness_policy: &DMonitorFreshnessPolicyV1,
) -> Result<DMonitorHistoryQueryResponseV1, String> {
    let limit = limit.clamp(1, MAX_HISTORY_RESULTS);
    let (start, end) = resolve_window(start_utc, end_utc, queried_at_utc)?;

    let mut matched: Vec<(&DMonitorObservationRecordV1, DateTime<Utc>)> = Vec::new();
    for stream in state.streams.values() {
        if stream.project_scope != project_scope {
            continue;
        }
        if !matches_optional(filter.node_id.as_deref(), Some(stream.node_id.as_str())) {
            continue;
        }
        for record in stream.observations.values() {
            let Ok(received_at) = parse_timestamp(&record.received_at_utc, "received_at_utc") else {
                // A persisted record with an unparsable timestamp is a data
                // integrity problem that predates this query, never caused
                // by it -- fail-closed by dropping it from this view rather
                // than failing the whole historical read.
                continue;
            };
            if received_at < start || received_at > end {
                continue;
            }
            let (application_id, service_id) = subject_logical_identity(&record.observation.subject);
            if !matches_optional(filter.application_id.as_deref(), application_id) {
                continue;
            }
            if !matches_optional(filter.service_id.as_deref(), service_id) {
                continue;
            }
            matched.push((record, received_at));
        }
    }

    // Deterministic ordering (Part B): chronological by received_at_utc,
    // tied-broken by observation_id -- stable across repeated identical
    // queries regardless of the BTreeMap/stream iteration order above.
    matched.sort_by(|(a, a_received), (b, b_received)| {
        a_received.cmp(b_received).then_with(|| a.observation.observation_id.cmp(&b.observation.observation_id))
    });

    let total_matched = matched.len();
    let truncated = total_matched > limit;
    matched.truncate(limit);

    let observations: Vec<HistoricalDMonitorObservationV1> = matched
        .into_iter()
        .map(|(record, _)| {
            let freshness = evaluate_freshness(record, freshness_policy, queried_at_utc)
                .map(|evaluation| evaluation.freshness)
                // Same fail-closed posture as the parse above: an
                // evaluation error here (e.g. queried_at_utc somehow
                // precedes this record's own received_at_utc, which cannot
                // happen given `resolve_window`'s own validation, but is
                // not statically unreachable to the compiler) reports the
                // conservative, least-trusting answer rather than panicking
                // or silently omitting the row.
                .unwrap_or(DMonitorFreshnessV1::Stale);
            HistoricalDMonitorObservationV1 {
                observation_id: record.observation.observation_id.clone(),
                node_id: record.observation.node_id.clone(),
                monitor_instance_id: record.observation.monitor_instance_id.clone(),
                sequence: record.observation.sequence,
                collector: record.observation.collector.clone(),
                subject: record.observation.subject.clone(),
                health: record.observation.health,
                observed_at_utc: record.observation.observed_at_utc.clone(),
                received_at_utc: record.received_at_utc.clone(),
                freshness,
                facts: record.observation.facts.clone(),
            }
        })
        .collect();

    Ok(DMonitorHistoryQueryResponseV1 {
        schema: DMONITOR_HISTORY_SCHEMA.to_string(),
        project_id: project_scope.to_string(),
        start_utc: start.to_rfc3339(),
        end_utc: end.to_rfc3339(),
        queried_at_utc: queried_at_utc.to_string(),
        filters: DMonitorHistoryFiltersEchoV1 {
            application_id: filter.application_id.clone(),
            node_id: filter.node_id.clone(),
            service_id: filter.service_id.clone(),
        },
        limit,
        count: observations.len(),
        truncated,
        observations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::canonical_dmonitor::{
        CanonicalDMonitorObservationV1, DMonitorContractRefV1, DMonitorHealthV1, CANONICAL_DMONITOR_OBSERVATION_SCHEMA,
    };
    use crate::core::dmonitor_state::{ingest_observation, DMonitorStateV1};

    const PROJECT: &str = "proj:lab";
    const OTHER_PROJECT: &str = "proj:other";
    const NODE_A: &str = "fog-01";
    const NODE_B: &str = "fog-02";

    fn policy() -> DMonitorFreshnessPolicyV1 {
        DMonitorFreshnessPolicyV1 {
            max_observed_age_seconds: 90,
            max_received_age_seconds: 90,
            max_future_clock_skew_seconds: 10,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn dserv_observation(
        node_id: &str,
        monitor_instance_id: &str,
        sequence: u64,
        application_id: &str,
        service_id: &str,
        observed_at_utc: &str,
        observation_id: &str,
        health: DMonitorHealthV1,
    ) -> CanonicalDMonitorObservationV1 {
        CanonicalDMonitorObservationV1 {
            schema: CANONICAL_DMONITOR_OBSERVATION_SCHEMA.to_string(),
            observation_id: observation_id.to_string(),
            project_id: Some(PROJECT.to_string()),
            node_id: node_id.to_string(),
            monitor_instance_id: monitor_instance_id.to_string(),
            sequence,
            observed_at_utc: observed_at_utc.to_string(),
            cycle_id: None,
            collector: "test".to_string(),
            subject: DMonitorSubjectV1::DServRealization {
                project_id: PROJECT.to_string(),
                application_id: application_id.to_string(),
                service_id: service_id.to_string(),
                node_id: node_id.to_string(),
                dmap_ref: Some(DMonitorContractRefV1 { id: "dmap:1".to_string(), revision: 1, digest: format!("sha256:{}", "a".repeat(64)) }),
            },
            health,
            facts: serde_json::json!({}),
        }
    }

    fn ingest(state: &mut DMonitorStateV1, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, PROJECT, observation, received_at_utc).unwrap();
    }
    fn ingest_scope(state: &mut DMonitorStateV1, scope: &str, observation: CanonicalDMonitorObservationV1, received_at_utc: &str) {
        ingest_observation(state, scope, observation, received_at_utc).unwrap();
    }

    const NOW: &str = "2026-09-25T00:10:00Z";

    #[test]
    fn chronological_retrieval_returns_rows_oldest_first_by_received_at_utc() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", "2026-09-25T00:00:01Z", "obs-1", DMonitorHealthV1::Healthy), "2026-09-25T00:00:05Z");
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-1", "svc-a", "2026-09-25T00:00:02Z", "obs-2", DMonitorHealthV1::Healthy), "2026-09-25T00:00:02Z");
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-2", "obs-1"], "obs-2's received_at_utc (00:00:02) precedes obs-1's (00:00:05) despite its HIGHER sequence/observed_at_utc");
    }

    #[test]
    fn deterministic_tie_break_by_observation_id_when_received_at_utc_collides() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-b", "2026-09-25T00:00:01Z", "obs-z", DMonitorHealthV1::Healthy), "2026-09-25T00:00:05Z");
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-1", "svc-a", "2026-09-25T00:00:01Z", "obs-a", DMonitorHealthV1::Healthy), "2026-09-25T00:00:05Z");
        let first = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        let second = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = first.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-a", "obs-z"], "identical received_at_utc must tie-break lexicographically by observation_id");
        assert_eq!(first.observations.iter().map(|o| &o.observation_id).collect::<Vec<_>>(), second.observations.iter().map(|o| &o.observation_id).collect::<Vec<_>>(), "repeated identical queries over unchanged persistence must return identical ordering");
    }

    #[test]
    fn start_and_end_time_filtering_excludes_rows_outside_the_window() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", "2026-09-25T00:00:00Z", "obs-before", DMonitorHealthV1::Healthy), "2026-09-20T00:00:00Z");
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-1", "svc-a", "2026-09-25T00:00:00Z", "obs-inside", DMonitorHealthV1::Healthy), "2026-09-24T00:00:00Z");
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), Some("2026-09-23T00:00:00Z"), Some("2026-09-25T00:10:00Z"), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-inside"]);
    }

    #[test]
    fn result_bounding_clamps_to_max_and_reports_truncated_honestly() {
        let mut state = DMonitorStateV1::empty();
        for i in 1..=5u64 {
            ingest(&mut state, dserv_observation(NODE_A, "mon-1", i, "app-1", "svc-a", NOW, &format!("obs-{i}"), DMonitorHealthV1::Healthy), NOW);
        }
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 2, NOW, &policy()).unwrap();
        assert_eq!(result.observations.len(), 2);
        assert_eq!(result.limit, 2);
        assert!(result.truncated, "5 matched rows with limit=2 must report truncated=true");
    }

    #[test]
    fn requested_limit_above_the_maximum_is_clamped_never_widened() {
        let state = DMonitorStateV1::empty();
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 1_000_000, NOW, &policy()).unwrap();
        assert_eq!(result.limit, MAX_HISTORY_RESULTS);
    }

    #[test]
    fn project_isolation_a_different_project_scope_never_leaks_into_this_query() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", NOW, "obs-mine", DMonitorHealthV1::Healthy), NOW);
        let mut other_observation = dserv_observation(NODE_A, "mon-2", 1, "app-1", "svc-a", NOW, "obs-theirs", DMonitorHealthV1::Healthy);
        other_observation.project_id = Some(OTHER_PROJECT.to_string());
        other_observation.subject = DMonitorSubjectV1::DServRealization {
            project_id: OTHER_PROJECT.to_string(), application_id: "app-1".to_string(), service_id: "svc-a".to_string(), node_id: NODE_A.to_string(), dmap_ref: None,
        };
        ingest_scope(&mut state, OTHER_PROJECT, other_observation, NOW);
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-mine"]);
    }

    #[test]
    fn application_isolation_filters_out_a_different_applications_evidence() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", NOW, "obs-app1", DMonitorHealthV1::Healthy), NOW);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-2", "svc-a", NOW, "obs-app2", DMonitorHealthV1::Healthy), NOW);
        let filter = DMonitorHistoryFilterV1 { application_id: Some("app-1".to_string()), ..Default::default() };
        let result = query_dmonitor_history(&state, PROJECT, &filter, None, Some(NOW), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-app1"]);
    }

    #[test]
    fn node_isolation_filters_out_a_different_nodes_evidence() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", NOW, "obs-node-a", DMonitorHealthV1::Healthy), NOW);
        ingest(&mut state, dserv_observation(NODE_B, "mon-2", 1, "app-1", "svc-a", NOW, "obs-node-b", DMonitorHealthV1::Healthy), NOW);
        let filter = DMonitorHistoryFilterV1 { node_id: Some(NODE_A.to_string()), ..Default::default() };
        let result = query_dmonitor_history(&state, PROJECT, &filter, None, Some(NOW), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-node-a"]);
    }

    #[test]
    fn service_isolation_filters_out_a_different_logical_services_evidence() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", NOW, "obs-svc-a", DMonitorHealthV1::Healthy), NOW);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-1", "svc-b", NOW, "obs-svc-b", DMonitorHealthV1::Healthy), NOW);
        let filter = DMonitorHistoryFilterV1 { service_id: Some("svc-a".to_string()), ..Default::default() };
        let result = query_dmonitor_history(&state, PROJECT, &filter, None, Some(NOW), 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-svc-a"]);
    }

    #[test]
    fn native_and_container_observations_remain_independently_represented() {
        // Phase 139B established containers and native processes share the
        // exact same DServRealization subject shape, independently
        // epoched -- this proves the historical view keeps BOTH as
        // separate rows, never merged/deduplicated by service_id alone.
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-native", 1, "app-1", "svc-a", NOW, "obs-native", DMonitorHealthV1::Healthy), NOW);
        ingest(&mut state, dserv_observation(NODE_A, "mon-container", 1, "app-1", "svc-a", NOW, "obs-container", DMonitorHealthV1::Degraded), NOW);
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        assert_eq!(result.observations.len(), 2);
        let monitor_instances: std::collections::BTreeSet<&str> = result.observations.iter().map(|o| o.monitor_instance_id.as_str()).collect();
        assert_eq!(monitor_instances, std::collections::BTreeSet::from(["mon-native", "mon-container"]));
    }

    #[test]
    fn stale_evidence_remains_distinguishable_in_historical_results() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", "2026-09-20T00:00:00Z", "obs-old", DMonitorHealthV1::Healthy), "2026-09-20T00:00:00Z");
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), Some("2026-09-19T00:00:00Z"), Some(NOW), 100, NOW, &policy()).unwrap();
        assert_eq!(result.observations[0].freshness, DMonitorFreshnessV1::Stale, "5 days old vastly exceeds the 90s freshness policy -- an honest Stale, not fabricated");
    }

    #[test]
    fn clock_skew_remains_distinguishable_in_historical_results() {
        let mut state = DMonitorStateV1::empty();
        // observed_at_utc far AHEAD of received_at_utc -- ClockSkew, a pure
        // function of the two persisted timestamps, independent of "now".
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", "2026-09-25T00:05:00Z", "obs-skewed", DMonitorHealthV1::Healthy), "2026-09-25T00:00:00Z");
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        assert_eq!(result.observations[0].freshness, DMonitorFreshnessV1::ClockSkew);
    }

    #[test]
    fn historical_read_never_mutates_the_passed_in_state() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", NOW, "obs-1", DMonitorHealthV1::Healthy), NOW);
        let before = state.clone();
        let _ = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy());
        assert_eq!(state, before, "a read model must never mutate the state it reads");
    }

    #[test]
    fn repeated_identical_reads_are_byte_identical() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", NOW, "obs-1", DMonitorHealthV1::Healthy), NOW);
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-1", "svc-b", NOW, "obs-2", DMonitorHealthV1::Healthy), NOW);
        let a = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        let b = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, Some(NOW), 100, NOW, &policy()).unwrap();
        assert_eq!(serde_json::to_value(&a).unwrap(), serde_json::to_value(&b).unwrap());
    }

    #[test]
    fn empty_range_returns_an_honest_empty_result_not_an_error() {
        let state = DMonitorStateV1::empty();
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), Some("2020-01-01T00:00:00Z"), Some("2020-01-02T00:00:00Z"), 100, NOW, &policy()).unwrap();
        assert_eq!(result.observations.len(), 0);
        assert!(!result.truncated);
    }

    #[test]
    fn start_after_end_is_rejected_clearly() {
        let state = DMonitorStateV1::empty();
        let error = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), Some("2026-09-26T00:00:00Z"), Some("2026-09-25T00:00:00Z"), 100, NOW, &policy()).unwrap_err();
        assert!(error.contains("must not be after"));
    }

    #[test]
    fn a_window_wider_than_the_maximum_is_rejected_never_silently_narrowed() {
        let state = DMonitorStateV1::empty();
        let error = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), Some("2020-01-01T00:00:00Z"), Some(NOW), 100, NOW, &policy()).unwrap_err();
        assert!(error.contains("exceeds the maximum"));
    }

    #[test]
    fn omitting_both_bounds_defaults_to_the_recent_window_not_unbounded_history() {
        let mut state = DMonitorStateV1::empty();
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 1, "app-1", "svc-a", "2020-01-01T00:00:00Z", "obs-ancient", DMonitorHealthV1::Healthy), "2020-01-01T00:00:00Z");
        ingest(&mut state, dserv_observation(NODE_A, "mon-1", 2, "app-1", "svc-a", NOW, "obs-recent", DMonitorHealthV1::Healthy), NOW);
        let result = query_dmonitor_history(&state, PROJECT, &DMonitorHistoryFilterV1::default(), None, None, 100, NOW, &policy()).unwrap();
        let ids: Vec<&str> = result.observations.iter().map(|o| o.observation_id.as_str()).collect();
        assert_eq!(ids, vec!["obs-recent"], "the ancient row must fall outside the default recent window, never returned by default");
    }

    #[test]
    fn historical_response_never_carries_a_convergence_or_authority_field() {
        // Structural proof this module never reaches for ddeploy authority
        // at all -- the response type itself has no such field, and the
        // source never imports active_placement_authority.
        let source = include_str!("dmonitor_history.rs");
        let before_tests = source.split("#[cfg(test)]").next().unwrap_or(source);
        let implementation_only: String = before_tests
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!implementation_only.contains("active_placement_authority"));
        assert!(!implementation_only.contains("AuthorityCorrelationV1"));
        assert!(!implementation_only.contains("DMonitorConvergenceV1"));
    }
}
