# Phase 142A — DATUM Live Foundation

**Status:** implemented, unit-tested, **not committed**. Frontend-only. Read-only (GET only). No AI/LLM/GNN/MCP, no new node-health score, no server-side Live View persistence (reserved for Phase 142B).

## Why no new backend endpoint

The architecture audit that preceded this implementation confirmed every value DATUM Live needs already exists in two pre-existing, unmodified read models:

- `GET /api/v1/operations/overview` (`GlobalOperationsOverview`/`GlobalOverview`) — cross-project alerts (`AlertSection`), recent events, discovery, and `temporal_quality` (which already carries `node_id` as a top-level field).
- `GET /api/v1/operations/continuum` (`ContinuumViewV1`/`ContinuumView`) — the full, global, cross-project D-Node inventory, each node's `resource_observation` (host-level CPU/memory/swap), `accepted_placements`, and `observations` (DMonitor evidence, per accepted placement).

No backend file was touched. `dserver/` has zero diff in this phase.

## Known, honest limitation: partial alert/event-to-node correlation

`DatumEventSubjectV1` (the type behind `OperationalAlert.subject`/`OperationalEvent.subject`) only carries `node_id` on the `dnode`, `dserv` (optional), `runtime_instance`, and `dependency_edge` variants — never on `project`, `application`, `dcall`, `diot`, `service_artifact`, `dmap`, or `other`. `correlateAlertsByNode`/`correlateEventsByNode` (`src/live/live-view.ts`) therefore index only alerts/events whose subject genuinely carries a `node_id` — every other alert/event is correctly absent from every node's bucket, never guessed. This is a structural property of the existing event model, not a Phase 142A defect, and it is covered by a dedicated regression test (`live-view.test.mjs`: "never fabricates a node association for a subject with no node_id").

## No new node-health score

Node cards and the drawer render only canonical, already-existing values verbatim: DMonitor's own `freshness`/`health`/`placement_convergence` enums (reusing the Continuum page's own `FRESHNESS_LABEL`/`HEALTH_LABEL`/`CONVERGENCE_LABEL` tone maps, now exported from `pages/continuum.ts` rather than duplicated), raw observed CPU/memory values, and literal alert counts/severities. No threshold (`cpu > 90%`), no aggregation rule ("any placement degraded ⇒ node degraded") exists anywhere in `src/live/`. A node with multiple observations shows one badge per observation — never collapsed into one verdict. A structural test asserts no CPU-threshold comparison or `unhealthy =` derivation exists in the Live source.

## Two host-level vs. per-placement resource sources, never merged

A node's compact card shows `resource_observation` (one, host-level, Resource Registry-sourced) through the existing `terminalMetricPanel` component; per-placement `runtime_usage` (DMonitor-sourced, per accepted placement) is shown separately in the node drawer via the reused `nodeDetail()` panel. These are never averaged or summed into a single per-node number.

## Architecture: one Vite project, two entry points

`console/index.html` → Console (`src/main.ts`, unchanged). `console/live.html` → Live (`src/live/main.ts`, new). `vite.config.ts` gained `build.rollupOptions.input: { console: "index.html", live: "live.html" }` — the only build-level change multi-page required; `tsconfig.json`'s `include: ["src"]` already covered the new `src/live/` directory with no change. Both entries share every TypeScript type, the same `OperationsClient`, and the same presentation primitives (`components/presentation.ts`, `components/terminal-metrics.ts`) — no contract is duplicated.

Reused as-is from the existing Console: `OperationsClient.overview()`/`.continuum()`, `badge`/`severityBadge`/`el` (`components/presentation.ts`), `terminalMetricPanel` (`components/terminal-metrics.ts`), `activeAttention`/`subjectLabel`/`duration`/`kindLabel` (`pages/wallboard.ts`), `projectPath`/`applicationOverviewPath` (`app/router.ts`).

Newly exported (pure additive, zero behavior change) from `pages/continuum.ts` so Live could reuse them instead of re-deriving equivalent logic: `bytes`, `nodeDetail`, `FRESHNESS_TONE`/`FRESHNESS_LABEL`, `HEALTH_TONE`/`HEALTH_LABEL`, `CONVERGENCE_TONE`/`CONVERGENCE_LABEL`.

