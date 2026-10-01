# Phase 125 — Active Monitoring Scheduler

Status: FROZEN — Phase 125 hardening approved. Startup integration remains deferred.

## Runtime audit and selected source

DServer uses `#[tokio::main]`, builds its configuration and clients, binds a
listener, then awaits `axum::serve` directly. There is no periodic server
worker, cancellation token, or graceful-shutdown coordinator to reuse.
`api/operational_analysis_context.rs` uses request-scoped `tokio::spawn`
handles for concurrent Firebase acquisition and reports individual failures.
`api/service_artifacts.rs` isolates blocking registry work with
`tokio::task::spawn_blocking`. Phase 125 reuses Tokio, without a new runtime
or dependency. It does not copy the unbounded per-node fan-out pattern.

OperationalEvidence is an append-only reconciliation/evidence log with a
generic payload and an optional in-memory fallback. It does not define a
canonical freshness policy. Sentinel telemetry computes read-time freshness
(15-second default), and its failure-state helper advances a separate state
machine. Calling that helper would introduce unwanted authority changes.

The first check therefore uses accepted `DMonitorObservationRecordV1` stream
heads from `DMonitorStateV1::stream_head`. DMonitor already distinguishes
current sequence, freshness, collector health, and convergence. Its
`dmonitor_freshness::evaluate_freshness` is pure, takes explicit time, and is
the existing freshness-policy evaluator. Neither collector health nor desired
state is interpreted by the scheduler.

Each configured target is an exact `(server_project_scope, node_id,
monitor_instance_id)` stream, keyed with the existing `stream_key` helper.
The finding subject is the canonical `DatumEventSubjectV1::Other` escape
hatch for that DMonitor stream, not a claim about node health. Application
scope is absent: a stream may interleave observations of different subjects.
Accepted observation IDs become typed `DMonitorObservation` evidence refs.
No current head for a configured stream means missing evidence, not a failed
node. An empty registry does not invent expected streams.

## Architecture and phase boundary

Active monitoring means the server initiates evaluations as time passes; it
does not imply network probes. Passive ingestion alone cannot notice that an
unchanged observation has aged beyond policy. This scheduler can evaluate
the same evidence at successive times and produce different check outcomes.

```text
Existing observational evidence
          |
          v
 Active Monitoring Scheduler
          |
      MonitoringCheck
          |
          v
 MonitoringEvaluation
          |
          +---- no findings
          |
          +---- MonitoringFinding
                    |
                    v
             Phase 126 producer
                    |
                    v
                DatumEvent
                    |
                    v
                Event Store
                    |
                    v
               Alert Manager
```

Only the scheduler/check/evaluation portion is implemented here. No event
producer, Event Store writer, Alert Manager integration, HTTP endpoint,
notification, remediation, AI execution, or Incident Manager is added.
Resolving infrastructure health or convergence remains outside this module.

## Configuration and clock domains

Definitions and enabled flags are code/config-driven. Defaults are 30 seconds
for the freshness registration helper, four concurrent checks, one second
between driver polls, and 64 buffered completions. Configuration rejects zero
cadence, poll interval, concurrency, or output capacity. No deployment
configuration or environment variables change. Runtime timing/history is not
persisted; a newly constructed scheduler begins with enabled checks due.

Scheduling uses elapsed monotonic Duration values. The system clock adapter
has a Tokio Instant origin. Due decisions, cadence, overlap deadlines, and
capacity fairness use only this monotonic domain. The evaluation clock is UTC:
the driver obtains UTC through the same single clock boundary and supplies it
explicitly to the check. UTC controls evidence age and output timestamps,
never dispatch deadlines. Backward/forward UTC corrections cannot suppress
execution or completion collection. A backward monotonic input is an invalid
manual-driver input and is rejected without dispatch-state changes.

The deterministic seam is:

    run_due_checks_at(schedule_time: Duration, evaluated_at_utc: DateTime<Utc>)

