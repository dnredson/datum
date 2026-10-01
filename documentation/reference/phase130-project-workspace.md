# Phase 130 — Project Workspace Foundation

Status: CLOSED — independent review accepted; required F1/F2 fixes and final freeze gates passed.

## Purpose and authority boundary

The global wallboard answers what is happening across this DServer. The project
workspace answers what is happening inside the requested project and where the
operator can go next. It is a read-only projection, not a project registry,
canonical health score, deployment tool or second convergence evaluator.

```text
Global Overview / Known projects
               |
               v
/projects/:projectId — Project Workspace Overview
               |
               | one scoped GET; X-Datum-Project matches route identity
               v
GET /api/v1/operations/projects/:project_id
               |
               v
existing Phase 128 owned spawn_blocking API adapter
               |
               v
Phase 127 ProjectOperationsOverview
    +-- discovered application references and coverage
    +-- accepted placement authority projection
    +-- operational graph / desired aggregate assignments
    +-- alert lifecycle, event history and temporal evidence
```

No backend/API/schema changes, writes, authentication fiction or dependencies
are introduced. Events remain immutable history; alerts remain lifecycle state;
accepted D-Map authority remains in DDeploy. UI state does not accept maps,
deploy, reconcile, resolve alerts, create services or modify placement/graphs.

## Navigation and information hierarchy

Global Overview and Projects remain in the sidebar. A compact breadcrumb and
full canonical project ID identify the workspace context. One project
subnavigation strip avoids a second competing sidebar:

| Section | Ownership / availability |
| --- | --- |
| Overview | Phase 130; current read-only route |
| Services | Phase 133A / 133B; disabled informational label |
| Graph | Phase 131 / 132; disabled informational label |
| Placement | Phase 136; disabled informational label |
| Deploy | Phase 138; disabled informational label |
| Runtime | Phase 139; disabled informational label |
| History | Phase 140; disabled informational label |

Disabled sections have no href, click handler, write control or pretend page.
Only Overview is an implemented project destination. Future sections can be
entered independently; the strip is not a mandatory sequential workflow.

The overview contains a compact summary strip, discovery context, Attention Now
alongside applications, operational dimensions, temporal quality, recent project
events and source availability. Manual refresh/abort/focus semantics remain the
shared Console behavior.

## Summary and applications

Summaries show loaded discovered applications, active alerts, resolved/pending
closure, loaded events, loaded monitoring subjects, loaded accepted D-Maps and
reported source issues. Loaded page sizes are not total inventories. Source
sections that are unavailable/corrupt/indeterminate/not configured render their
state instead of zero. A zero **loaded reference** count is not a canonical
zero-application assertion, especially with partial discovery coverage.

Known applications are the backend's deduplicated references with multiple
provenance markers, not an "all applications" registry. The Console displays the
backend order and provenance; it does not infer applications from subject text.
The application preview is limited to 20, with explicit local/upstream truncation.

Each application presents independent dimensions:

- Accepted placement: only the matching entry in `accepted_deployments`.
  An omitted entry says "Not in loaded accepted page", not "not accepted".
- Planned/pending proposal: not included by the project response. Proposal
  provenance may be historical; it does not establish a current pending plan.
- Desired runtime: not included per application. The operational graph supplies
  a separate project-level aggregate desired-assignment count only.
- Observed runtime: not included. Temporal receipt evidence is not a realization.
- Convergence: not included. No frontend convergence algorithm is added.
- Alerts: explicitly scoped active records in the loaded alert page, not a total
  per-application count, outage count or infrastructure-health classification.

ApplicationOperationsOverview remains an existing backend contract, but is not
exposed or fetched per application in this phase. These deliberate gaps avoid
N+1 requests and invented certainty. No application detail routes are added.

## Attention, placement, temporal evidence and events

Attention Now retains backend ordering and up to eight Open/Acknowledged/InProgress
records. Resolved is pending closure and Closed is history, neither active.
Confirmation count means condition confirmations, not independent outages.
Condition duration uses the supplied episode value. The absence of active alert
records is explicitly not a positive runtime-health assertion.

Accepted D-Maps show application ID, actual map ID and revision, sourced only from
the accepted authority projection. The project operational D-Graph identity,
revision, structural node count and desired assignments appear separately. No
accepted state is reconstructed from `dmap_accepted` events or proposal provenance.
Accepted preview is limited to ten and indicates truncation.

Temporal preview (ten records) preserves canonical stream/subject identity,
explicit application scope and node context. Communication, Observation and Source
time remain orthogonal. Fresh/Stale/Behind or delayed is recent receipt of old
evidence, not a disconnected node. Stale/Stale/Aligned does not imply clock skew.
No internal stream-key parsing, combined health badge or new freshness evaluator.

Recent project events (ten) retain canonical project/application/subject identity,
kind, category, classification and occurrence time. Admission is inspectable
separately. Backend numeric occurrence ordering is preserved; recovery is history,
not a new active problem. This is not Historical Explorer or history pagination.

## Scope, unknown projects and degraded sources

The URL selects project scope and the client sends the same `X-Datum-Project`.
That is scoping, not authorization. The typed client validates consumed graph,
accepted, alert/event and temporal fields, rejects mismatched project scope in
their outer loaded records, and preserves the existing safe API error contract.
Nested correlated evidence follows the separate policy below.

No canonical project registry exists in this response. An undiscovered route can
return an empty/degraded scoped projection; it is not translated into "project
does not exist". The context note makes this limitation explicit. Backend failures
use the shared safe Error/refresh view, not an invented canonical Not Found.

