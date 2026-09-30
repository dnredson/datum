# Phase 131 — Project Graph Overview

Status: CLOSED after independent-review hardening, user graphical-browser approval
and final freeze validation. Phase 132 has not started.

## Authority/source audit

| Source inspected | Actual role | Phase 131 decision |
| --- | --- | --- |
| `core/canonical_dgraph.rs`, ADR-0001/0009/0017 | `datum.dgraph/1`: placement-free application D-Graph; vertices D-Serv, edges D-Call; immutable identity/revision | Use snapshots referenced by active DDeploy acceptance only |
| `core/ddeploy.rs`, `ddeploy_v2.rs`, ADR-0018 | Validated proposal snapshots and explicit accepted active authority, v1 and v2; `active_placement_authority` exposes the accepted logical snapshot | Existing authority lookup, never proposal selection or acceptance |
| `core/operational_dgraph.rs`, `api/operational_dgraph.rs` | Operational deployment document v1: project-owned per-node desired service lists, no logical edges. Derived union of accepted applications (with legacy bootstrap/acquisition paths), project operational revision changes with effective accepted composition | Do not label as application D-Graph; do not render node assignment topology here |
| `core/canonical_dmap*.rs` | Single accepted placement authority, with its own identity/revision | Not drawn; not used to invent logical edges |
| `core/canonical_dcontinuum.rs`, `dnode_registry.rs` | Declared infrastructure inventory/capabilities and physical links, not current liveness | Not drawn; Phase 135/136 |
| `core/dgraph.rs` / controlled WASM APIs | Legacy controlled composite graph mixes infrastructure/application/placement/health | Not reinterpreted as logical graph |
| `api/graph.rs`, snapshot graph builder/topology | Observed runtime graph; `get_latest_graph` also persists materialization | Not used: not a side-effect-free logical structural source |
| `api/live_dashboard.rs`, `/api/v1/graph-overview` | Telemetry-enriched observed topology derived from operational document | Not used: runtime/monitoring/placement semantics are outside 131 |
| `core/dserv_artifact.rs`, D-Code registry, ADR-0021 | Separate governed content-addressed executable/artifact bindings | Not synthesized into vertices; current D-Graph does not carry an artifact vertex relation |
| `core/canonical_dforward.rs`, ADR-0023 | DIoT/middleware logical composition in a separate D-Forward contract | Not silently merged into application D-Graph |
| Phase 127 `OperationsSources` / application projections | Validated read snapshots, explicit source availability, accepted logical snapshot accessible through existing DDeploy authority lookup | Extend with a narrow structural read projection |

There is **no general canonical application/D-Graph repository or project-to-application
registry/read API**. Canonical contracts do exist, and reliable immutable logical
snapshots exist inside DDeploy's accepted proposal records. Phase 131 displays
**Accepted application D-Graph snapshots · logical structure**, not every known
application, not the latest editable definition, and not pending proposals. An
application that exists but has never been accepted will not appear. A valid empty
accepted-snapshot source is therefore not proof that a project has no applications.

## Read architecture and scope

```text
/projects/:projectId/graph
           |
           | one GET + matching X-Datum-Project
           v
/api/v1/operations/projects/:project_id/graph
           |
           | Phase 128 owned, awaited, capacity-bounded spawn_blocking
           v
OperationsSources.project_graph_overview
           |
           | validated DDeploy state -> existing active_placement_authority
           v
accepted immutable datum.dgraph/1 snapshots (v1 and v2)
           |
           v
project_graph_read_model -> bounded typed DTO -> Cytoscape + accessible list
```

The HTTP adapter only validates route/header scoping and delegates. It reuses
safe bounded errors and `Cache-Control: no-store`, has no write route, performs
no schema/persistence initialization and does no SQLite work on async workers.
Project scoping is **not authorization**. Canonical D-Graph itself has application
identity, not project identity: project context comes exclusively from the active
accepted authority reference. Source acquisition remains the existing Phase 128
provider; independent sources are not claimed globally atomic.

Only matching-project active references are selected; pending proposals/events/
monitoring never supply logical graph authority. Full persisted DDeploy integrity
validation precedes projection, preserving fail-closed corruption detection. An
unrelated corrupt history record can degrade this source (existing full-source
preflight); the graph does not repair or selectively ignore it.

