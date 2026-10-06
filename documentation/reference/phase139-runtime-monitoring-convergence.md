# Phase 139 — Runtime Monitoring & Convergence (CLOSED)

**Status:** closed at commit `23e903fa8c4d098e85b06aaf86aca9bb7b5868c3`. Phase 139A was a read-only architecture audit (no commit). Phase 139B is the only code change, documented in full in [`phase139b-container-observation-convergence.md`](phase139b-container-observation-convergence.md). This file records the phase's closure: what the four MVP producers now cover, the closure invariants verified at close, and the limitations accepted — not fixed — as explicitly out of Phase 139's scope.

## Producer coverage at close

| Producer | Subject | Trigger | Identity source | Continuous vs one-shot |
|---|---|---|---|---|
| `native_process_dmonitor` | `DServRealization` | every reconciliation cycle | persisted `ManagedProcessRuntimeStateV1` | continuous (re-observed every cycle) |
| `container_dmonitor` (Phase 139B) | `DServRealization` | every reconciliation cycle | this-cycle `OperationalArtifactAssignment` via the report | continuous (re-observed every cycle) |
| `dcode_execution_dmonitor` | `DServRealization` (deliberately, not `DCodeExecution` — `dmonitor_interpretation` never reads that variant) | one event per actual D-Code invocation | governed `DCodeInvocationAuthorizationV1`/evidence | **one-shot**: one observation per invocation; absence of a new invocation is never itself Diverged/Missing — the last invocation's evidence simply ages out under ordinary freshness rules, same as any other evidence |
| `diot_runtime_dmonitor` | `DIoTRealization` (its own type, its own `evaluate_application_diot_convergence` path) | per governed MQTT listener session | `DiotRuntimeBindingIdentity` | continuous (session-liveness), health means transport/session liveness only, never D-Serv health |

No remaining supported Phase 139 MVP runtime class lacks a canonical producer. D-Code's one-shot nature is intentional and does not need a "missing" state invented for it: the existing freshness policy already governs how long one invocation's evidence counts as current proof, without any special-casing.

## Closure invariants — verified, not merely asserted

- **Desired != Observed:** desired state comes only from `core::ddeploy::active_placement_authority`/`PlacementAuthorityView`; observed state comes only from `DMonitorStateV1` via `evaluate_application_dserv_convergence`. Neither ever substitutes for the other.
- **Observed != Healthy:** `exact_placement_with_unhealthy_health_still_converges` (pre-existing, untouched) and container's own `wrong_labels_never_falsely_healthy`/healthcheck tests prove convergence and health are independent axes.
- **Healthy != Fresh, Fresh != Converged:** `DMonitorFreshnessV1`/`DMonitorHealthV1`/`DMonitorConvergenceV1` are three structurally separate enums; the UI renders all three as distinct badges, never collapsed.
- **Command Succeeded != Observed, Command Failed != Runtime Absent:** proven by fixture scenario I (command Partial, the failed node's runtime independently Converged) and scenario K (command Succeeded, runtime independently Diverged) — both exercised over real HTTP, not only asserted in code.
- **DMonitor observation != placement authority, Convergence evaluation != remediation:** structural source scans confirm the Console convergence client exposes exactly one method (`evaluate`), and no Phase 139 code anywhere calls `createBatch`/`authorize`/accepts a proposal/creates a reconciliation request.

## Continuum consistency (Audit AI) — by construction, not by convention

`core::continuum_view::build_continuum_view` and `api::dmonitor_convergence`'s evaluate handler both call the **identical** `evaluate_application_dserv_convergence` function with the same `DMonitorStateV1` and the same default freshness policy. They cannot disagree about convergence state for the same application at the same moment — there is only one evaluation path, not two consistent ones.

## Live monitoring, not a one-time check

Phase 139B's own independent review (Audit 45) found convergence polling stopped permanently once settled; this was fixed before that phase closed (slower cadence, never a terminal stop). This closure review additionally proved — over real HTTP against the fixture server, not merely asserted — that a later poll genuinely replaces an earlier Converged result with Diverged, and the reverse, with no remediation action taken in between (`reconciliation-visual-fixture.test.mjs` scenarios M/N; `container_dmonitor.rs`'s own `mixed_node_skips_native_shaped_report_and_still_emits_for_the_container_shaped_one` proves the agent side of mixed native+container nodes).

## Accepted limitations (explicitly out of Phase 139's scope — not fixed here)

- **Node-offline vs. service-missing:** still not canonically distinguished (confirmed unchanged from Phase 139A). The Console never renders "node offline"; a regression test (`reconciliation.test.mjs`) now enforces this directly.
- **SentinelRegistration integration:** `core::sentinel_registration` is already live and exposed via its own API (`/api/v1/sentinels`, registration + heartbeat), but is **not** integrated into Phase 139's observation surface. Recommendation: **do not integrate it now.** It would require new Console wiring (not "already exposed and trivial" from the Deploy/Reconciliation page's own perspective) and real risk of re-introducing the exact "node offline" overstatement this phase deliberately avoids. Revisit only if a future phase explicitly takes on node-liveness semantics.
- **Ghost/unexpected-runtime detection:** producers (native and container alike) only iterate the agent's own *currently desired* services each cycle; a runtime from a superseded placement that is no longer in that desired set is never discovered by either producer. `DMonitorUnexpectedServiceEvidenceV1`/`unexpected_realizations` exist and are rendered by the Console, but will stay empty for this specific case — never fabricated with invented examples.
- **Shared/external runtime ownership:** unchanged — no ownership model exists for a container/process DATUM did not place; such a runtime is simply never matched by any producer's own governed-identity gate, never classified as "Unexpected" or "Diverged" by inference.
- **Wrong-image/artifact vs. placement-convergence:** re-confirmed and now explicitly documented (see `phase139b-container-observation-convergence.md`'s corrected wording): `AuthorityCorrelationV1` compares only `dmap_ref` fields, never artifact/image identity. A wrong-image container can be placement-Converged while health-Unhealthy. This is accepted as Phase 139's defined semantic scope — convergence means *placement/provenance* agreement, not artifact/runtime-integrity verification — and is not silently redefined.
- **DMonitor history retention:** confirmed still unbounded (`dmonitor_state.rs`'s own doc comment: "Retention/pruning is explicitly deferred"). Unchanged this phase. P2 architectural debt, explicitly deferred to a future phase.
- **Removed D-Node observations:** confirmed `remove_dnode` still does not cascade-delete `DMonitorStateV1` rows. Orphaned observations persist (storage growth, same P2 as above) but do not leak into active convergence beyond ordinary freshness-bounded staleness — convergence evaluation has no dependency on D-Node registry liveness at all, so a removed node's old evidence is treated exactly like any other node's evidence, admitted only if it still exactly matches a current desired placement's `dmap_ref` within the freshness window (the same generic exposure live nodes have, not a special leak).

## No remediation, no new authority (restated)

Nothing in Phase 139 creates a `ReconciliationRequest`, authorizes host mutation, creates or accepts a placement proposal, or wires a new event/alert flow in response to an observation. `active_monitoring.rs`'s own pre-existing, unwired `dmonitor_recovery_v1` scheduler remains untouched and unwired, exactly as Phase 139A left it.
