# Phase 128 — DATUM Console Foundation

Status: implemented, uncommitted, awaiting independent review and visual feedback.
This is the shell/API bridge, not the Phase 129 wallboard or Phase 130 workspace.

## Architecture and audited precedent

```text
Browser: Bootstrap 5 + TypeScript + Vite
                     |
              GET /api/v1/operations/*
                     |
          bounded owned spawn_blocking
                     |
           Phase 127 OperationsSources
                     |
       validated snapshots / READ_ONLY SQLite
                     |
         existing canonical/observational state
```

DServer already uses Axum 0.7 and Tokio; the repository default is port 8080.
Legacy `/dashboard/live`, `/api/v1/dashboard/live` and `/api/v1/graph-overview`
remain unchanged. The embedded `static/live-dashboard.html` is not the new
Console architecture. No existing modular frontend/client generator was found.
Existing CORS is configurable and defaults to allowing any origin; this phase
does not widen or replace that policy. A same-origin reverse proxy avoids CORS
for the Console. CORS, route identity and `X-Datum-Project` are **not authorization**.
No authentication/login/avatar fiction is added. Deployment authentication and
access controls must be addressed before exposing operational data publicly.

## Stack and directory

`console/` contains index.html, package.json/package-lock.json, tsconfig.json,
vite.config.ts, src/main.ts, app/router.ts, api/client.ts, types/operations.ts,
components/presentation.ts, pages/views.ts, styles/console.css and lightweight
Node tests. Dependencies are Bootstrap 5.3.8 and Bootstrap Icons 1.13.1;
build tools are Vite 8.3.1 and TypeScript 7.0.2. Node 24 was used for validation.
No React, Axios, router framework, Cytoscape, external font service or BFF.
Bootstrap CSS and icon fonts are local bundled assets; Bootstrap JS is not needed.
Rust remains the canonical contract. TS declares only the fields consumed here,
with runtime checks for section/discovery shapes, supported temporal tokens and
safe integer counts. Unsupported/malformed responses yield an explicit error,
not invented zero. A future export/schema mechanism can replace this narrow subset.

## Read-only endpoints

- `GET /api/v1/operations/overview`: exact Phase 127 global overview, fixed limit
  100 and recent 24-hour window based on one supplied server evaluation time.
- `GET /api/v1/operations/projects/:project_id`: exact Phase 127 project overview,
  same bound/time policy. A route/header match is required.

The adapter does not rebuild counts, discovery, temporal or lifecycle semantics.
It supplies immutable sources to the existing builders. No application, event
history, cursor, write or broad management endpoints are exposed in this phase.
Unknown project references can still yield a valid scoped, empty/degraded view;
there is no canonical project registry whose absence could justify a false 404.

Production source acquisition:

- operational graphs: cloned already-restored in-process graph map under existing
  synchronization. GET never triggers persistence initialization or canonical overlay.
- DDeploy/DMonitor/DNode: current domain blobs read in one READ_ONLY SQLite
  transaction, then existing Phase 127 canonical validators. Missing domain means
  NotConfigured; missing/unreadable database means Unavailable, not empty.
- Event/Alert: Phase 127 read-only repository, using Phase 126's retained Event/Alert
  runtime path. No schema initialization or missing-database creation.
- scheduler: absent runtime registration remains explicitly Unwired.

Control-state path is resolved against startup working directory once by the
Console runtime. Event/Alert configuration remains the Phase 126 process-stable
configuration. Configured malformed Event/Alert path produces a safe 503; individual
readable-source failures degrade independent sections as Phase 127 specifies.
Control-state SQLite corruption codes/invalid column type remain explicit Corrupt;
other acquisition failures are Unavailable. No detailed path/SQL is serialized.
Expected DMonitor streams are not invented; the existing accepted observations
are projected with existing default freshness policy. All sections use the same
generated/evaluation UTC timestamp; cross-store transactional atomicity is not claimed.

### Blocking work and capacity

