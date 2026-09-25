# Phase 115 — Production D-Serv Observation Producer: Closure Record

**Status:** 115A–115E closed and locally validated. 115F (live clean-node proof) partially completed live, then stopped cleanly at a precisely-isolated blocker — see Section 6.
**Branch:** `agent/phase115-dserv-runtime`.
**Baseline:** Phase 114 closure, commit `9baf96d6e155f9781235a5f1077d6e1220213bc7`.

This document is a closure record, not a new specification. It distinguishes, throughout, between **implementation proof** (unit/integration tests run in this repository), **live experiment proof** (real HTTP calls against a real running DServer and a real Raspberry Pi), and **architecture inference** (conclusions drawn from reading code, not from execution). Nothing here overclaims: where a gate was not reached, that is stated plainly, not implied to have passed.

---

## 1. Baseline: Phase 115A audit

Phase 115A (`documentation/reference/phase115a-dserv-observation-authority-audit.md`) established, from evidence, that no production canonical `DServRealization` producer existed anywhere in DATUM or dserver. It found the gap was not uniform:

- **Gap A (D-Code/WASM invocations):** a pure wiring gap. `dcode::invoke` already carries the full governed identity tuple (`project_id`/`application_id`/`service_id`/`node_id` + current D-Map `{id,revision,digest}`) at its real execution boundary; nothing forwarded it to DMonitor.
- **Gap B (continuously-running placements):** an identity-propagation gap. `application_id` and the D-Map reference were lost one hop below dispatch authorization, before reaching persisted local runtime state.

Neither gap required inventing new canonical authority — both were "propagate what already exists," never "add a new identity concept."

## 2. Phase 115B — Canonical D-Code DServRealization producer

Commit `da37bdf`. Added `agent::dcode_execution_dmonitor`, a pure builder turning one real `DCodeInvocationEvidenceV1` into one canonical `DServRealization` observation. `DServRealization` (not `DCodeExecution`) was chosen deliberately: only `DServRealization` evidence is consumed by dserver's existing D-Serv convergence/readiness logic, and a successful invocation is an honest "this D-Serv placement was realized at `observed_at_utc`" claim — no new persistence semantics invented.

