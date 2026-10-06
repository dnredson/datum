# Phase 141B — Application Operational Context Foundation

**Status:** implemented, unit/integration-tested, **not committed, not pushed**. Read-only. No new context engine, no LLM/GNN/MCP, no remediation, no new authority.

## Purpose

Phase 141A's audit found that the application-level operational context Phase 141 needed was **already built** — `core::operations_read_models::OperationsSources::application_overview`, fully tested, reusing every canonical read model Phase 127-140 already established — but had no HTTP route and no Console consumption. Phase 141B closes exactly that gap, plus the two narrow additions 141A identified as genuinely missing: canonical DMonitor **readiness** alongside the existing **convergence** section, and a small, bounded **recent-history pointer**. Nothing else was built from scratch.

## Route

`GET /api/v1/operations/projects/:project_id/applications/:application_id`, registered in `api::operations::router` alongside the existing `project_overview` route, with the identical validation/header-scoping convention (`project_id` must match the `X-Datum-Project` header exactly, both project and application ids are bounded/control-character-free). The handler (`application_overview`) performs **no assembly logic of its own** — it is a thin delegation to `OperationsSources::application_overview(...)`, the same already-built, already-tested method Phase 141A found.

## What the existing `ApplicationOperationsOverview` already contained (Part D — audited, not duplicated)

`canonical_application_registry`, `accepted` (current placement), `planned` (pending proposals), `desired_runtime`, `observed_runtime`, `temporal_quality`, `convergence`, `reconciliation_execution` (placeholder, not yet wired — pre-existing, untouched), `alerts`, `recent_events` — all already `SourceSection`-wrapped and bounded. Phase 141B added exactly two new fields: `readiness` and `recent_history`.

## Readiness addition

`readiness: SourceSection<ApplicationReadinessView>` reuses `core::dmonitor_readiness::evaluate_application_readiness` **verbatim** — no readiness logic was re-derived in `operations_read_models.rs`. It uses the engine's own canonical, server-owned `default_readiness_policy()` (90s/90s/10s), **not** whatever looser freshness policy (`DMonitorFreshnessPolicyV1::default()`, 300s/300s/30s) the pre-existing `convergence` section happens to be using — this is intentional: readiness must match the dedicated `GET /api/v1/dmonitor/readiness/:p/:a` endpoint bit-for-bit (proven by `readiness_reuses_the_canonical_engine_exactly_as_the_dedicated_endpoint_would`), not whatever policy the unrelated pre-existing convergence section was configured with. One new source was added to `OperationsSources` to make this possible: `diot_runtime_binding: SourceSection<DIoTRuntimeBindingRegistryStateV1>`, loaded the same way `nodes`/`deployment`/`dmonitor` already are (a new domain read, `diot_runtime_binding_registry`, added to the existing batch domain fetch — no new I/O round trip, no new lock beyond what the dedicated readiness endpoint itself already acquires).

Readiness is fail-closed, exactly two states (`Ready`/`NotReady`) — no third aggregate, no `Healthy`/`Warning`/`Critical`/traffic-light state was added anywhere, Console included (structurally proven).

## Recent-history pointer addition

`recent_history: SourceSection<RecentHistoryPointerView>` — `{ recent_observation_count, latest_received_at_utc, latest_observation_id }`. This is **evidence only**, never a historical convergence/readiness reconstruction: it carries no `was_converged_at_time` field, no comparison against any authority, and no raw collector `facts`.

**Subject-kind scope (independent review Audit 21, clarified explicitly):** only subjects carrying `project_id`+`application_id` count toward `recent_observation_count` — `DServRealization`, `DCodeExecution`, `DCallDelivery`, `DIoTRealization` (via the pre-existing `subject_scope` helper `observed_runtime` already used). `DNode`, `RuntimeProcess`, `NetworkReachability`, and `Unbound` subjects carry no application correlation and are never counted — this is not new behavior, it is the same filter `observed_runtime` has always applied, confirmed by reading `subject_scope`'s own match arms rather than assumed.

Crucially, it costs **nothing extra**: it is derived from the exact same `DMonitorStateV1` scan `observed_runtime` already performs (same `subject_index` walk, same project/application match), computed from the same in-memory `records` collection before that collection is paginated for `observed_runtime`. There is no second `core::dmonitor_history::query_dmonitor_history` call and no second state load — Phase 140's own O(total-history) read model is not invoked here at all, by construction (proven by `recent_history_pointer_reflects_the_same_observations_as_observed_runtime_never_a_second_scan`).

## Command vs. observation vs. convergence vs. readiness (Part E)

