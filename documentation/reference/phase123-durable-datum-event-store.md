# Phase 123 — Durable `DatumEvent` Store

**Status:** implemented, unit-tested, not yet committed. Hardened per an
independent adversarial freeze review (a second review pass, after the
Phase 122-style freeze review this module itself went through). Not wired
to any producer or HTTP route. Additive only.

**Scope:** `dserver/src/storage/datum_event_store.rs` (new module),
registered in `dserver/src/storage/mod.rs`. Nothing else changed. No
changes to the frozen `datum.event/1` contract (`core::datum_event`).

## Objective

Phase 122 froze `datum.event/1` as a typed, versioned, non-authoritative
occurrence contract but implemented no persistence. Phase 123 implements
exactly one thing: storing and retrieving `DatumEventV1` occurrences
safely and durably. It does not implement `DatumAlert`, alert lifecycle,
condition-level deduplication, correlation, an active-monitoring
scheduler, producer wiring, or any HTTP API — all later-phase work.

## Hardening pass

An independent adversarial review of the first implementation found two
correctness gaps before the store could be considered safe to freeze:

- **F1 (chronological ordering/range correctness):** `ORDER BY` and the
  time-range filters compared `occurred_at_utc` as TEXT, which is not
  chronologically correct across Phase 122's variable fractional-second
  precision tiers. See "Why `occurred_at_utc` is never sorted or
  range-compared as TEXT" below.
- **F2 (denormalized-column consistency):** corruption detection on read
  never verified the indexed/filter columns (`project_id`, `severity`,
  etc.) against the decoded `event_json`, so a divergence between them
  could silently leak an event into the wrong project's filtered results.
  See "Denormalized Column Consistency" below.

Both are fixed in this document's current revision. The sections below
describe the FINAL, hardened design throughout — the superseded original
design is not reproduced here.

## Architecture

```
Producer (future, Phase 126)
      |
      v
DatumEventV1
      |
   validate()                (Phase 122, reused unmodified)
      |
 canonical digest             (Phase 122's recompute_datum_event_digest,
      |                        reused unmodified — no second implementation)
      |
 derive sortable timestamp    (Phase 123's own, single, internal helper —
      |                        see "Why occurred_at_utc is never sorted...")
      v
Durable Event Store           (Phase 123, this module)
      |
      +--> get_event(event_id)              — verifies event_json AND every
      |                                        denormalized column on read
      +--> list_events(query)                 bounded, deterministically,
      |                                        chronologically ordered
      +--> Phase 124 Alert Manager (future)
      |
      +--> Phase 127 Operations Read Model (future)
```

## Storage architecture decision: dedicated table, not the generic control-state domain model

Two existing SQLite persistence patterns were audited before implementing
anything:

1. **`storage::control_state`** — a generic per-`domain` JSON-blob model
   (`control_state_current` overwrite + `control_state_history` full-blob
   append on every write). Ten existing domains use it (`ddeploy_state`,
   `dmonitor_state`, `dnode_structural_registry`, etc.).
2. **`storage::operational_evidence`** — a dedicated, purpose-built table
   (`operational_evidence`) for an append-only, individually-queryable,
   potentially-high-volume event stream, with its own indexes and a
   duplicate-detection lookup baked into the insert path.

