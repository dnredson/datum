# Phase 141 — Operational Context & Reporting (Closure)

**Status: CLOSED.** Read-only. No AI/LLM/GNN/MCP, no remediation, no new authority, no second context model. This document summarizes Phase 141's final state and the Phase 142 handoff; it does not duplicate [phase141b-application-operational-context.md](phase141b-application-operational-context.md), which remains the detailed record of the 141B implementation and its first review.

## What Phase 141 delivered

One commit (`2dd87fd`, Phase 141B) across `dserver` + Console only — DATUM was never touched. It:

- Routed the already-built `OperationsSources::application_overview(...)` (Phase 127) at `GET /api/v1/operations/projects/:project_id/applications/:application_id`.
- Added the two fields Phase 141A identified as genuinely missing: `readiness` (reusing `dmonitor_readiness::evaluate_application_readiness` verbatim, with its own canonical 90s/90s/10s policy, matching the dedicated readiness endpoint bit-for-bit) and `recent_history` (a bounded, zero-extra-cost evidence pointer derived from the same scan `observed_runtime` already performs).
- Built the Console page at `/projects/:projectId/applications/:applicationId`.

## Canonical context architecture (confirmed, not re-derived)

There is exactly **one** application-scoped context base: `ApplicationOperationsOverview`. A structural grep across `dserver/src/core/*.rs` finds no competing `OperationalContext`/`ApplicationContext` type; `ProjectOperationsOverview` and `GlobalOperationsOverview` (both pre-dating Phase 141, from Phase 127/131) are the project- and global-scoped siblings with distinct, non-overlapping roles — project/global summary vs. per-application context. Continuum (physical/node view) and History (Phase 140's recorded-evidence explorer) remain separate, specialized read surfaces, not additional context engines.

Command (`reconciliation_execution`, still an unwired pre-existing placeholder), observation (`observed_runtime`), convergence (is placement correct), and readiness (fail-closed governance gate) are four structurally separate fields, never merged or derived from one another. A live probe directly proved **Converged + NotReady** is representable (an admitted Unhealthy observation against a correctly-matched accepted placement yields `convergence: converged` / `readiness: not_ready`, finding `health_unhealthy`) — confirming the two axes are independent, not redundant.

Every list-shaped field (`planned`, `desired_runtime`, `observed_runtime`, `temporal_quality`, `recent_events`, both findings lists, both alert pages) uses the single existing `BoundedPage<T>`/`has_more` convention under one `OverviewQuery.limit` (default 100, max 1000, enforced server-side) — no unbounded array exists anywhere in the response. `recent_history` is a fixed-shape summary, never a list. All partial-availability is expressed through the single pre-existing `SourceSection<T>` enum (`Available | NotConfigured | Unavailable | Corrupt | Indeterminate`) — no second ad hoc mechanism was introduced.

Alerts and events reuse the existing composition over the real canonical `datum_alerts`/`datum_events` SQL tables (Phase 122/124) — confirmed live via a real `datum_alert_store`/`datum_event_store` insert, which surfaced correctly in both `alerts` and `recent_events` with `occurred_at_utc` and `admitted_at_utc` preserved as distinct fields.

## Legacy analysis-context

`core::operational_analysis_context`/`core::active_ai_context`/`core::ai_gnn_data_contract` remain live but **untouched and unused** by Phase 141 (structurally proven: zero references in the implementation text of either touched file). They are not removed. For **new consumers**, `ApplicationOperationsOverview` is now the recommended canonical application operational-context source — it reads the current canonical DMonitor/DDeploy/DIoT pipeline directly, where the legacy pathway still reads the pre-DMonitor Firebase snapshot. Deprecating/removing the legacy pathway is out of scope for Phase 141 and left for a future phase to decide.

## Known, carried-forward debt (not fixed in Phase 141)

- `PlannedPlacementView.lifecycle` is a pre-existing `format!("{:?}", p.state)` Debug-formatted string (not a typed enum on the wire). Untouched by Phase 141; no new Phase 141 code path (readiness, convergence, recent_history) relies on it. **Recommendation:** leave as-is; revisit only if/when a future consumer needs a typed, stable value here.
- `placement-model.ts`/`reconciliation-model.ts` timestamp validators remain strict-`Z`-only (Phase 140C's own documented finding) — unrelated to and untouched by Phase 141.
- `recent_history`/`observed_runtime` still read the full in-memory `DMonitorStateV1`; Phase 140's O(total DMonitor history) query-cost debt is unchanged.
- DMonitor history retention remains indefinite (no TTL/eviction) — unchanged.

## Phase 142 prerequisite matrix

| # | Criterion | Verdict |
|---|---|---|
| 1 | Single canonical application-context base exists | PASS |
| 2 | No second/competing context model | PASS |
| 3 | Command/observation/convergence/readiness structurally separate | PASS |
| 4 | Readiness is fail-closed, exactly two states | PASS |
| 5 | Converged+NotReady and Unknown+NotReady both representable | PASS |
| 6 | All arrays bounded with explicit `has_more`/truncation signaling | PASS |
| 7 | Partial availability uses one honest mechanism (`SourceSection`) | PASS |
| 8 | Alerts/events read the real canonical stores, not legacy Firebase | PASS |
| 9 | Project/application isolation holds under a shared physical node | PASS |
| 10 | Timestamps preserved per-section, never flattened to one clock | PASS |
| 11 | Response is explicitly documented as non-atomic/derived/non-authoritative | PASS |
| 12 | No AI/LLM/GNN/MCP reference anywhere in the contract or its code | PASS |
| 13 | No mutating endpoint/method anywhere on this read surface | PASS |
| 14 | Console page renders every section honestly, no fabricated aggregate status | PASS |
| 15 | Determinism: same inputs produce the same recent_history pointer every time | PASS |

All 15 PASS. There is no structural blocker to starting Phase 142.

## Phase 142 projection decision

**Recommendation: Option B** — a narrower, explicitly versioned AI-context projection (e.g. `datum.ai-context/1`) built *over* `ApplicationOperationsOverview`, not direct exposure of the overview itself.

Reasoning grounded in actual shape, not assumed: `ApplicationOperationsOverview` carries several fields with no clear AI-consumption value (`canonical_application_registry` echo, `planned`'s full per-proposal placement lists, the full `attention_ordered` alert objects with nested `latest_condition_event`), and at `MAX_LIMIT=1000` per section a busy application's payload could reach several hundred KB to low-MB — far larger than an AI context window should be handed by default. A versioned projection also decouples Phase 142's consumer contract from this struct's own evolution (new fields can be added to the overview without silently changing what an AI contract receives). Estimated payload sizes at default `limit=100`: a small app with no deployment yet, ~1-3 KB; a typical Converged+Ready app with a handful of services/observations, ~3-8 KB; a busy multi-node app with many recent events/alerts at the default page size, roughly 50-150 KB (driven mostly by the full `attention_ordered` alert objects, each several hundred bytes).

Phase 142 is **not started** by this document; this is a recommendation for that phase to evaluate, not an implementation.
