# Phase 115A — Production D-Serv Observation Authority Audit

**Status:** closed (audit-only; no production code was changed).
**Date (UTC):** 2026-09-25.
**Branch audited:** `agent/dmonitor-v1` at `9baf96d6e155f9781235a5f1077d6e1220213bc7` (identical to `main` at the time of this audit).
**Nature of this document:** a read-only investigation record, not an implementation. It answers one question with evidence, and proposes (without implementing) what a follow-on phase should do.

---

## 1. Executive conclusion

**Central question:** does DATUM currently possess a production surface with sufficient governed canonical identity (`project_id`, `application_id`, `service_id`, `node_id`, current D-Map identity) to honestly and fail-closedly emit a `CanonicalDMonitorObservation` with `subject = DServRealization`?

**Answer: NO — proven absent, not merely undiscovered.**

Every plausible production surface in both crates (`DATUM` and `dserver`) was searched for a non-test construction of `DMonitorSubjectV1::DServRealization { .. }`. There are zero. Every single construction site of that literal in the entire repository is inside `#[cfg(test)]` modules, golden-vector tests, or a documented integration-proof fixture. This is independently corroborated by:

- A direct, repository-wide grep of both crates' source trees (Section 7).
- Three independent deep-dive investigations (runtime executors; reconciliation/ServiceArtifact/operational-graph; DNode-local runtime + SmartSentinel collectors + persistence) that each separately concluded no production path carries the full identity tuple through to an actual DMonitor emission call.
- The project's own pre-existing documentation: ADR-0024 already contains a section titled *"Exact limitation: canonical D-Serv evidence has no production emitter yet"* and states plainly that "producing real `DServRealization` evidence is a future collector's job, not 114C's" (ADR-0024, lines 725–732, 1457–1467). A Phase 114D1 test comment (`dmonitor_readiness.rs:1015–1018`) makes the identical claim in code, predating this audit.

This audit does not merely repeat that prior claim — it independently re-derived it from call-path evidence, and in doing so found something the prior closure document did not state: **the identity gap is not uniform.** For D-Code/WASM invocations, the full governed identity tuple (including a current D-Map `{id, revision, digest}` reference) already exists at the real production execution boundary (`dcode::invoke`) and is simply never forwarded to DMonitor — a pure wiring gap, zero missing identity. For continuously-running (container/native-process) D-Serv placements, `application_id` and the D-Map contract reference are lost one hop below dispatch authorization, before any local runtime observation or persistence occurs — an identity-propagation gap, not a missing-authority gap. Neither gap requires inventing new canonical authority (see Q11, Section 9, Section 10).

### Answers to the twelve required questions

- **Q1 — does a producer exist today?** **NO-PROVEN-ABSENT.** (See Section 7 for the exhaustive negative-evidence list.)
- **Q2 — inventory of all plausible candidates.** Fourteen candidates identified and classified; see Section 5's table.
- **Q3 — where does identity originate for each candidate?** See Section 6 (authority traces) and the per-candidate notes in Section 5.
- **Q4 — does identity survive to the execution boundary?** Yes, completely, for D-Code invocations (`dcode::invoke`). Partially for native-process/container placements: `service_id`/`artifact_id`/generation/digest survive; `project_id`/`application_id`/D-Map reference do not.
- **Q5 — do app_id/service_id/node_id/current-D-Map all survive together anywhere?** Only in two places, neither of which is a runtime *observation*: `DCodeInvocationEvidenceV1` (a discrete invocation-evidence record, never forwarded to DMonitor) and `ddeploy::ActiveApplicationDeployment`/`active_placement_authority` (pure desired-state authority, never itself evidence of anything running).
- **Q6 — is missing identity ever heuristically reconstructed?** **No, never.** This is a deliberate, tested, fail-closed design choice (`no_dserv_identity_is_ever_guessed_from_names`, `dmonitor_projection.rs:1035–1045`), not an oversight or a gap in enforcement.
- **Q7 — could an existing runtime-ownership object be extended without new authority?** Yes, two seams identified: (a) `dcode::invoke`'s `DCodeInvocationEvidenceV1` could be wired into a `DCodeExecution` observation almost directly; (b) `integrated_service_evidence`'s per-cycle evidence loop could be extended to carry `application_id` + D-Map reference from `active_placement_authority`. **Neither currently does this** — "could be extended" is explicitly not "a producer exists."
- **Q8 — is DCall/DIoT identity ever improperly promoted into D-Serv identity?** No evidence of this found anywhere. The Phase 114D0B DIoT producer (`diot_runtime_dmonitor.rs`) constructs only `DIoTRealization` subjects and explicitly documents why it does not route through the generic bridge; nothing crosses that line.
- **Q9 — is identity discarded before runtime observation (dispatch-time-exists-but-lost hypothesis)?** **Partially confirmed**, and the two paths differ: for native-process/container placements, `project_id`/`application_id`/D-Map reference are discarded one hop below dispatch authorization (at `OperationalArtifactAssignment`) and never reach `ManagedProcessRuntimeStateV1`. For D-Code invocations, nothing is discarded — the full tuple survives all the way through real execution; it is simply never forwarded onward to DMonitor.
- **Q10 — the exact smallest gap.** Two distinct smallest gaps (Section 9): a pure wiring gap for D-Code execution evidence, and an `application_id` + D-Map-reference propagation gap for continuously-running placements.
- **Q11 — would a producer require new authority, or merely propagation of already-governed identity?** **Merely propagation.** `active_placement_authority` is already the canonical, live-recomputed, server-owned source of the complete identity tuple. No new canonical concept is missing.
- **Q12 — what should Phase 115B be?** See Section 13 (a proposed decomposition, evidence-derived, explicitly not implemented here).