`DatumEvent` is append-only, individually queryable by many independent
dimensions (project, application, category, kind, severity, source, time
range), historical, and expected to be high-volume once producers are
wired (Phase 126) — exactly `operational_evidence`'s profile, not
`control_state`'s. Reusing `control_state`'s blob-per-domain model would
mean either (a) one blob per event (defeating any relational
indexing/filtering SQLite could otherwise do for us, and making
`control_state_history`'s "append a full copy of the *entire* domain on
every write" behavior absurd at event volume), or (b) one blob holding
*all* events for a domain, rewritten whole on every single admission.
Neither is viable at event volume.

**Decision: a dedicated `datum_events` SQLite table**, modeled directly on
`operational_evidence`'s established pattern but extended with the richer
semantics `datum.event/1` requires: digest-based conflict detection, a
typed `Inserted`/`IdempotentReplay` result, corruption detection on read
(now including denormalized-column consistency — see below), and a
richer, more general, chronologically-correct filter/pagination surface.

## SQLite Table / Schema (final, post-hardening)

```sql
CREATE TABLE IF NOT EXISTS datum_events (
    event_id       TEXT PRIMARY KEY,
    event_digest   TEXT NOT NULL,
    schema         TEXT NOT NULL,
    project_id     TEXT,           -- nullable: global events
    application_id TEXT,           -- nullable
    category       TEXT NOT NULL,
    kind           TEXT NOT NULL,
    severity       TEXT NOT NULL,
    source         TEXT NOT NULL,  -- no CHECK constraint (forward-compat)
    occurred_at_utc            TEXT NOT NULL,  -- canonical Phase 122 TEXT; DESCRIPTIVE ONLY, never sorted/range-compared
    occurred_at_epoch_seconds  INTEGER NOT NULL,  -- derived sortable representation (see below)
    occurred_at_subsec_nanos   INTEGER NOT NULL,  -- derived sortable representation (see below)
    admitted_at_utc  TEXT NOT NULL,
    event_json       TEXT NOT NULL -- full canonical DatumEventV1, losslessly recoverable
);
```

`event_id` is the table's `PRIMARY KEY` directly — Phase 122 defines
`event_id` as a globally unique occurrence identity, not one scoped per
project. `project_id`/`application_id` are nullable with no `CHECK`, and
no fake sentinel value is ever substituted for `NULL` — see "Global /
Project Scoping" below. `source` has no `CHECK` constraint restricting it
to today's 15 known tokens — see "Source Storage" below. The full
canonical `DatumEventV1` is stored verbatim as `event_json`; every other
column exists purely for indexed filtering/ordering and is never used to
*reconstruct* an event (only `event_json`, deserialized, re-validated,
and — new in this hardening pass — cross-checked against every other
column, is ever treated as the source of truth on read).

## Why `occurred_at_utc` is never sorted or range-compared as TEXT

Phase 122's canonical timestamp form (`SecondsFormat::AutoSi`) omits the
fractional-second part entirely when it is exactly zero, rather than
padding to a fixed width. This means plain lexicographic TEXT comparison
of two canonical timestamps is **not** chronologically correct in
general. Proven precisely, not assumed: comparing
`"2026-09-29T12:00:00.100Z"` (100ms past the second) against
`"2026-09-29T12:00:00Z"` (exactly the whole second, chronologically
*earlier*) — the first byte the two strings differ at is `'.'` (0x2E) vs.
`'Z'` (0x5A); since `0x2E < 0x5A`, the fractional string sorts
lexicographically *before* the whole-second one, even though it occurred
*later*. The original Phase 123 implementation shipped with, and never
tested, exactly this defect in both its `ORDER BY` and its time-range
filters — an independent adversarial review caught it by direct
character-code analysis, not by a failing test (none existed for the
scenario).

**The fix:** every admitted row also stores a numeric, chronologically
correct sortable representation, derived once (in
`canonicalize_and_derive_sortable`, the single place this module parses
an RFC 3339 string) from the same canonical `occurred_at_utc` string
Phase 122 already validated, and used for every `ORDER BY` and range
comparison instead of the TEXT column. `occurred_at_utc` itself is
completely unchanged — still the canonical Phase 122 field, still stored,
still what `event_json` and the digest are computed from — it is now
purely descriptive event content, never an ordering primitive.

### Why two columns (`occurred_at_epoch_seconds` + `occurred_at_subsec_nanos`), not one `epoch_nanos` column

A single combined `i64` nanoseconds-since-epoch value was considered and
**rejected after proving it unsafe, not assumed safe**, per the hardening
brief's explicit instruction:

- `chrono`'s `NaiveDate` (and therefore `DateTime<Utc>`) represents dates
  across a range of roughly ±262,143 years (bounded by chrono's internal
  `i32` day-number representation) — confirmed against the `chrono
  0.4.45` version this crate depends on.
- `i64` nanoseconds since the Unix epoch overflows for any instant
  outside roughly **1677-09-21 to 2262-04-11** (`i64::MAX` nanoseconds is
  only ~292 years) — a FAR smaller range than chrono itself can
  represent.
- `DatumEventV1::validate()` does **not** restrict `occurred_at_utc`'s
  year range (Phase 122 validates canonical *form*, not *year range*).
  A canonical, Phase-122-valid event dated e.g. `"3000-01-01T00:00:00Z"`
  is a value this codebase already accepts as valid today, and would
  silently overflow a combined nanoseconds-since-epoch `i64`.

**Quantified proof the chosen representation is safe:** `i64` seconds
covers roughly ±292 **billion** years (`i64::MAX` ÷ (365.25×86400) ≈
2.92×10¹¹ years). Chrono's own representable range (±262,143 years, in
seconds ≈ 8.27×10¹² seconds total) is therefore roughly **1.1 million
times smaller** than what `i64` seconds can hold. There is no realistic
overflow exposure.

