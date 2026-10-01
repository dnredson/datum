# Phase 132 — Expandable runtime graph

Status: frozen after independent review, F1/F2 hardening and user graphical approval.

## Authority audit and selected source

Phase 131's accepted application D-Graph snapshots remain the logical source.
Phase 132 adds **admitted DMonitor DServRealization evidence**, not a runtime
instance registry. This wording is intentional: evidence includes negative and
unknown collector claims as well as successful observations. A teal vertex is
an evidence record for a canonical subject/monitor epoch, not proof that a
process is running now. DServ != realization evidence != physical resource.

Audit references and implementation boundaries:

- `core/canonical_dmonitor.rs`: typed DServRealization identity; observations
  carry health/facts, not desired state. DCodeExecution and RuntimeProcess are
  different subject kinds and are not promoted into DServRealization.
- `core/dmonitor_state.rs`: durable accepted records, globally unique
  observation IDs/digests, server receipt time, per-subject/per-stream sequence
  index. A stream is project scope + reporting node + monitor epoch. An epoch
  is not an execution-instance ID; several epochs can observe one subject.
- `core/dmonitor_interpretation.rs` and `dmonitor_readiness.rs`: exact
  application/service/node correlation; unmatched subjects exist; current
  DMap, freshness, health, convergence and readiness are independent. Phase
  132 does not reimplement those interpretation algorithms.
- `core/ddeploy.rs`, `ddeploy_v2.rs`, `operational_dgraph.rs`: accepted authority
  and desired operational composition are not runtime observations.
- `DATUM/src/agent/native_process_state.rs`, `native_process_runtime.rs`,
  `native_process_dmonitor.rs`, `operational_service_loop.rs`: governed local
  native process ownership + live identity checks produce DServRealization
  evidence. Process internals are intentionally not published in this subject.
- `DATUM/src/agent/dcode_execution_dmonitor.rs`, `dcode/invoke.rs`,
  `dcall/delivery.rs`, `bin/smartsentinel-dcode-invoke.rs`: governed invocation
  evidence can produce DServRealization observations; this does not claim a
  continuously-running process. CLI publishing is opt-in; delivery has its
  own publisher. DCall delivery itself is not service realization evidence.
- `core/runtime_kind.rs`: container/native dispatch vocabulary, not a field
  of the canonical DServRealization subject. D-Code uses fresh WASM instances
  per invocation, independently from Management WASM.
- `api/dmonitor.rs`, `api/dmonitor_convergence.rs`, `api/dmonitor_readiness.rs`:
  ingestion/read/diagnostic surfaces; receipt time is server generated.
- Phase 114 final closure, Phase 115A observation-authority audit, Phase 115
  producer closure, Phase 126 canonical event producers and Phase 127 read
  models; ADR-0021 (D-Code), ADR-0022 (DCall), ADR-0024 (DMonitor).

The earlier Phase 115A absence-of-producer conclusion is historical: Phase
115B/115D subsequently wired D-Code/native producers. Container realization
tracking remains a documented gap, not something filled from Docker names.

### Twelve audit answers

1. Runtime execution ownership exists locally for governed native processes;
   invocation evidence exists at D-Code execution. DServer's trustworthy
   read source here is admitted typed DMonitor observation evidence.
2. A durable DMonitor evidence store exists. A separate durable, globally
   addressable DServ execution-instance registry does **not** exist.
3. DServRealization has project/application/service/node IDs and optional
   DMap reference. Observation ID/digest and monitor epoch identify evidence;
   there is no process/container/instance ID in this subject. Server scope
   and receipt time are server-owned; observed identity/health are claims.
4. Multiple nodes and independent monitor epochs are supported. Multiple
   distinct processes on the same service/node cannot be individually
   identified from this subject; epochs must not be interpreted as processes.
5. Container/native dispatch and D-Code/WASM execution exist, but this subject
   has no typed runtime-kind field. The Console says Not included, rather than
   inferring kind from collector names, facts or accepted desired runtime.
6. Desired placement is DDeploy/DMap; observed evidence is DMonitor. They
   remain separate. No convergence is inferred by matching IDs in this view.
7. Yes: typed evidence can reference a service absent from the loaded accepted
   snapshot. It remains an inspectable unmatched record, never arbitrarily
   attached. Missing logical source and preview omissions also cause no match.
8. The current subject is scoped to one project/application. It cannot encode
   one shared instance serving several projects. This is a current schema
   limitation, not an architectural prohibition.
9. Shared/tenant-aware realization identity is not modeled by this subject.
10. Subject node is an observed/associated reference; reporting node identifies
    transport provenance. Neither establishes placement/reservation/ownership.
11. DMonitor proves admission, digest/sequence integrity and server receipt;
    it records a collector's observation. It does not prove desired state,
    current health, exclusive resource use or convergence by itself.