All source acquisition and synchronous projection work is inside `spawn_blocking`.
Four permits bound outstanding work; extra concurrent reads get safe 503
`read_capacity_exhausted` rather than building an unbounded task queue. The permit
belongs to the blocking closure until it finishes. Normal handlers await the task.
Request cancellation cannot abort already-started Tokio blocking work; it finishes
with owned data/permit cleanup. Existing SQLite reads use five-second busy timeouts;
CPU-sized snapshot validation is not a hard real-time deadline. No detached periodic
consumer, scheduler or immortal Console task is started.

### Scope and safe errors

Project path must be nonblank/unpadded, <=256 UTF-8 bytes and contain no control
characters. Browser paths use encodeURIComponent/decodeURIComponent. Header value
is trimmed according to existing API practice and must exactly match route identity.
The existing textual HTTP-header scope convention limits non-ASCII header values;
URL parsing supports encoded Unicode but this phase does not introduce a new
Unicode header encoding protocol. Scope is not tenant isolation/authentication.
Global overview intentionally aggregates this DServer's projects without a project
header. Federation is not implemented.

Errors use `{ "error": { "code", "message", "source", "scope" } }`.
Invalid scope/path is 400; capacity/configuration/storage availability is 503;
top-level corruption/projection failure is 500. Independent source degradation
inside a valid projection remains 200 with Phase 127 source issues. Messages are
fixed safe classifications or Phase 127 bounded safe messages (<=512 UTF-8 bytes).
Raw exception diagnostics, SQL, paths, environment values and credentials never
become response text. Detailed diagnostics stay internal to their existing error
objects; this adapter does not add raw diagnostic logging.

## Shell, navigation and interaction

### DATUM identity and terminology