The chosen representation: `occurred_at_epoch_seconds` (`i64`, whole
seconds since the Unix epoch, via `chrono::DateTime::timestamp` — can be
negative for pre-1970 instants) and `occurred_at_subsec_nanos` (`i64`
storing a `u32` value, the non-negative forward sub-second offset,
`0..=999_999_999`, via `chrono::DateTime::timestamp_subsec_nanos`).
Ordering/comparing the `(seconds, nanos)` pair lexicographically (`seconds
DESC, nanos DESC` in every index and `ORDER BY`) is chronologically
correct for the entire supported range with no overflow risk.

**Derivation is centralized in exactly one function**
(`canonicalize_and_derive_sortable`), used for both admission (deriving
from an already-Phase-122-validated `occurred_at_utc`) and query-bound
validation (`occurred_from_utc`/`occurred_to_utc`, which have NOT been
through `DatumEventV1::validate()` and therefore need the full
parse-canonicalize-compare check) — per the hardening brief's explicit
instruction not to parse timestamps differently in several places.

## Indexes (final, post-hardening)

| Index | Columns | Justification |
|---|---|---|
| `idx_datum_events_project_occurred` | `(project_id, occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC)` | The single most likely Phase 127 read pattern: "recent events for project X." Matches this store's fixed default ordering exactly. |
| `idx_datum_events_project_severity_occurred` | `(project_id, severity, occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC)` | "Recent HIGH/CRITICAL events for project X" — a near-certain triage dashboard widget. |
| `idx_datum_events_project_kind_occurred` | `(project_id, kind, occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC)` | Drill-down from one specific condition/producer within a project. |
| `idx_datum_events_occurred` | `(occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos DESC, event_id DESC)` | Serves `ProjectFilter::Any` (global wallboard, cross-project) and `ProjectFilter::GlobalOnly` queries without a full table scan/sort — confirmed by the freeze review to already correctly cover the "global wallboard" pattern. |

**Deliberately NOT indexed separately:** `application_id`, `source` —
unchanged reasoning from the original design; secondary filters layered
on a project-scoped query, not primary partition dimensions. Revisit only
if real Phase 127 query patterns prove otherwise.

## Query / Pagination Model

`list_events(db_path, &DatumEventQuery)` — a small, fixed filter set, not
a query DSL. No public HTTP surface exists yet.

**Ordering: `occurred_at_epoch_seconds DESC, occurred_at_subsec_nanos
DESC, event_id DESC`, always, not configurable, and now chronologically
CORRECT** (see above — this is the F1 fix). Chosen over `admitted_at_utc`
for the same reasoning as before: operators reason about incident
timelines by *when things happened*, not *when the server received them*.
Proven, including the exact adversarial scenario from the freeze review
(a whole-second event vs. same-second fractional events at 100ms/900ms),
by `whole_second_and_fractional_events_in_the_same_second_sort_chronologically`
and `mixed_fractional_precision_tiers_sort_chronologically` (millisecond/
microsecond/nanosecond tiers interleaved).

