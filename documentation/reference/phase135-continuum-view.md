# Phase 135 — Continuum View

**Status:** implemented, unit/model-tested, not committed. Read-only. No new canonical contract, no new authority, no new persistence. Phase 135A (dashboard + visual canvas + inspector, see below) adds presentation only, on the same unchanged read model.

## Purpose

Answers five questions only: which physical D-Nodes exist; what declared structural capacity/capabilities they expose; which accepted logical D-Serv placements currently reference them; what DMonitor currently observes for those placements; and what is unknown/stale/divergent. It never answers ownership, allocation, or reservation questions — no canonical model for those exists today, and this phase does not invent one.

## Sources (three domains, never collapsed)

| Domain | Source | Authority class |
|---|---|---|
| Physical structure | `core::dnode_registry::DNodeRegistryStateV1` (via `load_state`) | STRUCTURAL AUTHORITY, GLOBAL |
| Accepted placement | `core::ddeploy::active_placement_authority`, enumerated across every currently-active `(project_id, application_id)` in `DDeployStateV1.active_by_application`/`phase113_v2.active` | PLACEMENT AUTHORITY |
| Observation | `core::dmonitor_interpretation::evaluate_application_dserv_convergence`, using `core::dmonitor_readiness::default_readiness_policy().freshness_policy` | OBSERVATION (DMonitor, ADR-0024) |

**Deliberately NOT used**, and why:

- `core::resource_observation` — an older, project-manifest/Firebase-snapshot-scoped telemetry pathway that pre-dates DMonitor/ADR-0024. Not the canonical observation authority for this view.
- `core::resource_registry` — a separate, project-scoped Stage 107 "O2" hardware/OS-identity bootstrap subsystem (confirmed unrelated by `dnode_registry`'s own doc comment). Its capacity is an opaque `capacity_profile_id` digest, never resolved to real numbers, and its project-scoping would falsely imply a specific project owns a piece of hardware.
- `core::resource_binding` — project-scoped, and its `node_id` is a free-form string never cross-checked against the D-Node registry. Not canonical placement authority.

None of the three are imported anywhere in `core::continuum_view` or `api::continuum_view`.

## Critical ownership rule

D-Nodes are GLOBAL. `core::dnode_registry`'s own module doc states this explicitly: a structural D-Node declaration is keyed only by `node_id`, never by project, and `CanonicalDContinuumV1.project_id` is a wire-identity SCOPING LABEL, not a claim that infrastructure is partitioned by project — `build_canonical_dcontinuum` stamps the SAME global node set for every project. Phase 135 preserves this exactly: `core::continuum_view::build_continuum_view`'s physical node list is identical regardless of `project_overlay_id`. A project overlay only adds which of the same global nodes that project's own accepted placements currently reference (`ProjectOverlayV1::node_ids_in_use`) — never "this node belongs to project X." The Console's project-scoped copy uses the wording "Used by this project," never "Project nodes."

## Global vs. project route

- `GET /api/v1/operations/continuum` — global, the real full physical D-Node set.
- `GET /api/v1/operations/projects/:project_id/continuum` — identical physical data, with `project_overlay` always populated for that project. Mirrors the existing `/api/v1/operations/projects/:project/dgraph-editor` path convention.

Both call the same `build_response` composition in `api::continuum_view`; the only difference is whether `project_id` is passed through as the overlay argument. A dedicated structural test (`global_route_never_filters_nodes_by_project`) asserts the handler never branches its read path on `project_id`.

## Shared-node behavior

A single physical node may legitimately carry accepted placements from multiple different projects/applications simultaneously — nothing in the canonical model forbids it, and Phase 135 does not invent a new tenancy/allocation concept to explain it. `ContinuumNodeV1::accepted_placements` and `::observations` are both lists (never a single value), proven by `shared_node_exposes_two_distinct_accepted_placements`. The Console renders this as a plain list of `project / application / service` rows — never "shared because owned by A+B."

## Physical capacity semantics

`NodeStructureV1::capacity` is DECLARED total static capacity (`CanonicalNodeCapacityV1`, unchanged from `canonical_dcontinuum`), never "available"/"free"/"allocated." No current canonical source computes those; Phase 135 does not invent them.

## Accepted placement overlay

Read-only. `AcceptedPlacementRefV1 { project_id, application_id, service_id }`, read verbatim from `active_placement_authority`'s own `placements`. No move, drag-to-place, recommend, accept, or proposal-creation action exists anywhere in this phase. Phase 136 owns Placement UI.

## DMonitor observation — kept as four separate dimensions, never one status

`NodeObservationV1` is per ACCEPTED PLACEMENT (not per node) — a node with several placements gets several independent observation entries. Each carries, copied verbatim from `dmonitor_interpretation`, never recomputed:

- `placement_convergence` (`Converged` / `Diverged` / `Unknown`)
- `admitted_evidence` — evidence DMonitor currently counts as live proof, each entry's own `freshness` / `health` / `authority_correlation` / timestamps
- `excluded_evidence` — evidence DMonitor saw but did not admit (e.g. stale, mismatched reference), each with its own `exclusion_reason`. **This was a real bug caught during implementation**: the first version of this module dropped excluded evidence entirely, silently hiding every stale/mismatched observation. Fixed by keeping admitted and excluded evidence as genuinely separate lists, each independently inspectable.

If DMonitor has no evidence for a placement at all, both lists are empty and `placement_convergence` is `Unknown` — never inferred as "offline," proven by `node_without_any_dmonitor_evidence_reports_unknown_convergence_and_no_offline_inference`. A mismatched/stale reference is also `Unknown`, never fabricated as `Diverged` — absence of CURRENT proof is not proof of drift, proven by `mismatched_authority_reference_is_unknown_never_fabricated_as_converged_or_diverged`.

`age_seconds` on each evidence entry is informational only (`generated_at_utc - observed_at_utc`), computed for display convenience — it never feeds back into deciding `freshness`, which is always DMonitor's own verbatim verdict.

## Links / topology

`CanonicalDContinuumV1` supports `links` structurally, but the one production builder (`build_canonical_dcontinuum`) hardcodes `links: Vec::new()` — there is no persisted link-declaration path anywhere today. `ContinuumViewV1::links` is present in the schema for forward compatibility and is always empty in practice (proven by `links_are_present_but_empty_today`). The Console renders an explicit "No physical topology links are currently declared." message when empty, never infers topology from canvas layout or node proximity.

## Filters

Node-id search, freshness (fresh/stale/clock skew), "has accepted placement," "has observation" — all client-side over the already-fetched response, grounded entirely in existing response fields. No capability taxonomy beyond the declared `dnode_abi_versions` set is invented.

## Bounds

`MAX_CONTINUUM_NODES = 500`, `MAX_PLACEMENTS_PER_NODE = 200`, `MAX_OBSERVATIONS_PER_NODE = 200`, `MAX_EVIDENCE_PER_OBSERVATION = 50`, `MAX_LINKS = 2000`. Exceeding the node/link bound sets `ContinuumViewV1::truncated`; exceeding a per-node bound sets that node's own `*_truncated` flag. Proven by `response_is_bounded_when_node_count_exceeds_the_limit`. No historical/timeline query exists — Phase 140 (Historical Explorer) owns that; this view is current-state only.

Every truncation flag the backend emits is surfaced to the user, never silently dropped: the page-level `truncated` flag ("This view was bounded..."), `placements_truncated`/`observations_truncated` per node ("Additional placements/observations were truncated."), and — fixed during this final review, which found the flag was validated and transmitted but never rendered — `admitted_evidence_truncated`/`excluded_evidence_truncated` per observation ("Additional evidence for this placement was truncated.").

## Visual semantics

Reuses `console/src/components/graph-visuals.ts` directly, with zero new categories or relations: D-Node cards use the existing `infrastructure` category color (blue); the "Accepted workloads" section heading uses the existing `placement` relation color; the "DMonitor observation" section heading uses the existing `observation` relation color. Freshness/health/correlation/convergence are rendered as labeled, colored **and** text badges (`status-badge`), never color alone. Every state (fresh/stale/unknown/empty) has its own textual label.

## Phase 135A — dashboard, visual canvas, and inspector

Phase 135A adds a dashboard summary and an interactive Cytoscape canvas on top of the unchanged Phase 135 read model (`core::continuum_view`/`api::continuum_view` were not touched). The accessible DOM list described above remains the guaranteed, always-present fallback — the canvas and its inspector are an additional, independent way to browse the same `ContinuumViewV1` data, never the only one.

### Dashboard summary cards

`computeContinuumMetrics(data)` (`console/src/pages/continuum.ts`) is a pure function deriving counts only from fields already present on the response: D-Node count, total accepted placements, nodes with/without observation, fresh/stale/clock-skew evidence counts (counted per evidence entry, admitted and excluded alike — freshness is DMonitor's own classification of that evidence, independent of admission), and healthy/degraded/unhealthy/unknown counts (counted from **admitted evidence only** — a conservative choice: an excluded entry's health claim is historical, not current-authority proof, so it is deliberately excluded from the current-state health tally). No available/allocated/utilization/ownership figure is computed or displayed anywhere.

### Visual continuum canvas

`continuumElements(data)` builds a Cytoscape element list reusing the existing visual vocabulary with zero new categories or relations:

- One node per D-Node, `visual_category: "infrastructure"`.
- One compact node per accepted placement reference, `visual_category: "software"` (the existing "logical software" category) — visually distinct from the D-Node it is placed on.
- One `placement`-relation edge (dashed, the existing `GRAPH_RELATIONS.placement` token) joining each placement reference to its D-Node — never D-Call (`logical`) styling, since this is not an invocation.
- Observation is rendered **only** as a border overlay (`observation_state: "fresh" | "stale" | "unknown"`) on the placement-reference node — a local, additive stylesheet (`observationOverlayStyles()`, defined in `continuum.ts`, not added to the shared `graph-visuals.ts`) that changes border color/style only, never the node's category fill or shape.
- No D-Node-to-D-Node edge is ever synthesized for layout. The only source of such an edge is a real `ContinuumLink` entry in `data.links` (always empty in production today, per the "Links / topology" section above); proven by `continuum-dashboard.test.mjs`'s `no fake D-Node-to-D-Node edge is synthesized when no links are declared`.
- Layout is deterministic `breadthfirst`. Rendering follows the exact try/catch/graceful-fallback pattern established in `project-graph.ts`: if Cytoscape or canvas 2D is unavailable, the canvas area shows "Graph renderer unavailable in this browser. The accessible list below remains fully available." and the page remains fully usable via the list.

### Node inspector

Selecting a D-Node (by clicking a canvas node, or a list card) renders a detail view with three explicit, never-collapsed headings — **Structure**, **Accepted placement**, **DMonitor observation** — produced by one shared function, `nodeDetail()`, reused identically by both the inspector panel and the accessible list card, so the two views can never drift apart in content.

### Accessible, non-canvas alternative

The full per-node card list (`continuum-node-list`) remains present below the canvas unconditionally, independent of whether the canvas rendered successfully. Every card is a keyboard-focusable `<section tabindex="0">` activated by Enter/Space, every filter input has an associated `<label>`, and selecting a card updates the same inspector the canvas uses. No operation requires the canvas or pointer dragging; nothing is draggable, and no canvas interaction mutates data.

### Project overlay wording

The project-scoped page uses only "Used by this project" (badge) and "Accepted workloads for this project" (section copy) — never "Project infrastructure," "Project-owned node," or "Project resource," preserving the Phase 135 ownership rule above.

### Rich deterministic visual fixture — integrated Console review fixture

`console/tests/continuum-visual-fixture.mjs` (TEST ONLY, not matched by `tests/*.test.mjs`; its own behavior is covered by the separate `continuum-visual-fixture.test.mjs`, which **is** part of the automated suite) serves the exact `datum.continuum-view/1` contract shape at the real production routes (`/api/v1/operations/continuum` and `/api/v1/operations/projects/:project_id/continuum`), reusing `continuum-fixture.mjs`'s deterministic three-node scenario:

- `node-a` — two accepted placements from two different projects, both admitted evidence fresh/healthy/exact_match, `placement_convergence: converged`.
- `node-b` — one accepted placement whose only evidence is **excluded** as stale (`exclusion_reason: "stale_evidence"`), so `placement_convergence` is `unknown` — a visible accepted≠currently-observed-confirmed case.
- `node-c` — no accepted placement, no observation at all (honest empty state).

Both fixture responses were confirmed, end-to-end over real HTTP, to pass the Console's own `validContinuumView` validator — the fixture is contract-true, not a frontend-only hardcoded visualization.

**Phase 135A addendum:** the same fixture process also serves minimal, honest "not configured"/empty responses for the handful of other read-only endpoints the Console shell requests while navigating during an integrated manual review session — `/api/v1/operations/overview` (read by both the Overview and Projects pages; see `console/src/main.ts`), `/api/v1/catalog/services`, and `/api/v1/catalog/artifacts`. This exists only so that navigating away from Continuum during manual review does not show misleading `fixture_not_found`/"Disconnected" noise on unrelated pages; it adds no new production endpoint or behavior, and every other route still fails explicitly with `fixture_not_found`. There is no separate "shell health/connectivity" endpoint in the real contract — the Console's connectivity badge is derived entirely from whether the active page's own request succeeds, so satisfying each page's own existing read contract is what resolves the symptom.

### Manual visual review procedure

1. `cd console && node --experimental-strip-types tests/continuum-visual-fixture.mjs` (defaults to port 8137; override with `DATUM_CONTINUUM_FIXTURE_PORT`).
2. In another shell: `cd console && DATUM_DEV_API_TARGET=http://127.0.0.1:8137 npm run dev`.
3. Open the printed dev URL, navigate to **Overview**, **Projects**, **Catalog**, **Continuum** (global), and a project's **Continuum** section (project overlay).
4. Confirm: Overview and Projects render a valid minimal/empty page (no project/event/alert counts fabricated, each shown as "not configured") with the connection badge reading **Connected**, never Disconnected/Degraded; Catalog renders its configured-but-empty state ("No definitions in the configured catalog..."); Continuum's summary cards render with the counts described above; the canvas shows 3 D-Nodes, 3 placement-reference nodes (2 attached to node-a, 1 to node-b, none to node-c), dashed placement edges, and node-a/node-b's placement nodes carry different border overlays (fresh vs. stale); clicking a node opens the inspector with Structure/Accepted placement/DMonitor observation headings; the accessible list below still shows the identical full detail; the "No physical topology links are currently declared." message appears; resizing the window to 768–900px wide makes the dashboard cards wrap, the canvas+inspector grid stack to one column, and introduces no horizontal page scrollbar; navigating between all four pages in sequence never shows a false Disconnected/Degraded flash.

### Known limitation: DOM-level automated tests

This environment does not have `happy-dom` installed (`Cannot find module 'happy-dom'`), the same limitation recorded in Phase 135. All new Phase 135A tests (`continuum-dashboard.test.mjs`) therefore test the pure, DOM-independent logic (`computeContinuumMetrics`, `continuumElements`) rather than rendered markup. No DOM-level test result is fabricated; the manual procedure above is the only current way to confirm actual rendered/responsive behavior.

## Security / multi-project

Unchanged trusted/local control-plane posture — no authentication is added. The project-scoped route takes `project_id` as a URL path segment (not a trusted client-supplied header), and the response never embeds another project's logical D-Graph/service internals — only `project_id`/`application_id`/`service_id` labels already exposed by `active_placement_authority`. This keeps a future per-project authorization boundary addable at the route layer without reshaping the response.

## Non-goals (explicit)

No placement editing, no deployment, no runtime mutation, no resource allocation, no ownership assignment, no `ProjectResourceBinding`/`ResourceAllocation`/`ServiceInstance`/`ComponentInstance`/`RequirementBinding`/`TenantBinding`, no historical explorer, no AI recommendations, no topology acceptance. Phase 136 (Placement UI) and Phase 137 (Proposal/Review/Acceptance) own everything about changing placement; this phase only reads the current accepted state.

## Zero-mutation proof

`core::continuum_view::build_continuum_view` takes only `&`-references to already-loaded state (`&DNodeRegistryStateV1`, `&DDeployStateV1`, `&DMonitorStateV1`) — there is no `&mut` parameter anywhere in its signature, so it is structurally incapable of mutating any of the three states it composes. `api::continuum_view`'s own test suite additionally greps its production source for every known mutating function name (`persist_state`, `accept_proposal`, `declare_dnode`, `ingest_observation`, etc.) and asserts none appear.
