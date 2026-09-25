# Phase 115H — Post-Closure Hardening: Stage 107A Lease-Cycle Gate

**Status:** Phase 115H CLOSED. **Phase 115 itself remains CLOSED** (see
`phase115-dserv-observation-producer-closure.md`, Section 9) — this document
does not reopen or reinterpret that closure. 115H is a narrowly-scoped,
post-closure correctness task: it investigates and fixes the "separate Stage
107A lease-cycle gate" that Phase 115F's live Raspberry Pi run reached and
explicitly deferred (closure document, Section 8.6).
**Branch:** `agent/phase115-dserv-runtime`.
**Baseline:** Phase 115 closure, commit `2053216`.

This document distinguishes **implementation proof** (unit tests in this
repository), **live evidence** (the Phase 115F proof artifacts, read but not
re-executed unless stated), and **architecture inference** (conclusions drawn
from reading code). It does not mix in dependency ordering, automatic
recovery, rollback orchestration, cross-node scheduling, DScript, DCompile,
AI/GNN, or any other later-phase feature.

## 1. The live blocker, reconstructed exactly

Phase 115F's live run reached `control_plane_authorized = true` for the first
time (the Phase 115 closure fix), then the next governed native-process
reconciliation cycle was rejected with:

```
message: "blocked_authorization_not_leased: no valid Stage 107A cycle gate for this cycle"
authorization_expires_at_utc: "2026-09-25T22:15:56.384930300+00:00"
control_plane_authorized: true
execute_requested: true
host_mutation_performed: false
```
(`C:\Users\dnred\datum-115f-live\proof\09-agent-report-blocked-on-unrelated-stage107a-lease-gate.json`)

The authorization's `expires_at_utc` is in the future relative to the proof
capture, `control_plane_authorized` is `true`, and the execution-generation
and digest fields match — none of the "obvious" causes (expiry, wrong
authority tuple, stale generation) apply. The message string alone gives no
further information: it is the generic catch-all string emitted by
`native_process_runtime::reconcile_native_service` whenever the Stage 107A
cycle gate is unavailable, regardless of *why* it is unavailable.

## 2. Source audit: what actually produces that message

`operational_reconciliation.rs` computes one `stage107a_cycle_gate` per
reconciliation cycle, only when `execute_requested && control_plane_authorized`:

```rust
let stage107a_cycle_gate = if execute_requested && control_plane_authorized {
    Some(resolve_stage107a_cycle_gate(cfg, context.authorization.as_ref(), &bootstrap.control_plane.resource_id))
} else {
    None
};
```

`resolve_stage107a_cycle_gate` (same file) chains four independent checks, in
order, short-circuiting on the first failure:

1. `authorization` must be `Some` (`BLOCKED_AUTHORIZATION_NOT_LEASED` if not).
2. `check_resource_identity` — the authorization's `resource_id` must exactly
   match the agent's locally-bootstrapped `resource_id`
   (`BLOCKED_WRONG_RESOURCE_IDENTITY`).
3. `effective_pinning_mode` — the agent's configured
   `dispatch.minimum_execution_pinning_mode` may only make enforcement
   *stricter*, never weaker, than the authorization's own
   `execution_pinning_mode` (`BLOCKED_PINNING_MODE_INSUFFICIENT`).
4. `resolve_clock_skew_seconds` — **the exact root cause**: under
   `effective_mode = "required"`, `dispatch.authorization_clock_skew_seconds`
   must be explicitly configured; there is no implicit zero
   (`BLOCKED_MISSING_CLOCK_SKEW_CONFIG`, ADR-0012 design-review finding 2).
5. `LeaseGuard::from_authorization` — builds the finite-lease state from
   `persistence_scope`/`expires_at_utc` and the resolved skew.

The live `ServiceArtifact` bound `execution_pinning_mode = "required"`. The
hand-written `config-rpi-01.toml` used for the Phase 115F live run never set
`authorization_clock_skew_seconds`. Step 4 above therefore returned
`Err("blocked_missing_clock_skew_config: required mode needs an explicit
authorization_clock_skew_seconds; there is no implicit zero")` —
**this is the real, exact reason the cycle gate failed to resolve.**

That `Err` value is threaded correctly all the way to the Container-runtime
path, which already has an established pattern for surfacing it verbatim
(`stage107a_service_block_before_image`, computed as
`(Some(Err(reason)), _) => Some(reason.clone())`). The **native-process**
path, however, discarded it: `reconcile_native_service`'s match on
`(authorization, lease, effective_pinning_mode, &recomputed_digest)` had a
catch-all arm that substituted a hardcoded generic string whenever `lease`/
`effective_pinning_mode` were `None` — which happens in exactly the case
where `resolve_stage107a_cycle_gate` returned `Err`. This is what made a
correctly-fail-closed rejection look, from the live evidence alone, like an
unexplained architectural gap.

