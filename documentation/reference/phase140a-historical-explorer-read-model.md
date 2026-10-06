# Phase 140A — Historical Explorer: Canonical Historical Read Model

**Status:** implemented, unit/integration-tested, **not committed, not pushed**. Read-only. No new event store, no retention/pruning, no second convergence engine.

## Purpose

Phase 139 established the canonical current-state runtime observation and convergence model. Phase 140A adds the read-only historical foundation: a bounded, deterministic, auditable way to query what DMonitor has *actually persisted* over time — never a reinterpretation of it, never a second monitoring model.

## Historical data source

`core::dmonitor_history::query_dmonitor_history` is a pure function over the **existing**, unmodified `DMonitorStateV1` (loaded via the existing `dmonitor_state::load_state`). No new event store or SQL table was introduced: `DMonitorStateV1` is itself a single JSON-blob domain object (`storage::control_state::{load_domain_json,persist_domain_json}`), and Phase 140A deliberately builds on top of it rather than migrating it, exactly matching the brief's own instruction not to introduce new persistence unless the existing model demonstrably cannot satisfy the requirements — it can, at the cost documented below.

**Accepted performance limitation (inherited, not introduced):** because the entire `DMonitorStateV1` blob must be loaded and deserialized for every historical query regardless of how narrow the requested time window is, query cost is O(total persisted history), not O(matched rows). This is the direct consequence of Phase 139A's own already-documented P2 ("DMonitor history retention is currently unbounded") — Phase 140A does not fix that, and does not need to for its own bounded-result-size/bounded-time-window requirements to be met; it only means the read itself gets slower as history grows, never that an unbounded response is ever returned.

## Canonical identity

Filtering uses exactly the same logical identity Phase 139 already established:

- **project** — `DMonitorStreamV1::project_scope`, the server's own enforced ingestion scope (never re-trusted from the observation's own `project_id` a second way).
- **node** — the observation's own top-level `node_id` (universal across every subject variant; not every variant's own `subject` carries one).
- **application** / **service-like subject** — extracted per `DMonitorSubjectV1` variant: `service_id` for `DServRealization`/`DCodeExecution`, `diot_placement_id` for `DIoTRealization`, `call_id` for `DCallDelivery`; `None` for `DNode`/`RuntimeProcess`/`NetworkReachability`/`Unbound` (no application-scoped logical subject exists for those).

PID, container id, process id, and `monitor_instance_id` are **never** exposed as a filter dimension, client or server side — consistent with Phase 139's own "logical identity, never ephemeral runtime instance" rule.

## Query bounds

- `limit`: clamped to `[1, MAX_HISTORY_RESULTS]` (500). A request for more is silently clamped (same convention as `list_operational_evidence`'s own `.clamp(1,1000)`), never an error.
- Time window: `[start_utc, end_utc]`, defaulting to the most recent `DEFAULT_HISTORY_WINDOW_SECONDS` (24h) when either bound is omitted — there is no "return everything" mode. A requested window wider than `MAX_HISTORY_WINDOW_SECONDS` (7 days) is **rejected** (422), never silently narrowed, because silently narrowing a time window would make the response quietly incomplete in a way nothing in the response shape discloses.
- `start_utc > end_utc` is rejected (422) with a clear message naming both values.

## Ordering

Primary-chronological **ascending** by `received_at_utc` — the one timestamp only the server can honestly assert (never the node-authored `observed_at_utc`, which is exactly the field ClockSkew detection exists to distrust for time reasoning). Ties (identical `received_at_utc`) are broken by `observation_id`, the same stable tie-break convention `dmonitor_interpretation`'s own evidence ordering already uses. Repeated identical queries over unchanged persistence are proven byte-identical (`repeated_identical_reads_are_byte_identical`).

## Pagination / continuation

No cursor-based continuation was introduced — there is no existing cursor convention anywhere in the dserver API surface to extend (confirmed by survey), and the brief's own instruction is to prefer existing conventions. Bounding follows the established `limit` + honest `truncated: bool` flag pattern instead (mirroring Continuum's own `*_truncated` fields): a caller who needs more narrows the time window or raises `limit` (within the hard cap).

## Honest historical semantics — no retroactive authority

`core::dmonitor_history` never imports or reads `core::ddeploy::active_placement_authority`, and the response type carries **no** `convergence`/`AuthorityCorrelationV1` field at all — proven structurally (`historical_response_never_carries_a_convergence_or_authority_field`), not merely by convention. A past observation is exposed exactly as persisted: raw `subject`, raw `health`, both timestamps, raw `facts`.