12. Native and invocation producer wiring gaps were addressed in Phase 115.
    Container ownership tracking, distinct same-node instances, runtime-kind
    identity and shared tenancy remain gaps. Phase 132 invents none of them.

## Read projection / API

`GET /api/v1/operations/projects/:project_id/runtime-graph` returns a separate
envelope: project ID, the unchanged logical graph projection, runtime source
kind `admitted_dserv_realization_evidence`, runtime SourceSection and temporal
quality for each included observation. `/graph` retains its Phase 131 contract.

The HTTP adapter delegates to OperationsSources through Phase 128's owned
capacity permit / spawn_blocking boundary. The same validated read-only
DDeploy/DMonitor acquisition is reused, including full integrity preflight.
No ingestion, acceptance, reconciliation or schema initialization is added.
Route identity and X-Datum-Project must match; this is scope, **not auth**.

Latest sequence is selected per exact typed subject AND independent stream,
not merely the stream head. Stable order is application, service, subject node,
reporting node, monitor epoch. An opaque JSON tuple names each presentation
vertex without parsing internal keys. Observation identity/digest remain
inspectable. Logical association is exact application/service identity only,
relative to loaded accepted logical snapshots. DMap reference is retained as
observed provenance, not a current-deployment/convergence verdict.

Runtime output is capped at **1,000 latest evidence records**, a modest
additional bound alongside the existing 2,000 logical vertices / 4,000 DCalls.
Whole service evidence sets are included atomically in deterministic order;
processing stops before the first set exceeding the remaining budget. Total,
included and omitted counts are source-relative. An oversized first service
is Partial with zero included, not an empty runtime universe. Later small
services are not selected as an optimization. Frontend validates bounds and
scope without inventing defaults. Full stored integrity preflight is not
weakened to optimize projection latency.

## Interaction, presentation and accessibility

Graph remains at `/projects/:projectId/graph`; Runtime navigation stays planned
for Phase 139. One bounded GET loads logical structure and evidence; no
per-application/service fan-out. Evidence is collapsed by default. Accessible
DServ buttons expand/collapse local evidence with aria-expanded. DCalls remain
between logical DServ nodes; a separate realization relation means logical
service → admitted realization evidence, never placement.

Evidence node/relation selection shows full typed subject, observation ID,
digest, epoch, collector, node references, observed/received times, original
collector health claim and observed DMap reference. Runtime kind/process/
container/artifact identity is explicitly Not included. Arbitrary facts/JSON
and internal diagnostics are not displayed. Unmatched records are listed
separately and inspectable even when the logical source is absent.

Application filtering is local and clears expansion/selection. Reset collapses
all evidence, clears selection and restores deterministic layout. Dragging is
presentation-only and never persisted. Fit changes viewport only. No expand-all
control is added. Expanded service-local vertical evidence blocks enlarge the
existing local application grid; measured parent bounds, labels and gutters
are then packed into non-overlapping outer rows. Collapsed layout remains the
Phase 131 layout. Edge direction/cycles/self/parallel calls do not affect it.

Shared `datum-graph-visual-semantics.md` tokens are reused: software purple,
runtime teal, neutral application containers, logical DCall and realization
edge identities. Only visible categories/relations appear in the legend.
Collector health is a secondary textual historical claim, not a fill color or
new status overlay. No absence-of-alert or recent-receipt health inference.

Communication freshness, observation freshness and source time quality reuse
Phase 127's server interpretation independently. Full source observation and
server receipt timestamps remain visible. Fresh communication + stale
observation + delayed source does not mean disconnected runtime.

Canvas is not claimed fully accessible. The keyboard-accessible entity and
relationship list exposes expansion and details; it remains usable if canvas
creation fails. Existing semantic headers, focus styles, wrapping full IDs,
1440/1024/768 layout and stacked details are preserved. No new dependencies.

Available-empty, Partial bounded-empty, NotConfigured, Unavailable, Corrupt
and Indeterminate remain distinct. Logical and runtime sections degrade
independently; unavailable evidence is never displayed as zero realizations.
Fields intentionally absent from the subject are Not included, not a source
failure or unconfigured source.

## Multi-project compatibility and phase boundaries

Logical graph and associated node references do not imply physical ownership.
Global DNodes, project bindings, reservations, shared service instances and
tenant-aware software remain future-compatible concepts. A shared software
service remains software/runtime; tenancy is a separate future marker.

No infrastructure vertices or physical links (Phase 135), placement edges or
manual assignments (136), catalog/modeling (133A/B), graph editing (134),
deploy/reconcile controls (138), or full runtime/monitoring workspace (139).
No graph mutation/persistence and no new operational authority.

## Review workflow

