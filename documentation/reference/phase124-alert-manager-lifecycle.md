# Phase 124 — Alert Manager, Lifecycle & Deduplication

**Status:** APPROVED AND FROZEN — `datum.alert/1` and the Phase 124
Alert Manager foundation. Phase 125 has not started.

**Scope:** the alert core/store modules, their module registrations, this
reference, and a shared storage schema-init lock. Phase 123 retains its DDL,
public behavior and tests; its schema initializer, verified row reader and
numeric timestamp derivation are reused internally by the Alert Store.
No Phase 125 work is included.

## Objective

Phase 122 froze `datum.event/1` as an immutable occurrence contract. Phase
123 made occurrences durable. Neither phase gives an operator (or a future
Console, or a future GNN/LLM consumer) a single, stable, mutable thing to
act on when the same underlying condition produces many occurrences over
time. Phase 124 implements exactly that: a canonical `DatumAlertV1` model,
durable normalized persistence, condition-level deduplication, and a
lifecycle command/query API. It does **not** implement an Incident Manager,
an active-monitoring scheduler, event producers, DDeploy/DMonitor wiring, a
notification system, a Console, an operations dashboard, AI/GNN action
execution, remediation, or any HTTP API.

## Event ≠ Alert ≠ Incident

- **`DatumEventV1`** (Phase 122): an immutable occurrence. Never mutated
  once admitted (Phase 123).
- **`DatumAlertV1`** (this phase): a MUTABLE operational condition derived
  from one or more `DatumEventV1`s. Owns a lifecycle, a stable correlation
  identity across repeated occurrences, and provenance for who or what
  changed it.
- **Incident**: **not implemented in Phase 124.** A future phase may group
  multiple related alerts into one incident; nothing in this phase's data
  model, naming, or code anticipates or names that concept. Confirmed by
  direct source search: no `Incident`-named type or module exists anywhere
  in the crate.

## Architecture

```
Phase 123 Event Store
      |
      v
StoredDatumEvent                      (never a bare DatumEventV1 or a
      |                                 caller-supplied event_id — see
      |                                 "Phase boundary" below)
      v
derive_correlation_key(event)         (core::datum_alert, pure function)
      |
      v
process_event(db_path, &stored_event) (storage::datum_alert_store)
      |
      +--> no active alert for key    -> Created   (new alert_id, Open)
      +--> historical occurrence       -> LateAssociated (lifecycle unchanged)
      +--> Resolved, post-resolution   -> Reopened (same alert_id, -> Open)
      +--> active alert, other status -> Updated    (occurrence tracking only)
      +--> event_id already associated -> AlreadyProcessed (no-op)
      |
      v
datum_alerts / datum_alert_events / datum_alert_transitions  (SQLite,
      |                                                        same db file
      |                                                        as datum_events)
      v
acknowledge_alert / start_alert_work / resolve_alert / close_alert
      |
      v
get_alert / list_alerts / list_alert_events / list_alert_transitions
      |
      +--> future Console (read/act only, not implemented here)
      +--> future AI/GNN consumer (read/act only, not implemented here)
      +--> future Incident Manager (not implemented here)
```

## Phase boundary: admitted events only

`process_event` accepts `&StoredDatumEvent`. Since that wrapper has public
fields, the store rereads the admitted event within the write transaction
using Phase 123's corruption-checked reader and requires exact equality.
A forged wrapper or an event missing from this database is rejected.

## Lifecycle States

- **`Open`** — requires attention; nobody has accepted ownership yet.
- **`Acknowledged`** — an actor explicitly accepted awareness/ownership.
  Does NOT mean remediation has started.
- **`InProgress`** — actively being investigated or treated.
- **`Resolved`** — the underlying condition is believed no longer active.
  NOT the same as `Closed` — a resolved alert may still be reopened.
- **`Closed`** — the lifecycle episode is administratively complete and
  **terminal**. No transition out of `Closed` exists.