**Known limitation, unchanged by this hardening pass, still not solved:**
`occurred_at_utc` is producer-asserted and not guaranteed globally
monotonic across DIFFERENT producers (clock skew). This is a materially
different, narrower concern than F1 (which was wrong even for a SINGLE,
perfectly-synchronized producer/timestamp source) — F1 is fixed
unconditionally; this remaining limitation is about relative clock
accuracy *between* producers, which no amount of TEXT-vs-numeric fixing
inside this store can address, and is left for a future cursor design
(Phase 127+) to account for.

**Time-range filters: half-open `[from, to)`, converted to the same
numeric sortable representation before querying — never a TEXT
comparison.** `occurred_from_utc`/`occurred_to_utc` must each be Phase
122's exact canonical UTC form; malformed or non-canonical values are
**rejected** (`InvalidQuery`), never silently compared as meaningless
text — proven by `malformed_query_timestamp_is_rejected` and
`noncanonical_query_timestamp_is_rejected` (the latter covering both a
non-UTC offset and a non-minimal-tier fractional representation). Correct
behavior across the same fractional-precision boundary the ordering fix
addresses is proven directly by
`time_range_filter_is_correct_across_same_second_fractional_boundary`.

**Reversed ranges are rejected; equal bounds are not.** If both bounds
are present and `occurred_from_utc` denotes a strictly later instant than
`occurred_to_utc`, the query is rejected as `InvalidQuery` (very likely a
caller mistake) — proven by `reversed_time_range_is_rejected`. Equal
bounds are accepted and correctly produce an empty result with no
special-casing (a zero-width half-open window is a legitimate, if
degenerate, query) — proven by
`equal_time_range_bounds_are_valid_and_produce_an_empty_result`.

**Limit:** unchanged — `1..=MAX_QUERY_LIMIT` (`DEFAULT_QUERY_LIMIT = 100`,
`MAX_QUERY_LIMIT = 1000`), rejected outside that range, not clamped.

## Global / Project Scoping

Unchanged from the original design. `ProjectFilter<'a>` is a three-way
enum — `Any` / `GlobalOnly` / `Project(&'a str)` — a type-level guarantee
against the "collapse global events into a fake project ID" failure mode.
An empty/blank `Project("")` is rejected (`InvalidQuery`), now explicitly
tested by `empty_specific_project_filter_is_rejected` (this test did not
exist before the hardening pass; the underlying validation code was
already correct but untested).

`application_id` remains a plain `Option<&str>` filter, unchanged.

## Source Storage

Unchanged. No `CHECK` constraint; arbitrary forward-compatible
`Other(...)` source tokens are storable and filterable without a schema
change — proven by `source_filter_works_including_forward_compatible_other_source`.

## Attributes / JSON

Unchanged. Phase 122's 16 KiB cap and validation rules are enforced once,
at admission, via `DatumEventV1::validate()`. The full canonical event is
persisted verbatim as `event_json`, proven losslessly recoverable by
`evidence_refs_and_attributes_round_trip_through_storage_losslessly`.

## Admission Algorithm (Event Identity / Replay)

1. `event.validate()` — reject (`InvalidEvent`) before any database
   interaction if invalid.
2. Recompute the canonical digest via
   `core::datum_event::recompute_datum_event_digest` — unchanged, no
   second implementation.
3. **(New in this hardening pass)** Derive the numeric sortable
   timestamp via `canonicalize_and_derive_sortable`.
4. Open a `TransactionBehavior::Immediate` SQLite transaction and, inside
   it: `event_id` absent → insert, commit → `Inserted`; present with the
   same digest → commit (no write) → `IdempotentReplay`, returning the
   already-stored record with its ORIGINAL `admitted_at_utc` unchanged;
   present with a different digest → roll back → `Conflict`, fail closed.

`admitted_at_utc` is generated exactly once, only on first insertion,
and is never refreshed on replay — proven directly (not just by
inspection) by `idempotent_replay_preserves_the_original_admitted_at_utc`,
which sleeps 20ms between the original admission and the replay to
guarantee wall-clock time actually advances, then asserts the returned
(and separately re-read) `admitted_at_utc` is byte-identical to the
original.

