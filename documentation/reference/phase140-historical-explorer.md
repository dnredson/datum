# Phase 140 — Historical Explorer (CLOSED)

**Status:** closed. Phase 140A (canonical read model) and Phase 140B (UI) are both committed; Phase 140C is a polish/closure pass with no architectural changes. This file is the top-level Phase 140 summary; full subphase detail remains in [`phase140a-historical-explorer-read-model.md`](phase140a-historical-explorer-read-model.md) and [`phase140b-historical-explorer-ui.md`](phase140b-historical-explorer-ui.md) — this file does not repeat their detail, only the facts an operator or future phase needs without reading both.

## What Phase 140 is

A read-only way to ask "what canonical DMonitor evidence was actually recorded, over a bounded recent window" — nothing more. It is a VIEW over persisted evidence, never a second monitoring model, never a historical reinterpretation of current placement authority.

- **Canonical read model (140A):** `core::dmonitor_history::query_dmonitor_history`, exposed at `GET /api/v1/dmonitor/history/:project_id`. Filters: `application_id`/`node_id`/`service_id` (logical identity only, never PID/container-id/`monitor_instance_id`). Time window: default most recent 24h, maximum 7 days (rejected, never silently narrowed, if exceeded). Result size: default 100, max 500 (silently clamped, always honestly disclosed via `truncated`). Ordering: ascending `received_at_utc` (the one timestamp only the server can honestly assert), tie-broken by `observation_id` — deterministic across repeated identical queries.
- **UI (140B/140C):** `/projects/:id/history`, reachable from the project's own "History" sub-navigation entry. A filter form, a chronological observation list, and an expandable per-observation drill-down. Health and freshness badges are rendered verbatim from the backend; `Stale` and `ClockSkew` are always distinct labels, never collapsed. Raw collector `facts` are shown honestly as diagnostic JSON, never relabeled as "findings" (the canonical historical contract has no findings field — findings are a *convergence-evaluation* concept this contract deliberately does not carry).

## Time and ordering semantics

Two different timestamps exist on every observation, and the UI labels both explicitly so an operator never has to know the backend's own clock-domain architecture to use the page correctly:

- `received_at_utc` — server-asserted, drives ordering and the time-window filter. The only clock this system fully trusts for "when."
- `observed_at_utc` — agent-asserted, shown for context, never used for ordering or filtering.

`ClockSkew` is a pure function of comparing these two timestamps against each other (not against "now"), so it remains exactly as meaningful for an old observation as a fresh one.

## Identity

Project (server-enforced scope), node, application, and a per-subject-kind "logical service/subject" (`service_id` for D-Serv/D-Code, `diot_placement_id` for DIoT, `call_id` for D-Call, none for D-Node/runtime-process/network-reachability/unbound). `monitor_instance_id` and `sequence` are shown only as explicitly-labeled diagnostic metadata, never presented as canonical identity.

## What Phase 140 deliberately does NOT provide