## 3. Root cause classification

Per the required classification:

- **(B) — an intentional, already-correct security invariant, invoked
  correctly, against an incompletely-configured live test environment.**
  `resolve_clock_skew_seconds` refusing to default an unconfigured skew to
  zero under `required` mode is exactly the behavior ADR-0012's design review
  demanded: an unconfigured skew is indistinguishable from claiming perfect
  clock synchronization, which the architecture must never silently assume.
  The live `config-rpi-01.toml` simply never set
  `dispatch.authorization_clock_skew_seconds`. This is a test-environment
  configuration gap, not a governance bug — no code changed to address it.
- **A genuine, narrow (A)-class bug, of a different kind:** the
  native-process path's diagnostics discarded the real block reason,
  asymmetric with the already-correct Container path. This is fixed (Section
  4) as a diagnostics-only change; it does not touch the blocking decision.

No (C), (D), (E), or other classification applies: the authorization was
current, correctly bound, correctly persisted and restored, and its lease was
not actually expired — the gate never got as far as evaluating expiry at all,
because clock-skew resolution (a prerequisite the lease guard depends on)
failed first.

## 4. Fix implemented

`DATUM/src/agent/native_process_runtime.rs`:

- `reconcile_native_service` gained one new parameter,
  `stage107a_gate_error: Option<&str>`.
- Its catch-all match arm now uses that value verbatim when present, falling
  back to the original generic string only in the (should-be-unreachable)
  case where no reason was supplied at all:

```rust
_ => (
    Some(stage107a_gate_error.map(str::to_string).unwrap_or_else(|| {
        "blocked_authorization_not_leased: no valid Stage 107A cycle gate for this cycle".to_string()
    })),
    None,
),
```

`DATUM/src/agent/operational_reconciliation.rs`:

- The one production call site now computes `stage107a_gate_error` the same
  way the Container path already computes
  `stage107a_service_block_before_image`:

```rust
let stage107a_gate_error: Option<&str> = stage107a_cycle_gate
    .as_ref()
    .and_then(|result| result.as_ref().err())
    .map(String::as_str);
```

This is a **diagnostics-only** change. `resolve_stage107a_cycle_gate`,
`LeaseGuard`, `effective_pinning_mode`, `resolve_clock_skew_seconds`, and
`check_resource_identity` are all unmodified. The blocking decision for every
scenario this document discusses is byte-for-byte identical before and after
this fix; only the *message* now names the real reason.

## 5. The Stage 107A lease contract, reconstructed from code + tests

**What LeaseGuard protects:** whether a *durable-scope* authorization's
finite lease is still current at the instant of a governed mutation boundary
(`M1`–`M8`). A `persistence_scope = "process"` authorization, or one with no
`expires_at_utc`, never has a finite lease and can never mutate a host,
regardless of `allow_host_mutation` or identity match
(`LeaseGuard::from_authorization`, `stage107a_enforcement.rs`).

**What invalidates a lease:** `now + skew >= expires_at`, checked with a
freshly-sampled wall clock at **each** mutation boundary independently — not
once per cycle and not from a cycle-wide timestamp threaded through
(`check_before_mutation` calls `Utc::now()` itself; see the P0-2 regression
tests `production_check_before_mutation_samples_the_real_clock_*` and
`sequential_boundaries_in_one_governed_intent_each_get_a_fresh_clock_sample`).
Configured skew only ever shortens the effective window, never lengthens it.

**One governed mutation cycle vs. authorization lifetime — these are
deliberately separate concepts, not conflated anywhere in this codebase:**

- A "cycle" in the observation/telemetry sense (`core/cycle.rs`'s
  `cycle_id`, a fresh UUID per SmartSentinel observation) is a property of
  the *DMonitor telemetry* model. `OperationalReconciliationAuthorization`
  (the Stage 107A authorization struct) has **no `cycle_id` field at all** —
  grep confirms it appears nowhere in that type or in `LeaseGuard`.
- `resolve_stage107a_cycle_gate` is a pure function of
  `(cfg, authorization, local_resource_id)` alone. It has no cycle counter,
  no memory of a previous call, and no way to distinguish "the 1st
  reconciliation attempt" from "the 100th" — by construction (referential
  transparency), calling it twice with the same still-current authorization
  and the same config necessarily produces the same result both times.