All four stay structurally separate fields on the response: `reconciliation_execution` (command-side, still a placeholder — unrelated pre-existing TODO, not touched), `observed_runtime` (current DMonitor evidence), `convergence` (is the placement currently correct), `readiness` (fail-closed governance gate). None is derived from another; none is merged into a combined status anywhere, Console included.

## Alerts and events (Parts F/G)

Both reuse the **existing**, already-wired `alert_section`/`recent_events` composition inside `operations_read_models.rs`, which reads the real, canonical `datum_alerts`/`datum_events` SQL tables (Phase 122/124's `DatumEventV1`/`DatumAlertV1`) — **not** the legacy Firebase-backed `/api/v1/events/latest` path, and not a fabricated alert derived from convergence findings. No new alert/event logic was written. Under normal current operation these sections may show near-zero counts (no producer is yet wired to populate those tables in volume) — that is an honest reflection of the system's current state, not a defect introduced here.

## Legacy analysis-context isolation (Part H/AN)

`core::operational_analysis_context`/`core::active_ai_context`/`core::ai_gnn_data_contract` (the Phase 67-era `/api/v1/analysis-context/*` endpoints) were **not touched, not deleted, not called**. Phase 141B has zero dependency on them or on their own observed-state half's legacy Firebase pathway (`state.firebase.get_latest_snapshot`). They remain legacy/pre-canonical, not the source for this or any future Phase 141 work; a future phase should consider deprecating them separately.

## Structural semantics (Part I)

The overview is **derived, read-only, non-authoritative, and non-atomic across sources** — document and type comments say so explicitly. `generated_at_utc` is one snapshot-level timestamp; it is never used in place of each section's own `evaluated_at_utc` (convergence, readiness), `observed_at_utc`/`received_at_utc` (observed runtime, recent history), or `occurred_at_utc`/`admitted_at_utc` (events) — every component timestamp is preserved verbatim.

## Partial data (Part J)

The existing `SourceSection<T>` enum (`Available | NotConfigured | Unavailable | Corrupt | Indeterminate`) is reused exactly as-is for both new fields — no second `available: boolean` mechanism was introduced, Rust or Console side.

## Boundedness (Part K)

Every list-shaped section (`planned`, `desired_runtime`, `observed_runtime`, `temporal_quality`, `recent_events`, findings on both convergence and readiness) already used, and continues to use, the existing `BoundedPage<T>`/`has_more` convention. `recent_history` itself is a fixed-shape summary (count + one latest pointer), never an unbounded list.

## Determinism (Part L)

`recent_history`'s `latest_*` fields are derived from the SAME sorted (`received_at_utc` then `observation_id`, matching Phase 140A's own tie-break convention) collection `observed_runtime` already produces — proven deterministic by `recent_history_pointer_reflects_the_same_observations_as_observed_runtime_never_a_second_scan` and the pre-existing `BTreeMap`-keyed iteration the rest of this module already relies on.

## Performance (Part M)

No accidental O(total-history) scan: `recent_history` reuses the already-loaded `DMonitorStateV1` in memory; `readiness` reuses the already-loaded `authority`/`dmonitor`/`nodes` sources plus one new `diot_runtime_binding` domain read (added to the existing single batched domain fetch, not a separate round trip).

## No persistence, no cache (Parts N/O)

No new table, no new cache. Both new sections are derived fresh on every call, exactly like every other section in this response.

## Console (Parts P-T)

