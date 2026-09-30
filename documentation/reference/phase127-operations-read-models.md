# Phase 127 — Operations Read Models

Status: FROZEN — Phase 127 closed. Phase 128 has not started.
Phase 126 is closed at commit `3a9b4738a496928fb21a91236b623badd76543e9`.

## Purpose and boundaries

Internal typed projections hide storage layouts and subsystem implementation
details from future Console/API/wallboard and machine consumers. Everything is
read-only: no acceptance, placement changes, lifecycle commands, monitoring
mutation, deployment, remediation, notification or AI execution. No HTTP routes,
startup workers, frontend assets or Phase 128 implementation are included.
Global means the projects managed by this DServer, not DServer federation.

```text
Canonical Event + Alert SQLite     Validated immutable source snapshots
         |                        DDeploy / DMonitor / DNode / operational graphs
         |                                     |             scheduler status
         +--------------------+----------------+------------------+
                              v
                    Operations Read Models
                              |
        +----------+----------+----------+----------+----------+
        |          |          |          |          |          |
      global     project  application  alerts     events    temporal/scheduler
        +----------+----------+----------+----------+----------+
                              |
                 future Phase 128+ Console / API / AI context
```

The semantic invariant remains:

```text
planned != accepted != desired runtime != observed runtime
observed != healthy != converged
alert lifecycle != infrastructure authority
```

Read projections are not a new health authority. Attention is an explained
presentation classification, not a global health verdict.

## Source audit and adapters

- Phase 123 Event Store supplies immutable envelopes, admission timestamps and
  numeric `(epoch_seconds, subsec_nanos)` chronology. Its existing row/digest/
  denormalized-field validation is reused.
- Phase 124 Alert Store supplies current lifecycle/aggregation and immutable
  association/transition history. Its existing deep consistency verifier is
  reused through a read-only connection; it is not replaced with weaker mapping.
- Phase 126 recovery events and resolution transitions provide clearance history.
  A cleared event is lifecycle evidence, not another anomaly confirmation.
- DDeploy's persisted-state integrity validator and sole v1/v2
  `active_placement_authority` interpretation are reused. Proposals never become
  accepted authority merely because they appear in a projection.
- DMonitor snapshots reuse accepted-state integrity validation. Stream heads
  serve global/project freshness; an application's latest accepted subject heads
  are used when a stream interleaves applications. No pending observations enter.
- DMonitor's existing freshness evaluator and policy interpret source/receipt
  times. Existing DServ convergence interpretation remains a diagnostic source;
  Phase 127 does not invent reconciliation or DIoT convergence authority.
- DNode registry supplies structural declarations, not Sentinel leases/liveness.
- Existing operational DGraph snapshots supply managed operational project
  references and desired-service assignment counts. The existing project listing
  API is a graph-backed operational listing, not a canonical application registry.
- Phase 125's captured runtime status supplies scheduler checks/results/actions.

`OperationsSources` owns validated source values with private fields. Callers
capture each source under its established locking/persistence conventions and
transfer an immutable snapshot. Acquisition errors can be injected explicitly
with `with_source_error`; absence of configuration is different from failure.
There is no second persisted projection database or source-state cache.

`OperationsRepository` uses `SQLITE_OPEN_READ_ONLY`, never creates a database,
creates directories, initializes schemas, or performs DDL. An absent file is
unavailable, not an empty database. Its runtime adapter reuses Phase 126's retained
absolute path through a nonmutating accessor; it does not reread env/cwd or open
the producer's initializing write pipeline. Synchronous database reads belong
behind an owned blocking boundary when a future async API integrates them.

## Global and project views

`GlobalOperationsOverview` contains one supplied `generated_at_utc`, managed
operational-project/structural-node/desired-service-assignment counts when their
sources are available, lifecycle counts, bounded recent events, recent-event
count, project attention summaries, discovered application/project references,
temporal quality, scheduler status and explicit source issues.