- Each top-level reconciliation invocation (one HTTP fetch of
  `operational-reconciliation-context`, one pass over desired services) is
  itself "one cycle" in the reconciliation sense. `stage107a_cycle_gate` is
  recomputed fresh, from whatever authorization DServer is *currently*
  serving, at the start of every such cycle (`operational_reconciliation.rs`
  line ~1323).

**The derived contract, therefore:** a durable authorization with a
still-unexpired lease is **reusable across multiple reconciliation cycles**,
exactly as `persistence_scope = "durable"` is designed to allow. Cycle
identity has no bearing on validity. What *does* end a still-current
authorization's usable lifetime, before its own `expires_at_utc`, is anything
that makes it no longer the CURRENT authority for the context: DServer
serving a different (or no) authorization on a later fetch, `control_plane_authorized`
failing to hold (operation/`allow_host_mutation`/`project_id`/`dgraph_id`/
`dgraph_revision`/`node_id` no longer matching — `operational_reconciliation.rs`
line ~1304), a resource-identity mismatch, an insufficient pinning mode, or
(this section's own subject) an unresolvable clock-skew configuration. This
was not assumed going in — it follows directly from `resolve_stage107a_cycle_gate`
having no cycle-scoped state and `control_plane_authorized` being the actual,
independently-recomputed-every-cycle "is this authority still current" check.

## 6. Security invariants explicitly preserved

None of the following were done, per the standing constraint:

- Lease validation was not disabled, shortened, or bypassed.
- No lease was extended or treated as non-expiring.
- No expired lease is now accepted.
- `LeaseGuard`/`resolve_stage107a_cycle_gate` were not touched.
- Durable authorization was not converted into permanent authority.
- Persisted authorization is still independently revalidated at each cycle
  (Section 5) — nothing is trusted merely because it was durable/persisted.
- Execution-cycle / mutation-boundary checks (`check_before_mutation`) are
  unchanged, still evaluated fresh at each boundary.
- Required-mode pinning is still enforced exactly as before.
- Resource binding matching (`check_resource_identity`) is unchanged.
- No `rpi-01`, node, or project special-case was added.
- No environment variable or config flag disables any governance check —
  `authorization_clock_skew_seconds` is a pre-existing, already-required
  (under `required` mode) configuration key; setting it correctly in a test
  environment is not a governance weakening.

## 7. Tests

Two new focused tests were added to
`native_process_runtime.rs`'s existing test module, exercising the exact new
code path (the `stage107a_gate_error` → `generation_projection_block` glue):

- `stage107a_gate_error_is_surfaced_verbatim_and_still_blocks_mutation` — with
  `stage107a_gate_error = Some(<the real blocked_missing_clock_skew_config
  string>)`, asserts the final report's `message` equals that string exactly,
  `execution_status == NATIVE_PROCESS_OBSERVED_ONLY`, and
  `host_mutation_performed == false`.
- `absent_stage107a_gate_error_falls_back_to_the_generic_message_and_still_blocks`
  — with `stage107a_gate_error = None`, asserts the original generic fallback
  string is used, and the same no-mutation outcome holds.

Both are `#[cfg(target_os = "linux")]`-gated, consistent with this file's
pre-existing, explicitly-documented precedent: `reconcile_native_service`
calls the free function `platform_supported()` (`cfg!(target_os = "linux")`,
not injectable), so on a non-Linux host every cycle short-circuits to
`NativeReconciliationAction::ObserveOnly { reason:
NATIVE_PROCESS_RUNTIME_UNSUPPORTED_PLATFORM }` before ever reaching the
lease/generation match arm this fix touches. This mirrors the same
already-disclosed limitation covering the rest of this file's mutation-path
tests (see `linux_integration_tests`, `platform-test-coverage-audit.txt`).

**The remaining focused-matrix items (L1–L8, L10, L11) already have deep,
pre-existing coverage** in `stage107a_enforcement.rs`'s own test module,
none of which this task modified:

| Item | Existing coverage |
|---|---|
| L1 valid lease → allowed | `durable_with_expiry_is_valid_before_expiry` |
| L2 expired lease → blocked | `durable_lease_expired_blocks_next_boundary`, `production_check_before_mutation_samples_the_real_clock_expired_case` |
| L3 no `expires_at_utc` → blocked | `process_scope_never_has_finite_lease` |
| L4 wrong operation → blocked | `control_plane_authorized`'s exact-match `authorization.operation == expected_authorization_operation` (`operational_reconciliation.rs` ~1306) — simple boolean AND, out of Stage 107A's own scope (this gates *before* the cycle gate is even computed) |
| L5 `allow_host_mutation=false` → blocked | same `control_plane_authorized` computation, `authorization.allow_host_mutation` term |
| L6 binding/resource changed → blocked | `mismatched_resource_id_blocks`, `missing_resource_id_blocks` |
| L7 D-Map/revision changed → blocked | `generation_mismatch_blocks`, and `control_plane_authorized`'s `authorization.dgraph_revision == Some(context.dgraph_revision)` term |
| L8 artifact projection changed, required mode → blocked | `recomputed_digest_mismatch_blocks`, `agent_side_bare_image_is_always_a_gap` and siblings (`required_completeness_gaps`) |
| L9 cycle changes, same authority tuple current | See Section 5 — proven by construction (pure function, no cycle-scoped state), not by a redundant runtime test |
| L10 `mutation_boundaries_crossed[]` accuracy | Pre-existing, unmodified; covered by this file's existing M1–M8 boundary-record tests |
| L11 no mutation when unauthorized | Both new tests above assert `host_mutation_performed == false`; pre-existing coverage throughout this file's `ObserveOnly`/`decide_native_reconciliation_action` tests |

L4–L7's `control_plane_authorized` computation is pre-existing code from the
Phase 115 closure fix, upstream of and outside Stage 107A's own scope (it
gates whether the cycle gate is even computed at all); 115H does not modify
or re-test it, consistent with the instruction not to reopen Phase 115's own
closed scope.