The injectable MonitoringClock also supplies completion timestamps. UTC start
and completion metadata can consequently move backward; these fields describe
wall-clock observations, not monotonic execution duration. Status exposes exact
Duration cadence/deadlines instead of misleading predicted UTC deadlines.

## Fair dispatch, cadence, and concurrency

Registrations are sorted by stable check ID. Dispatch scans them from a
round-robin cursor, advancing the cursor after each successful start. Capacity
deferred checks retain their due deadline and remain eligible on the next
poll. They are not evaluations. For ten equal-cadence short checks with four
slots, successive due cycles select 1–4, 5–8, then 9,10,1,2.

A start establishes next_due = current monotonic time + cadence. Completion
does not shift that deadline. A still-running check at a due tick records an
overlap skip and next_due = skip time + cadence. Delayed ticks start at most
one instance of each selected check; there is no catch-up burst. Each ID's
running guard, capacity calculation, spawn, and task ownership are serialized
under the same mutex. Independent checks have independent Tokio execution
tasks. One slow check does not block available sibling slots.

The background driver is a thin interval loop around the same dispatch core.
It has an immediate first poll and skips missed interval ticks. Actual live
cadence is quantized by the poll interval; cadences shorter than that interval
are accepted for deterministic/manual callers but cannot be honored by the
default live driver. There is no arbitrary minimum cadence. Empty registries
still sleep between polls.

## Task supervision and bounded completion output

An owned supervisor awaits each check task, captures panic/typed error,
validates successful output, updates status, and publishes one
MonitoringCompletion. Joining and publication happen independently of all
future driver ticks and of UTC ordering. A scheduler owns the check abort
handle and its supervisor handle; retired supervisors are removed only once
finished. The concurrency bound applies to executing checks; at most one
supervisor accompanies each outstanding execution.

MonitoringCompletion carries check ID, evaluation UTC, completion UTC, and a
typed result: Evaluation (Satisfied/Finding/Indeterminate), Error
(MonitoringCheckError), Panicked, or Cancelled. Errors retain their typed
classification; diagnostic strings are bounded to 4096 UTF-8 bytes. Scheduler
skips/deferrals never masquerade as successful or failed check evaluations.

Manual mode takes the single receiver via take_output_receiver(). Background
start() returns an owning handle exposing that same bounded mpsc receiver.
The receiver is taken once; start fails if manual code has already taken it.
Completion publication uses try_send:

- Full buffer: drop the new completion, increment dropped_output_full, record
  OutputDroppedFull, retain the latest evaluation summary, and continue.
- Closed receiver: drop the new completion, increment dropped_output_closed,
  record OutputReceiverClosed, and continue.
- Available capacity: preserve the full typed completion/finding details.

No send waits for a slow consumer. Counters are saturating, observable runtime
status. Individual payload byte/count limits remain a future translation/live
integration concern: bounded channel count does not bound an arbitrary check's
individual JSON payload. Phase 126 must validate canonical DatumEvents before
admission and choose its own severity/emission policy. None is implemented here.

Status separates last_evaluation_result from last_scheduler_action. A skip or
output drop does not erase the last meaningful evaluation result. Status is
sorted lexically by check ID and exposes running, enabled, cadence, start and
completion UTC, monotonic next deadline, lifecycle, completion/drop counters,
and driver polls. This is runtime status for future Phase 127 read models.

## Result and identity validation

Check IDs must be nonblank and trimmed. Custom Deserialize calls the validated
constructor; registry admission revalidates IDs and rejects duplicates before
any task is started.

The scheduler validates every successful check result, including direct
public-field construction. Satisfied requires zero findings; Finding requires
at least one; Indeterminate permits explanatory findings or none. Every finding
must have a nonblank trimmed code, object attributes, nonblank subject identity,
and project/application fields exactly consistent with the canonical subject
scope getters. Evaluation IDs and canonical UTC timestamp shape are checked.
Invalid output becomes typed InvalidResult error output/status, never a
successful completion. Finding order is supplied deterministically by the
check and preserved. The built-in freshness check returns zero or one finding.