Reference: [published DATUM documentation](https://dnredson.github.io/datum-docs/),
`documentation/reference/visual-identity.md`, `glossary.md`, ADR-0001 and
`mkdocs.yml`. The published home is newer than the local documentation home;
current repository assets and frozen backend contracts take precedence. No new
logo, symbol, or image variant was generated.

Original, byte-identical PNG copies are required in Vite's public directory so
the independently deployed build does not depend on the documentation tree:

| Source | Console destination | Usage |
| --- | --- | --- |
| `documentation/assets/brand/datum-monochrome.png` | `console/public/brand/datum-monochrome.png` | White mark on dark expanded/compact navigation |
| `documentation/assets/brand/datum-logo-horizontal.png` | `console/public/brand/datum-logo-horizontal.png` | Compact light-topbar wordmark, preserving aspect ratio |
| `documentation/assets/brand/datum-icon.png` | `console/public/brand/datum-icon.png` | PNG favicon; page title is DATUM Console |

The existing `datum-symbol.png` was inspected but not copied. Original PNGs are
relatively large; preservation is intentional, with no lossy resize or redraw.
Decorative logo images have empty alt text inside explicitly labeled home links.

Operator labels use D-Monitor evidence, accepted D-Map and D-Deploy proposal.
Legacy operational graph data is labeled operational deployment documents, not
misrepresented as canonical application D-Graph. D-Node, D-Serv, D-Code, D-Call,
continuum, placement and reconciliation retain their distinct documented meanings;
this foundation does not add screens for those subsystems.

Desktop navigation expands to 238px or collapses to a 76px icon rail. Both states
retain accessible link names. At tablet width it becomes an inert, hidden drawer
until opened; Escape closes it and restores menu focus. No fake workspace links,
account controls, or operational buttons are added. The light topbar is 64px high;
compact summary cards and shared spacing tokens establish the foundation only.
Projection notices remain visible as a count with expandable source details,
explicitly distinguished from node availability. Discovery and provenance labels
are human-readable, and long identities remain wrap-safe and inspectable.

Dark engineering sidebar, light workspace, subtle borders, restrained dense cards,
DATUM identity, current page/context, connection indicator and read-only disclosure.
Navigation is Overview and Projects only; no fake operational controls.
History API URL state drives `/`, `/projects`, `/projects/:projectId`; browser
back/forward and deep links are supported. Unknown/malformed/future routes render
Not Found; no future workspace page is falsely presented as implemented.

Overview proves real integration through graph-backed managed project count,
active alert count, resolved-pending-close count, 24-hour recent event count,
source issues and a small alert-backed projects-requiring-attention preview.
These are not canonical system health or opaque scores. Projects shows **Known
projects**, discovery coverage per source and explicit bounded-page limitations.
`has_more` and source completeness remain independent. Empty partial discovery says
“No projects are currently discoverable from the available sources,” not “no projects.”
Project shell shows scoped attention/event-page counts, known application references
and provenance, a small temporal preview and accepted-source availability, not a
full workspace or runtime reconstruction. Event-page length is labeled loaded,
not historical total. Pending proposals are never used as accepted placement.

### Async and presentation contracts

Each route explicitly renders Loading, Ready, Empty, Degraded/Partial or Error.
Refresh is manual only; no polling timers, WebSockets or SSE. Navigation aborts
obsolete requests; aborted results cannot update the current page. The fetch client
uses relative URLs, GET, no-store, caller AbortSignal and a cleaned-up 15-second
timeout. Non-JSON/unsupported contracts have safe error states. Connection labels
Connecting/Connected/Degraded/Disconnected describe Console connectivity/projection
quality only; reachable HTTP failures are degraded, network failures disconnected.

Available/NotConfigured/Unavailable/Corrupt/Indeterminate have reusable textual
badges and explicit source states; none turns into zero. NotConfigured is neutral.
Severity Info/Warning/High/Critical and lifecycle Open/Acknowledged/InProgress/
Resolved/Closed use centralized primitives. Resolved means pending closure;
Closed means history. Alert counts are not node health or outage counts.

Temporal primitives always show **Communication**, **Observation**, **Source time**
separately. Aligned stale evidence is not clock discrepancy; recent receipt with
old observed time can be fresh communication, stale observation, behind/delayed
source time. No Down/Offline/Unreachable derivation exists in the Console.

Backend strings are rendered via textContent, never dynamic HTML interpolation.
Semantic nav/header/main/sections, a skip link, aria-current, aria-live/aria-busy,
alert roles, labeled menu button, textual statuses and visible keyboard focus are
provided. Desktop/laptop layout compresses to two metric columns; tablet navigation
collapses behind an aria-expanded toggle. Phone perfection is not this phase's goal.

## Development and production deployment

Terminal 1 (repository root):

```powershell
Set-Location 'C:\Users\dnred\OneDrive\projetos\PD'
cargo run --manifest-path dserver/Cargo.toml
```

Terminal 2:

```powershell
Set-Location 'C:\Users\dnred\OneDrive\projetos\PD\console'
npm ci
npm run dev
```

Open http://127.0.0.1:5173. Vite proxies `/api` to http://127.0.0.1:8080.
Inspect `/projects` and `/projects/<URL-encoded-project-id>` using a known project
link. No demo mode is installed. With no configured/available data the shell and
honest empty/degraded states are still reviewable; populated project detail requires
existing real source data. Tests use isolated storage and response fixtures only.
Set `DATUM_DEV_API_TARGET` in a console/.env.local file or process environment to
override the development/preview proxy target (not browser application code).
Production build: `npm run build`; strict compilation and Vite emit `console/dist`.
`npm run preview` allows local built-asset integration testing; it is not production hosting.

Production static hosting is deliberately separate from DServer/cargo builds.
Serve dist at the host root through an existing static server/reverse proxy:
`/assets/*` serves files, `/api/*` proxies DServer, other browser paths fall back
to `/index.html`. For nginx, the essential SPA rule is `try_files $uri /index.html`;
give `/api/` its own proxy location before that fallback. No automatic frontend
build, embedded assets, filesystem serving route or cargo dependency is added.
Deploy TLS/auth/access controls at the deployment boundary; do not expose the
current permissive CORS/non-authenticated APIs as an authenticated Console.

## Validation and limitations

Backend endpoint tests cover exact Phase 127 mapping, degraded/corrupt safe issues,
top-level errors, encoded route/header scoping, invalid path and POST rejection,
current-thread Tokio blocking isolation/capacity, and noncreating/byte-preserving
read-only acquisition. Frontend uses Node's built-in test runner for pure routes,
semantic presentation and fetch/error contracts, plus strict production compilation.
No testing framework dependency is introduced.

HTTP smoke: real isolated DServer + Vite proxy returned overview and matching
project responses; `/`, `/projects`, project deep link, unknown-route SPA fallback
and transformed main module returned 200. This proves serving/API connectivity,
not visual DOM rendering. Browser smoke was attempted via computer-use; no browser
was available (Edge/in-app unavailable, empty browser inventory). Visual review,
keyboard/back-forward and tablet render checks remain an explicit pre-freeze gate.
Fixture runtime used disposable temp storage, not production authority data.

## Future phase ownership

| Phase | Ownership |
| --- | --- |
| 128 | Foundation, shell, read-only API bridge |
| 129 | Global overview / wallboard |
| 130 | Project workspace |
| 131 | Graph overview |
| 132 | Expandable runtime graph |
| 133 | Service modeling |
| 134 | Visual DGraph editor |
| 135 | Continuum view |
| 136 | Placement UI |
| 137 | Proposal / review / acceptance |
| 138 | Deploy / reconciliation |
| 139 | Runtime / monitoring |
| 140 | History |

Filter/auth-bound opaque cursors, broad projection performance measurement,
scoped preflight, full auth policy and optional module decomposition remain future
integration concerns. No Phase 129 wallboard/charts/history browser, graph/placement
visualization or write action is implemented here.

## Final Phase 128 interaction gate

The graphical capability inventory remained empty. The authorized fallback uses
the actual built Console JavaScript in Happy DOM 20.14.5, with a real isolated
DServer and the existing Vite API proxy. Happy DOM is installed only in disposable
external test storage; it is **not** a Console dependency or a graphical browser.
No screenshot/rendered-layout/native keyboard verification is claimed.

`tests/freeze-gate.mjs` is an optional executable gate. Build first, install Happy
DOM in a temporary directory, set `DATUM_DOM_GATE_MODULE` to that directory's
`node_modules/happy-dom/lib/index.js`, then run:

```powershell
node --experimental-strip-types --test tests/freeze-gate.mjs
```

Nine gate checks cover: byte-identical served/build brand assets and compact
desktop navigation; actual bundle/real API boot; URL navigation/scoping,
deep links and popstate boundaries; 1440/1024/768 CSS cascade and tablet focus
contract; lifecycle/long-error semantics; numeric muted-text contrast >=4.5:1;
the three temporal examples and long identifiers/provenance; complete/partial
empty discovery and availability; network error/retry and obsolete request cleanup.
The regular seven frontend tests and strict production build also passed.
The real isolated server had no discovered projects; long-content, temporal and
nonempty discovery cases use explicit nonpersisted response fixtures, not fabricated
production state. Native history traversal is not implemented by this DOM emulator;
tests exercise URL boundaries and the production router's popstate listener.

Foundation fixes from this gate:

- collapsed tablet navigation is inert/aria-hidden, preventing invisible focus;
  Escape closes it and restores the menu-button focus, while route changes focus
  the workspace; no focus trap is introduced;
- unknown navigation clears stale loading/Connecting state;
- long identifiers/messages/provenance wrap through shared min-width/grid/flex
  rules; full project identity remains in the page heading and topbar tooltip;
- discovery coverage uses human-facing source labels and expandable detail rather
  than exposing a dense wall of backend-token badges;
- node identity sits above three equally distinct temporal dimensions; source-time
  anomalies use warning rather than a combined health badge or red connectivity claim.

Actual rendered spacing/overflow, keyboard Tab order, browser Back/Forward and
subjective hierarchy still require a graphical/manual look. The fallback establishes
runtime/DOM/CSS contracts, not visual certification. No backend file or dependency
manifest/lockfile was changed by this final pass; no Phase 129 feature was added.