Desired-service **assignment** count is not a unique physical service inventory.
Node count means structurally declared DNodes, not live/healthy nodes. Known
project/application references are `DiscoveryPage` values, not canonical indexes.
Their sorted provenance and explicit source coverage are retained. No unavailable
count is fabricated as zero.

### Discovery coverage versus result pagination

`DiscoveryPage.page.has_more` means additional keys exist in the gathered result
set beyond this result page. It does **not** certify that source discovery was
complete. `DiscoveryCoverage` separately records Complete, Partial or Indeterminate
and a deterministic map of source coverage: FullyScanned, Truncated, Unavailable,
Corrupt, NotConfigured, NotApplicable or Indeterminate. Unavailable/Corrupt entries
carry safe typed issues. Complete requires every relevant source to be fully
scanned (or explicitly not applicable). Missing configuration is not proof that
the source contains no identities. With no usable sources, coverage is Indeterminate.

Events and alerts use dedicated bounded DISTINCT/GROUP BY identity-key queries,
independent of recent windows and attention pages. Each source scans at most 1000
distinct application keys and 1000 distinct project keys, using a lookahead key
to identify truncation. Selected representative records use existing canonical
verification; FullyScanned certifies identity-key enumeration, **not integrity of
every record sharing a key**. In-memory validated sources enumerate all identity
keys before the combined result page is limited. Exact identities deduplicate
with multiple sorted provenance markers; no alias normalization is invented.

Project keys are independently queried/enumerated, never reconstructed solely
from a truncated application page. A fully consumed result page can still have
Partial coverage: more source identities may exist beyond a bounded source scan,
or a source may be unavailable/corrupt/unconfigured. Zero discovered references
with unavailable sources is not a canonical inventory count. Neither coverage
nor discovery provides a canonical application registry.

`ProjectOperationsOverview` isolates project alerts/events, operational graph
summary, discovered applications, accepted-placement summaries and DMonitor
temporal quality. Accepted summaries use the shared placement-authority adapter.
Large discovery/accepted lists may be truncated; neither is a canonical registry.

## Unified application view and known gaps

`ApplicationOperationsOverview` keeps these sections separate:

- canonical application registry: explicitly NotConfigured;
- accepted DMap and associated accepted DGraph identity/revision;
- pending proposals, including proposed service/node placements;
- accepted v1 desired runtime summaries, without artifact secrets/configuration;
- accepted v2 placements; its desired-runtime payload is explicitly indeterminate
  without the additional authoritative artifact join;
- observed DMonitor subject evidence and collector-reported health at observation
  time, never relabelled current infrastructure health;
- temporal quality and existing DServ convergence diagnostic summary;
- alert and recent-event views;
- reconciliation execution: NotConfigured, not inferred from desired/converged.

Accepted proposals are not shown as pending plans. Legacy desired records with
unknown application labels are not used to invent application identity.
There is still no reliable canonical project-to-application registry in this
repository. `KnownApplicationReference` records discoveries with sorted provenance
(`accepted_dmap`, `ddeploy_proposal`, `operational_graph`, `dmonitor`, `event`,
`alert`), not applications inferred solely from event history. Canonical application
counts remain NotConfigured. No registry is fabricated.

## Alert presentation, attention and recovery

`OperationalAlertView` includes generated time, identity/scope/category/kind/
subject, current and peak severity, observation window, checkpoints with structured
actor provenance, latest condition event and current recovery event.

| Lifecycle | Presentation group | Operationally active |
| --- | --- | --- |
| Open / Acknowledged / InProgress | operationally active | yes |
| Resolved | pending closure | no |
| Closed | historical closed | no |

`condition_confirmation_count` preserves canonical occurrence_count and explicitly
means observations/confirmations, **not independent outages**. A 24-hour condition
checked every 30 seconds may have about 2,880 confirmations in one episode.
Clearing evidence is not added to this count.