`TransactionBehavior::Immediate` is what makes the lookup-then-insert
sequence safe under concurrent admission of the SAME `event_id` — see
"Concurrency" below.

## Typed Admission Result and Errors

```rust
pub enum DatumEventAdmission {
    Inserted(StoredDatumEvent),
    IdempotentReplay(StoredDatumEvent),
}

pub enum DatumEventStoreError {
    InvalidEvent(Vec<CanonicalValidationFinding>),
    Storage(String),
    Conflict { event_id: String, existing_digest: String, submitted_digest: String },
    CorruptStoredEvent { event_id: String, reason: String },
    InvalidQuery(String),  // now also covers malformed/non-canonical/reversed query timestamps
}
```

Unchanged in shape from the original design; `InvalidQuery`'s scope
widened (documented above) to cover the new timestamp-validation cases.

## Denormalized Column Consistency (new in this hardening pass — F2)

The freeze review's most important finding besides F1: the original
corruption-detection path verified `event_json`'s *internal*
self-consistency (valid JSON, passes `DatumEventV1::validate()`, digest
matches) but **never compared the separately-stored, indexed/filter
columns against the corresponding fields of the decoded event**. A row
whose `event_json` said `project_id: "B"` but whose indexed `project_id`
COLUMN said `"A"` would have passed every existing check — because the
digest is computed from the decoded event's own content, entirely
independent of the separate columns — and could have been silently
returned to a `ProjectFilter::Project("A")` query, a cross-tenant data
leak through the filter layer.

**Fix:** on every read, after the three original checks (deserialize,
`validate()`, digest match), `verify_denormalized_columns_match` compares
EVERY denormalized column against the decoded event:

- `schema`, `project_id`, `application_id`, `kind`, `occurred_at_utc` —
  direct equality against the corresponding `DatumEventV1` field.
- `category`, `severity`, `source` — the stored TEXT compared against the
  decoded field's own wire-string form (`enum_wire_str`, the same helper
  used to write them — never a hand-duplicated comparison that could
  drift independently).
- `occurred_at_epoch_seconds`/`occurred_at_subsec_nanos` — the stored
  values compared against a FRESH re-derivation from the decoded event's
  `occurred_at_utc` (not merely assumed consistent because the TEXT
  column matched — this specifically catches tampering of the sortable
  columns alone, a corruption surface this very hardening pass
  introduced).

Any mismatch is reported as `CorruptStoredEvent` with a reason string
naming every field that diverged (not just the first one found).

**Proven with adversarial tests that tamper ONE raw column at a time via
a bypass connection, leaving `event_json` and `event_digest` completely
untouched** — exactly the divergence scenario above:
`project_id_corruption_is_never_returned_under_the_wrong_project_filter`
(the specific scenario the freeze review named explicitly — proves a
row tampered to `project_id = "proj:b"` is neither returned by a
`ProjectFilter::Project("proj:b")` query nor by a direct `get_event`, in
both cases failing closed as `CorruptStoredEvent`),
`application_id_corruption_is_detected`, `category_corruption_is_detected`,
`kind_corruption_is_detected`, `severity_corruption_is_detected`,
`source_corruption_is_detected`, `occurred_at_utc_corruption_is_detected`,
`sortable_timestamp_column_corruption_is_detected`, and
`schema_column_corruption_is_detected`.

## Corruption Detection (updated)

Every read (`get_event`, each row in `list_events`, and the
idempotent-replay branch of `admit_event`) now runs, in order, failing
closed on the first that fails:

1. Deserialize `event_json` into `DatumEventV1`.
2. Run `DatumEventV1::validate()` on the decoded event.
3. Recompute the canonical digest and compare against the stored
   `event_digest` column.
4. **(New)** Compare every denormalized column against the decoded
   event (see above).

If any row in a `list_events` page fails any of these checks, the
**entire call** returns `Err(CorruptStoredEvent)` — this fail-whole-query
policy is unchanged and was reassessed, not merely carried over: the
freeze review concluded this remains the correct choice for Phase 123's
foundational-library scope (never silently return a page that looks
complete but dropped something), with a more lenient degraded-read mode
(partial results + corruption diagnostics) explicitly left as a possible
Phase 127 read-model decision, not a Phase 123 one.

