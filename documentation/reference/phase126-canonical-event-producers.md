# Phase 126 — Canonical Event Producers & Wiring

Status: implementation for independent review; not frozen or committed.

## Architecture and selected boundaries

Two initial producer families are implemented:

1. Successful Active Monitoring freshness/anomaly findings: explicit completion
   consumption translates selected findings, admits events, then processes alerts.
2. DDeploy v1/v2 proposal acceptance: existing HTTP acceptance handlers call the
   recording adapter only after their existing authoritative SQLite persistence
   succeeds. Historical acceptance retries also attempt recording idempotently.

The audit considered all six requested candidates. DMonitor's accepted ingestion
already separates immutable observations, receipt timestamps and validated stream
heads; it supplies monitoring evidence, but adding raw ingestion events would
duplicate that history without an initial alert policy. DDeploy acceptance has a
clear durable commit point. Reconciliation has authorization, desired projection
and node execution/reporting stages rather than one universal outcome. The DCode
API declares artifacts and governance; DServer explicitly does not execute DCode.
DForward projection/readiness is derived read-time evidence, not a committed
transition. These latter producers are deliberately deferred.

```text
Existing authority / validated observational evidence
                         |
                   producer adapter
                         |
                 validated DatumEventV1
                         |
                  durable Event Store
                         |
              +----------+----------+
              |                     |
          history              explicit policy
                                    |
                              Alert Manager
```

No Incident Manager, notification, HTTP management route, remediation, AI/GNN
execution or Phase 127 read model is introduced. Producers hold no mutable
DMonitor/DMap/runtime/reconciliation references. DDeploy continues using its
existing acceptance/governance/persistence paths, not event decisions.

## Temporal provenance and distributed-clock posture

These are different facts in different clock domains:

| Timestamp | Meaning / owner |
| --- | --- |
| source observation | Remote producer's claim about its observation time |
| server receipt | DServer-generated accepted DMonitor receipt time |
| server evaluation | Scheduler-supplied UTC time used by the existing check |
| event occurrence | Monitoring: evaluation time; acceptance: accepted-at time |
| store admission | Phase 123's actual durable admission time, outside envelope |

```text
Source                 DServer                  Event / Alert DB
  observed_at             |                            |
      |---- transmission / queue delay --------------->|
                         received_at                  |
                         evaluated_at                 |
                            |-- event occurred_at ---->| admitted_at
                            |                         | alert association

The drawn sequence is explanatory, NOT a timestamp-order assertion:
remote clocks may lead/lag; local UTC can be corrected; arrivals can be late.
```

Phase 122 remains unchanged. Monitoring attributes contain:

```text
temporal_context:
  server_evaluated_at_utc
  evidence[]:
    observation_id
    source_observed_at_utc
    server_received_at_utc
    observed_receipt_delta_seconds
    source_age_seconds
    receipt_age_seconds
  interpretation: observation_time_quality_not_node_availability
```

The signed whole-second delta is source-observed minus server-received time.
It is NOT measured NTP offset: buffering, collection/transport delay and clock
skew can all contribute. Both raw timestamps and evaluation time are preserved,
so future AI/GNN can reconstruct communication age separately from source age.
Timestamps are normalized to canonical UTC without changing their instants.
Canonical observation references and their scopes are resolved from one validated
immutable snapshot. Available check age/timestamp diagnostics must agree with
the referenced record; missing references or scope/time contradictions fail
translation rather than inventing temporal evidence. No full snapshot is stored.

**Availability/liveness must not be inferred solely from a remote clock when
recent server receipt evidence exists.** There is no `node_down` translation.
A recent receipt with an old source timestamp may still produce the existing
DMonitor `observation_stale` finding: that means observation freshness, NOT failed
communication or unavailable infrastructure. Its source and receipt ages remain
separate. Old receipts support communication/freshness investigation, still not
a new node-availability verdict. Source-ahead anomalies use the distinct
`observation_clock_skew` event/alert condition. No competing freshness evaluator
or new clock-quality authority is created.

## Monitoring translation and output consumption

Selected finding codes are `observation_stale`, `observation_missing`,
`observation_clock_skew`, and `evaluation_before_receipt`. They become Observation
category / ActiveMonitor source events with unchanged specific kind tokens.
Meaningful Indeterminate findings are included. Satisfied outputs emit no anomaly;
they can produce exact-condition recovery history as described below.
Typed check errors, panics and cancellation remain distinguishable dispositions
and do not fabricate monitoring findings or clear conditions. A configured
`DMonitorProducerBinding` retains the expected selector and freshness policy.
The existing pure DMonitor check recomputes the supplied evaluation against the
same validated immutable snapshot. Check identity, outcome and findings must
match exactly; fabricated/unknown/custom findings are rejected before writes.
This is semantic provenance integrity, not authentication or authorization.