---

## 2. Audit scope

This audit inspected, read in full (not merely grepped), and cross-referenced:

- Both canonical `DMonitorSubjectV1` mirrors (`DATUM/src/agent/canonical_dmonitor.rs`, `dserver/src/core/canonical_dmonitor.rs`).
- Every DMonitor ingestion/interpretation/readiness module in `dserver/src/core/` (`dmonitor_state.rs`, `dmonitor_freshness.rs`, `dmonitor_interpretation.rs`, `dmonitor_readiness.rs`, `dforward_dependency_readiness.rs`) and their HTTP surfaces in `dserver/src/api/`.
- Every DATUM execution surface plausibly capable of observing a running D-Serv: `dispatch/*`, `wasm_runtime/*`, `dcode/*`, `agent/native_process_*`, `agent/operational_reconciliation.rs`, `agent/service_deployment_*`, `agent/integrated_service_evidence.rs`, `agent/operational_evidence.rs`, and the `smartsentinel-*` binaries that wire them together.
- Every reconciliation/placement-authority surface in `dserver/src/core/`: `ddeploy.rs`, `ddeploy_v2.rs`, `operational_dgraph.rs`, `operational_dgraph_persistence.rs`, `artifact_reconciliation_context.rs`, `dserv_artifact.rs`.
- Every collector, Python and Rust: `smartsentinel/` (Python tree) and `DATUM/src/collectors/`, `DATUM/src/agent/discovery*.rs`, `DATUM/src/agent/resource_discovery.rs`.
- Persistence/restart paths: `dserver/src/storage/*`, and `dmonitor_state.rs`'s `load_state`/`persist_state`.
- Relevant ADRs: ADR-0002, ADR-0004, ADR-0005, ADR-0009, ADR-0010, ADR-0017, ADR-0018, ADR-0021, ADR-0022, ADR-0023, ADR-0024.
- The full commit history and the prior Phase 114 closure document (`documentation/reference/phase114-dmonitor-final-closure.md`), read as a prior claim to be independently re-verified, not trusted.