An alert timeline includes anomaly associations plus resolution-related admitted
events. Operators/machine consumers can distinguish start, confirmations, clear
evaluation time, actual lifecycle resolution time and explicit closure. Current
reopened alerts do not present an old recovery as their current resolution.
`condition_duration_seconds` is a nonnegative elapsed episode span where valid,
not continuous outage duration; it uses current clearance evaluation time when
available, otherwise resolution time or the supplied generation time. Negative
durations become None, not invented zero.

Attention ordering is explicit: active before pending closure before historical,
current severity descending, latest condition occurrence descending, then alert ID
ascending. Attention items reference existing active alerts, never duplicate them
as an independent condition record. Project attention uses transparent active/
critical/warning counts and deterministic project IDs, not opaque scores.
Source issues, temporal dimensions and scheduler loss remain separate supporting
sections; there is no automatic alert creation from read-model attention.

## Event views, histories, limits and chronology

`OperationalEventView` includes canonical identity/scope/category/kind/severity/
source/subject, occurrence/admission times, evidence references and retained
temporal context. Classification distinguishes historical, attention-related,
monitoring, recovery and control-plane presentation. Monitoring/control-plane flags
remain independently available. Association IDs and lifecycle-reference IDs are
separate bounded lists: one event can support multiple alerts, while a recovery
event may reference a resolution without being an anomaly association.

`EventHistoryQuery` supports global/all-project/project/application/subject/
alert-associated history, kind/category/source/severity filters and inclusive-lower,
exclusive-upper time bounds. Global-only and project-specific scopes cannot be
combined. Default limit is 100, hard maximum 1000; zero/oversized limits and reversed
ranges are typed InvalidQuery errors, never silently clamped. JSON subject matching
is semantic member matching, not object-key text ordering.
Nonblank identity filters retain exact canonical strings without implicit trimming
or competing identity normalization.

Recent history is newest occurrence first; `(occurred_at, event_id)` keyset cursors
page without equal-time duplication. `event_history_at` adds one supplied generation
time and next cursor to the presentation snapshot. Overview recent windows default
to 24 hours and include only occurrences at/before generation time; general history
queries can inspect future-dated evidence explicitly. Default-window construction
handles chrono's minimum timestamp without arithmetic panic.

`alert_timeline_page` supplies a bounded **oldest-first** stream over the complete
logical union of condition-associated events and all related lifecycle events.
Entries expose ConditionObservation, Recovery and/or LifecycleReference
relationships. One event is returned once even if it has multiple relationships.
The continuation key is `(occurred_epoch_seconds, occurred_subsec_nanos, event_id)`;
the next page uses strict `>` over that same union. Recovery beyond the first page
is reachable, including multiple resolve/reopen cycles. The current recovery also
remains conveniently available in alert detail. `alert_timeline` is the legacy
bounded-prefix convenience wrapper; consumers exploring the episode must use
the page API and follow `next_cursor`. Both directions use one read transaction
per request, not a frozen multi-request database snapshot.

Chronology never compares variable-precision RFC3339 text. SQL reuses Phase 123's
safe numeric seconds/nanoseconds columns, with deterministic event-ID tie-breaking.
Attention ordering is not history ordering. Every returned list has a strict bound
and deterministic ordering; BTree maps/sets stabilize scopes and provenance.

## Temporal quality and AI/GNN interpretation

`TemporalQualityView` keeps source observation, server receipt, common projection
evaluation time, signed observed-minus-received delta, both ages and observation ID.
The delta is diagnostic, **not measured NTP offset**; transport/buffering can explain
it. The supplied evaluation time is a projection calculation, not proof that a
scheduler ran then. Historical producer evaluation/admission remain in event views.

Communication freshness, observation freshness and source-time relationship are
three orthogonal fields:

- recent receipt plus old source time: communication Fresh, observation Stale,
  source BehindOrDelayed when the observed/receipt delta supports it;
- observed=received=14:00, evaluation=15:00: communication Stale, observation
  Stale, delta zero, source AlignedOrNoDiscrepancy (no clock discrepancy inferred);
- stale receipt with an even older observation: both ages can be stale while the
  independent negative observed/receipt delta supports BehindOrDelayed;