## "Seen" is deliberately not a lifecycle concept here

Phase 124 does not track "an operator viewed this alert in a UI." That is a
Console/observability concern with no bearing on alert identity,
correlation, or lifecycle validity — the three things this phase is scoped
to. `Acknowledged` remains the first real, meaningful, persisted lifecycle
event: an actor explicitly accepting ownership, never merely rendering the
alert on a screen. View-tracking, if ever needed, is entirely deferred to a
future Console-adjacent phase — nothing here reserves a column, a state, or
a name for it.

## Transition Graph

```
        Open ──ack──> Acknowledged ──start──> InProgress
         │                  │                      │
         └──────resolve─────┴──────────resolve──────┘
                             │
                             v
                          Resolved ──close──> Closed (terminal)
                             │
                             └────reopen (recurrence)────> Open
```

Valid edges: `Open→Acknowledged`, `Open→Resolved`, `Acknowledged→InProgress`,
`Acknowledged→Resolved`, `InProgress→Resolved`, `Resolved→Closed`,
`Resolved→Open`.

- **`Open→InProgress` directly is NOT allowed.** Work must go through
  `Acknowledged` first, so there is always a recorded "who accepted this"
  moment before "who is working on it."
- **`Closed` is terminal — deliberately, carefully decided, not a default.**
  The alternative (allowing `Closed→Open`) was evaluated and rejected: a
  recurrence of the same `correlation_key` after closure creates a **new**
  `alert_id` (a new episode) instead. Reopening a closed episode would blur
  exactly the historical episode boundary an operator or auditor needs
  ("was this the same incident continuing, or a coincidentally identical
  new one?"), and matches how comparable incident-management systems treat
  resolved-and-closed as terminal. A correlation key therefore maps to
  zero-or-one **active** alert plus arbitrarily many **closed historical**
  ones.
- **`Resolved→Open` is a reopen, not a new episode.** The condition
  recurred before the episode was administratively closed — it is still
  the same episode, continuing. Only `resolved_by`/`resolved_at_utc`/
  `resolution_reason` are cleared; `acknowledged_by`/`in_progress_by` and
  their timestamps persist as historical fact, and `occurrence_count` is
  never reset.
- Any combination not listed (e.g. `InProgress→Acknowledged`,
  `Acknowledged→Open`) is not evidenced by the brief and was not invented.

The graph is a pure function, `core::datum_alert::is_valid_transition(from,
to) -> bool`, unit-tested independent of any database (`baseline_forward_
path_is_valid`, `documented_shortcuts_are_valid`, `closed_is_terminal`,
`open_cannot_skip_directly_to_in_progress`,
`no_transition_to_or_from_itself_is_valid`).

## Active vs. Operationally-Active vs. Pending-Closure vs. Historical

Four helper predicates, each with a distinct purpose:

- `is_active` — `status != Closed`. The broadest definition; governs the
  at-most-one-active-alert-per-correlation-key uniqueness rule. A
  `Resolved` alert still occupies its correlation key's active slot,
  because it can still be reopened.
- `is_operationally_active` — `Open`/`Acknowledged`/`InProgress` only.
  Genuinely still needs work.
- `is_pending_closure` — `Resolved` only.
- `is_historical` — `Closed` only.

## Correlation Key — Deduplication at the Condition Level

`core::datum_alert::derive_correlation_key(event: &DatumEventV1) -> Result<String, String>`
is a JCS (RFC 8785) + SHA-256 digest (reusing Phase 122's `core::jcs`
directly, ADR-0006) over `{category, kind, subject}`:

- **Participates:** `category` and `kind` together (guards against two
  unrelated producers reusing the same open, producer-chosen `kind` string
  in different categories); `subject` in full, used verbatim — Phase 122's
  `DatumEventSubjectV1` already IS the correct granularity of "what
  operational entity," so hand-picking sub-fields per variant would be both
  more complex and less correct.
- **Never participates:** `severity` (an escalating/de-escalating
  condition is still the same condition — explicit brief requirement),
  `event_id`, `occurred_at_utc`, `confidence_permille`, `attributes` (pure
  occurrence metadata).
- **`source` is deliberately excluded — a real design decision.** Two
  different sources observing the same real-world fact (e.g. DMonitor and
  SentinelTelemetry both reporting the same unreachable peer) describe ONE
  condition, not two; correlating them into a single alert is more correct
  than fragmenting one real condition into a per-source alert. `source`
  remains available as occurrence-level provenance on each individually
  associated event.
- **`event.correlation_hint` is deliberately NOT used at all in Phase
  124.** Phase 122 defines it as producer-supplied, non-authoritative
  metadata, and the brief is explicit that the Alert Manager must own the
  canonical key rather than trust a hint. Phase 124's policy: the key is
  derived entirely from the Alert-Manager-owned dimensions above;
  `correlation_hint` is inert, stored-but-unused metadata for this phase.
  Folding it in later (as a refinement or override) is explicitly deferred
  to a future phase once real producer usage patterns exist to justify a
  specific policy — inventing one now, with no such evidence, is exactly
  the kind of casual global business rule the brief warned against.
- **`project_id`/`application_id` are not separately included** —
  redundant, since `subject` already carries them wherever relevant, and
  Phase 122's own `validate()` guarantees the event's top-level fields are
  consistent with its subject.

Proven by 8 dedicated `core::datum_alert` tests: determinism; invariance to
severity/event_id/occurred_at/correlation_hint/source; sensitivity to
kind/subject/category; and an explicit positive test proving a
project-agnostic `DNode` subject correctly ignores a differing top-level
`project_id` (this exact case was caught as a real test-fixture bug during
implementation and turned into a permanent, documented, positive
assertion rather than silently patched away).

## Chronological aggregation and severity

Processing order does not define chronology. Every comparison reuses
Phase 123's safe `(epoch_seconds, subsec_nanos)` derivation; raw RFC3339
TEXT is never an ordering primitive. Equal timestamps use lexical event ID:
the smallest ID wins first, the greatest ID wins latest.

- `first_observed_at_utc` / `first_event_id`: minimum occurrence tuple.
- `last_observed_at_utc` / `latest_event_id`: maximum occurrence tuple.
- `severity`: severity of that latest event.
- `peak_severity`: maximum severity of every association in the episode,
  ordered Info < Warning < High < Critical.
- `occurrence_count`: exact association count, including historical evidence.

Thus 12:02 Critical then 12:00 Warning yields current Critical / peak Critical;
12:02 Warning then 12:00 Critical yields current Warning / peak Critical.
Reversing arrival order preserves the aggregation.

## Resolved late evidence and recurrence

An occurrence at or before `resolved_at_utc` augments the same episode
without reopening, changing resolution metadata, or adding a transition.
Equality is historical. An occurrence strictly after resolution reopens the
same ID to Open, records a System transition with its related event ID, and
clears resolution actor/time/reason. Acknowledgement and work checkpoints
remain historical facts; counts and peak do not reset.

## Closed lifecycle immutability and historical evidence

Closed has no outgoing transition; all four lifecycle commands reject it.
Its lifecycle status, actor checkpoints, reasons and transition history are
immutable. Its evidence set and occurrence aggregation may grow when
previously unseen historical evidence arrives.

For a correlation key with closed episodes, choose the episode with the
earliest closure instant greater than or equal to the event occurrence time.
Equal closure instants use ascending alert row ID as the tie-break. This
partitions time into intervals ending at each closure; evidence earlier than
the oldest observed event may extend the oldest episode's first observation.
This historical routing takes precedence even when a newer active episode
exists. Closure equality belongs to the historical episode.

An occurrence after all closure boundaries uses the active episode if one
exists; otherwise it creates a new Open episode with a new alert ID and count
one. It never reopens the old Closed lifecycle. Occurrence times are producer
claims: this policy does not solve producer clock skew.

## Actor Model

`DatumAlertActorTypeV1` — `Operator | System | RuleEngine | Gnn | Llm |
Automation | Other(String)` — mirrors `DatumEventSourceV1`'s hand-written,
forward-compatible wire representation (a bare lowercase string on the
wire, with an `Other` catch-all for future provenance categories). Paired
with optional `actor_id` (principal/agent/instance identity) and structured
`model_ref` (model/version identity) in `DatumAlertActorV1`. Gnn and Llm
require nonblank `model_ref`; other types may carry it without gaining any
authority. Actor IDs and model references must be nonblank when present.
Unknown actor types remain forward-compatible lower-snake-case tokens;
invalid tokens and known-token collisions in `Other` are rejected.
Validation runs both on commands and when reading persisted actors. This is
**audit/provenance metadata only** — recorded identically regardless of
which actor type performs an action. Nothing in `core::datum_alert` or
`storage::datum_alert_store` treats any actor type as pre-authorized to
perform any action; authorization policy is explicit future work.

## AI/GNN Posture

The data model supports a future GNN/LLM consumer reading alerts and
recording its own actions as first-class, typed, auditable provenance
(`DatumAlertActorTypeV1::Gnn` / `::Llm`) — exactly the same lifecycle
commands and transition history any human operator uses, no separate code
path. Phase 124 implements **no authorization policy** for when AI-driven
provenance is legitimate, and no AI code calls any Phase 124 function
anywhere in this codebase (confirmed by source search — the only call
sites of `process_event`/the lifecycle commands are this module's own
tests).

## Persistence — Dedicated Normalized Tables, Same Database File as Phase 123

Three tables, all created in whatever `db_path` the caller supplies — by
convention, the same physical SQLite file `datum_events` lives in (both
modules independently open whatever path they're given and create their
own tables there — the same multi-module-one-file pattern
`storage::control_state`/`storage::operational_evidence`/
`storage::datum_event_store` already established). This is what makes real
SQLite `FOREIGN KEY` references into `datum_events` possible; see "Foreign
Keys" below for why that's safe.

```sql
CREATE TABLE datum_alerts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    alert_id TEXT NOT NULL UNIQUE,
    schema TEXT NOT NULL,
    project_id TEXT,
    application_id TEXT,
    correlation_key TEXT NOT NULL,
    category TEXT NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    peak_severity TEXT NOT NULL,
    subject_json TEXT NOT NULL,
    status TEXT NOT NULL,
    first_observed_at_utc TEXT NOT NULL,
    last_observed_at_utc TEXT NOT NULL,
    first_event_id TEXT NOT NULL REFERENCES datum_events(event_id),
    latest_event_id TEXT NOT NULL REFERENCES datum_events(event_id),
    occurrence_count INTEGER NOT NULL CHECK(occurrence_count > 0),
    acknowledged_by_type TEXT, acknowledged_by_id TEXT, acknowledged_by_model_ref TEXT, acknowledged_at_utc TEXT,
    in_progress_by_type TEXT, in_progress_by_id TEXT, in_progress_by_model_ref TEXT, in_progress_at_utc TEXT,
    resolved_by_type TEXT, resolved_by_id TEXT, resolved_by_model_ref TEXT, resolved_at_utc TEXT, resolution_reason TEXT,
    closed_by_type TEXT, closed_by_id TEXT, closed_by_model_ref TEXT, closed_at_utc TEXT, close_reason TEXT
);
CREATE UNIQUE INDEX idx_datum_alerts_active_correlation_key
    ON datum_alerts(correlation_key) WHERE status != 'closed';
CREATE INDEX idx_datum_alerts_project_status ON datum_alerts(project_id, status, id DESC);
CREATE INDEX idx_datum_alerts_correlation_key ON datum_alerts(correlation_key, id DESC);

CREATE TABLE datum_alert_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    alert_id TEXT NOT NULL REFERENCES datum_alerts(alert_id),
    event_id TEXT NOT NULL REFERENCES datum_events(event_id),
    associated_at_utc TEXT NOT NULL,
    UNIQUE(alert_id, event_id)
);
CREATE INDEX idx_datum_alert_events_alert ON datum_alert_events(alert_id, id);

CREATE TABLE datum_alert_transitions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    transition_id TEXT NOT NULL UNIQUE,
    alert_id TEXT NOT NULL REFERENCES datum_alerts(alert_id),
    from_status TEXT,            -- NULL denotes the creation transition
    to_status TEXT NOT NULL,
    actor_type TEXT NOT NULL,
    actor_id TEXT,
    model_ref TEXT,
    reason TEXT,
    transitioned_at_utc TEXT NOT NULL,
    related_event_id TEXT REFERENCES datum_events(event_id)
);
CREATE INDEX idx_datum_alert_transitions_alert ON datum_alert_transitions(alert_id, id);
```

### Current row is a verified materialized view

SQL columns reconstruct the typed alert; only the nested subject uses JSON.
Every complete alert read (get/list), replay and pre-write fetch validates
the full association and transition histories in the same SQLite snapshot.
Public page limits never truncate verification, including episodes exceeding
1,000 occurrences or transitions. Integrity contradictions fail closed as
`CorruptStoredAlert` before an update can perpetuate them.

Association verification recomputes count, first/latest IDs, first/last
timestamps, current severity and peak from all admitted events using Phase
123's verified row reader. Transition verification requires the first record
to be None → Open, every subsequent source to equal the previous target,
every edge to be legal, and current status to equal the final target.
Actor/time checkpoints and resolution/closure reasons must match the
traversed history, including resolution clearing on reopen. Malformed
transition actors, timestamps and reasons are rejected.

Acknowledged requires actor/time; InProgress requires both acknowledgement
and work actor/time; Resolved requires resolution actor/time; Closed requires
resolution and closure actor/time. Skipped acknowledgement/work stages are
legal for Open → Resolved → Closed. Missing checkpoints for stages actually
traversed are detected through history verification.

Integrity scans cost O(occurrences + transitions) per complete read/write
fetch. A future read model may optimize this without weakening verification.

## Foreign keys and schema initialization

Both modules use one process-local storage-level `SCHEMA_INIT_LOCK`.
Under it, each connection configures WAL/FULL/foreign_keys and the Alert Store
calls `ensure_datum_events_schema` before creating its own FK tables. Event
DDL is defined only in Phase 123. No migration framework is introduced.

First/latest event IDs, event associations and optional transition-related
event IDs reference `datum_events`; child alert IDs reference
`datum_alerts`. Association uniqueness is composite, never global on event
ID. Existing experimental databases with an older pre-freeze alert schema
are not automatically migrated; validation here uses genuinely fresh files.

## Active-Alert Uniqueness

Enforced twice, independently, deliberately layered as defense-in-depth:

1. **Transactional discipline** — every mutating operation
   (`process_event`, and each lifecycle command) runs inside a
   `TransactionBehavior::Immediate` transaction, the same serialization
   pattern proven safe by Phase 123's own `admit_event`. The lookup
   ("does an active alert already exist for this correlation key?") and
   the branch (create vs. update vs. reopen) happen atomically.
2. **A partial unique SQLite index**,
   `CREATE UNIQUE INDEX ... ON datum_alerts(correlation_key) WHERE status
   != 'closed'` — a database-level backstop independent of the Rust logic
   being perfectly correct. Proven directly by
   `database_enforces_active_uniqueness_and_foreign_keys`, which bypasses the normal transactional path and
   attempts a raw second active-row insert for the same key, confirming
   the index — not just application logic — rejects it.

## Concurrency

Mutations use SQLite BEGIN IMMEDIATE transactions, serializing lookup,
validation, row update, association and transition writes. The shared
initialization mutex coordinates Event and Alert Store first-touch calls
from separate threads against one fresh file.

Tests cover mixed-store first touch (8 threads, 5 fresh databases per test),
concurrent occurrences/replays, and conflicting acknowledgement commands.
The concurrency group is repeated five times during acceptance validation.

The initialization mutex is process-local. Multi-process first-init safety
remains deferred: future integration must arrange single-process startup or
explicit cross-process coordination. No new cross-process guarantee is made.

## Durability

Every public function takes an explicit `db_path: &Path`; there is no
`Option<PathBuf>` and no in-memory fallback anywhere in this module,
mirroring Phase 123 exactly. **Store-level durability only** — the same
distinction Phase 123 documented: DSERVER-INTEGRATION durability (which
concrete `db_path` a running DServer process actually points this module
at) remains an explicit Phase 126 open question, not resolved here.
Preferring the same database file as the Event Store is a deliberate
choice, not an accident: it is what makes the real `FOREIGN KEY`
relationships above possible, and it means a Phase 126 integration only
needs to resolve and durably configure **one** database path for both the
Event Store and the Alert Manager, not two independently-drifting ones.
Restart durability across a full connection close/reopen for all three tables
is checked by `restart_durability_all_three_tables_and_direct_resolve_close`.
This is store-level durability, not a DServer integration claim.

## Deduplication vs. Idempotency — Two Distinct Mechanisms

Phase 124 implements two different guarantees that are easy to conflate
but must not be:

- **Condition-level deduplication** (this phase's own concern): many
  distinct events describing the same underlying condition collapse into
  one alert, tracked via `occurrence_count` and the correlation key.
- **Event-identity idempotency** (inherited from, and distinct from,
  Phase 123's own event-replay idempotency): if the exact same `event_id`
  is passed to `process_event` more than once — a genuine retry of the
  same call, not a new occurrence — the second and subsequent calls return
  `AlreadyProcessed` with the alert unchanged: no `occurrence_count`
  increment, no duplicate `datum_alert_events` row. Enforced by the composite `UNIQUE(alert_id, event_id)`
  constraint, checked explicitly at the
  start of `process_event`, scoped to the event's canonical correlation key.
  The same event may support another alert condition without being globally
  consumed. Canonical processing still returns only one alert. Selection of
  existing associations is deterministic by alert creation ID.


## Internal Command API

```rust
pub fn process_event(db_path: &Path, stored_event: &StoredDatumEvent)
    -> Result<AlertProcessingOutcome, DatumAlertStoreError>;

pub fn acknowledge_alert(db_path: &Path, alert_id: &str, actor: DatumAlertActorV1, reason: Option<&str>)
    -> Result<DatumAlertV1, DatumAlertStoreError>;
pub fn start_alert_work(db_path: &Path, alert_id: &str, actor: DatumAlertActorV1, reason: Option<&str>)
    -> Result<DatumAlertV1, DatumAlertStoreError>;
pub fn resolve_alert(db_path: &Path, alert_id: &str, actor: DatumAlertActorV1, resolution_reason: Option<&str>)
    -> Result<DatumAlertV1, DatumAlertStoreError>;
pub fn close_alert(db_path: &Path, alert_id: &str, actor: DatumAlertActorV1, close_reason: Option<&str>)
    -> Result<DatumAlertV1, DatumAlertStoreError>;
```

`AlertProcessingOutcome` is `Created | Updated | Reopened | LateAssociated | AlreadyProcessed`
— deliberately not collapsed into one opaque success, mirroring Phase 123's
own `Inserted`/`IdempotentReplay` design: a future Phase 126 caller needs
to know exactly what happened. There is no separate "reopen" command — a
reopen only ever happens as a side effect of `process_event` observing
recurrence against a `Resolved` alert, never as a directly callable action;
this is intentional (see "Resolved late evidence and recurrence").

## Internal Query API

```rust
pub fn get_alert(db_path: &Path, alert_id: &str)
    -> Result<Option<DatumAlertV1>, DatumAlertStoreError>;
pub fn list_alerts(db_path: &Path, query: &AlertQuery<'_>)
    -> Result<Vec<DatumAlertV1>, DatumAlertStoreError>;
pub fn list_alert_events(db_path: &Path, alert_id: &str, limit: u32)
    -> Result<Vec<AlertEventAssociation>, DatumAlertStoreError>;
pub fn list_alert_transitions(db_path: &Path, alert_id: &str, limit: u32)
    -> Result<Vec<DatumAlertTransitionV1>, DatumAlertStoreError>;
```

`AlertQuery` reuses `storage::datum_event_store::ProjectFilter` directly
(the same `Any`/`GlobalOnly`/`Project(&str)` three-way guarantee), and adds
`application_id`, `status`, `severity`, `kind`, `correlation_key`, and
`active_only` filters, plus the same `1..=MAX_QUERY_LIMIT` bounded-limit
discipline as Phase 123 (`InvalidQuery`, never silently clamped).
`list_alert_events` returns each association's full, corruption-checked
`StoredDatumEvent` via Phase 123's own `get_event` — **every associated
`DatumEvent` remains independently queryable in the Event Store**, never
collapsed or replaced. Association order and query bounds are tested by
`query_filters_limits_and_association_order_remain_explicit`.

## Future Console Semantics (documented only — not implemented)

A future Console/operations layer, when built, is expected to:

- Render `is_operationally_active`/`is_pending_closure`/`is_historical` as
  the natural "needs attention" / "awaiting closure" / "resolved history"
  groupings, rather than inventing its own status taxonomy.
- Treat "seen"/viewed-tracking, if ever wanted, as a per-viewer read marker
  layered entirely on top of this data model (e.g. a separate
  viewer-scoped table keyed by `alert_id` + user), never as a
  `DatumAlertStatusV1` variant — this phase deliberately reserved no space
  for it in the lifecycle itself.
- Surface `peak_severity` alongside `severity` in any alert list/detail
  view, specifically so a de-escalated-but-still-open condition doesn't
  visually understate how bad it got.
- Distinguish, in any timeline view, between `datum_alert_transitions`
  (lifecycle changes) and `datum_alert_events` (raw occurrences) — the two
  are deliberately separate tables with separate audiences (an operator
  timeline vs. a full forensic occurrence log).
- Never write directly to `datum_alerts`/`datum_alert_transitions` — always
  go through this module's lifecycle commands, so every future mutation
  (including ones a Console initiates) is automatically transition-audited.

None of this is implemented, routed, or wired in Phase 124.

## Reason limits and internal retries

`MAX_DATUM_ALERT_REASON_BYTES = 4096` applies to every lifecycle input note,
resolution/close reason and persisted transition reason. UTF-8 byte length
is checked before trimming; blank reasons normalize to None. Exact 4,096-byte
values are accepted. Reasons are audit summaries, not unbounded transcripts.

Self-transition retries return `InvalidTransition`. There is no HTTP/API
idempotency translation. Open → InProgress remains prohibited; a future
Console may compose acknowledgement and work commands.

## Query ordering and tests

`list_alerts` retains creation order (`id DESC`), not future operational
dashboard ranking. `list_alert_events` returns association order (`id ASC`),
not a chronological occurrence time series. Phase 127 must supply the
appropriate read models and safe numeric occurrence ordering.

Core and store tests cover actors, status-specific fields, chronology,
severity, tie-breaking, lifecycle boundaries, multi-alert evidence,
corruption, history consistency, UTF-8 limits, concurrency and restart
durability. Acceptance results are recorded in the final hardening report.

## Hardening acceptance results (2026-09-29)

Recovered HEAD: `c708fcb9beca35bd228ae1c28f64d69b5fff7c6b`, branch
`agent/phase120-deterministic-placement`. The interrupted store ended after
`fetch_alert_by_id`: schema/write helpers and all storage tests were absent.
The initial compile check reported 42 errors. Missing helpers were completed;
the existing core tests and partial implementation were preserved.

Validation: 39 core alert tests and 32 store tests pass (71 combined).
Full DServer tests: 2,101 main tests and one inspector test pass.
Cargo build passes. Ten concurrency tests passed in each of five runs;
the mixed Event/Alert first-touch test itself uses five fresh databases,
giving 25 fresh-database runs in that repeated group. No SQLITE_LOCKED failure.
The three-table close/reopen durability test passes.

Repository-wide cargo fmt --check reports existing differences in 99 files,
none in the seven-file Phase 124 scope. Targeted rustfmt checks pass.
Git diff --check passes, including separate checks of untracked additions.
Compiler warnings include existing unused/dead code and unwired Phase 124
internal APIs; Git reports ignore-file access and line-ending warnings.
At hardening acceptance, no commit or push had yet been made. Final repository
closure was subsequently authorized. No migration framework or Phase 125
implementation was added.

## Exact reconstructed storage test inventory

All names below are under `storage::datum_alert_store::tests`. All 39
pre-existing interrupted core tests remain in `core::datum_alert::tests`;
their assertions were preserved, with formatting only in this continuation.

- `actor_validation_prevents_persistence_and_model_provenance_round_trips`
- `closed_late_evidence_preserves_lifecycle_and_post_close_creates_new_episode`
- `concurrent_conflicting_lifecycle_commands_have_one_winner`
- `concurrent_event_and_alert_first_touch_fresh_database`
- `concurrent_occurrences_and_replays_are_serialized`
- `corrupt_current_severity_detected`
- `corrupt_first_event_id_detected`
- `corrupt_first_observed_detected`
- `corrupt_last_observed_detected`
- `corrupt_latest_event_id_detected`
- `corrupt_occurrence_count_detected`
- `corrupt_peak_severity_detected`
- `corrupt_status_history_detected`
- `corrupt_transition_actor_detected`
- `database_enforces_active_uniqueness_and_foreign_keys`
- `equal_timestamp_latest_id_determines_current_severity`
- `forged_stored_event_is_not_accepted`
- `fractional_seconds_and_event_id_ties_are_chronological`
- `integrity_verification_is_not_limited_to_public_query_page_size`
- `late_critical_changes_peak_but_not_current_warning`
- `lifecycle_retries_and_skipped_start_are_invalid_closed_is_terminal`
- `missing_checkpoint_and_broken_transition_chain_fail_closed`
- `missing_creation_detected`
- `non_creation_first_transition_detected`
- `orphaned_actor_model_detected`
- `out_of_order_critical_then_warning_and_reverse_have_identical_aggregation`
- `query_filters_limits_and_association_order_remain_explicit`
- `reasons_are_bounded_by_utf8_bytes_for_every_command`
- `resolved_pre_boundary_and_equal_events_do_not_reopen_post_boundary_does`
- `restart_durability_all_three_tables_and_direct_resolve_close`
- `same_event_can_support_another_alert_but_pair_is_unique`
- `same_event_same_alert_is_idempotent_even_after_closure`

## What This Phase Deliberately Does Not Do

Per its own scope boundary, confirmed by direct source search rather than
only by intent: no Incident Manager or `Incident`-named type anywhere in
the crate; no active-monitoring scheduler; no event producer wiring
(`process_event`/the lifecycle commands have zero call sites outside this
module's own tests); no DDeploy/DMonitor/DForward/sentinel-telemetry/
reconciliation/DCode/runtime/ServiceArtifact integration; no Console or
operations-overview API; no notification system; no AI/GNN action
execution or authorization policy (the data model supports future
AI/GNN-attributed actions, but nothing calls into this module on their
behalf); no HTTP API; no retention/pruning policy (append-only, by design,
same posture as Phase 123); no schema-init-at-startup optimization or
cross-process schema-init coordination (unchanged, still-open Phase 126
questions, identical in nature to Phase 123's own).
