# Phase 140B — Historical Explorer UI & Observation Drill-down

**Status:** implemented, unit/structural-tested, **not committed, not pushed**. Read-only inspection/navigation only. No backend changes.

## Purpose

Phase 140A built the canonical, bounded, read-only historical query API (`GET /api/v1/dmonitor/history/:project_id`). Phase 140B gives an operator the first user-facing surface over it: a project-scoped page to pick a bounded time range, filter by application/node/service, see observations in chronological order, and drill into one observation's canonical detail. Nothing here recomputes health, freshness, convergence, authority correlation, clock quality, or desired state — all of it is rendered exactly as the backend returns it.

## Route

`GET /projects/:id/history` (`project_history` route kind), parsed and built the same way `project_continuum`/`project_placement` already are — project comes from the URL, like every other project-scoped page; application/node/service are **in-page filters**, never separate path segments, since Phase 140A itself treats them as optional query filters, not a required scope.

The page lives in the existing per-project sub-navigation (`PROJECT_SECTIONS` in `project-workspace.ts`) under the already-reserved "History" entry — it was already listed there (Phase 130's own navigation plan named it "History · Phase 140"), just rendered as a disabled "planned" label until now; this phase turns it into a real link, nothing else about that navigation bar changed.

## Filter semantics

Start/end time, application, node, service, and result limit — all optional text/number inputs, directly mirroring Phase 140A's own filter dimensions one-to-one. No PID/container-id/`monitor_instance_id` filter exists, client or server side.

- **Default window:** most recent 24 hours, computed client-side at page load (`defaultHistoryFilters()`) and pre-filled into the start/end inputs so the operator sees exactly what will be queried before touching anything.
- **Maximum window:** 7 days, checked **proactively, client-side**, before any request is sent (`clientSideHistoryValidationError`) — a wider request is never silently narrowed; the form shows a clear validation message instead and refuses to submit. The backend's own 422 rejection remains authoritative in every case (the client check is a convenience, not the enforcement boundary).
- **Start after end:** rejected the same way, client-side first, with the backend's own check as the real guarantee.

## Ordering

Observations are rendered in exactly the order the backend returns them (`response.observations`, iterated directly). The page never calls `.sort()` or `.reverse()` on them — proven by a structural regression test.

## Observation list

Each row shows, at a glance: `received_at_utc`, `node_id`, a one-line logical-identity summary (subject kind + application + service/logical-subject, from `subjectDisplayIdentity`/`subjectKindLabel`), a health badge, and a freshness badge. Rows are plain expandable buttons (`aria-expanded`, keyboard-activatable via native `<button>` semantics) — no modal/drawer framework was introduced, since none exists anywhere in this codebase yet and a plain expandable row is the lowest-risk, most consistent choice among the options the brief itself allowed.

## Drill-down contents

Expanding a row shows: observation id, node, both timestamps (`observed_at_utc` agent-asserted, `received_at_utc` server-asserted), collector, and the raw collector `facts` object pretty-printed as-is. `monitor_instance_id` and `sequence` are shown too, but explicitly labeled "diagnostic only — not canonical identity" in the UI itself, never presented as part of logical subject identity. Expanding/collapsing only flips local component state and re-renders from the already-loaded response; it never calls the API client again (proven structurally).

## Health / freshness / ClockSkew / findings

Health and freshness badges reuse `dmonitor-history-model.ts`'s own label/tone maps (`HEALTH_LABEL`/`FRESHNESS_LABEL` and their tones), textual labels always paired with color, never color-only. `ClockSkew` and `Stale` are distinct labels/tones, never collapsed into one another (`clock_skew.toLowerCase().includes("clock")`, proven by test).

**No "findings" are rendered, and this is deliberate, not an oversight:** the canonical historical response (Phase 140A) carries no `findings` field at all — findings are a *convergence-evaluation* concept, produced only by `evaluate_application_dserv_convergence` for admitted/excluded evidence during a live evaluation, never persisted on a raw historical observation. Phase 140B does not fabricate one; the closest honest analog it can show is the raw collector `facts` object, labeled plainly as "Collector facts (raw, as persisted)" — never relabeled as "findings."

## Timestamp handling

Phase 140A found that `chrono`'s `to_rfc3339()` renders UTC with a `+00:00` suffix, never a literal `Z`. Phase 140B's own display code never reparses or reformats `observed_at_utc`/`received_at_utc` at all — both fields are rendered as the raw string the backend sent, so both valid RFC 3339 spellings display identically by construction (proven by a structural test that the detail view never calls `Date.parse`/`toLocaleString` on these two fields). The one place this page *does* parse a timestamp is the client-side window-width pre-check (`Date.parse` on the form's own start/end inputs, which `Date.parse` already accepts in either spelling natively).

`placement-model.ts`/`reconciliation-model.ts` were not touched or audited for the same `Z`-only issue this phase — they render different timestamp fields (request/batch/attempt lifecycle, not DMonitor `_utc` fields), were not encountered while building this page, and auditing them is explicitly out of 140B's scope per the brief's own instruction ("leave it deferred" unless naturally encountered and demonstrably broken here).

## Empty / loading / error states

Four distinct, mutually exclusive render branches, in priority order: loading (`!loaded`) → error (`errorMessage` set, only ever from a genuinely thrown error) → empty (`observations.length === 0`, rendered as an honest "no evidence recorded" message, never as a failure) → populated list. A `truncated: true` response additionally shows a visible warning banner rather than silently looking complete.

A 422 from the backend surfaces its own specific `message` field directly (e.g. the exact window/range problem), not a generic "query failed" string — the same honest-error-surfacing convention `DMonitorConvergenceClient`/`DMonitorHistoryClient` already share.

## URL / shareable state — not implemented, and why

The existing Console router (`app/router.ts`) parses and tracks only `window.location.pathname`; it has no concept of query-string state anywhere in the codebase (confirmed by survey — no existing page reads/writes `location.search` or `URLSearchParams` for its own state). Adding query-parameter persistence for this page's filters would mean either extending the shared `Route`/`startRouter` machinery to understand an entirely new state dimension every other page would then have to account for, or building an independent, page-local history-management mechanism that bypasses the shared router — both are a new routing/state framework, which the brief explicitly says not to build "solely for this." Phase 140B therefore keeps filter state as plain in-memory component state, not persisted in the URL. A future phase can revisit this once a second page also wants shareable filter state, at which point extending the router once is justified by more than one consumer.

## Request race safety

Mirrors Phase 138's own established generation-counter pattern (`reconciliation.ts`'s `sequence`/`token`, independently re-proven here): `load()` captures `const token = ++sequence` before awaiting the client call, and checks `token !== sequence` immediately after — both on the success path and the failure path — before writing `response`/`errorMessage`/triggering a re-render. A stale, slow, or out-of-order response from an abandoned query can never clobber a newer one's result. Proven by a dedicated structural regression test mirroring the Phase 138 "Audit 27" precedent.

## Relationship between Phase 140A and 140B

140A is the canonical read model and API; 140B is a thin, purely presentational consumer of it. No new Console-side business logic recomputes anything 140A already computes (freshness) or deliberately does not compute (convergence, authority correlation). If 140A's contract ever needs a field this UI cannot honestly represent without fabrication, the correct fix is extending 140A's own contract — not working around the gap in the UI layer, which Phase 140B does not do anywhere in this pass.

## Known limitations (carried forward, not solved here)

- No URL/shareable query state (see above).
- No charts, trend analytics, hourly/daily aggregation, or anomaly detection — explicitly out of scope.
- `placement-model.ts`/`reconciliation-model.ts`'s own `Z`-only timestamp validators were not audited in this phase (not encountered, not touched).
- DMonitor history retention remains unbounded (Phase 139A's finding, restated in 140A's own doc, unchanged here).