Subject/scope and evidence refs are retained. Attributes carry check ID, stable
evaluation occurrence ID, severity hint, bounded source finding diagnostics,
threshold and temporal context. Initial event severity is explicitly Warning
for these selected temporal classes; hints are diagnostic input, not authority.
Every evaluation is revalidated, its completion identity/time must agree, and
every constructed event passes canonical validation and digest computation.

The same bounded Phase 125 completion receiver is the consumer seam. A caller
wraps a received completion once with `MonitoringOccurrence::from_completion`,
retains that envelope for retries, obtains the same validated snapshot containing
its referenced immutable observations, and calls `EventProducerPipeline::monitoring`
with the configured binding. Dangling heads, invalid observations/digests and
scope contradictions fail snapshot validation, not missing-evidence classification.
No hidden task is spawned by this API. Blocking SQLite work should run through
an owned blocking boundary in a future live consumer, not on the driver task.

Scheduler startup remains intentionally unwired: current DServer has neither a
graceful worker shutdown coordinator nor an expected-stream configuration and
validated publication service. A real completion-to-store adapter is available
and tested with the actual background scheduler; an unreliable half-wired live
service is not installed. No consumer task exists to detach. Explicit owners must
stop their scheduler and drain retained outputs before ending consumption.

Output delivery is best effort, not durable: Phase 125 still records bounded
buffer-full/receiver-closed counters. `report_scheduler_output_loss` exposes
these through explicit diagnostics; status retains the counters. A future live
consumer must monitor them. Dropped outputs cannot honestly be described as
persisted evaluations. No retry history is fabricated.

## Automatic, condition-scoped recovery

Recovery is explicit typed condition processing, not generic Satisfied=healthy.
The trusted accepted stream independently proves which conditions have cleared:
stream presence clears `observation_missing`; sane evaluation/receipt ordering
clears `evaluation_before_receipt`; the existing DMonitor skew predicate clears
`observation_clock_skew`; both existing age predicates being satisfied clears
`observation_stale`. Age == threshold remains fresh, age > threshold stale.
The existing evaluator exposes its independent age predicate even when ClockSkew
has display priority. Missing evidence proves no recovery; evaluation before
receipt proves presence only, not sane age/skew. There is no competing threshold
algorithm or global health verdict. Stale can resolve while skew opens separately.

For a matching active episode, `<original_kind>_cleared` is an Info historical
DatumEvent. It retains the original condition kind/correlation key, stream subject,
check/envelope identity and observed/received/evaluated times with signed delta and
ages. Its event occurrence time is evaluation UTC. Admission comes FIRST, then an
atomic Alert Store operation verifies that admitted immutable event and targets
the original category + kind + subject correlation. Cleared kinds never open alerts.
Automatic resolution uses actor System and reason `monitoring condition cleared`;
the transition references the recovery event. Lifecycle timestamp is local action
UTC, distinct from evidence evaluation time. No automatic CLOSED transition exists.

No matching episode, already Resolved or Closed yields explicit no-op without a
new repetitive clear event. Closed lifecycle/aggregation is not modified by recovery.
An admitted recovery is always retryable. An additive `datum_alert_recoveries`
table atomically stores event ID, exact correlation and outcome alongside resolution;
old recovery replay cannot resolve a later recurrence. Recovery older than the
episode's latest anomaly evidence is ignored. Previously recorded events remain
historical, including a clear event whose resolution must be retried after failure.
Acknowledged/InProgress episodes use the same legal resolution transition; closure
remains an explicit administrative action. Recovery is not an anomaly association
and does not increment its occurrence count.

## Identity, retries, volume and bounds

Monitoring event IDs are SHA-256/JCS-derived from the versioned producer
namespace, retained occurrence ID, check ID, evaluation UTC, finding code and
canonical subject. The occurrence envelope has a UUID assigned ONCE per consumed
completion, not during translation or retry. Separate executions have separate
identities even if UTC repeats. Recreating the envelope loses retry identity;
callers must retain it AND its input snapshot for uncertain-result retries. There
is no durable envelope retention; after a process crash the same physical occurrence
cannot necessarily be reconstructed with the same ID. This is not exactly-once
delivery. Recovery uses its own versioned `monitoring_recovery/1` namespace.
Duplicate finding identities
within one evaluation are rejected before writes.

DDeploy IDs derive from acceptance schema version, project/application and the
immutable acceptance ID. Event content references the acceptance's OWN proposal
and DMap revision/digest, never a later active map. Occurrence time is immutable
accepted-at UTC; replay does not substitute the request's new timestamp.

All selected findings from an evaluation are translated/validated before any
write. Attribute JSON exceeding 16 KiB is a typed `PayloadTooLarge` error, never
silently truncated. Up to 64 findings per occurrence and 16 evidence refs per
selected finding are accepted. There is no claim to bound already-allocated
arbitrary custom-check memory. Canonical JSON/JCS rules remain authoritative.

