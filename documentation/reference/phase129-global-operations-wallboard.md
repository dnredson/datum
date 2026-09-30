# Phase 129 — Global Operations Wallboard

Status: frozen after independent review and required F1/F2 hardening. Phase 128's
foundation was frozen after user graphical approval. This phase is read-only;
it does not implement the Phase 130 Project Workspace.

## Purpose and architecture

The wallboard answers what requires attention, which scope/subject is involved,
which operational dimension is represented, and where an operator can navigate.
It is a NOC/control-plane projection, not a generic KPI dashboard or health score.
Existing DATUM assets, Bootstrap/TypeScript/Vite and Phase 128 styles are preserved.
No dependencies, charts, graph library, write routes or frontend framework are added.

```text
Console / — compact wallboard
        |
        | one GET /api/v1/operations/overview
        v
Phase 128 bounded spawn_blocking API adapter
        |
        v
Phase 127 GlobalOperationsOverview
        |
        +-- Event/Alert read transaction
        +-- independently validated D-Monitor / deployment / graph / node snapshots
        +-- scheduler projection (currently unwired)
```

No backend change is needed: the existing canonical read-model response already
contains ordered alert details, recent events, temporal evidence, discovery and
scheduler metadata. TypeScript consumes a narrow validated subset; it does not
rebuild subsystem joins, reevaluate freshness or infer authority from history.
Project details are requested only after navigation, avoiding initial N+1 calls.
Route/header project scope remains scoping, not authorization.

## Information hierarchy and bounds

1. Compact summary strip: managed operational-document projects, known application
   references loaded, active alerts, resolved/pending closure, 24-hour recent event
   count, reported source issues, and monitoring output-loss counters when available.
2. Attention Now: up to eight active alert-backed conditions in Phase 127 attention
   order, with severity, lifecycle, scope, canonical subject, first occurrence,
   condition duration, latest evidence ID/time and typed temporal evidence.
3. Known projects (ten) beside infrastructure/continuum summary (six observed
   stream references). Project clicks open the existing read-only project shell.
4. Temporal Quality: ten stream records with three separate dimensions.
5. Recent Operational Events (fifteen) beside observability/source and scheduler
   state. This is a bounded preview, not a history browser.

Every preview reports truncation when upstream `has_more` or local display bounds
apply. Existing APIs retain a 100-item page and Phase 127's maximum 1000 internal
query bound. No raw cursor or unbounded history endpoint is introduced.

## Attention, events and lifecycle

**ALERT** is persistent condition/lifecycle state. Open, Acknowledged and InProgress
are operationally active. Resolved means pending explicit closure; Closed is
historical. Neither enters Attention Now. Backend attention ordering is retained;
the Console does not invent a second priority/health policy. A temporal condition
already represented by an alert is not emitted again as an independent attention
item. Temporal lists show evidence, not competing condition records.

Condition confirmations are not outage/incident counts. Duration comes directly
from the Phase 127 episode projection, not a browser-inferred outage interval.
An unavailable/negative duration is indeterminate, not zero. Temporal receipt/source
ages shown beside an alert refer to its evidence evaluation, not a new browser
freshness calculation.

**EVENT** is an immutable occurrence. The recent preview preserves Phase 127's
newest-first numeric occurrence chronology and event-ID tie-break; it does not
resort RFC3339 strings or use admission order. Occurrence time is visible; event
ID, source and admission time remain inspectable in the row title. Recovery and
control-plane classifications remain distinct. A recovery does not create new
active attention; D-Map acceptance remains historical-only by this presentation.

**SOURCE ISSUE** concerns observation/read/projection quality, not infrastructure
availability. Safe Phase 127 codes/messages appear separately in expandable
projection notices. NotConfigured and scheduler Unwired are neutral, not critical
failures. Corrupt/Unavailable/Indeterminate sections never become fabricated zero.
Independent available sections remain renderable; cross-source atomicity is not
claimed. Console connection states describe connectivity/projection quality only.

## Projects, infrastructure and discovery

Known projects/applications are discovered references, not canonical registries.
Loaded application counts are explicitly bounded-page counts, with discovery
coverage alongside them. `has_more=false` does not imply complete source coverage.
Empty partial discovery says no projects are currently discoverable from available
sources, not that no projects exist. Provenance and coverage remain Phase 127 data.

Per-project rows use loaded application references, canonical project alert counts
when present in the bounded project-attention page, and loaded D-Monitor streams.
Absent alert summaries with truncated upstream coverage are explicitly unknown.
Placement/runtime per-project summaries are not supplied by the global contract;
the UI says so rather than manufacturing availability or health. No per-project
detail fetch is performed to fill the gap. Planned, accepted, desired, observed,
converged and alert lifecycle remain distinct; richer summaries belong to Phase 130.

Structural D-Node count comes from the node registry projection. Desired service
assignments come from operational deployment documents, not observed runtime.
Infrastructure rows are observed stream references and may repeat a node across
streams/projects. They are not inventory, online state or actual service counts.
No continuum topology or manual placement UI is implemented.

## Temporal quality

Canonical subjects retain their complete typed entity identity, without parsing
internal stream keys. Temporal and observed-stream rows show the canonical stream
subject plus project, node and explicitly supplied application context. Recent
events retain application scope; D-Map and deployment-proposal subjects display
their actual `dmap_id` and `proposal_id`. Full identifiers wrap and remain readable;
only implemented project destinations are linked.

Project continuation distinguishes local preview truncation from upstream paging:
when more than ten references are loaded, Projects shows the remaining **loaded**
references. Upstream `has_more` instead reports identities beyond the loaded page;
Console pagination is not exposed and Projects does not promise access to them.
Discovery coverage remains independent of both forms of pagination.

Each stream retains **Communication**, **Observation**, **Source time**:

- recent receipt + old source observation: Fresh / Stale / Behind or delayed;
- equally old observation and receipt: Stale / Stale / Aligned, no clock discrepancy;
- excessive source lead: source ahead/skew, communication still independent.

All classifications come from Phase 127/D-Monitor. Behind/delayed can reflect
buffering, queueing, transport or source time; it is not exact NTP offset. An old
source timestamp does not imply Down, Offline or Unreachable. The Console does
not add a combined health badge, new threshold evaluator or health authority.

## Scheduler and refresh

Configured state, driver activity and owner lifecycle are separate. An unwired
scheduler is explicitly not configured. A Running owner does not prove a live
background driver. Full and closed output-drop counters are displayed separately;
unavailable counters are not zero, and reported loss gets an observability warning.
These transient counters do not certify durable monitoring delivery.

Refresh remains manual, intentionally. There are no polling timers, SSE or sockets.
The refresh button is disabled during a request, and its handler also ignores
programmatic duplicate clicks. Navigation aborts obsolete requests; late results
cannot replace the new route. Existing fetch timeouts/abort cleanup remain intact.

## Responsive design and accessibility

The summary strip uses seven columns at wide desktop, three at 1024px and two at
768px. Project/infrastructure and event/source panels stack below 1200px. Shared
min-width, wrapping, grid and flex rules keep long identities inspectable. There
is no horizontally scrolling giant alert table. Severity/lifecycle use text, not
color alone; semantic sections/headings/time elements and project links remain
keyboard accessible. Phase 128 skip link, focus management and tablet inert drawer
are retained. Graphical layout review remains distinct from DOM/CSS checks.

## Validation and local smoke

Final freeze gate: 21 lightweight frontend tests passed (14 wallboard tests),
31 DOM/freeze-gate tests passed, strict TypeScript/Vite production build passed,
and full DServer validation passed with 2233 tests, zero failures and one ignored.
Rust build passed; existing unused/dead-code warnings remain. F1 tests cover
same-node distinct streams, explicit/absent application scope, distinct D-Map and
proposal IDs, complete wrapping identities and project-only links. F2 covers
6/15 loaded references without upstream continuation, 10/15 with continuation,
and Partial coverage with/without continuation, including Projects showing all
currently loaded references. No graphical certification is added by this gate.

The regular Node runner adds wallboard selection/bounds/duration/identity and
expanded-client contract tests. The optional Happy DOM gate adds fifteen wallboard
checks covering available/unavailable summaries, active vs resolved attention,
coverage, temporal distinctions, source issues, bounded events, degraded rendering,
navigation without N+1 and nonoverlapping manual refresh. Response fixtures are
test-only, imported by no production module, and do not modify canonical stores.

Local HTTP smoke uses the existing DServer/Vite on 8080/5173 without changing user
processes or authority data. Six GET samples: first request about 216ms; five warm
requests averaged about 25ms (14–34ms), 3081-byte degraded overview. These are
client-observed projection/API/proxy timings on sparse local data, not a production
SLO or load test. `/`, `/projects`, a discovered project deep link and unknown-route
SPA fallback returned 200. DOM uses the actual production bundle and real API;
populated attention/events/temporal scenarios use explicit test fixtures.
No graphical browser was available to this agent; no screenshot, native Tab order,
rendered-overflow or native Back/Forward certification is claimed.

Run DServer and `console/npm run dev` as documented in Phase 128; open
http://127.0.0.1:5173/. Rust/read models remain canonical. Filter/auth-bound
cursors, projection performance work and authentication remain separate API/deployment
concerns. No backend/schema/index or production static-serving change occurs here.

## Updated roadmap and future ownership

Deferred independent-review findings (not part of this hardening): F3 attention
continuation uses lifecycle-wide `has_more`; F4 refresh temporarily replaces the
snapshot with Loading; F5 future contract/enum mismatches may appear Disconnected
rather than Degraded; F6 tooltip-only timestamps and signed future evidence ages;
F7 temporal badge tones depend on display strings; F8 a future configured scheduler
with null loss counters may be presented as NotConfigured. These are deferred,
not claims of implementation or new authority.

| Phase | Ownership |
| --- | --- |
| 128 | Console foundation |
| 129 | Global overview / wallboard |
| 130 | Project workspace |
| 131 | Graph overview |
| 132 | Expandable runtime graph |
| 133A | Artifact & Service Catalog |
| 133B | Guided service modeling |
| 134 | Visual DGraph editor |
| 135 | Continuum view |
| 136 | Placement UI, including explicit manual service → node assignment |
| 137 | Proposal / review / acceptance |
| 138 | Deploy / reconciliation |
| 139 | Runtime / monitoring |
| 140 | Historical Explorer |
| 141 | Operational Reports / Context Engine |
| 142 | AI Context Contract |
| 143 | MCP / Tooling Contract |
| 144 | GNN |
| 145 | DATUM-specialist LLM |
| 146 | Closed-loop AI orchestration |

133A/B eventually cover reusable components and container/native/WASM/D-Code
artifacts, runtime configuration, artifact/image references, commands/arguments,
environment, ports, volumes, capabilities, resource requirements, health checks,
monitoring definitions/cadence, dependencies, deployment behavior, revisions,
templates and guided/chatbot-assisted modeling. The assistant helps draft; the
operator reviews/saves, and the assistant never becomes canonical authority.

Phase 136 explicit intent such as “place P4 Sniffer on fog-01” remains distinct
from automatic planner selection. Phase 140 owns metrics/ranges/granularity/units,
multiple series and event/alert/deployment/temporal overlays. Phase 141 owns
deterministic reports/consolidated context for humans and future LLM consumers.
None is implemented by this phase; no fake history/report/catalog controls appear.