**Explicitly out of scope (per this phase's mandate):** implementing any producer; modifying readiness, convergence, DMap, DForward, or reconciliation semantics; modifying any test; touching the protected paths (`CLAUDE.md`, `docs/STAGE107_DATUM_SPEC_GAP_REPORT_AND_VERTICAL_SLICE_PLAN.md`, `.claude/settings.local.json`, `scripts/__pycache__/`, `site/`).

**Method note:** the "Controlled WASM Deployment" phase (C59–I44) structures encountered along the way (`controlled_dgraph_deployment.rs`, `deployment_lifecycle_observed.rs`, `resource_observation.rs`) were confirmed to belong to that already-closed, untouched phase (see `documentation/adr/README.md`'s "Relationship to closed phases" section) and carry no typed `project_id`/`application_id`/`service_id` identity; they are noted but excluded from the candidate table as out-of-lineage for the canonical D-Map v2 / DMonitor architecture.

---

## 3. Canonical authority criteria

Six identity classes were used throughout, exactly as mandated:

1. **Canonical identity** — a value taken directly from an accepted, digest-bound canonical contract (`CanonicalDMapV1`, `CanonicalDForwardV1`, `CanonicalDServArtifactV1`) or a server-issued, freshly-fetched governed response derived from one (e.g. `DCodeInvocationAuthorizationV1` from `POST /api/v1/dcode/authorize`).
2. **Governed/transitive identity** — a value that originates canonically but has passed through at least one boundary that does not itself re-validate it against current authority (e.g. `execution_projection_digest`, which is a real digest of a real governed projection, but is compared only against its own prior value, never against the D-Map that produced it).
3. **Operational identity** — a value with real operational meaning (a `container_name`, a `service_name` string in `ai_context`, a `manifest_id`) that was never derived from an accepted D-Map/D-Forward contract.
4. **Heuristic/inferred identity** — a value derived by pattern, convention, or environmental inspection (hardware fingerprints, `docker inspect` output, name-matching) with no claim of canonical correlation.
5. **Test-only identity** — a value that exists solely inside `#[cfg(test)]` modules or documented integration-proof fixtures, exercising governance logic without any production emission path behind it.
6. **Missing identity** — no representation of the field exists in the surface at all.

Evidence-quality preference order applied throughout (highest first): (1) an actual current production call path; (2) a persisted/runtime ownership contract; (3) an explicit canonical type/API contract; (4) an ADR/documented invariant matching implementation; (5) tests confirming production semantics; (6) comments; (7) naming/convention alone — **never sufficient by itself**, and no claim in this report rests on naming/convention alone.

---

## 4. Production execution surfaces

| Surface | Wired into a real binary? | Identity present | Class |
|---|---|---|---|
| `dispatch::executor` / `container_executor.rs` | Yes — `main.rs:805-807` → `cli::dispatch::run_dispatch_command` | `project_id`/`node_id` only at the `DispatchResponse` envelope level (`dispatch/model.rs:6-24`); `DispatchAction` itself has no `application_id`/`service_id`/D-Map ref (`model.rs:46-73`, confirmed by direct read); execution resolves via `container_name`/`service_name` string matching against local config (`container_executor.rs:254-273`) | Operational/heuristic |
| `wasm_runtime::live_deployment` / `deployment.rs` | Yes — `wasm-deploy-live` CLI (`main.rs:178`) | `project_id`, `node_id`, `manifest_id` (a DGraph-slice manifest id, **not** a D-Map contract reference); no `application_id` | Governed/transitive (manifest is server-fetched) but not D-Map-anchored |
| `dcode::invoke` (`DCodeInvocationEvidenceV1`) | Yes — `smartsentinel-dcode-invoke invoke` and the D-Call MQTT delivery loop (`dcall/delivery.rs`) | Full tuple: `project_id`, `application_id`, `service_id`, `node_id`, plus `dmap_id`/`dmap_revision`/`dmap_digest` and `dgraph_id`/`dgraph_revision`/`dgraph_digest`, all copied verbatim from a freshly-fetched `DCodeInvocationAuthorizationV1` (`dcode/invoke.rs:398-439`, `dcode/authorization.rs:1-40`) | **Canonical** — the only execution surface with the complete tuple |
| `agent::operational_reconciliation` / `native_process_runtime.rs` / `native_process_state.rs` | Yes — the real host-mutating reconciliation loop | `project_id`, `dgraph_id`, `dgraph_revision`, `node_id` at the authorization level (`operational_reconciliation.rs:60-133, 379-393`); narrows to `service_id`/`artifact_id`/`execution_generation`/`execution_projection_digest` at the per-assignment level (`OperationalArtifactAssignment`, lines 197-282) — **no `application_id`, no D-Map reference, anywhere in this file** | Governed/transitive at the authorization level; missing identity below it |
| `wasm_runtime::runner`/`generic_runner`/`dcode::runtime`/`dcode::worker` | Yes (execution mechanics) | None — pure sandbox/process-isolation layers with no identity fields at all, by design (source-guard tests assert they never import networking/docker) | Missing (mechanics-only layer) |

---

## 5. Candidate producer inventory

Every row below reflects a real, independently-verified file:line trace (Section 14 lists them all). "PARTIAL" and "N/A" entries are explained in the notes beneath the table, per the audit's requirement that no such entry stand unexplained.

| Candidate | Production? | project_id | application_id | service_id | node_id | Current D-Map Authority | Heuristic? | Can Emit Canonical `DServRealization`? |
|---|---|---|---|---|---|---|---|---|
| `agent::dmonitor_projection` (SmartSentinel Phase 114B2 bridge) | YES | NO | NO | NO (collector-local `entity_id`/name only) | YES | NO | YES | **NO** — explicitly, deliberately, and test-enforced |
| `dcode::invoke` / `DCodeInvocationEvidenceV1` | YES | YES | YES | YES | YES | YES (`id`+`revision`+`digest`) | NO | **PARTIAL** — has every field `DServRealization` needs, but describes a discrete invocation outcome (`DCodeExecution` shape), never forwarded to DMonitor at all today |
| `dispatch::executor`/`container_executor` | YES | PARTIAL (only at response envelope, not per-action) | NO | NO | YES | NO | YES (container/service-name matching) | **NO** |
| `wasm_runtime::live_deployment`/`deployment.rs` | YES | YES | NO | PARTIAL (`module_id`/`assignment_id`, not a D-Serv `service_id`) | YES | NO (manifest id ≠ D-Map ref) | NO | **NO** |
| `agent::operational_reconciliation`/`native_process_state.rs` (`ManagedProcessRuntimeStateV1`) | YES | NO (discarded before persistence) | NO | YES | YES | NO (`dgraph_id`/`revision` + `execution_projection_digest`, not a D-Map ref) | NO | **NO** |
| `agent::integrated_service_evidence` (`OperationalEvidenceEvent`) | YES | YES | NO | PARTIAL (only inside an untyped JSON `payload`) | YES | NO (merged per-project `dgraph_id`/`revision`, no digest) | NO | **NO** — closest actively-firing candidate; still missing `application_id` and a D-Map digest reference |
| `dserver::core::ddeploy::ActiveApplicationDeployment` / `active_placement_authority` | YES (server-owned, live-recomputed) | YES | YES | YES | YES | YES | NO | **NO** — this is pure desired/placement authority; it has never observed anything actually running, so it cannot itself be *evidence* |
| `dserver::core::artifact_reconciliation_context::ArtifactReconciliationContext` | YES (revalidated on every read) | YES | NO | YES | YES | NO (`execution_projection_digest` only) | NO | **NO** |
| `dserver::core::operational_dgraph::OperationalDGraph` (merged) | YES (server-owned, persisted) | YES (top level only) | PARTIAL (only in an untyped `metadata.applications` JSON array, lost from the typed merged graph — `ddeploy.rs:1740-1818`) | YES | YES | PARTIAL (same untyped-metadata caveat) | NO | **NO** |
| SmartSentinel Python collectors (`smartsentinel/collectors/*.py`, `agent.py`, etc.) | **DEAD-UNUSED-UNREACHABLE** — every file is a literal 0-byte git blob at HEAD (`git ls-tree`, confirmed) | N/A | N/A | N/A | N/A | N/A | N/A | **N/A** |
| `DATUM/src/collectors/docker.rs` | **DEAD-UNUSED-UNREACHABLE** (0-byte file, confirmed) | N/A | N/A | N/A | N/A | N/A | N/A | **N/A** |
| `dserver/src/storage/file_store.rs`, `sqlite_store.rs` | **DEAD-UNUSED-UNREACHABLE** (0-byte files; real storage goes through `control_state.rs`/`firebase.rs`) | N/A | N/A | N/A | N/A | N/A | N/A | **N/A** |
| `agent::discovery`/`discovery_collectors`/`resource_discovery` | YES | NO | NO | NO | YES | NO | YES (hardware fingerprint / OS install id) | **NO** |
| Test fixtures (`canonical_dmonitor.rs` golden vectors, `dmonitor_d1_readiness_integration.rs`, `dforward_dependency_readiness.rs`/`dmonitor_readiness.rs`/`dmonitor_interpretation.rs` test helpers) | **TEST-FIXTURE** | YES (in the fixture) | YES | YES | YES | YES | NO | **N/A** — exercises governance logic only; ADR-0024 itself states this "is testing governance, not inventing a production D-Serv evidence producer" |

---

## 6. Authority traces

### 6.1 D-Code invocation path (the strongest candidate)

`project_id` → `application_id` → `service_id`(`dserv_id`) → `node_id` → D-Map identity → execution boundary → runtime ownership → potential observation:

1. **project_id/application_id/service_id**: explicit, canonical, server-owned — resolved by `DATUM/src/bin/smartsentinel-dcode-invoke.rs` calling `client::fetch_dcode_authorization` (`POST /api/v1/dcode/authorize`, governed).
2. **node_id**: explicit, canonical, local — `dcode::local_identity::DNodeLocalIdentityV1`, resolved once via `--node-id`/`dcode/local_identity.rs`, never guessed.
3. **D-Map identity**: explicit, canonical, server-owned, **persisted and revalidated at fetch time** — `authorization.dmap_ref` (`{id, revision, digest}`), fetched fresh immediately before every invocation (never cached across invocations).
4. **Execution boundary**: `dcode::invoke::invoke_dcode` cross-checks `artifact.application_id`/`artifact.dserv_id`/`artifact.artifact_id` against the authorization before running (`invoke.rs:465-494`) — TOCTOU-aware at this boundary (governed at call time, not merely at some earlier point).
5. **Runtime ownership**: `DCodeInvocationEvidenceV1` (`invoke.rs:398-439`) — the complete tuple, all fields "can become stale" = **no** (freshly fetched every call), "fail-closed-checked" = **yes** (cross-checked against artifact identity), "heuristically-reconstructed" = **no** (100% governed).
6. **Potential observation**: **absent.** The evidence is serialized to JSON and printed/written to a file (`smartsentinel-dcode-invoke.rs:223-229`) — never passed to `agent::dmonitor_emission`, never becomes a `CanonicalDMonitorObservationV1`. This is the pure wiring gap referenced in Section 1 and Section 9.

### 6.2 Continuously-running (container/native-process) placement path

1. **project_id/application_id/D-Map identity**: explicit, canonical, server-owned, live-recomputed — `ddeploy::active_placement_authority()` (`ddeploy.rs:561-647`) never returns a stale cache; it is recomputed from `DDeployStateV1` on every call.
2. **service_id/node_id**: explicit, canonical — `PlacementAuthorityView.placements[]` (`ddeploy_v2.rs:144-183`), the D-Map's own persisted `Vec` order.
3. **First narrowing boundary**: `get_operational_reconciliation_context` (`dserver/src/api/artifact_reconciliation.rs:33-79`) re-snapshots this authority on every read — genuinely revalidated, server-owned — but its output type, `ArtifactReconciliationContext` (`artifact_reconciliation_context.rs:146-160`), **has no `application_id` field** and no D-Map `{id,revision,digest}` reference, only a per-service `execution_projection_digest` (a digest of the runtime spec content, not of the D-Map).
4. **Second narrowing boundary**: `OperationalArtifactAssignment` (`operational_reconciliation.rs:197-282`), the type actually threaded into `native_process_runtime.rs`'s execution call, carries `service_id`/`artifact_id`/`execution_generation`/`execution_projection_digest` — `project_id` and `application_id` are **absent from this type entirely**, confirmed by direct grep across both files.
5. **Runtime ownership / persisted local state**: `ManagedProcessRuntimeStateV1` (`native_process_state.rs:27-58`) persists exactly what `OperationalArtifactAssignment` gave it — `service_id`, `artifact_id`, `execution_generation`, `execution_projection_digest`, `pid`, `cmdline_digest`, timestamps. **No `project_id`, no `application_id`, no D-Map reference field exists in this persisted type at all.** This is where the identity chain genuinely breaks — not at dispatch time (where `project_id`/`dgraph_id`/`node_id` are still present), but one hop below it.
6. **Potential observation**: absent, and even if attempted today would be structurally unable to honestly populate `application_id` or a D-Map `dmap_ref`, because neither survived this far.

### 6.3 The generic SmartSentinel bridge (the honest-refusal case)

`agent::dmonitor_projection` receives `runtime.services[]`/`runtime.containers[]`/`runtime.processes[]` telemetry carrying only a collector-local `entity_id`/`name` (ADR-0024 lines 240-244, 493-501; `dmonitor_projection.rs` doc comments). It correctly refuses, at every step, to promote any of this into `DServRealization` identity — enforced by a dedicated test (`no_dserv_identity_is_ever_guessed_from_names`, `dmonitor_projection.rs:1035-1045`). This is documented, tested, intentional behavior, not a gap to close by relaxing it.

---

## 7. Production vs. tests — the exhaustive negative-evidence list

A repository-wide search for `DMonitorSubjectV1::DServRealization {` (construction, not merely reference) across `DATUM/src` and `dserver/src` returns exactly these sites, and no others:

- `DATUM/src/agent/canonical_dmonitor.rs:816, 834, 969` — inside `#[cfg(test)]` (golden-vector digest tests).
- `dserver/src/core/canonical_dmonitor.rs:671` — inside `#[cfg(test)]` (cross-crate golden-vector parity test).
- `dserver/src/core/dforward_dependency_readiness.rs:726` — a private test-helper function `dserv_observation(...)`, called only from `#[cfg(test)]` code.
- `dserver/src/core/dmonitor_readiness.rs:543` — a `#[cfg(test)]` fixture builder.
- `dserver/src/core/dmonitor_interpretation.rs:1644, 2291` — `#[cfg(test)]` fixtures.
- The Phase 114D1 integration proof `dmonitor_d1_readiness_integration.rs` — an explicitly-documented integration-proof fixture (ADR-0024 lines 1464-1467: "explicitly constructs and publishes a canonical `DServRealization` fixture directly ... to prove the GOVERNANCE logic — this is testing governance, not inventing a production D-Serv evidence producer").

The single non-test appearance of `DMonitorSubjectV1::DServRealization` outside a match/type-position reference is `dserver/src/core/dmonitor_state.rs:190`, which is `subject_key()` — a generic key-derivation function that pattern-matches over *every* subject variant uniformly to build an internal index key. It consumes an already-constructed subject; it does not construct one. It is not a producer.

The equivalent search for `DMonitorSubjectV1::DCodeExecution {` construction yields the identical pattern: `canonical_dmonitor.rs:940, 995` (tests) in DATUM, `canonical_dmonitor.rs:695` (test) in dserver, and `dmonitor_state.rs:203` (the same generic `subject_key()` consumer). No production construction exists for this variant either.

---

## 8. Existing ADR/contract alignment

| ADR | Relevant invariant | Conformance |
|---|---|---|
| ADR-0002 (D-Map single placement carrier) | D-Map is the ONLY placement/realization authority | **Conforms** — nothing found anywhere manufactures a placement decision from DMonitor evidence; DMonitor remains pure observation throughout |
| ADR-0009 (application_id distinct from project_id) | `application_id` must be a first-class, separately-tracked identity | **Partially diverges in practice** below the D-Map/authorization layer — `application_id` is canonical at `ActiveApplicationDeployment` and at `dcode::invoke`'s authorization, but is dropped by `OperationalArtifactAssignment`/`ManagedProcessRuntimeStateV1`/`ArtifactReconciliationContext`/`OperationalEvidenceEvent`. This is a real, evidenced implementation gap relative to the ADR's intent, not a contradiction of the ADR's decision itself |
| ADR-0010 (cross-node readiness, fresh evidence only) | Readiness must never manufacture evidence; absence of proof stays `NotReady`/`Unknown`, never a false positive | **Fully conforms** — `dmonitor_readiness.rs`/`dmonitor_interpretation.rs` correctly stay `NotReady`/`no_evidence` in the total absence of `DServRealization` evidence; this was directly re-verified, not merely assumed |
| ADR-0021 (canonical D-Serv artifact / D-Code) | `(application_id, dserv_id)`-keyed identity; ADR-0008 fresh-instance-per-invocation | **Conforms** — `DCodeInvocationEvidenceV1` implements exactly this identity shape; the fresh-instance-per-invocation design also explains why `DCodeExecution` (a discrete event), not `DServRealization` (a continuous realization), is the natural fit for D-Code-based services |
| ADR-0023 (canonical D-Forward/DIoT v2) | DIoT identity distinct from D-Serv identity | **Conforms** — no evidence anywhere of DIoT/D-Call identity being promoted into D-Serv identity (Q8) |
| ADR-0024 (canonical DMonitor observation model) | Defines `DServRealization`'s shape and already documents the producer gap | **Fully conforms**, and this audit independently corroborates, rather than merely repeats, the ADR's own stated limitation |

No ADR was found to be contradicted by the current implementation. The one real divergence — `application_id` failing to survive the reconciliation chain — is a gap relative to ADR-0009's *intent* for application-scoped identity to remain traceable, not a violation of any explicit ADR rule (no ADR mandates that `OperationalArtifactAssignment` carry `application_id`; it simply happens not to).

---

## 9. The exact gap

There are **two** distinct smallest gaps, corresponding to the two execution models DATUM supports:

**Gap A — D-Code/WASM invocations: a pure wiring gap, zero missing identity.**
`DCodeInvocationEvidenceV1` already contains everything `DMonitorSubjectV1::DCodeExecution` needs (`project_id`, `application_id`, `service_id`, `node_id`, and a digest reference). It is produced at a genuine, governed, currently-executing production call site (`dcode::invoke`, exercised by `smartsentinel-dcode-invoke` and the D-Call MQTT delivery loop). Nothing forwards it to DMonitor. Closing this gap requires writing a new adapter function and calling it from the existing invocation call site — no new authority, no new identity, no changes to `dcode::invoke` itself.

**Gap B — continuously-running (container/native-process) placements: an identity-propagation gap.**
`active_placement_authority` already holds the complete tuple, live and current. `application_id` and a D-Map `{id, revision, digest}` reference are lost at the `ArtifactReconciliationContext` → `OperationalArtifactAssignment` boundary (`operational_reconciliation.rs`), before they ever reach `native_process_state.rs`'s persisted runtime state or `integrated_service_evidence.rs`'s per-cycle evidence-publishing loop. `service_id`, `node_id`, and `project_id` (at the authorization level) already survive; only `application_id` and the D-Map reference need to be threaded through the same existing structures. This is squarely "propagation of already-governed identity across an existing boundary" (Q11), not new authority.

Neither gap is a missing *canonical concept*. Both are missing *plumbing*.

---

## 10. Existing reusable mechanisms

A future producer, for either gap, does not need to invent new machinery. What already exists and should be reused as-is:

- **`agent::dmonitor_emission::DMonitorEmissionContext`** (`dmonitor_emission.rs:39-74`) — the neutral epoch/sequence/`observation_id` engine, already shared by the SmartSentinel projection and the D0B DIoT producer. Subject-agnostic; works unchanged for `DCodeExecution` or `DServRealization`.
- **The `diot_runtime_dmonitor.rs` architectural pattern** — bind governed identity ONCE from a server-issued response, hold it immutably, emit repeatedly. This is the exact template a D-Serv producer (for either gap) should follow, explicitly documented as deliberately *not* routed through the generic `dmonitor_projection` bridge (`diot_runtime_dmonitor.rs:8-20`).
- **`ddeploy::active_placement_authority`/`PlacementAuthorityView`** — already the correct, live, single source of desired-side identity; a Gap-B producer would read from this (via the reconciliation chain), never re-derive it.
- **`dmonitor_interpretation.rs`'s D-Serv convergence/readiness logic** — already correct and complete (Phase 114C/114D1), already tested against exactly this scenario (`missing_dserv_evidence_blocks_readiness_even_with_diot_ready`), and requires **zero changes** once real evidence starts flowing. This significantly lowers the risk of any future 115B/C/D work: the consumer side is proven; only the producer side is missing.
- **`POST /api/v1/dmonitor/observations/:project_id`** (`dserver/src/api/dmonitor.rs`) — already a fully generic, schema-validated ingestion endpoint; no new endpoint is needed for either gap.

---

## 11. Security / fail-closed considerations

- The DMonitor ingestion endpoint (`dmonitor.rs:122-146`) is, by design, a generic sink: it performs no cross-check against D-Deploy/D-Node authority at ingest time (its own module doc states this explicitly, lines 4-14). This means identity honesty depends **entirely** on the producer being trustworthy — any future producer must fetch identity from a governed, server-issued source (mirroring `dcode::client`/the D0B runtime-binding pattern), and must never accept a caller-supplied `project_id`/`application_id`/`service_id`/`node_id` claim from an untrusted or local-only source.
- The *absence* of a producer today is fail-safe, not merely fail-neutral: readiness correctly stays `NotReady` for any application with a required D-Serv placement, with no fallback to `RuntimeProcess`/container/`Unbound` evidence (re-verified directly in this audit, Section 8). Phase 115A itself changes zero runtime risk.
- The real security risk to flag for whichever phase eventually builds a producer: a rushed implementation that reintroduces heuristic identity (container name, process name, image tag) as a substitute for the missing `application_id`/D-Map fields would be a **regression** relative to the current, deliberately conservative state — worse than continuing to report `NotReady`. Any 115B/C/D work must preserve the existing refusal-to-guess invariant, not work around it.

---

## 12. Phase 115 recommendation (not implemented here)

Two strategic paths were named in this audit's mandate:

- **(A) Build a production canonical `DServRealization` producer first.**
- **(B) Proceed directly toward reconciliation/recovery using only the DIoT signals Phase 114 already proved live.**

This is presented as an open decision, not resolved here. What the evidence does support: **Gap A (D-Code execution evidence) is low-cost and orthogonal to that decision** — it requires no new authority, touches no reconciliation/recovery logic, and would validate that the 114D0B "governed non-bridge producer" pattern generalizes beyond DIoT. It could reasonably proceed regardless of how the (A)/(B) question is eventually resolved. **Gap B (continuously-running placements) is the one genuinely relevant to that decision** — it is real but bounded work (identity propagation through already-identified structures), not a redesign.

---

## 13. Proposed Phase 115 decomposition (evidence-derived; not implemented)

- **Phase 115B — D-Code execution observation producer.** Wire `DCodeInvocationEvidenceV1` (already produced at `dcode::invoke`'s real call site) into a `DMonitorSubjectV1::DCodeExecution` observation, following the `diot_runtime_dmonitor.rs` pattern (bind once, emit via `agent::dmonitor_emission`, reference evidence by digest only, per `canonical_dmonitor.rs`'s own design intent at lines 211-213). Zero new authority; zero changes to `dcode::invoke` itself.
- **Phase 115C — application_id / D-Map reference propagation.** Thread `application_id` and a D-Map `{id, revision, digest}` reference from `active_placement_authority` through `ArtifactReconciliationContext` → `OperationalArtifactAssignment` → `ManagedProcessRuntimeStateV1`/`integrated_service_evidence.rs`. Server- and agent-side coordinated change; no new canonical concept, purely propagation of fields that already exist upstream.
- **Phase 115D — continuously-running D-Serv realization producer.** Once 115C lands, build the actual `DServRealization` emitter for container/native-process placements, reusing the now-complete identity and the same `diot_runtime_dmonitor.rs`-style architecture (health = local runtime liveness only, `dmap_ref` populated when current authority is known).
- **Phase 115E — end-to-end readiness re-verification.** No logic changes expected in `dmonitor_interpretation.rs`/`dmonitor_readiness.rs` (already correct); this phase would be an integration-proof only, confirming real evidence now flows through the existing, unmodified governance logic.

This decomposition is a hypothesis derived strictly from the evidence gathered above. It is explicitly not authorized or implemented by this document.

---

## 14. Evidence index

**Type definitions:** `DATUM/src/agent/canonical_dmonitor.rs:180-300` (`DMonitorSubjectV1`), mirrored `dserver/src/core/canonical_dmonitor.rs:90-330`. `DCodeInvocationEvidenceV1`: `DATUM/src/dcode/evidence.rs:60-120`. `CanonicalDMapV1`/`CanonicalDServPlacementV1`: `dserver/src/core/canonical_dmap.rs:37-66`. `DCodeInvocationAuthorizationV1`: `DATUM/src/dcode/authorization.rs:1-40`.

**Production call sites:** `DATUM/src/dcode/invoke.rs:398-494`; `DATUM/src/bin/smartsentinel-dcode-invoke.rs:143-229`; `DATUM/src/agent/operational_reconciliation.rs:60-282, 379-393`; `DATUM/src/agent/native_process_state.rs:27-58`; `DATUM/src/agent/native_process_runtime.rs:657-670, 1996-2009`; `DATUM/src/dispatch/model.rs:6-73`, `container_executor.rs:254-273`; `DATUM/src/agent/integrated_service_evidence.rs:77-677`; `dserver/src/core/ddeploy.rs:306-326, 561-647, 1740-1818`; `dserver/src/core/ddeploy_v2.rs:144-183`; `dserver/src/core/artifact_reconciliation_context.rs:146-160`; `dserver/src/api/artifact_reconciliation.rs:33-79`; `dserver/src/api/operational_evidence.rs:52-83`.

**Governance / interpretation:** `dserver/src/core/dmonitor_interpretation.rs:1-100, 440-530, 685-, 854-`; `dserver/src/core/dmonitor_readiness.rs:1-60, 1000-1030`; `dserver/src/api/dmonitor.rs` (full file, esp. lines 4-20, 119-146).

**Reference producer (working exemplar):** `DATUM/src/agent/diot_runtime_dmonitor.rs:1-90`; `DATUM/src/agent/dmonitor_emission.rs:39-74`.

**Tests proving the refusal-to-guess invariant:** `DATUM/src/agent/dmonitor_projection.rs:1035-1045` (`no_dserv_identity_is_ever_guessed_from_names`); `dserver/src/core/dmonitor_readiness.rs:1015-1018` (test comment, Phase 114D1); `documentation/adr/ADR-0024-canonical-dmonitor-observation-model.md:440-465, 715-732, 1457-1467`.

**Dead/unreachable scaffold, independently confirmed via `git ls-tree -r -l HEAD`:** `smartsentinel/agent.py`, `smartsentinel/collectors/{docker,host,process}.py`, `smartsentinel/config/load.py`, `smartsentinel/core/{cycle,ids,model,timeutils}.py`, `smartsentinel/outputs/{base,file_json,firebase}.py` (all `e69de29b...`, the empty-blob hash); `DATUM/src/collectors/docker.rs` (same empty-blob hash); `dserver/src/storage/file_store.rs`, `dserver/src/storage/sqlite_store.rs` (same).

**Prior claim independently re-verified, not merely trusted:** `documentation/reference/phase114-dmonitor-final-closure.md:315` ("A production canonical `DServRealization` emitter ... does not yet exist").

---

## Confirmation

No production code was modified, added, or removed during this audit. No test was modified. No ADR was modified. No DMap/DForward/DMonitor/readiness/runtime-binding/placement/execution semantics were changed. This document is the only file this phase creates.
