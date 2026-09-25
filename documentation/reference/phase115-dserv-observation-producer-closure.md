# Phase 115 — Production D-Serv Observation Producer: Closure Record

**Status:** Phase 115 CLOSED. 115A–115E closed and locally validated; the persistence-domain incompatibility 115F's live Raspberry Pi run discovered (Section 6.3) is fixed, tested, and live-reverified (Section 8). 115F itself remains an optional, non-closure-gating live experiment — see Section 8 for exactly how far it now reaches and why the remaining Pi gates are out of this closure's scope.
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

## 7. Historical record: the blocker as originally found (superseded by Section 8)

This section is preserved unedited from the moment the bug was discovered, as the record of what the live Raspberry Pi experiment actually found. It is superseded by Section 8, not deleted.

The fix identified in Section 6.3 was not applied at the time this section was written, because it touches dserver's security/correctness-sensitive authorization-persistence validation and this environment's own safety classifier flagged the specific proposed edit (an unconditional `if graph is missing { accept }`) as validation-weakening, correctly requiring explicit human authorization rather than being pushed through autonomously. That authorization was subsequently given, on the explicit condition that the fix distinguish a canonical-authority project from genuinely corrupt legacy state and never merely treat a missing graph as proof of anything — see Section 8 for the fix actually implemented under that constraint.

## 8. Closure fix: canonical-D-Map-aware operational reconciliation authorization

### 8.1 Startup/recovery authority flow (audited before implementing anything)

Traced from `dserver/src/main.rs::main()`: `api::operational_dgraph::initialize_operational_dgraph_persistence_from_env()` runs **first**, before config, Firebase, the ServiceArtifact registry, or the HTTP listener exist. It populates two process-global `OnceLock<RwLock<...>>` caches (`graph_store()`, `authorization_store()`) exactly once per process, via `prepare_restored_authorizations`.

`core::ddeploy`'s canonical-authority accessors (`is_canonical_authority_active`, `active_operational_projection`, wrapped by `api::ddeploy::effective_canonical_operational_graph`) are, by contrast, **stateless per call**: each opens the control-state SQLite file fresh (`load_state(&resolved_db_path())`), with no `OnceLock`, no cached global state, and no dependency on `AppState`/config/Firebase. Calling them from anywhere — including from inside `initialize_operational_dgraph_persistence_from_env`'s own startup sequence — introduces no initialization-order cycle and no recursion (the ddeploy read path never calls back into `operational_dgraph`). The only lock held across the startup sequence, `operational_dgraph`'s own `state_transaction()`, is never needed by the ddeploy read. **Conclusion: calling `effective_canonical_operational_graph` from the restoration path is safe at startup, not merely at request time (where `authorize_operational_reconciliation` already called it safely before this fix).**