- **Historical convergence.** There is no "was this service converged at time T" answer, and none is fabricated. Computing one would require either reconstructing historical desired-placement-authority snapshots (not persisted today) or comparing an old observation against *today's* authority (an explicit, absolute Phase 140 invariant violation — "Historical observation from rev X must not be compared against today's rev Y"). If a future phase wants this, it is a new, separately-designed capability requiring new persistence (historical authority snapshots), not a UI-layer addition.
- **Remediation, mutation, or any write path.** Structurally proven: the Console client exposes exactly one method (`query`), and no Phase 140 file anywhere calls a mutating client method, creates a reconciliation request, or touches placement authority.
- **Analytics, charts, trends, or aggregation.** See the decision below.
- **Retention/pruning.** DMonitor history retention is still indefinite (Phase 139A's own finding, unchanged by Phase 140). Phase 140's bounded *reads* are not a retention policy — they bound what one query returns, never how much is stored.

## Known, accepted architectural debt (not fixed in Phase 140, explicitly deferred)

- **Query cost is O(total persisted history), not O(matched rows) or O(window).** `DMonitorStateV1` is a single JSON-blob domain object; every query deserializes the entire thing regardless of how narrow the requested window is. The 7-day cap bounds the *semantics* of a query (what can match), not its *scan cost*. This is a direct, inherited consequence of the same unbounded-retention limitation above, not something Phase 140 introduces or needs to fix for its own correctness. A future storage/indexing phase would need to address retention and query cost together, since they share the same root cause.
- **`placement-model.ts`/`reconciliation-model.ts` timestamp validators.** Both still use a strict, literal-`Z`-only RFC 3339 regex — the exact same bug class Phase 140A found and fixed in `dmonitor-convergence-model.ts`/`continuum-model.ts` (real backend timestamps, generated via `chrono`'s plain `to_rfc3339()`, render with a `+00:00` suffix, never `Z`). Confirmed, read-only, during Phase 140C: `reconciliation_request.rs`, `ddeploy.rs`, and related modules backing those two Console pages also use plain `to_rfc3339()`, so this is a **real, currently-live defect** affecting already-shipped Phase 136/137/138 pages, not merely theoretical. It was **not fixed in Phase 140C** — doing so is out of this phase's own boundary (polish/closure of the Historical Explorer only) and risks scope creep into unrelated, already-closed phases. **Recommended for prompt follow-up** as a small, independent, two-file fix (apply the same regex relaxation already proven safe in `dmonitor-convergence-model.ts`/`continuum-model.ts`), ideally before either of those pages is exercised against a real production dserver.
- **No URL/shareable filter state.** The Console router has no query-string convention anywhere to extend without building a new state-management framework for a single page. Deferred until a second page also wants shareable filter state justifies that investment.

## Analytics decision (Part O/P)

The canonical historical contract already carries enough honest raw material to support some *narrow, purely-counting* future analytics without fabrication: observations over time, health-state counts, freshness-state counts, grouping by node/service/application, and ClockSkew frequency — all of these are direct aggregations of fields the backend already returns, requiring no new semantics and no authority reinterpretation.

What would be **actively misleading** if built on today's data, and must not be attempted without new architecture:

- Any historical-convergence-shaped chart ("% converged over time") — no historical authority snapshot exists to compare against.
- Any uptime/availability/SLA-styled metric — health and freshness are explicitly not business-health measures (Phase 139's own invariant), and presenting their counts as "uptime %" would smuggle in a business meaning this contract never asserted.
- Any "ghost runtime" or unexpected-realization trend — producers only discover realizations for *currently* governed desired services (Phase 139A's own confirmed limitation), so a trend line here would systematically undercount and imply completeness the system cannot back up.

**Decision: A — STOP AT EXPLORER FOR NOW.** The Historical Explorer is complete and sufficient for Phase 140's own MVP. Nothing currently requires narrow counting analytics (no confirmed operator need, no open gap the Explorer itself fails to cover), and building them now would be scope expansion without a driving requirement. If a future, separately-scoped phase is ever justified, its scope should be limited to the "legitimate" list above, explicitly excluding every item in the "misleading" list, and should be initiated only when a real need is identified — not spawned reflexively from this decision gate.

## Current vs. historical monitoring — the operator-facing distinction

- **Continuum** and an application's own **Deploy** page show *current* operational state (live convergence, current health/freshness as of the last evaluation).
- **History** shows *recorded* evidence from a bounded past window — never "current," never a convergence verdict.

The Historical Explorer's own page copy states this distinction explicitly ("For current operational status, use Continuum or an application's own Deploy page instead") so an operator is never left to infer it from page location alone.

## Closure

All of Part A-T's structural guarantees (no mutation, no convergence recomputation, no current-authority dependency, no analytics/chart code, project/application/node/service isolation, deterministic ordering, honest truncation disclosure, accessible expand/collapse with `aria-expanded`/`aria-controls`, responsive layout with no page-level horizontal overflow) were verified directly — by code reading, by executable tests, and by a real-backend manual HTTP pass — across the 140A, 140B, and 140C review rounds. Phase 140 is closed at the scope described above; the two documented debts (timestamp validators, storage/query-cost) are explicitly carried forward, not resolved here.