## Contract, identity and budgets

`ProjectGraphOverview`: project ID, one supplied generated timestamp, fixed typed
source kind, and `SourceSection<GraphContent>`.

`GraphContent`: included application references, typed nodes/edges, coverage with
scope `active_accepted_applications_only`, Complete/Partial, active reference count
and included count. Complete applies only to that accepted source, **not registry
or discovery completeness**. Budgets: 100 applications, 2000 nodes (including
application groups), 4000 D-Calls. Applications are included atomically, in stable
application-ID order; the first application exceeding a budget and subsequent
applications are omitted with Partial coverage. No truncated edges or dangling
endpoints. A single oversized application can yield a partial zero-node preview,
which has its own nonempty-source message.

Application references include actual D-Graph ID/revision/digest and acceptance
reference. These are not DMap revisions. Nodes have canonical ID, application,
kind, optional parent, and a small declared execution/resource summary. Decimal
memory-byte strings preserve exact canonical u64 values in JavaScript. Other
numeric fields currently require safe JavaScript integers at the client boundary;
an unsupported large revision is rejected rather than rounded.

Projection IDs are collision-free JSON tuples of application/graph/kind/canonical
ID; consumers treat them as opaque visual keys. Identical service/call IDs in
different applications remain separate. They are not new canonical identities and
are not parsed to recover authority. Full original IDs remain in the accessible
list/details; canvas labels alone are abbreviated. No arbitrary raw metadata,
artifact configurations, credentials, addresses or source diagnostics are exposed.

## Nodes, edges and interaction

- Application: presentation-only compound grouping of one accepted snapshot.
- D-Serv: actual canonical logical service, rounded rectangle.
- D-Call: actual canonical directed call with call ID, source and target.

Only one canonical edge kind is present in this selected contract. No fake
dependency/containment/network/placement edge is invented to create another kind;
application grouping is a compound parent, not a canonical edge. D-Code and DIoT
are not fabricated from other stores.

D-Call cycles, reciprocal calls and self-calls are accepted and drawn with directed
Bezier edges/self-loops. Communication topology has **no deployment-order
implication**. No cycle detection rejects them.

The toolbar offers Fit graph, Reset selection/view and a local application snapshot
filter. Filtering changes only displayed elements; it neither fetches applications
nor modifies the DTO/source. Project Overview and Graph are real routes; Services,
Placement, Deploy, Runtime and History remain informational disabled items. The
global wallboard is unchanged.

Cytoscape.js 3.34.3 is the only new dependency; it includes TypeScript declarations.
No React wrapper or layout plugin. The deterministic two-level layout sorts opaque
application keys, sorts D-Serv children within each group, and places children on
a local grid (200px horizontal / 160px vertical spacing). Group header/padding and
minimum dimensions are included in measured Cytoscape bounding boxes, including
labels. Groups are packed in stable outer rows with an 80px gutter; each row uses
its largest measured height. Empty groups retain explicit minimum sizing.
This replaces the unsafe single global child grid, whose compound rectangles could
overlap. Edges never affect coordinates; no random force layout or graph analysis.
Layout positions are presentation only, not execution/deployment order.
Pan/zoom uses standard gestures, bounded zoom 0.15–3. Dragging changes visual
positions only; nothing is saved. Reset restores the same grouped layout and clears selection,
preserving the chosen application filter.
Filtering recomputes compact geometry for the visible application set; returning
to All restores the original deterministic layout. No hidden geometry is retained.

Partial coverage now explicitly says **Bounded graph preview**, shows included /
active accepted snapshot counts, and derives the exact omitted count as active
minus included (never DOM element counts). It explains the 100-application /
2000-node (including groups) / 4000-D-Call limits and stop-at-first-exceeded policy.
Later smaller snapshots are not considered after the first budget violation; this
is a deterministic prefix, not globally optimized selection. No partially included
application is emitted. An oversized first snapshot gets an explicit bounded-empty
message, distinct from a valid empty accepted source. No exact budget-reason enum
was added: the bounded-preview cause is sufficient for this narrow hardening.

Node/edge selection updates a bounded typed details panel: full canonical identity,
project/application context, D-Graph provenance/revision/digest, acceptance reference,
incoming/outgoing D-Call counts and declared ABI/resources where applicable. A
self-call counts once incoming and once outgoing. Edge detail shows its canonical
call ID and full source/target service identities. No raw JSON dump or write control.

