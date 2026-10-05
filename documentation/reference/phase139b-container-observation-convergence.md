# Phase 139B — Container Observation & Deploy→Observe Handoff

**Status:** implemented, unit/integration-tested, **not committed, not pushed**. No new canonical contract, no new authority, no new convergence engine, no automatic remediation. Closes the one confirmed gap from the Phase 139A architecture audit: container-managed services had no DMonitor producer at all.

## Purpose

Phase 139A's read-only architecture audit found that a mature, complete, runtime-kind-agnostic convergence engine already existed (Phase 114C/114D0A: `core::dmonitor_interpretation`, the `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate` API, and existing Console consumption via Continuum) — the **one** confirmed gap was that no container DMonitor producer existed, so container-managed services never produced any observation evidence at all. Phase 139B closes exactly that gap, plus wires its result into the Phase 138 Deploy/Reconciliation workspace as a clearly-separate "Runtime observation / convergence" section. Nothing about convergence semantics, placement authority, or remediation was touched.

## What did NOT change (by design)

- `core::dmonitor_interpretation.rs` (2964 lines, Phase 114C/114D0A) — **zero edits**. `DMonitorConvergenceV1 { Converged, Diverged, Unknown }`, `AuthorityCorrelationV1`, every finding code, `evaluate_application_dserv_convergence`/`evaluate_application_diot_convergence` — all unmodified.
- `POST /api/v1/dmonitor/convergence/:project_id/:application_id/evaluate` — unmodified, reused as-is by the new Console client.
- `core::dmonitor_state.rs`, `core::dmonitor_freshness.rs`, `core::continuum_view.rs` — unmodified.
- No second convergence engine, no new placement/reconciliation authority, no self-healing/automatic remediation, no new event/alert producer.

This is deliberate: the Phase 139A audit already proved the engine needs no container-specific change, because `DMonitorSubjectV1::DServRealization` carries no `runtime_kind`/instance-id field at all — a container's observation is, on the wire, indistinguishable in shape from a native process's. Phase 139B's dserver-side integration tests (`core::dmonitor_interpretation::tests::container_producer_*`, see below) prove this directly rather than merely asserting it.

## Container DMonitor producer (`DATUM/src/agent/container_dmonitor.rs`)

Mirrors `native_process_dmonitor.rs`'s own established architecture exactly, rather than inventing new semantics:

- **Identity provenance (Part A2):** `application_id`/`dmap_ref` are read from the governed desired/reconciliation context — two new additive fields on the existing per-cycle `OperationalServiceReconciliationReport` (`application_id: Option<String>`, `dmap_ref: Option<CanonicalContractReferenceV1>`), populated the same way `artifact_id`/`execution_generation` already are. They are **never** inferred from the container's own Docker labels, even though those labels (`datum.artifact_id`, `datum.dmap_id`, etc.) do exist — a mismatched/relabeled container must not be able to assert its own governed identity.
- **Inspection reuse:** health comes from a single fresh, read-only `docker inspect` via the already-existing `inspect_container_full`/`inspected_container_labels_match` helpers (now `pub(crate)` so this module can call them) — no second Docker output parser, no mutation of any kind.
- **Identity/health mapping** (`ContainerIdentityCheck`, 6 variants: `Confirmed`, `ContainerAbsent`, `LabelMismatch`, `Stopped`, `HealthcheckUnhealthy`, `Unavailable`) mirrors native's own `IdentityCheck` precedent: `ContainerAbsent`/`LabelMismatch` map to `Unhealthy` (never a separate "absent" convergence state — the engine already handles "no admitted evidence" honestly via `FINDING_NO_EVIDENCE`/`Unknown`). Docker's own HEALTHCHECK concept has no native-process equivalent, so exactly one extra value was added: an explicit `"unhealthy"` healthcheck status demotes to `Degraded`; absent/healthy/starting healthcheck all conservatively fall into the same "no problem proven" bucket as `Confirmed`.
- **Subject correctness:** the emitted `DMonitorSubjectV1::DServRealization`'s `dmap_ref` is always the **expected/governed** reference, never derived from a mismatched container's own (possibly stale or absent) labels — exactly mirroring `native_process_dmonitor`'s own existing precedent (`build_dserv_realization_from_native_state` posts the persisted/expected `dmap_ref` regardless of `IdentityCheck`, including `ExecutableMismatch`/`ArgsMismatch`). **Precise claim, verified against `assess_dserv_evidence`'s actual comparison (`dserver/src/core/dmonitor_interpretation.rs`):** `AuthorityCorrelationV1` compares only `dmap_ref.{id,revision,digest}` against current authority — it never inspects `artifact_id`/image/`execution_projection_digest` at all. This means a wrong-image or relabeled container (`ContainerIdentityCheck::LabelMismatch`) is reported `Unhealthy` but can still be evaluated `Converged` (placement-identity proof, independent of health — the same pre-existing "Converged != Healthy" invariant `exact_placement_with_unhealthy_health_still_converges` already proves for native). **Wrong image/artifact does NOT cause `Diverged`** under current, unmodified convergence semantics — only an actual `dmap_ref` mismatch (stale/superseded D-Map revision) does. Artifact/image authenticity is proven on the COMMAND side only (`verified_image_identity`/`execution_projection_digest` on `ReconciliationAttempt`), a structurally separate concern this phase does not merge into DMonitor evidence.