## 8. Test results

- **DATUM lib** (`cargo test --locked --lib --features dcall-test-support`,
  Windows host): **1240 passed; 0 failed; 1 ignored** — unchanged from the
  Phase 115 closure baseline. The two new Phase 115H tests are
  `#[cfg(target_os = "linux")]`-gated and are therefore compiled but not run
  on this host (Section 7); they have not yet been executed on Linux in this
  session.
- **dserver** (`cargo test --locked`): **1630 passed; 0 failed** — unchanged;
  dserver was not modified by this task.
- `cargo build --locked --release` (DATUM): succeeded, exit code 0. 86
  pre-existing warnings (dead-code/unused-function lints in unrelated code),
  none newly introduced by this change.
- `git diff --check`: clean.
- `cargo fmt --check`: both modified files (`native_process_runtime.rs`,
  `operational_reconciliation.rs`) already had extensive pre-existing
  formatting deviations throughout, unrelated to this change (consistent
  with this repository's established practice of not blanket-reformatting
  pre-existing files). The code this task added follows the exact
  argument-list style already used at the four pre-existing call sites of
  `reconcile_native_service` it touches.

## 9. Optional live re-validation

Not performed as part of closing this task in this session; the Pi's
`smartsentinel` binary and `config-rpi-01.toml` were not touched. Attempting
F4 onward would require, at minimum, setting
`dispatch.authorization_clock_skew_seconds` in `config-rpi-01.toml` (a test
environment fix, not a code change — Section 3) and rebuilding/redeploying
the agent binary with this fix so the *next* rejection, if any, reports its
real reason. This remains available as a future optional step; per the
Phase 115 closure precedent, live Pi success is not a closure criterion for
115H either.

## 10. Remaining gaps

- The two new Linux-gated tests have not been executed on a real Linux host
  in this session (only compiled). They are expected to pass given the
  traced code path, but this is stated honestly rather than claimed as
  verified.
- Optional live re-validation (Section 9) was not attempted.
- L4–L7's `control_plane_authorized` computation, while simple and already
  covered by the reasoning in Section 7, has no dedicated unit test in this
  repository (it lives inside a larger HTTP-calling function that is
  architecturally difficult to unit-test in isolation, per the Phase 115
  closure document's own note on the Resource Registry's process-global
  `OnceLock`). This is pre-existing, out of 115H's scope, and unchanged by
  this task.

## 11. Phase 115H closure criteria — checked

1. Exact blocker understood, not merely described. ✅ (Section 1–3.)
2. Intended lease contract proven from architecture, not assumed. ✅ (Section 5.)
3. Implementation matches that contract. ✅ (Section 4 changes nothing about the contract, only its diagnostics.)
4. A valid mutation can cross the governed boundary given a correctly-configured environment. ✅ (Section 3: the live blocker's true cause — an unconfigured skew under `required` mode — is a configuration gap, not a code defect; the code path is unchanged and was already correct.)
5. Expired/stale/mismatched authority still fails closed. ✅ (Section 6; no gating logic touched.)
6. Regression green. ✅ (Section 8.)
7. No new authority introduced. ✅ (Section 6.)

**Phase 115H status: CLOSED.**