The Resource Registry's `active_resource_binding_for_node`, however, reads through its **own** process-global `OnceLock` (`resource_state_store()`, initialized once via `ensure_resource_registry_persistence_initialized()`), populated from whichever caller reaches it first in the process. This makes it — and, transitively, any test that calls it — impossible to drive deterministically from a shared `cargo test` binary: there is no way to guarantee which test's environment "wins" the one-time initialization, regardless of thread-parallelism. This is a pre-existing property of the codebase (confirmed: no existing test anywhere sets `DATUM_CONTROL_STATE_DB_PATH` or the Resource Registry's equivalent env var, and the crate has no `serial_test` dependency), not something introduced here. It directly shaped the test strategy in 8.4.

### 8.2 Root cause, precisely (two independent bugs, both fixed)

1. **`dserver/src/core/operational_dgraph_persistence.rs::validate_operational_dgraph_persisted_state`** — structural persistence validation — unconditionally required every durable authorization's project to have an entry in this domain's own legacy `graphs` map, and additionally cross-checked `dgraph_id`/`dgraph_revision`/node-presence against it. A canonical-D-Map-authority project legitimately has no such entry (the legacy `put_operational_dgraph` write path explicitly refuses to write one once canonical authority is active, precisely to prevent a stale legacy write from clobbering the canonical projection) — so this check rejected every durable authorization for such a project, even a freshly, correctly issued one, at the moment it was persisted.
2. **`dserver/src/api/operational_dgraph.rs::prepare_restored_authorizations`** — the restart/reload path — independently made the identical wrong assumption: `let Some(graph) = state.graphs.get(&authorization.project_id) else { continue; };` silently **dropped** any authorization whose project had no legacy graph entry, with no error at all. Fixing only bug 1 would have let issuance succeed while leaving every canonical-authority authorization invisible again after every restart.

### 8.3 Fix implemented (preserves fail-closed semantics; does not weaken validation)

Per the explicit design constraint: a missing legacy graph entry is **evidence of nothing** by itself — it is never treated as proof that canonical authority is active. The fix always asks the actual authority resolver.

- **Structural validation** (`validate_operational_dgraph_persisted_state`) no longer cross-checks authorizations against `state.graphs` at all. It still enforces every other structural invariant unchanged: key format, `persistence_scope == durable`, `allow_host_mutation`, non-empty `binding_id`, and valid RFC 3339 `expires_at_utc`/`authorized_at_utc`. This function's job is now *purely* "is this blob well-formed," never "is this still current authority" — that question belongs entirely to restoration, which runs on every load, not once at persist time.
- **Authority restoration** (`prepare_restored_authorizations`) now resolves the project's actual **effective graph** — canonical (`effective_canonical_operational_graph`) when active, the legacy `state.graphs` entry only when it is not, via a new pure function `select_effective_graph(canonical, legacy) -> canonical.or(legacy)`. There is exactly one effective authority per project; the two are never blended, and a stale/mismatched legacy entry coexisting with active canonical authority is never consulted at all (proven by test `c9_canonical_wins_over_a_mismatched_coexisting_legacy_graph`).
- **A second, independent gap found while building the test matrix:** the existing helper `authorization_is_valid_for_context` is always called with `&authorization.node_id` as its own "current node" parameter — by both the original code and this fix — which makes that half of its check tautological (an authorization's node id trivially equals itself). The actual "is this node still placed in the current graph" enforcement had lived *only* in the structural check this fix removes. It is re-established in the correct place: `should_restore_authorization` now explicitly checks `graph.nodes.iter().any(|n| n.node_id == authorization.node_id)` before calling the reused helper. Caught by test `c4_node_absent_from_current_graph_never_restores` failing on the first implementation attempt — the test matrix did its job.
- No new authority field, discriminator, or client-suppliable "authority kind" was added to `OperationalReconciliationAuthorization`. Effective authority is derived exclusively from already-governed server-owned state on every read, exactly as the design constraint required.

### 8.4 Why validation remains fail-closed (not weakened)

- A structurally malformed authorization (missing `binding_id`, wrong `persistence_scope`, invalid timestamp) is still rejected at persist/load time, unconditionally — proven by `c7_malformed_authorization_still_rejected_by_structural_validation`.
- A stale D-Map revision, a changed `dgraph_id`, a node no longer in the current graph, a changed or absent Resource Registry binding, and an expired lease all still independently and correctly prevent restoration — proven by the C3/C4/C5/C6 tests below. None of these paths were loosened; the fix only changed *which graph* is consulted, and closed the newly-found node-presence gap.
- A canonical-authority project with **no** effective graph at all still cannot restore anything (`c8_legacy_project_missing_required_graph_never_restores`).
- No authority is ever inferred from the persisted authorization alone, from a process/container/ServiceArtifact name, or from a heuristic — every restoration decision reads current server-owned state (the ddeploy domain or the legacy graph store) fresh, every time.

### 8.5 Test results

**Pure, race-free unit tests** (`dserver/src/api/operational_dgraph.rs::tests`, `select_effective_graph`/`should_restore_authorization`/structural validation — no SQLite, no env var, no global store touched):

| Case | Scenario | Result |
|---|---|---|
| C1 | Canonical project, no legacy graph entry | restores |
| C2 | Persist+load JSON round trip (see honest scope note below) | survives, unchanged |
| C3 | D-Map revision changed / `dgraph_id` changed | does not restore (both variants) |
| C4 | Node no longer present in current graph | does not restore |
| C5 | Binding id mismatch / no active binding | does not restore (both variants) |
| C6 | Lease expired / lease still valid | does not restore / restores |
| C7 | Malformed authorization (missing `binding_id`) | still rejected structurally |
| C7 | Well-formed canonical authorization, no legacy graph | persists (the exact original failure, now fixed) |
| C8 | Legacy project: exact match / missing graph / revision mismatch / node absent / binding mismatch | restores only on exact match; every negative case still fails |
| C9 | Canonical authority active + a stale, structurally-mismatched, coexisting legacy graph | canonical wins; the stale legacy graph neither helps nor hurts |

20 new tests, all passing on the second implementation attempt (the first caught and fixed the node-presence gap in 8.3).

**Honest scope note on C2:** `prepare_restored_authorizations`'s *decision* logic is exhaustively covered above via `should_restore_authorization` directly. The `c2_canonical_authorization_json_round_trip_survives_persist_and_load` test proves the JSON persist/load round trip no longer rejects a canonical-authority authorization structurally (the exact defect this closure fixes) — it does **not** exercise the full `initialize_operational_dgraph_persistence_with_policy` startup path, because that path also depends on the Resource Registry's process-global `OnceLock` (Section 8.1), which cannot be safely or deterministically driven inside this crate's existing shared `cargo test` binary. This is a scope boundary, not a gap in the fix's correctness — the live re-validation in Section 8.6 exercises the real startup-adjacent path end to end, against a real running process.

**Regression:**

| Suite | Before this fix | After this fix |
|---|---|---|
| DATUM lib (`--features dcall-test-support`) | 1240 passed, 0 failed, 1 ignored | 1240 passed, 0 failed, 1 ignored (unchanged — no DATUM files touched) |
| dserver | 1610 passed, 0 failed | 1630 passed, 0 failed |

`cargo fmt --check`: the two modified files carry the same pre-existing, unrelated formatting deviations already present throughout this codebase before this session (confirmed in Phase 115B/D's own closure notes) — no new deviation category introduced; not reformatted, to avoid an unrelated whole-file diff. `cargo build --locked --release` is clean for `dserver` (pre-existing dead-code warning counts unchanged). `git diff --check` clean.

### 8.6 Live re-validation (optional; not a closure criterion, performed because the environment was still available)

The Section 6 environment was still intact (`C:\Users\dnred\datum-115f-live\control-plane-115f.db`, the Pi's accepted D-Map/binding/ServiceArtifact, the Pi's built agent). DServer was rebuilt with the fix and restarted against the same isolated database.

- **The exact original failing call now succeeds:** `POST /api/v1/nodes/rpi-01/operational-reconciliation/authorize` with the identical request body that previously returned `"does not match active graph identity/revision/node"` now returns a real, persisted `OperationalReconciliationAuthorization`, with `application_id`/`dmap_ref` correctly present in its resolved artifact snapshot (`08-authorization-succeeds-after-fix.json`).
- **D-Map authority remained unchanged** throughout (`10-dmap-authority-unchanged-after-fix-validation.json`: still revision 1, same `dmap_id`).
- The Pi's agent was restarted with `--execute` and reached `control_plane_authorized: true` for the first time — F4 (real spawn) still did not complete, but for a **different, unrelated** reason: a separate Stage 107A lease-cycle gate (`blocked_authorization_not_leased: no valid Stage 107A cycle gate for this cycle`, `09-agent-report-blocked-on-unrelated-stage107a-lease-gate.json`) that is outside this closure's scope and outside Phase 115B–115E's own work entirely. This is additional confirmation the fix is correctly scoped: it resolved exactly the one bug it targeted and did not mask or interact with anything else.
- Both the Pi's agent and the local DServer instance were left in a clean, stopped/running-idle state respectively; no D-Map, binding, or artifact state was mutated beyond what Section 6 already recorded.

F4–F10 remain formally NOT COMPLETED live. Per this closure's own criteria (Section 9), that does not block Phase 115 closure.

## 9. Phase 115 closure criteria — checked

1. 115A–115E remain green. ✅ (Sections 2–5; regression counts in 8.5.)
2. Canonical operational reconciliation authorization can be durably persisted without a fake legacy operational graph. ✅ (8.3, live-confirmed 8.6.)
3. Reload/restart semantics revalidate against CURRENT effective authority. ✅ (`prepare_restored_authorizations`/`should_restore_authorization`, 8.3; pure-logic-tested exhaustively in 8.5; the live-process restart path itself is architecturally untestable in-suite per 8.1/8.5, not left unaddressed — it is the same code path the live re-validation in 8.6 exercised.)
4. A stale authorization cannot survive a D-Map change. ✅ (C3, 8.5.)
5. Legacy behavior remains fail-closed. ✅ (C8, 8.5.)
6. Full relevant regression is green. ✅ (8.5: DATUM 1240/0/1 unchanged, dserver 1630/0, up from 1610.)
7. No new placement authority was created. ✅ (8.3: no new field, no client-suppliable authority claim; effective authority is always derived from existing server-owned state.)
8. Documentation records the discovered bug, the fix, its authority semantics, test evidence, and the optional live result. ✅ (This document, Sections 6–8.)

**Phase 115 status: CLOSED.** RPi F4–F10 completion was explicitly not a closure criterion and remains an open, optional, non-blocking follow-up.

## 10. Proposed next phase

The Phase 115 objective — a trustworthy production D-Serv observation + governed-authorization foundation — is now in place: real evidence producers (115B/115D), correct identity propagation (115C), correct readiness consumption (115E), and now correct authorization persistence/restoration under canonical D-Map authority (115, this closure). Per the standing instruction not to implement automatic recovery before this foundation is proven, the recommended next subphase is:

**Phase 116 — Multinode reconciliation dependency ordering**, scoped narrowly to: given the now-trustworthy `DServRealization`/readiness signal, define and test how one application's D-Serv/DIoT readiness gates or sequences another's governed reconciliation across nodes — read-only dependency *evaluation* only (mirroring Phase 114D2's `any_element` DForward pattern), explicitly still deferring any automatic corrective action (restart, redeploy, rollback) to a later phase once dependency evaluation itself is proven. The unrelated Stage 107A lease-cycle gate surfaced in Section 8.6 should be investigated as a small, separate, focused fix before or alongside that phase if live multinode validation is desired again — it is unrelated to Phase 115's own scope and was not investigated further here.