Availability remains per source; useful independent sections survive degradation.
Source issues from sections and discovery coverage are collected/deduplicated
deterministically, preserving safe source/code/message. They concern read/projection
quality, not runtime failure. UI-safe messages remain bounded by the existing
512-byte contract. Indeterminate internal source strings are not displayed raw.
Discovery coverage and page continuation remain distinct. No Console pagination
promise is introduced. Cross-source atomicity is not claimed.

## Final hardening: application alert source state

Application cards expose **Alert source state** independently of the loaded active
alert-record count. Existing presentation primitives preserve Available, Not
configured, Unavailable, Integrity error (Corrupt), and Indeterminate as distinct
textual states. Only Available permits a numeric count, including a valid zero.
Not included remains reserved for dimensions intentionally absent from the project
contract (planned, per-application desired/observed runtime and convergence); it
does not replace NotConfigured, Unavailable or Indeterminate.

## Final hardening: outer scope and correlated evidence

The route and X-Datum-Project must match, and outer applications, alerts, recent
events, temporal records and accepted placements must match that requested project.
This is scoping, not authentication or authorization.

Canonical DNode correlation is intentionally global/project-agnostic. An alert
owned by Project A can legitimately reference a latest condition or recovery event
from Project B. The frontend retains that alert in A and presents the nested event
as **correlated evidence from another project**, with its own project, application
(when present), event ID, subject and occurrence time. It neither relabels the event
as A nor classifies the differing scope alone as corruption. Same-project evidence
is explicitly project-local; evidence without explicit project scope is labeled as
such without inventing a project. No internal stream-key or human-prose parsing is
used. Identity text wraps using the established classes, without fake entity routes.

Latest condition evidence appears beneath active alerts. Loaded recovery-event
references are in a secondary expandable section, explicitly **not active
attention**, with alert identity/lifecycle and the same evidence-scope treatment.
This includes resolved alerts without promoting them to active attention. The
recovery-reference preview is bounded to eight; it is not a complete timeline or
new history API. Other lifecycle metadata is not reconstructed into events.

Validation compares each typed subject's explicit project/application fields with
its own enclosing event or alert, not with a correlated alert's different scope.
Likewise project-local temporal subjects must match their own record. An internally
contradictory envelope/subject is rejected using the existing safe invalid_response
contract; the DTO is not repaired and raw values are not displayed. Global DNode
subjects carry no project field and are not forced into project-specific correlation.
No Rust, Event/Alert Store, lifecycle or Phase 127 authority was changed.

## Responsive design and accessibility

Existing DATUM branding, shell, badges and design tokens are reused. Summary cards
use four columns at wide desktop, three at 1024px and two at 768px; paired sections
stack at the existing narrow-desktop breakpoint. Project navigation wraps rather
than overflowing. Full IDs remain readable with `overflow-wrap:anywhere` and DOM
textContent, never HTML injection or destructive abbreviation.

Breadcrumb/project navigation have accessible names; Overview has aria-current;
future sections have aria-disabled and are noninteractive. Semantic headings,
textual severity/lifecycle, skip link, keyboard navigation, visible focus and global
navigation are preserved. DOM/CSS checks are not graphical-browser certification.

## Validation and review workflow

Final frontend validation: 29 lightweight tests passed, including eight project-specific
tests; 60 DOM/freeze-gate tests passed, including 13 final F1/F2 checks.
Strict TypeScript/Vite production build passed. Existing operations API tests:
nine passed. The explicit global/project-agnostic DNode correlation regression
also passed. Full DServer suite: 2233 passed, zero failed, one ignored (472.96s); Rust build
passed with existing unused/dead-code warnings. Real isolated DServer/Vite deep
links and non-discovered project API returned HTTP 200. Populated known-project
scenarios use test-only fixtures; no real populated-project or graphical-browser
verification is claimed. Git whitespace checks passed, including new files.

Lightweight tests cover provenance/accepted boundaries, scoped active records,
future ownership, structured issues, one scoped GET and consumed-field validation.
The optional existing Happy DOM gate uses the built Console bundle and real isolated
DServer/Vite APIs plus explicitly test-only populated project fixtures. It covers
identity, applications, lifecycle, independent runtime/temporal dimensions, source
degradation, event scope, unknown projects, N+1, long IDs and disabled navigation.

```powershell
# Terminal 1, repository root
cargo run --manifest-path dserver/Cargo.toml
# Terminal 2
cd console
npm run dev
```

Open `http://127.0.0.1:5173/projects`, then a discovered project, or directly
`http://127.0.0.1:5173/projects/<URL-encoded-project-id>`. Vite proxies to DServer
8080. No demo data is inserted into canonical stores. A sparse project will show
honest empty/unavailable sections rather than fabricated production data.

## Deferred findings and future ownership

Phase 130 F3–F8 remain safe to defer: application continuation wording, source issue
scope display, dot-segment route IDs, narrower canonical ID validation, partial DTO
repair, and attention helper parameterization. Only required F1/F2 were hardened.

Phase 129 F3–F8 remain deferred: active-only attention continuation, retaining the
snapshot during refresh, future enum incompatibility classification, timestamp/
signed-age accessibility refinements, typed temporal tone mapping, and scheduler
optional-counter availability. No general hardening of those contracts occurs here.

Phase 131/132 owns graph views, not this workspace shell. Phase 133A owns Artifact
& Service Catalog and 133B Guided Service Modeling (artifact/runtime, health and
monitoring policies/cadence, resources, capabilities, dependencies, versioning).
An assistant may draft; the operator reviews/saves and remains responsible.
Phase 136 owns explicit manual service-to-node intent such as P4 Sniffer → fog-01,
distinct from automatic planning. No catalog, modeling assistant, graph editor,
manual placement, acceptance/deployment controls, Reports or Historical Explorer
are implemented here.