## Durability — No Volatile Fallback (Store-Level Only)

Unchanged: every public function takes an explicit `db_path: &Path`; no
`Option<PathBuf>`, no in-memory fallback anywhere in this module. This is
**STORE-LEVEL durability only.**

**DSERVER-INTEGRATION durability is explicitly a Phase 126 open question,
not resolved here, and must become an explicit Phase 126 acceptance
criterion:**

- The integration layer that eventually calls `admit_event`/`get_event`/
  `list_events` from real DServer code (Phase 126) MUST supply a durable
  `db_path` — e.g. via the existing
  `storage::control_state::default_control_state_db_path()` (always
  returns a path, falls back to the well-known `datum-control-plane.db`
  default, overridable via `DATUM_CONTROL_STATE_DB_PATH`).
- It MUST NOT introduce a new Option-returning/fallback-prone path
  resolver, and MUST NOT reintroduce an in-memory fallback at the
  integration layer — this is exactly the mistake Phase 121 found in
  `api::operational_evidence`'s existing HTTP handler (which uses
  `configured_control_state_db_path()`, the `Option`-returning sibling of
  `default_control_state_db_path()`, and falls back to an in-process
  `RwLock<BTreeMap>` when it returns `None`). This module's own structural
  guarantee (no fallback branch exists) cannot prevent a *future* caller
  from reintroducing exactly that mistake one layer up, at the call site.

This distinction — store-level durability (Phase 123, solved) vs.
runtime-integration durability (Phase 126, open) — must not be
conflated. Phase 123 does not claim, and this document does not imply,
that DServer as a whole is durable end-to-end yet.

## Concurrency

Unchanged from the original design; proven directly by three dedicated
tests (8 real `std::thread`s, `Barrier`-synchronized for genuine overlap)
covering distinct-ID admission, identical-event replay, and conflicting-
payload admission. Re-run five consecutive times after the schema
hardening changes with no failures. The `SQLITE_LOCKED` schema-init bug
found and fixed in the original implementation is unchanged by this
pass — see "Multi-Process Limitation" below for its scope, stated more
precisely than before.

## Multi-Process Limitation (stated explicitly, not solved in this pass)

**Phase 123 guarantees safe concurrent access from multiple THREADS
within one DServer process** — proven by the concurrency tests above.

**SQLite's own DML semantics (the actual `BEGIN IMMEDIATE` admission
transactions and ordinary reads, once schema exists) are compatible with
multi-process access** — WAL mode plus `IMMEDIATE` transactions are
SQLite's own standard, documented multi-process concurrency primitives.
This module does not add anything on top of that for ordinary reads/
writes, and does not need to.

**However, first-time schema initialization is only serialized
IN-PROCESS** (`SCHEMA_INIT_LOCK`, a plain `Mutex<()>`, has no reach across
OS process boundaries), **and true simultaneous multi-process schema
initialization against a freshly-shared database file has not been
proven safe** — it could plausibly hit the identical `SQLITE_LOCKED`
failure the in-process fix addresses, with no mitigation. This was not
fixed in this hardening pass (out of scope; the brief explicitly deferred
it) and is **not** a Phase 123 module-correctness question — Phase 123's
own contract only promises same-process safety.

**This is an explicit Phase 126 acceptance criterion:** Phase 126
integration must either (a) guarantee exactly one DServer process is
ever pointed at a given event database file, or (b) introduce explicit
cross-process-safe initialization/startup coordination if that guarantee
cannot be made. Freezing Phase 123 does not resolve this question; it
hands it forward, explicitly, rather than leaving it implicit.

## Restart Durability

Unchanged, still proven by
`events_survive_a_full_close_and_reopen_of_the_database`.

## Schema Initialization Hot Path (deferred, not fixed in this pass)

No centralized SQLite migration framework exists in this repository —
confirmed against `storage::control_state`/`storage::operational_evidence`,
neither of which has one either. This module follows the strongest
existing pattern: one idempotent `CREATE TABLE IF NOT EXISTS` plus four
`CREATE INDEX IF NOT EXISTS` statements, run (through the
`SCHEMA_INIT_LOCK` guard) on **every** `open_database` call — including
every read, not only writes or first-time setup.