- source ahead beyond existing skew policy: SourceAhead, without communication-down
  inference when receipt is recent;
- expected missing stream: MissingEvidence, never down;
- evaluation before receipt: Indeterminate communication and OrderingAnomaly,
  not corrupt evidence;
- source ahead within tolerance: no excessive-skew classification;
- normal accepted evidence: AlignedOrNoDiscrepancy, not a proof of synchronized clocks.

Freshness fields reuse ages and thresholds from the existing DMonitor evaluator;
source-ahead classification reuses its exact skew-policy verdict. The descriptive
behind/delayed relationship compares whole-second `(received-observed)` against
the existing `max_observed_age_seconds` tolerance; it never compares observation
age alone. Equality is within tolerance. This read-only presentation heuristic is
not a second freshness/liveness authority or a measured clock-offset policy.

Age == threshold stays fresh; age > threshold stale, using existing whole-second
DMonitor semantics. Existing freshness classification is retained independently.
Collector health, freshness and convergence remain distinct. No category is called
node_down/offline/unreachable. Recovery updates current evidence quality without
erasing historical anomalies, confirmations, recovery or operator lifecycle history.

Machine consumers can therefore distinguish placement changes, active/resolved/
closed alerts, repeated confirmations, communication freshness and clock/buffer
anomalies, plus observed/received/evaluated/occurred/admitted time domains. No
AI-specific score, authorization or operational authority is introduced.

### Typed event temporal metadata

Event views expose `TemporalMetadata`: Absent, Available with an
`OperationalTemporalContext`, or Invalid with a safe structured projection issue.
The typed context uses actual Phase 126 names: `server_evaluated_at_utc` and bounded
`evidence` records containing optional `observation_id`, source/receipt timestamps,
signed `observed_receipt_delta_seconds`, `source_age_seconds`, `receipt_age_seconds`.
No arbitrary JSON is exposed as the semantic temporal contract.

Absent temporal attributes are normal (including control-plane events). Claimed
metadata requires an object, canonical UTC RFC3339 timestamps, an evidence array
of at most 16 records, and signed i64 integers. Missing required values, wrong
types, out-of-range numbers, and inconsistent arithmetic produce Invalid, without
discarding the otherwise canonically valid DatumEvent. Source/receipt ages may be
negative for the existing skew/before-receipt producer semantics; no zero clamp
is introduced. The stored delta must equal `(observed-received).num_seconds()`;
stored ages must equal their evaluation-minus-evidence calculations. Evaluation
must match the occurrence instant for this Phase 126 monitoring contract.
Unknown extension members do not become semantic fields. Empty evidence is valid
for missing-observation output. No missing value is synthesized. Invalid metadata
has code `invalid_temporal_metadata`, distinct from canonical source corruption.

## Scheduler projection

`MonitoringSchedulerView` carries generated time, configured state, lifecycle,
sorted bounded check status, cadence, running, last start/completion, evaluation
result, scheduler action, monotonic next due and completed/output-drop counters.
Action and result remain separate. No UTC deadline is invented from monotonic time.
Unwired scheduler is explicit and is not reported failed; absent counters are None,
not fake zeros. Stopped/Stopping owner lifecycle is explicit. A Running owner can
be manually driven: the Phase 125 status contract does not prove a live driver,
so driver activity is Indeterminate rather than falsely labelled running.

## Availability, corruption and consistency

`SourceSection` distinguishes Available (including real zero), NotConfigured,
Unavailable, Corrupt and Indeterminate. Typed errors distinguish InvalidQuery,
SourceUnavailable, SourceCorrupt, NotConfigured, Storage and InternalProjection.
A corrupt DMonitor snapshot does not prevent independently safe Alert/Event views.
A failed Event/Alert database open marks those sections unavailable, not zero.
Canonical returned events and alerts fail closed; projection errors are never
silently discarded. A corrupt event page also prevents displaying a misleading
recent-event count.