Canvas lifecycle is owned by the page: old instances are destroyed on filtering,
refresh/navigation/pagehide/HMR and resize observers disconnected. Obsolete
navigation requests retain the existing AbortController guard. Manual refresh only;
no polling, SSE, persistence or backend N+1.

## Availability, accessibility and responsive behavior

Available empty, NotConfigured, Unavailable, Corrupt and Indeterminate remain
distinct. No unavailable source becomes an empty canvas or numeric zero. If a
renderer cannot initialize, the accessible structure/details remain usable with an
explicit renderer notice; that is not a backend corruption or infrastructure claim.
NotIncluded/unsupported graph semantics are explained by the fixed source boundary,
not substituted for source failure states. No runtime health score or overlays.

The legend describes node shapes, typed call arrows and source semantics in text;
structural colors do not reuse critical-alert colors. The canvas is not claimed
fully accessible. A keyboard-accessible entity/relationship list is the inspection
alternative, with native buttons and live selected details, semantic headings,
breadcrumb/navigation labels, full wrapping identities and existing focus styling.

Wide desktop/laptop uses canvas + compact details column. At <=900px, including
768px tablet, details stack below. Canvas height is 34rem desktop/28rem tablet with
25rem minimum; controls wrap and long identity buttons wrap. Target widths are
1440/1024/768. DOM/CSS checks are not graphical-browser certification.

## Validation and manual workflow

Initial implementation validation: 47 lightweight tests passed (18 graph-specific); 73 DOM/freeze
tests passed (13 graph-page checks). Five graph/read-model tests and all 11
operations API tests passed. Full DServer: 2240 passed, zero failed, one ignored,
459.46 seconds. Rust build and strict TypeScript/Vite build passed. npm installation
added only Cytoscape and reported zero vulnerabilities. Existing Rust unused/
dead-code warnings remain. One production build under concurrent Rust test load
emitted an informational slow-plugin timing warning; no compilation failure or
dependency advisory occurred. Frontend targeted formatting and Rust formatting of
the changed API/new module passed; only added sections of the existing large
Operations Read Models file were formatted to preserve the baseline.

Lightweight tests validate the DTO, scope, identities, ordering, source distinctions,
budgets, typed edge semantics, filter immutability and real headless Cytoscape cycles/
self-calls. The existing optional Happy DOM gate runs the actual production bundle
with isolated DServer/Vite API acquisition and test-only populated graph fixtures;
it exercises route/nav, toolbar, details, legend, filtering, source states, partial
coverage, long IDs and responsive CSS. Fixtures never enter production or canonical
storage. A graphical browser was not available through the automation surface;
the user subsequently completed the required populated graphical-browser review
and approved the visual gate. Sparse real server data tests
the real unavailable/empty source path, not a fabricated populated graph.

```powershell
# Terminal 1, repository root
cargo run --manifest-path dserver/Cargo.toml
# Terminal 2
cd console
npm install
npm run dev
```