**This was identified as real, avoidable per-call overhead by the freeze
review (F5) and deliberately NOT fixed in this hardening pass** — the
brief's own instruction was not to move initialization into DServer
startup "unless the timestamp schema change makes a correctness fix
necessary," and it does not: the added columns/indexes do not change
this tradeoff's safety, only (marginally) its per-call cost. **This
remains an explicit Phase 126 acceptance criterion:** Phase 126
integration should initialize the database/schema once at DServer
startup, not on every `admit_event`/`get_event`/`list_events` call.

**Acknowledged, separately, unchanged:** no mechanism exists here for a
future `ALTER TABLE`-style column migration — the same limitation
`operational_evidence`'s own table already has today. Since Phase 123 has
not been committed or deployed, the new sortable-timestamp columns were
added directly to the initial schema definition, at zero migration cost;
this remains true only because nothing has shipped yet.

## Retention

**Phase 123 is append-only and implements no pruning/retention policy.**
There is no `DELETE` API (see below) and no mechanism to bound table
growth — database size grows unboundedly with every admitted event, by
design, for this phase. This is a deliberate, explicit choice, not an
oversight: event history is expected to be consumed by the Console,
operators, and future AI/GNN or series/history analysis (Phase 127+), all
of which need the full history to remain available, not silently pruned.
Any future retention/archival policy is an **operational** decision for a
later phase and must never silently delete history that a consumer might
still need — it is out of scope for Phase 123 to design or implement.

## No Alert Lifecycle

`datum_events`' column set is exactly `event_id`, `event_digest`,
`schema`, `project_id`, `application_id`, `category`, `kind`, `severity`,
`source`, `occurred_at_utc`, `occurred_at_epoch_seconds`,
`occurred_at_subsec_nanos`, `admitted_at_utc`, `event_json` — pinned by
`no_alert_lifecycle_columns_or_state_exist` (updated in this hardening
pass to include the two new sortable-timestamp columns in its whitelist),
which reads the table's actual `PRAGMA table_info` and asserts an exact
column set, then explicitly asserts none of `status`, `open`,
`acknowledged`, `acknowledged_by`, `acknowledged_at`, `resolved`,
`resolved_at`, `suppressed`, `first_seen`, `last_seen`,
`occurrence_count`, or `correlation_key` appear. All of these remain
exclusively Phase 124's concern. `correlation_hint` continues to be
nothing more than a plain field inside the stored `DatumEventV1` JSON —
this store attaches no special meaning or indexing to it.

## No Update / Delete API

Unchanged. `no_update_or_delete_api_exists_for_admitted_events` (updated
to also scan for the new tamper-simulation helper's usage pattern, still
correctly scoped to production code only) confirms no
`update_event`/`delete_event`/`remove_event` function and no raw
`UPDATE`/`DELETE` SQL exists in production code. The only mutation this
module's production code ever performs against `datum_events` is a
single `INSERT` on a genuinely new `event_id`.

## Authority Boundaries

Unchanged. This module inherits Phase 122's authority posture: a stored
`DatumEventV1` is never treated as authorization, never mutates desired
state, and is never re-interpreted as anything other than an immutable
record of an occurrence. Persisting it durably does not add authority.

## What This Phase Deliberately Does Not Do

Per its own scope boundary: no `DatumAlert`, no alert lifecycle, no
condition-level deduplication, no correlation engine, no active-monitoring
scheduler, no producer wiring (DDeploy/DMonitor/DForward/sentinel
telemetry/reconciliation/DCode/runtime/ServiceArtifact are all untouched),
no Console/operations-overview APIs, no notifications, no GNN/LLM
integration, no schema-init-at-startup optimization, no runtime DB-path
selection, no cross-process schema coordination, no retention/pruning.
`datum_event.rs` (Phase 122) still has zero production call sites outside
this store, which is expected — that is exactly what Phase 126 is for.