Each meaningful repeated finding evaluation is an independent factual event;
there is no producer condition deduplication. This intentionally preserves
duration/frequency/persistence/recurrence history for AI/GNN and time series.
Phase 124 correlates repeated events into one condition episode. Future live
cadence must be conservative; default freshness registration remains 30 seconds.
`occurrence_count` counts associated condition observations/confirmations in an
episode, NOT independent outages: 24 hours at 30 seconds can yield about 2,880
confirmations of one episode. Phase 127 presentation must preserve that distinction.

## Event admission, alert policy and failure boundaries

Policy is a deterministic source/category/kind allowlist: the four selected
ActiveMonitor Observation kinds are alert-worthy. `dmap_accepted` and all other
classes are historical-only. Skew and staleness have different kinds and therefore
different condition keys. Recovery has a separate exact-condition resolution policy;
there is no generic rule engine or broad subject-wide resolution.

Only the StoredDatumEvent returned by successful Event Store admission reaches
Alert Manager. Inserted and IdempotentReplay both process alert-worthy events:
safe retries can repair a previous alert-processing failure without duplicating
occurrences. Event Store failure prevents alert processing. Alert failure leaves
the admitted event intact and returns a typed error carrying event identity.
Anomaly/recovery translations are validated before writes. Recording is sequential,
not batch atomic: earlier successes remain durable if a later event fails; later
events are not attempted. Event/Alert commits are NOT one atomic transaction;
retry the original retained envelope/snapshot after partial progress.

DDeploy acceptance and event recording are also NOT an atomic cross-store
transaction. The handler commits authority FIRST, releases its transaction
guards, then attempts recording. Canonical authority is never rolled back for
observability failure, and the successful API response is not changed into an
acceptance error. Typed recording failure is logged explicitly as
`authority operation succeeded; canonical event recording failed`. Existing
stderr diagnostics are used; no new response schema is introduced. Structured
`PostAuthorityEventContext` includes bounded project/application/acceptance identity,
producer kind and typed error class/safe message, never arbitrary secret-bearing
source error text. A process
crash in the gap may leave a missing event. Replaying the acceptance can repair
it; this phase does not add a durable outbox or claim guaranteed delivery.
Both v1/v2 handlers await owned `spawn_blocking` observability work after releasing
authority guards. SQLite busy waits/init/admission cannot pin a Tokio async worker;
response latency can still include recording time. Request cancellation cannot
abort already-started blocking work; its worker still reports ordinary recording
failures. No fire-and-forget task is deliberately launched. This is best-effort
post-commit delivery, not at-least-once. Monitoring remains bounded in-memory
best-effort delivery; alert association is repairable only when replay occurs.

## Durable database path

One dedicated file serves BOTH Event and Alert stores:
`DATUM_EVENT_ALERT_DB_PATH`, default `datum-events.db` relative to DServer's
working directory. A narrow process runtime resolves and retains the absolute
path once (startup captures configuration only, without opening SQLite or starting
monitoring). Later environment/current-directory changes cannot redirect live calls;
invalid configuration is retained as a typed error, not silently retried elsewhere.
Explicit component runtimes can select a separate test/configured path.
Blank/padded/memory/SQLite-URI and Windows drive-relative `C:events.db` paths are
rejected. Ordinary relative paths are resolved against the chosen absolute base;
Windows absolute paths are accepted on Windows. Retained paths must be absolute.
Missing parent directories are created, with errors surfaced explicitly. Initialization
ensures Phase 123's schema exists before alert FKs, reusing the shared schema
lock and DDL. No silent in-memory fallback exists.

The dedicated path avoids the legacy control-state optional fallback semantics.
Configure a stable absolute writable path in deployments; SQLite/disk/init
errors remain typed. Control-plane recording is lazy-initialized at the real
post-commit boundary; this does not gate or reverse authority persistence.
Same-process schema initialization guarantees remain; simultaneous multi-process
first initialization is still not proven. No migration framework is added.

## Validation and remaining work

Tests cover the five-case temporal matrix, selected/missing/skew/before-receipt
findings, no-event outcomes, payload/source validation, stable retries and
distinct repeated executions, correlation separation, Event-before-Alert ordering,
alert failure recovery, restart durability, durable path rejection, actual
scheduler completion consumption/closure, and a real DDeploy canonical acceptance
with event persistence failure proving authority remains committed. Call-site
ordering tests cover both acceptance API variants.

Live scheduler startup, stream publication/selection configuration, owned consumer
shutdown and output-loss monitoring integration remain explicit future deployment
work. Incident Manager, notifications, AI execution and Phase 127 are not started.