Terminal 1, repository root: `cargo run --manifest-path dserver/Cargo.toml`.
Terminal 2, `console/`: `npm run dev`. Existing proxy targets DServer port 8080;
open `http://localhost:5173/projects/<encoded-project-id>/graph`.

Real admitted evidence is needed for populated expansion. Tests never insert
fake canonical state. A manual development-only fixture can be mounted from
the browser console (not imported by production application code):

```js
const { runtimeVisualReviewFixture } = await import('/tests/runtime-graph-fixture.mjs');
const { projectGraphView } = await import('/src/pages/project-graph.ts');
const fixture = runtimeVisualReviewFixture();
const review = projectGraphView(fixture.logical, fixture);
document.querySelector('#page').replaceChildren(review.element);
review.mount();
// After review: review.dispose(); reload the page to restore real API data.
```

Validation: `npm test`; `node --experimental-strip-types --test
tests/runtime-graph.test.mjs`; production `npm run build`; optional external
Happy DOM module via DATUM_DOM_GATE_MODULE for `tests/freeze-gate.mjs` with
isolated DServer/Vite running. Backend focused `cargo test --manifest-path
dserver/Cargo.toml --bin DServer runtime_graph`, operations API regression,
full DServer test/build and git diff check. Graphical visual certification is
separate from headless bounding-box/DOM proof.

Previously deferred Phase 129/130 findings and Phase 131 application-group
degree/stricter duplicate validation remain deferred, not expanded here.

## Validation evidence

Visual semantics baseline: `e99cfad0c4696f5daeef8b220285c73789861b43`, pushed
with local/tracking/direct remote equality and clean tree before Phase 132.

- Frontend lightweight suite: 77 passed, including 13 runtime-specific tests
  and five new headless runtime compound-layout regressions.
- Bundled DOM/freeze suite: 79 passed with real isolated DServer/Vite running;
  populated, partial, unmatched and unavailable runtime states use test-only
  response fixtures. This is DOM verification, not graphical rendering.
- Focused runtime projection/API suite: 10 passed; operations API suite: 13
  passed. Full DServer: 2,254 passed, zero failed, one pre-existing ignored.
- Strict TypeScript/Vite build and Rust build passed. Existing Rust warning
  totals remain 45 for tests / 205 for DServer build. Vite warns about the
  508.83 kB minified main chunk (159.12 kB gzip); no threshold was hidden.
- npm audit: zero vulnerabilities; no dependency/lockfile changes in Phase 132.
- Real proxy smoke GET for `foundation-demo/runtime-graph`: HTTP 200, matching
  project scope, accepted logical source and admitted realization evidence
  source explicitly NotConfigured in this isolated empty environment.
- Browser capability inventory exposed no apps/browsers. No graphical visual
  certification is claimed. Manual populated review instructions are above.

Phase 132 passed independent review and final hardening.

## Final F1/F2 hardening

Expansion/collapse rebuilds the accessible list and then restores focus to the
replacement button by its application-qualified service presentation ID
(`data-service-id`), using `preventScroll`. It does not use label text or list
order. Filter and Reset keep their existing separate semantics.

The narrow client guard rejects duplicate canonical observation IDs, including
duplicates across different node/epoch/application/service tuples. It never
deduplicates or repairs a response: the existing safe `invalid_response` path
rejects it before any graph is rendered. Temporal rows are validated against
their corresponding evidence records; selection uses the now-unique validated
observation ID. Regression tests select evidence with different timestamps and
independent freshness/source-time values to verify exact association.

The explicit manual fixture above includes uneven application groups, two
evidence children per service, reciprocal/parallel/self DCalls and one unmatched
evidence record. It does not write DServer state or activate in production.
Inspect at 1440/1024: expansion/collapse focus, purple logical vs teal evidence,
realization edges, unchanged DCalls, details/temporal metadata, unmatched evidence,
filtering, Fit and Reset. No infrastructure vertices should appear.

F3 (20-character canvas label prefix) and F4 (acquisition/intermediate cost beyond
the output bound) remain explicitly deferred. No backend authority changes were
made by this hardening pass. No graphical browser was exposed to the agent;
headless DOM/layout checks are not graphical certification. User visual approval
was subsequently supplied by the user: populated graphical review PASSED
("ficou muito bom"). The agent does not claim its headless checks were graphical.
The user approved expand/collapse, evidence presentation and the populated graph.

Hardening validation: 85 lightweight tests (21 runtime-specific), 82 built-app
DOM/freeze tests, including three F1/F2 regressions, and strict frontend build
passed. Runtime projection/API: 10 passed; operations API: 13 passed; full
DServer: 2,254 passed, zero failed, one ignored. Rust build passed; npm audit
reported zero vulnerabilities. Five runtime headless layout regressions and
existing logical layout/visual-token tests remain green. Existing Rust warnings
and the Vite >500 kB chunk warning remain; no dependency changes were made.