In production, `console.datumlab.net` and `live.datumlab.net` are two Nginx vhosts, each pointed at the matching built HTML file (`dist/index.html` vs `dist/live.html`) from the same build output, both proxying `/api` to the same local DServer — a deployment detail that never leaks into the application contract.

## `LiveViewV1` — local profile model

`src/live/live-profile.ts`. Schema-versioned (`datum.live-view/1` per profile, `datum.live-views-store/1` for the envelope), persisted to `localStorage` under `datum.live-views/v1`. Tolerates, individually, without discarding the whole store: missing key, corrupted JSON, unknown top-level schema, a mix of valid/invalid profile entries (each profile is revived independently; a bad one is dropped, not the whole list), an out-of-range refresh interval (repaired to the default, never silently making the profile unusable), and duplicate pinned node ids (de-duplicated, order preserved). A pinned node absent from the current continuum is never silently dropped from the profile — it renders as a "Not currently available" card/drawer state instead (see `resolvePins`).

Multiple Lives coexist in the same browser as independent store entries; two tabs stay independent via `?view=<id>` in the URL (`src/live/main.ts`), which is read once at startup and written back with `history.replaceState` so reloading or duplicating a tab keeps the same profile. This is **not** the server-side sharing Phase 142B will add — it only disambiguates tabs on the same machine.

## Attention/Alarms and Recent Activity

Both sections render the existing canonical `OperationalAlert`/`OperationalEvent` objects via the existing `activeAttention()` ordering (Phase 127's own backend-supplied order, never re-ranked) and the existing `kindLabel`/`subjectLabel`/`duration` formatters. Filters (severity, project id, node id) only `Array.filter` the already-ordered list — they never resort it (`filterAlerts`/`filterEvents`, each covered by an order-preservation test). ALERT and EVENT remain two structurally separate lists; an event is never rendered as if it were an active alert.

## Polling

A recursive `setTimeout` loop (never overlapping `setInterval`), one `AbortController` per cycle, aborting the previous request before starting the next. A failed poll never clears `overview`/`continuum` — the last successful render stays on screen with the connection badge moved to Degraded/Disconnected and the age label indicating a stale update. Pause/resume stops/resumes scheduling only; an in-flight request is allowed to finish. No WebSocket/SSE was introduced — none existed in the Console codebase to reuse, and continuous GET polling was judged sufficient for this phase.

## Kiosk mode

`active.kiosk_mode` enlarges card density/typography; `active.hide_secondary_controls` hides operator-only chrome (profile switcher, add-node row, filter row, remove buttons) via CSS — it never hides any evidence section itself. A plain `requestFullscreen()`/`exitFullscreen()` toggle is provided; no remote TV management was implemented (out of scope, see below).

## Drill-down to the Console

The node drawer links to `applicationOverviewPath(project, application)` per distinct accepted placement, and the header links to `/continuum`. These are plain cross-bundle `<a href>` navigations (a full page load into the Console entry point) — Live never mounts the Console's SPA router, so these links deliberately carry no `data-route` attribute.

## Explicitly out of scope (reserved for Phase 142B)

- Server-side persistence/sharing of Live Views (stable cross-device URLs).
- Any new aggregator endpoint.
- WebSocket/SSE.
- Remote kiosk/TV management.
- A node `role`/`stage` field — `DNodeRegistryStateV1` has none today; the card and drawer simply omit it rather than inventing one.
- Deprecating `dserver/static/live-dashboard.html` (left untouched, as instructed).

## Tests

`console/tests/live-profile.test.mjs` (profile persistence, corruption tolerance, multi-profile coexistence) and `console/tests/live-view.test.mjs` (correlation, pin resolution, filter order-preservation, and structural regressions proving no mutation/AI reference, no direct `fetch`, and no fabricated health threshold). Full existing Console suite (396 tests) and `dserver` suite (2474 tests) both pass unchanged alongside the 28 new tests.