The **one** authority-independent judgment this layer is allowed to derive is `freshness` (`Fresh | Stale | ClockSkew`), computed by reusing the existing, unmodified `dmonitor_freshness::evaluate_freshness` against the query's own `queried_at_utc`. `ClockSkew` is a pure function of the two already-persisted timestamps (`observed_at_utc` vs. `received_at_utc`) and is therefore exactly as trustworthy for a historical row as for a live one, independent of "now". `Stale` is the expected, honest answer for most historical rows — this is not fabrication, it is the same canonical rule applied consistently to data that is, definitionally, old.

## Isolation — proven, not assumed

Nineteen core tests (`core::dmonitor_history::tests::*`) plus three real-HTTP integration tests (`api::dmonitor_history::tests::*`) prove: project isolation, application isolation, node isolation, logical-service isolation, native+container independence (two distinct `monitor_instance_id` streams for the same service never merge into one row), chronological ordering, deterministic tie-break, window/limit bounding, honest empty-range results, clear rejection of an invalid or oversized window, and that a historical read never mutates the `DMonitorStateV1` it reads.

## Relationship to Phase 139 current-state monitoring

Phase 139's convergence evaluation (`evaluate_application_dserv_convergence`) and Phase 140A's historical read model are **structurally independent, non-overlapping views** over the same underlying `DMonitorStateV1`: the former answers "is the CURRENT placement correct, per CURRENT authority"; the latter answers "what did DMonitor actually record, over time, with no authority reinterpretation at all". Neither calls the other. Nothing in Phase 140A changes DMonitor ingestion, current-state computation, placement authority, health semantics, freshness semantics, or convergence semantics — confirmed by an unmodified `dmonitor_interpretation.rs`/`dmonitor_state.rs`/`ddeploy.rs` (zero diff against Phase 139's own closing commit).

## Console contract foundation (Part G)

`console/src/types/dmonitor-history.ts`, `console/src/api/dmonitor-history.ts` (`DMonitorHistoryClient`, single read-only `query` method, structurally proven), and `console/src/pages/dmonitor-history-model.ts` (closed-set validators + display mappings, mirroring `dmonitor-convergence-model.ts`'s own established discipline) were added as the minimal client/type contract. **No new Console route/page was built in 140A** — the brief explicitly frames the interactive explorer surface as optional ("may be added... if it follows naturally"), and building one now would pull Phase 140A into UI/visualization work it explicitly defers ("no charts, aggregation dashboards, overlays, or analytics engine required in 140A"). The contract is proven sufficient and correct on its own; a future phase can build the page against it with zero contract changes.

## A discovered-and-fixed pre-existing defect (not introduced by 140A)

While building `dmonitor-history-model.ts`'s own timestamp validator, direct empirical testing (`chrono::Utc::now().to_rfc3339()`) confirmed that **every** server-generated `_utc` timestamp in this codebase (`evaluated_at_utc` in `dmonitor_convergence.rs`, `generated_at_utc` in `continuum_view.rs`, `received_at_utc` at ingestion in `dmonitor.rs`, and others) is rendered with a `+00:00` offset suffix, **never** a literal `Z` — despite every existing Console validator for these fields (`dmonitor-convergence-model.ts`, `continuum-model.ts`) requiring a strict `Z`-only suffix. This meant a REAL dserver response (as opposed to a hand-written test fixture, which always typed `Z` literally) would have been rejected by the Console's own validator. This is a confirmed, narrow, pre-existing P1 fixed as part of this phase (both regex relaxations, each covered by a new regression test proving the `+00:00` form is now accepted) — not a Phase 139/139B regression, since it predates both.

## Explicit non-goals honored

No DMonitor retention/pruning, no removed-D-Node garbage collection, no SentinelRegistration integration, no artifact-integrity convergence redesign, no remediation, no alerts, no anomaly detection, no analytics engine, no charts/dashboards, no historical mutation — none of these were implemented, per the brief's own explicit list.

## Deferred retention policy (restated, unchanged)

DMonitor history retention remains unbounded (Phase 139A's own finding, unchanged by 140A). This phase's bounded *reads* are not a retention policy — they bound what one query returns, not how much is stored. A future phase may add retention/pruning; 140A does not block or complicate that future work (the read model has no dependency on how long history is kept, only on what currently exists).