Open `http://127.0.0.1:5173/projects/<URL-encoded-project-id>/graph`.
Vite proxies `/api` to DServer 8080. Normal persisted accepted data is required to
see populated graphs; no production demo switch or injection is introduced.
The production JS bundle now includes Cytoscape; route-level code splitting and
source/projection latency measurement remain future optimization, not authority.
Library integration follows the [Cytoscape.js documentation](https://js.cytoscape.org/).

## Final hardening and user visual review

F1 tests measure padded compound bounding boxes for 2/3/2, 1/8/2/12,
30/1/2/1 and empty-group mixes. They verify repeatability, Reset after dragging,
subset filtering / All restoration, long-label header clearance and unchanged
coordinates for reciprocal/self/parallel D-Calls. F2 backend tests cover each
budget, atomic omission, sorted inclusion, oversized first snapshots by nodes and
edges, exact active/included/derived-omitted counts, and true-empty versus
bounded-empty partial results. DOM checks exercise visible reasons and omissions.

Graphical browser inventory during hardening was empty. Headless bounding-box and
DOM checks are not visual certification. The user subsequently performed the real
graphical-browser inspection using the populated Phase 131 fixture: **visual gate
PASSED**. Application groups were visually separated with no visible overlap and
clear DServ membership. Application filtering, directed reciprocal/parallel/self
DCalls, canonical-identity Details, Fit and Reset were inspected and approved;
layout remained usable in the inspected views. This user approval satisfies F1's
manual visual freeze gate. No screenshots were added to the repository.

Final hardening validation: 58 lightweight tests passed (29 graph-specific,
including eight layout/bounding-box cases and three coverage cases); 75 DOM/freeze
tests passed. Nine focused graph/projection tests and all 11 operations API tests
passed. Full DServer: 2244 passed, zero failed, one ignored (386.41 seconds).
Strict TypeScript/Vite build and Rust build passed. Existing Rust warnings remain:
45 for DServer tests, 205 for DServer build, 18 for the existing inspect binary.
No frontend dependency was added during hardening. Only the five touched frontend
implementation/test files and the graph Rust module received targeted formatting.

Final freeze rerun: 58 lightweight tests, 29 graph-specific tests, eight dedicated
headless layout/bounding-box tests, 75 DOM/freeze tests, nine focused graph/projection
tests and 11 operations API tests all passed. Full DServer rerun: 2244 passed,
zero failed, one ignored (144.65 seconds). Strict frontend build, Rust build,
targeted Rust formatting checks and whitespace checks passed; npm audit reported
zero vulnerabilities. The final semantic gate preserved accepted-snapshot-only,
read-only logical structure and source-relative bounded coverage.

For normal persisted data, run from the repository root:

```powershell
cd C:\Users\dnred\OneDrive\projetos\PD
cargo run --manifest-path dserver/Cargo.toml --bin DServer
# Second terminal
cd C:\Users\dnred\OneDrive\projetos\PD\console
npm run dev
```

Open `http://127.0.0.1:5173/projects/<URL-encoded-project-id>/graph`.
Normal server startup retains its existing recovery/configuration requirements;
do not enable empty bootstrap against an existing production state for this review.

For a populated **development-only, browser-local** fixture, open the Vite Console
then paste the following in browser developer tools. The explicitly imported test
module is not part of the production build; this replaces only one graph GET in
this browser session, never DServer canonical state. The fixture has 2/3/2/8
services, reciprocal calls, self-calls and parallel calls.
Fixture-only graphical review needs only Vite: DServer may be stopped, because the
explicit browser override supplies that one graph response. Other routes may then
show Disconnected; that is expected and not a graph health claim. The normal real
data workflow above still requires DServer.

```javascript
const { unevenGraphFixture } = await import('/tests/graph-fixture.mjs');
const originalFetch = window.fetch.bind(window);
window.fetch = (input, init) => String(input) === '/api/v1/operations/projects/phase131-visual-review/graph'
  ? Promise.resolve(new Response(JSON.stringify(unevenGraphFixture()), {
      headers: { 'Content-Type': 'application/json' }
    }))
  : originalFetch(input, init);
history.pushState(null, '', '/projects/phase131-visual-review/graph');
dispatchEvent(new PopStateEvent('popstate'));
```

Inspect at 1440px and 1024px: distinct compound membership, headers, arrows, loops,
Details, Fit, Reset and application filter. Reload to remove the override; no mock
activation flag or fake production data is introduced.

Deferred review items remain unchanged: application-group degree presentation
(F3); stricter application-qualified canonical service/call duplicate validation
(F4); acquisition/projection latency optimization. Full DDeploy integrity preflight
is not weakened.

**Logical project/application graph != physical resource ownership.** Nothing here
implies a DNode/server belongs exclusively to a project or a logical DServ maps
directly to one physical server. Future global DNodes, ProjectResourceBinding,
reservations, shared service instances and tenant-aware services remain compatible;
none is implemented in this phase.

## Phase boundaries

131: accepted logical structure and safe inspection only.
132: expandable runtime graph/observed realization and runtime overlays.
133A/B: Artifact & Service Catalog / Guided Service Modeling.
134: Visual DGraph Editor, creation/deletion/edge editing/save.
135: Continuum visualization.
136: placement visualization and explicit manual service → node intent.
137/138: proposal review/acceptance and deployment/reconciliation.
139/140: runtime monitoring and Historical Explorer.
None of those future write/runtime/placement/monitoring tools are implemented here.