## Independent emission stream, not a shared one

Each DMonitor producer gets its own `DMonitorEmissionContext` — one process-epoch `monitor_instance_id` plus a monotonic `next_sequence` — mirroring the pre-existing native/DIoT precedent (dserver's stream model is explicitly keyed by `(project_scope, node_id, monitor_instance_id)` and supports multiple independent streams per node; "no implicit monitor leader"). `operational_service_loop.rs` constructs a second, independent `DMonitorEmissionContext::new_process_epoch(...)` for `ContainerMonitorEmitter`, wired into the **existing** `run_hybrid_control_cycle` emission cadence — no new polling loop, no new cron, no command-side coupling. The container emission loop runs immediately after the existing native-process emission loop, once per service per cycle, and never blocks or affects command-side (`ReconciliationAttempt`) results in any way.

## Console: a clearly-separate "Runtime observation / convergence" section

`console/src/api/dmonitor-convergence.ts` (`DMonitorConvergenceClient`) wraps the existing evaluate endpoint. It is structurally read-only by construction — it exposes exactly one method, `evaluate` — proven by `reconciliation.test.mjs`'s `"DMonitorConvergenceClient exposes exactly one method -- evaluate..."` test, which also asserts `reconciliation.ts` never calls anything else on it.

`console/src/pages/reconciliation.ts` renders this data in its own `observationSection()`, visually and structurally distinct from the Phase 138 Desired/Command-side sections — never collapsed into one combined status. It:

- Polls independently of the Phase 138 batch poll loop (own `convergencePollToken` generation counter, same 3s-visible/15s-hidden cadence), starting immediately on page load regardless of whether a reconciliation batch exists, and stopping once `isConvergenceSettled` (`convergence !== "unknown"`).
- Never lets convergence data drive `canAct` (button enablement) or `authorizationDisplayStatus(...)` — proven structurally (`reconciliation.test.mjs`).
- Never calls a mutating method — `createBatch`/`authorize`/`cancel` — from its own `loadConvergence` function, and never opens an `EventSource`/`WebSocket`, nor references an `/alerts` endpoint — proven structurally (Part V regression test).
- Shows "waiting for observation" as a **UI-only** workflow label (`isWaitingForFirstObservation`) when a service has genuinely zero admitted *and* zero excluded evidence — never itself presented as a backend convergence state.
- Never renders a literal "Healthy"/"Converged" command-side status label (pre-existing Phase 138 regression test, still enforced).

## Continuum requires zero code changes

`continuum-model.ts`'s validator and `continuum.ts`'s renderer carry no runtime-kind field or branch anywhere — `monitor_instance_id` is validated only as an opaque non-empty string. `continuum.test.mjs` adds a direct regression proof: a second, independently-epoched `monitor_instance_id` in the container producer's own format (`dmonitor-instance:<node>:<uuid>`) validates and coexists with the native stream's own evidence entry with no schema or rendering change required. This confirms the Phase 139A audit's own finding rather than merely asserting it.

## Backend integration proof (not a backend change)

Five new tests in `dserver/src/core/dmonitor_interpretation.rs` (`container_producer_exact_ref_correct_node_fresh_is_converged`, `container_producer_old_dmap_ref_is_not_converged`, `container_producer_stale_observation_is_not_falsely_converged`, `container_producer_wrong_node_is_not_converged`, `container_producer_wrong_project_application_or_service_never_false_matches`) construct observations shaped exactly as the container producer emits them (its own `monitor_instance_id` epoch format, `collector: "container_dmonitor"`) and drive them through the real, unmodified `ingest_observation` → `evaluate_application_dserv_convergence` pipeline. Since dserver and DATUM share no crate (ADR-0007), this is the only way to prove the engine correctly processes container-sourced evidence without a shared-crate dependency; `collector` itself is never read by any evaluation logic (confirmed by grep), which is exactly the proof that the engine is runtime-kind-agnostic.

## Known, documented limitations (not fixed, not papered over)

- **Node-offline vs. service-missing:** unchanged from Phase 139A's own finding — `active_monitoring.rs`'s freshness-driven scheduler exists but remains unwired in `main.rs`; this phase does not wire it (explicitly out of scope, Part V of the implementation brief).
- **Ghost/unexpected-runtime detection across a node move:** if a container is moved to a different node (or an old container is left running on a superseded node) before any producer observes it there, the engine's own pre-existing `unexpected_realizations`/stale-unexpected-evidence handling applies unchanged — Phase 139B does not add active cleanup detection or confirmation. No cleanup is ever claimed to have happened merely because it is no longer observed.
- **Shared/external runtimes:** a container not under DATUM's own governed identity model (no matching labels) is correctly reported as `LabelMismatch` → `Unhealthy` against the expected placement; it is never silently adopted as "this is actually ours now."

## Phase boundary

Phase 139 Console/runtime-observation work is **not closed**: this is the narrow 139B foundation only. No automatic remediation, no self-healing, no new placement authority, no second convergence engine were introduced, per the phase's own explicit constraints.