Wired into both real production call sites:
- `smartsentinel-dcode-invoke` CLI, via an opt-in `--dmonitor-publish` flag (best-effort, never affects the CLI's own exit code).
- The governed MQTT D-Call listener loop (`dcall::delivery::run_mqtt_dcall_listener_loop`), reusing its existing `DMonitorPublisherHandle` and adding an independent process epoch.

Fail-closed guarantees (all tested): `service_id` is always the D-Serv placement identity, never the D-Code artifact identity; a node-identity mismatch (the only way `dcode::invoke`'s own `StaleAuthority` outcome arises) yields no observation at all; health is mapped directly from `DCodeInvocationOutcome`, never from the guest's own business-level `output_status`; `dmap_ref` is copied verbatim, never recomputed.

14 new tests (11 unit + 3 delivery-loop integration). Full DATUM lib suite (`--features dcall-test-support`): 1227 passing, 0 failures at close of this phase.

## 3. Phase 115C — Canonical runtime identity propagation

Commit `53ed050`. Re-derived the exact identity-loss boundary from source rather than trusting Phase 115A's prose: `derive_application_operational_services`/`union_project_operational_graph` (dserver) already have `application_id` and the accepted D-Map `{id,revision,digest}` in scope but discarded them when building `OperationalDesiredService` — the typed struct `ArtifactReconciliationAssignment` and, ultimately, DATUM's `OperationalArtifactAssignment`/`ManagedProcessRuntimeStateV1` are all derived from.

Propagated, server side: `OperationalDesiredService` gains `application_id` (set at its true origin, in `derive_application_operational_services`, from the D-Map already being processed) and `dmap_ref` (attached in `union_project_operational_graph`, from the already-computed `ActiveApplicationDeployment.dmap_digest` — never independently recomputed). `ArtifactReconciliationAssignment` carries both through verbatim.

Propagated, agent side (wire-mirrored): `OperationalDesiredService`/`OperationalArtifactAssignment` gain the same two fields; `ManagedProcessRuntimeStateV1` (the actual persisted local runtime-ownership object) now persists them, under the identical "observed-history evidence, never authoritative, never independently derived" contract already established for `execution_generation`/`execution_projection_digest`.

All new fields are `#[serde(default)]` — a not-yet-rebuilt legacy graph or a pre-migration persisted record deserializes with `application_id` empty / `dmap_ref` `None`, never heuristically reconstructed.

New tests included a full acceptance-to-assignment integration test proving two applications placing services on the *same* node keep independently correct `application_id`/`dmap_ref` (never cross-contaminated). 1610 dserver tests passing, 1229 DATUM lib tests passing at close of this phase.

## 4. Phase 115D — Continuous-runtime DServRealization producer

Commit `400faf8`. Built on 115C to close the continuously-running half of the Gap B. Added `agent::native_process_dmonitor`, wired into `operational_service_loop.rs` as an independent, read-only observation step that never touches `reconcile_native_service`'s own control flow.

**Scope, evidence-based:** only the native-process runtime model is covered. Container placements have no equivalent typed, persisted runtime-ownership object anywhere in this codebase (confirmed absent in the Phase 115A audit) — inventing one would be new tracking infrastructure, not propagation of already-governed identity, so it was explicitly left out rather than faked.

Design: `native_process_runtime::observe_current_native_state` re-observes a service's persisted state + live identity check via the exact same state-file derivation the reconciliation function already uses. Identity comes only from the persisted, governed record; the live `IdentityCheck` drives health only. Health mapping: `Confirmed`→Healthy; `ProcessAbsent`/`PidReused`/`ExecutableMismatch`/`ArgsMismatch`/`Zombie`→Unhealthy; `Unavailable`→Unknown (a procfs race is honest uncertainty, never coerced). Fail-closed: a record predating Phase 115C or an empty `project_id` produces no observation.

11 new unit tests. Full DATUM lib suite: 1240 passing, 0 failures at close of this phase.

## 5. Phase 115E — DServ readiness re-verification

Commit `335f91a`. No readiness/convergence/DMap/DForward logic was changed. Re-confirmed from source (not assumed): readiness policy id `dmonitor-readiness-v1`, `max_observed_age_seconds=90`, `max_received_age_seconds=90`, `max_future_clock_skew_seconds=10`, and that only exact Healthy admitted current evidence (not Degraded/Unknown/Conflicting) can produce Ready.

Added one full end-to-end integration test (`ddeploy.rs::readiness_pipeline_accepts_a_phase_115_shaped_dserv_realization_end_to_end`) exercising a **real** accepted D-Map, real `active_placement_authority`, and observations shaped exactly as the Phase 115B/115D producers actually build them. Covered: (A) no evidence → NotReady; (B) exact current healthy/fresh evidence → Ready; (C) same evidence 91s later → stale → NotReady; (D) service_id mismatch → NotReady; (E) application_id mismatch → NotReady; (F) node mismatch → NotReady; (G) D-Map re-acceptance makes prior evidence's `dmap_ref` stale → NotReady until fresh evidence exists under the new revision; (H) none of the above mutates D-Map/acceptance state. 1610 dserver tests passing, 0 failures.

## 6. Phase 115F — Clean-node live deployment proof

### 6.1 Environment

- Raspberry Pi, previously used for an unrelated experiment (a ChirpStack gateway-bridge container, left untouched): Debian 13 (trixie), aarch64, kernel 6.12.47, 4 CPU cores, 3.7 GiB RAM, 18 GiB free disk, Docker 26.1.5 present, `rustc`/`cargo` 1.94.1 present (via `~/.cargo/env`), no `podman`. SSH reachable via a pre-existing key (`~/.ssh/datum_ed25519`) once the exact identity file was specified explicitly.
- DServer control plane: started fresh on the existing Windows host (`192.168.10.101`, confirmed to be this same machine), bound to `0.0.0.0:8080`, with an isolated, dedicated control-state database (`C:\Users\dnred\datum-115f-live\control-plane-115f.db`) so this run could never read or write any prior phase's accumulated state.
- DATUM agent binaries were **built natively on the Pi** (aarch64; no cross-compilation or prebuilt artifact existed) from a tarball of the exact `agent/phase115-dserv-runtime` branch source, transferred directly (the GitHub repository is private and the Pi had no configured credentials for it).

### 6.2 What was proven live (real HTTP responses from the real running server; raw artifacts under `C:\Users\dnred\datum-115f-live\proof\`)

- **F1 — Clean node:** confirmed no DATUM process or container running before this test began; the one pre-existing container (`datum-chirpstack-gateway-bridge`, exited) was left untouched.
- **F2 — Canonical node:** `rpi-01` declared in the real DNode structural registry (`02-canonical-dnode-rpi01.json`), then the Pi's real hardware-discovered resource (`resource:hw:c1bd91343a8f910a044601105b4910360eb96418d672b0ddd54020c98ad3c92d`) bound to it (`02b-resource-binding.json`). The live agent subsequently reported `bootstrap_state=KnownBoundNode` for hundreds of consecutive cycles.
- **F3 — Current D-Map:** a real D-Map proposal was created and accepted (`proj:dserv-live-115f0` / `app:dserv-live-115f0`), producing `dmap_id=dmap:21:proj:dserv-live-115f020:app:dserv-live-115f0`, revision 1, placing `svc-115f0` on `rpi-01` (`04-active-dmap-final.json`).
- **Direct live confirmation of Phase 115C:** fetching the real operational D-Graph slice for `rpi-01` from the live server (`03-operational-dgraph-slice-rpi01.json`) shows `desired_services[0]` carrying **both** `application_id: "app:dserv-live-115f0"` **and** `dmap_ref: {id, revision, digest}` — this is the Phase 115C propagation fix, observed working in a real running server against real accepted state, not merely in unit tests.
- **Live confirmation of Phase 114/115E's fail-closed absence-of-evidence behavior:** querying live readiness (`05-dmonitor-readiness-not-ready-no-evidence.json`) for the application returns `readiness: "not_ready"`, finding `dserv_not_converged` / `no_evidence` — exactly the Case-A behavior Phase 115E proved in-process, now reproduced against a live server with a real accepted D-Map.
- The operational agent ran continuously on the Pi for the duration of this test (`06-agent-log-excerpt.txt`), reaching `bootstrap_state=KnownBoundNode` and `status=ok` on every cycle, was stopped gracefully via its documented stop-file mechanism, and the D-Map/acceptance state was confirmed unchanged (still revision 1, same `dmap_id`/digest) throughout — Case H (DMonitor/agent activity never mutates D-Map authority) held for everything this run actually exercised.

### 6.3 Exact gate reached, and why F4 onward did not run

**Gate reached: F3 complete; F4 (real governed deployment/spawn) blocked before it could start.**

Executing a real native-process spawn requires `execute_requested && control_plane_authorized`, which requires a persisted `OperationalReconciliationAuthorization` (`POST /api/v1/nodes/rpi-01/operational-reconciliation/authorize`). That call failed with:

```
persisted operational authorization '...' does not match active graph identity/revision/node
```

**Root cause (a genuine, pre-existing dserver gap, unrelated to Phase 115B–115E's own work, discovered only because this is the first test to combine canonical D-Map v1 authority with the operational-reconciliation-authorization persist/reload path end-to-end):** `dserver::core::operational_dgraph_persistence::validate_operational_dgraph_persisted_state` cross-checks every persisted authorization against its OWN domain-isolated `graphs` map (`DOMAIN_OPERATIONAL_DGRAPH`). That map is populated **only** by the legacy `put_operational_dgraph` write path — which itself explicitly **refuses** to write once canonical D-Map authority (Stage 111, ADR-0018) is active for a project, specifically to stop a stale legacy write from clobbering the canonical projection. The practical effect: a project using canonical D-Map authority (as this entire Phase 115 effort requires, since `dmap_ref` only exists under that authority) legitimately has **no** entry in this legacy `graphs` map — and the validator currently treats that absence as corruption rather than as the expected shape for a canonical-authority project. No existing test exercises this exact combination (canonical authority + this authorization endpoint + persist/reload), which is why it was never caught before.

A minimal, narrowly-scoped fix was identified — treat an absent legacy-graph entry for the authorization's project as "this project uses canonical D-Map authority, whose real graph identity this domain was never given custody of" rather than as corruption, while leaving every other check (binding_id, durable persistence scope, `allow_host_mutation`, timestamp validity, key format, and — when a legacy entry *is* present — the existing exact-match check) fully intact. That fix was **not applied**: it touches dserver's authorization-persistence validation logic, which is outside Phase 115B–115E's actual mandate (the production D-Serv *observation* path) and is security/correctness-sensitive; the change was flagged by this environment's own safety classifier as a validation-weakening edit requiring explicit human sign-off, and this closure record defers that decision to the user rather than pushing it through autonomously.

Per this phase's own instructions, this is reported exactly as such rather than downgraded: **115B–115E are fully implemented, tested, and passing locally; 115F's live proof reached and confirmed F1–F3 (plus direct live confirmation of the 115C propagation fix and the correct fail-closed NotReady/no_evidence behavior) against a real server and a real, previously-unused Raspberry Pi; F4 onward (real spawn, canonical runtime ownership observation, a live DServRealization observation, live readiness transition to Ready, staleness, and authority-immutability-under-load) were not reached, blocked by one specific, exactly-diagnosed, pre-existing dserver bug outside this phase's scope.**

### 6.4 Live environment left in place

- DServer is running locally as a background process (`C:\Users\dnred\datum-115f-live\proof\dserver-stdout.log`), bound to `0.0.0.0:8080`, using the isolated database at `C:\Users\dnred\datum-115f-live\control-plane-115f.db`. It can be stopped or left running; it touches no other phase's state.
- The Pi's operational agent was stopped gracefully. Its build artifacts, source checkout, and native-process root remain on the Pi under `~/datum-115f-repo` / `~/datum-115f-native-root` / `~/datum-115f-governed-config`, untouched, ready to resume once the blocker above is resolved.
- The D-Map, DNode registration, resource binding, and ServiceArtifact created for this test remain accepted/active in the isolated database — re-running the authorization call (after a fix) should be sufficient to proceed directly to F4 without repeating F1–F3.

## 7. Proposed next steps (not implemented; the user's decision)

1. Decide whether to authorize the narrowly-scoped fix identified in Section 6.3 for `operational_dgraph_persistence.rs`, then re-attempt Phase 115F's F4–F10 gates against the still-live environment described in 6.4.
2. If authorized and F4–F10 complete, this document's Section 6 should be updated with the remaining live evidence (real spawn, real `ManagedProcessRuntimeStateV1`, a real `DServRealization` observation, live Ready/staleness transitions) rather than treated as a separate phase.
3. Independent of the above, Phase 115A's own proposed decomposition (a continuously-running container-runtime producer, should one ever be justified by a real persisted container-ownership object) remains unstarted and out of scope here.