Serialized `OperationsReadError` carries a fixed source classification, kind,
stable code, optional structured scope and fixed safe message (maximum 512 UTF-8
bytes). Safe messages never interpolate SQLite diagnostics, SQL, paths, environment
values or source JSON. Detailed diagnostics remain nonserialized and available
through an explicit internal-only accessor; Display uses the safe message.
Acquisition failures use the same constructor rather than public-field fabrication.
Project/application scope is retained exactly where known; oversized identity
scope members are omitted, not truncated into another identity. Invalid temporal
metadata also includes the canonical subject when its encoding is at most 2 KiB.
Raw scheduler failure messages are replaced by safe messages while preserving
their typed failure class and separating action/result.

Physical SQLite DatabaseCorrupt/NotADatabase codes now classify as Corrupt through
structured codes. No new brittle diagnostic-string matching is used.

Each overview uses one Event/Alert SQLite read transaction for counts, selection,
row verification and references. Other source snapshots are independently captured
and validated. There is **no global atomic snapshot across subsystems**. All temporal
calculations use one explicitly supplied generation time. Repeating identical
snapshots/time/configuration gives field/byte-equivalent projections.
The time parameter is an evaluation/window boundary, not bitemporal historical
reconstruction or a cutoff on source admission/lifecycle commits. Related lifecycle
evidence is also checked against its exact original condition; contradictory
recovery references fail closed rather than displaying unrelated clearance.

Counts are SQL aggregates of materialized rows, not a claim to validate every
unreturned historical record. Returned records receive existing deep canonical
verification; that verification can read an entire selected alert episode even
though UI lists are bounded. Project/reference discovery can be partial. These
limitations are exposed/documented rather than described as a complete audit.
The Event Store also remains best-effort authority history under Phase 126's
documented post-commit crash gap.

Deferred API/operations limitations remain explicit: event and timeline cursors
are not bound to filter/authorization scope; a future API must validate that scope.
The dangling-alert preflight still scans the whole database, so unrelated-project
corruption can degrade a scoped alert section. Actual projection latency/query
plans and repeated deep episode verification must be measured before live API
rollout; no premature cache is introduced. Storage error classifications not
covered by structured corruption codes remain typed Storage/Unavailable. Reads
remain synchronous and require an owned blocking boundary in future async APIs.

## Indexes and validation

Existing project/status Alert and project/time/severity/kind Event indexes are
reused. Two narrowly justified reverse indexes are added to the existing writer
schema initializer:

- `idx_datum_alert_events_event(event_id, alert_id)` for event-to-many-alert lookup;
- `idx_datum_alert_transitions_related_event(related_event_id, alert_id)` for recovery
  lifecycle links.

EXPLAIN QUERY PLAN tests prove both covering lookup paths are used. Reads never
install them; existing databases gain them through normal shared writer schema
initialization. No migration framework or additional projection tables are added.

Focused tests cover global counts/degraded sources, alert sorting/recovery/time,
the temporal matrix/boundaries, event filters/limits/cursors/multi-alert cardinality,
scheduler distinctions, pending-versus-accepted and desired-versus-observed,
listing gaps, snapshot determinism, read-only configuration reuse, evidence
immutability and query-plan indexes. Phase 128+ can consume these internal
contracts after review; no Console or new management/write API exists here.

## Exact focused test inventory

All functions below are in `core::operations_read_models::tests`:

```text
empty_dserver_has_explicit_not_configured_sections
initialized_empty_store_counts_are_real_zeros
multiple_projects_are_deterministic
active_resolved_closed_counts_are_distinct
critical_attention_references_existing_alert
resolved_is_not_operationally_active
closed_is_historical_only
unavailable_store_is_not_zero_or_created
corrupt_dmonitor_is_explicit_but_other_sources_render
attention_order_is_lifecycle_severity_recency_identity
confirmation_count_is_not_outages
recovery_event_is_visible_in_timeline
timeline_is_occurrence_not_association_order
equal_times_have_event_id_tie_break
current_and_peak_severity_remain_distinct
automatic_resolution_is_pending_closure_not_active
recent_receipt_old_source_is_fresh_communication_not_down
stale_receipt_is_communication_stale
source_ahead_is_clock_quality_not_communication_failure
expected_missing_stream_is_missing_not_down
evaluation_before_receipt_is_ordering_anomaly
recovery_updates_current_temporal_projection
event_project_filter_is_scoped
event_application_filter_is_scoped
event_kind_category_source_severity_filters_intersect
every_query_limit_is_strictly_bounded
time_ranges_use_numeric_chronology_and_half_open_bounds
control_plane_event_is_historical_only
recovery_event_has_separate_presentation_and_admission_time
one_event_can_reference_multiple_valid_alert_episodes
unwired_scheduler_is_not_reported_failed
scheduler_check_status_is_sorted
scheduler_output_loss_is_preserved
scheduler_action_does_not_erase_last_evaluation
pending_proposal_is_never_accepted_placement
desired_and_observed_runtime_are_separate
alerts_cannot_become_application_runtime_authority
discovered_applications_are_not_canonical_registry
common_generation_time_reaches_all_top_level_views
fixed_time_and_snapshots_are_field_equivalent
source_age_uses_one_supplied_generation_time
keyset_pagination_has_no_equal_time_duplicates
subject_filter_is_semantic_not_json_field_order
corrupt_materialized_alert_fails_closed_not_zero
corrupted_event_is_not_hidden_by_counts
invalid_source_snapshot_never_becomes_accepted
projection_does_not_mutate_evidence_or_authority
reverse_association_index_is_used
exact_freshness_boundary_reuses_existing_policy
application_temporal_uses_subject_head_when_global_head_is_another_app
acquisition_failure_is_unavailable_not_unconfigured
presentation_history_has_generation_and_cursor
accepted_proposals_are_not_presented_as_pending_plans
recovery_event_reference_is_not_a_confirmation_association
related_event_index_supports_recovery_links
read_repository_reuses_retained_runtime_path_without_initializing_it
contradictory_recovery_reference_fails_closed
critical_temporal_aligned_stale_is_not_clock_discrepancy
critical_temporal_recent_receipt_old_observation_is_independent
critical_temporal_stale_receipt_can_also_have_delayed_source
critical_temporal_ahead_boundary_uses_existing_skew_policy
critical_temporal_ordering_and_missing_dimensions_are_explicit
critical_discovery_events_are_independent_of_recent_page
critical_discovery_alerts_are_independent_of_attention_page
critical_discovery_unavailable_source_is_not_complete_zero
critical_discovery_corrupt_source_keeps_other_references
critical_discovery_bounded_source_scan_marks_truncation
critical_discovery_multiple_provenance_is_sorted_and_deduplicated
critical_discovery_production_cap_does_not_hide_independent_project
critical_discovery_unconfigured_is_indeterminate_not_registry_zero
critical_timeline_recovery_is_reachable_beyond_first_page
critical_timeline_reopen_cycles_paginate_complete_union_once
critical_timeline_relationship_union_deduplicates_equal_time_events
critical_timeline_invalid_cursor_is_rejected
critical_metadata_valid_timestamps_and_signed_delta_are_typed
critical_metadata_absence_is_normal_and_does_not_invent_values
critical_metadata_invalid_shapes_preserve_canonical_event
critical_metadata_delta_age_range_and_timestamp_checks_fail_explicitly
critical_metadata_negative_ages_preserve_clock_anomalies
critical_errors_unavailable_database_does_not_serialize_path
critical_errors_corrupt_snapshot_has_structured_safe_scope
critical_errors_injected_secret_is_internal_only_and_utf8_bounded
critical_errors_scheduler_failure_retains_class_not_raw_details
```

Hardening adds 26 deterministic synchronous tests (83 focused tests total). The
critical_ filter also includes the original critical-alert attention test. No new
concurrency or wall-time sleep assumptions are introduced. The reopen fixture
anchors the initial lifecycle resolution/checkpoint consistently to fixed UTC
before testing recurrence; production lifecycle audit time is unchanged.