## Validated read-only DMonitor snapshot contract

MonitoringEvidenceSnapshot consumes an existing DMonitorStateV1 and invokes
validate_persisted_dmonitor_state_integrity before publication. The original
canonical state is retained immutably behind Arc, not copied into a second
authority/history store. This reuses all existing DMonitor rules: state version,
stream key/identity, existing maximum-sequence head, head observation ID,
canonical observation validation, recomputed digest, declared/server project
consistency, receipt timestamp, and bidirectional indexes.

Checks receive a narrow source returning one validated immutable snapshot.
The publication-lock adapter uses try_read, clones the Arc, and releases the
guard before evaluation. Busy/poisoned publication is EvidenceUnavailable.
The selected stream-head record is then cloned from that consistent snapshot.
Corrupt state is InvalidEvidence at construction/publication; a source that
cannot provide a valid snapshot returns a typed error. A dangling head can
never turn into observation_missing. Snapshot acquisition/publication is future
integration work; this module neither loads nor persists a database.

## Freshness semantics

Reuse DMonitorFreshnessPolicyV1 and dmonitor_freshness::evaluate_freshness.
Observed and received age limits default to 300 seconds; observed-ahead-of-
received tolerance defaults to 30 seconds. The existing evaluator compares
signed whole seconds: age equal to the limit is fresh; age greater than the
limit is stale. Thus 300.999 seconds still compares as 300; 301 is stale.
Both observed and received dimensions must satisfy their own configured limit.

Stale evidence yields Finding with observation_stale. No accepted evidence
yields Indeterminate with observation_missing. Excessive observed-ahead-of-
received skew yields Indeterminate with observation_clock_skew. Evaluation
before server receipt yields Indeterminate with evaluation_before_receipt.
Small lead within tolerance is treated exactly as DMonitor specifies.
Invalid evidence/policy is an error, never a healthy fallback. These outcomes
describe this check, not global infrastructure health. Severity remains a hint.

## Shared lifecycle and shutdown ownership

Manual and background modes use identical ownership/supervision/state.
Lifecycle is Running -> Stopping -> Stopped. Initiating shutdown atomically
gates new dispatch, clears future deadlines, signals lifecycle watchers, and
aborts the driver and owned check children. An independent cleanup worker owns
the joins. Every async shutdown caller awaits the same Stopped barrier.
Cancelling one caller's wait cannot cancel cleanup or let another caller return
early. The worker closes the output sender only after the driver and all owned
supervisors/children have terminated; queued outputs remain drainable.

After async shutdown returns, no owned top-level check or driver remains,
no later dispatch is accepted, and output is closed. Drop initiates the same
abort/cleanup in manual or background mode, including last-owner Drop. Drop
cannot await and does not promise synchronous termination. The live Tokio
runtime must remain available for async shutdown completion. Custom checks must
cooperate with cancellation, avoid blocking a Tokio worker, and own/cancel/join
any child work; arbitrary detached or already-running blocking child work is
outside the scheduler's guarantee. The pure local freshness check needs no
timeout machinery.

## Authority and Phase 126 integration seam

DServer startup remains unwired. There are no Event Store or Alert Manager
calls, HTTP routes, notifications, remediation, AI/GNN execution, or new
operational authority. DMonitor/DMap/DDeploy/runtime/reconciliation retain their
existing domains. Execution outcomes are not a health or convergence claim.

The bounded MonitoringCompletion receiver is the future Phase 126 consumer
seam. That layer can select successful findings, validate/translate them into
canonical events, and separately decide whether internal check errors/panics
merit events. Satisfied or empty Indeterminate results need registration/scope
context if that future policy chooses to translate them; no recovery/event
policy is invented here. Events then flow through the existing Event Store and
Alert Manager, rather than findings directly mutating alerts. Operational
ranking, Console views, retention, and runtime DB integration remain future work.