- `api/application-overview.ts` — `ApplicationOverviewClient`, single `get(applicationId)` method, structurally proven read-only.
- `types/application-overview.ts` — mirrors the Rust response field-for-field, reusing `SourceSection`/`Page`/`AlertSection`/`OperationalEvent`/`TemporalQuality` from the existing `types/operations.ts`, and the existing `DMonitorConvergence`/`ConvergenceFinding`/`DMonitorSubject`/`DMonitorHealth` primitives rather than redefining them.
- `pages/application-overview-model.ts` — validation (closed-set enums, permissive-but-correct timestamp spelling given this response mixes a `canonical_at()`-generated `Z`-only clock domain with DMonitor's own `+00:00`-producing ingestion clock domain) and display mappings only; no authority logic, no convergence/readiness recomputation.
- `pages/application-overview.ts` — the presentation surface, reached at `/projects/:projectId/applications/:applicationId` (distinct from `/projects/:projectId/applications/:applicationId/deploy`), linked from each known-application row in the existing project workspace page (several of that page's own "unavailable dimension" placeholders — observed runtime, convergence — are exactly what this new page now actually provides). Readiness and convergence render as two visually separate sections; health/freshness badges are reused verbatim from the existing DMonitor history model, never recomputed.

## Drill-down links (Part AC)

Placement, Deploy/Reconciliation, Continuum, and History — all via the existing router path builders. No new alert page exists yet, so none is linked. The History link is a **plain navigation link** to the project's own History page; Phase 140 does not persist filter state in the URL, so this page does not fabricate an unsupported `?application=...` query parameter — it links to the project-scoped History page and separately displays the application id as context, exactly as Phase 141A recommended.

## No AI, no remediation, no new authority (Parts AK-AM)

Structurally proven: no LLM/GNN/MCP/OpenAI reference anywhere in the new files; no mutating client method exists to call; no endpoint accepts this response back as desired state.

## Manual acceptance scenarios (Part AS) — live-verified during independent review

The implementation pass noted this had not been separately re-verified live. The independent review performed it: a temporary real-backend probe (same pattern as Phases 140A/140B — a real axum `router()`, real `OperationsSources` built from real `DDeployStateV1`/`DMonitorStateV1`/accepted-proposal fixtures, real HTTP via `reqwest`) exercised, then removed:

- **Accepted app, no deployment yet:** `accepted` correctly `Available(null)`; `readiness` correctly `not_ready` with finding `no_active_placement_authority`.
- **Converged + Ready:** with a fresh, healthy, exact-`dmap_ref`-matching observation, `convergence` is `converged` and `readiness` is `ready` with finding `ready` ("every required DServ and DIoT placement is Converged with admitted current Healthy evidence") — confirming readiness parity with the dedicated endpoint is not just asserted by a unit test but observable over real HTTP.
- **Shared node, two applications:** a second application (`app-nodeploy`) sharing the exact same physical node as the first (`app-ready`) correctly shows zero observations and a zero-count history pointer — no cross-application leakage despite the shared node.
- **Partial source unavailable (DMonitor down):** the endpoint still returns `200 OK`; `readiness` and `recent_history` (both DMonitor-dependent) correctly show `unavailable` with a safe, non-leaking error envelope, while `accepted` (DMonitor-independent) remains `available` — confirming partial failure never collapses to a whole-request error.

A Vite dev server was additionally started against the real served module (no backend needed for this check) to confirm `/projects/:id/applications/:id` serves 200 and the transformed module genuinely contains `ApplicationOverviewClient`/`applicationOverviewView`/`READINESS_LABEL` — real wiring, not just source-file presence.

The remaining scenarios (unhealthy-but-converged, Unknown convergence, stale, ClockSkew, failed-command-but-observed, active alert, recent events) are each already proven generically by the pre-existing `dmonitor_interpretation`/`dmonitor_readiness` test suites this module reuses unchanged (Phase 139's own closure already established these); re-deriving them again at this composition layer was judged unnecessary — the automated tests below already confirm the composition layer passes the right inputs through.

| Scenario | Covered by |
|---|---|
| A. Accepted app, no deployment yet | `readiness_is_notready_with_no_accepted_authority_never_fabricated_ready` |
| C. Converged + Ready | `readiness_reuses_the_canonical_engine_exactly_as_the_dedicated_endpoint_would`, `readiness_is_a_structurally_separate_section_from_convergence` |
| L. Partial source unavailable | `readiness_section_reflects_dmonitor_unavailability_like_every_other_dmonitor_backed_section`, `acquisition_failure_is_unavailable_not_unconfigured` (pre-existing) |
| K. Shared node with another application | `application_overview_isolates_project_and_application_scope_for_the_new_sections_too` |
| (history pointer honesty) | `recent_history_pointer_is_honestly_empty_with_zero_matching_observations`, `recent_history_pointer_never_carries_raw_facts_or_a_convergence_field` |
| (route wiring) | `application_overview_scoping_and_encoded_identity_are_preserved`, `application_overview_wires_readiness_and_recent_history_sections_delegating_to_the_existing_builder`, `application_overview_invalid_path_is_safe` |

Scenarios B, D-J (waiting observation, unhealthy-but-converged, unknown convergence, stale, ClockSkew, failed-command-but-observed, active alert, recent events) are each already proven generically by the pre-existing `dmonitor_interpretation`/`dmonitor_readiness` test suites this module reuses unchanged (Phase 139's own closure already established these); Phase 141B does not need to re-prove them at this composition layer, only that the composition layer passes the right inputs through — which the 9 new core tests directly confirm.

## Known limitations carried forward

- `reconciliation_execution` remains an unwired placeholder (pre-existing, not part of 141B's scope).
- Alert/event sections may show near-zero data until a producer is wired to the canonical `datum_alerts`/`datum_events` tables in volume (Phase 141A's own finding, unchanged).
- No URL-persisted filter/drill-down state (Phase 140's own documented limitation, unchanged).
- The pre-existing `placement-model.ts`/`reconciliation-model.ts` timestamp-validator debt (Phase 140C's own finding) is unrelated to and untouched by this phase.
