# Phase 119H1 — Minimal Reconciliation Bootstrap Decoupling Fix

- Status: Small foundation fix. Local (non-live) only — no RPi/fog/cloud
  infrastructure used.
- Branch: `agent/phase119-migration`, continuing from Phase 119H0's audit,
  HEAD `e1f146a583707136b1000b3455e6433f7240b715`.
- Scope: implement exactly the two graceful-degradation changes Phase 119H0
  §9 recommended, and no more — remove the unnecessary fatal dependency on
  the two legacy Firebase-backed bootstrap *enrichment* reads, while every
  authoritative fact (Resource Registry binding, requested/assigned-node
  conflict detection, Stage107A authorization) remains exactly as strict as
  before.

## 1. The exact defect (re-confirmed against current source)

`run_bootstrap_with_resource_discovery` (`DATUM/src/agent/bootstrap.rs`)
made two calls whose *success* was never actually required once a Resource
Registry binding exists, and one whose result is never consumed at all —
yet both were wired with `?`, so any non-404 failure (a Firebase outage on
DServer surfaced as a 500, a transport error, a decode error) aborted the
**entire** bootstrap call, for `reconcile`, `rollback`, and `cleanup`
identically:

1. `query_node_context` (`GET /api/v1/nodes/:id/latest`, legacy
   Firebase-backed server-side via `state.firebase.list_nodes`/
   `get_latest_snapshot`) — contributes `node_known`/`latest_snapshot`.
2. `query_optional_capabilities` (`GET /api/v1/nodes/:id/capabilities`,
   also Firebase-backed via `state.firebase.get_node_capabilities`) —
   contributes `expected_capabilities`, confirmed by Phase 119H0 to be
   **never read** by reconciliation in any case.

## 2. Authority reasoning (unchanged from Phase 119H0, re-verified)

- `resource_id` is derived purely locally (`derive_resource_lookup_id`,
  hardware/OS fingerprint) and independently confirmed against the
  Resource Registry (`GET /api/v1/resources/:resource_id`,
  `resource_state_store()`) — both already Firebase-free, both **untouched**
  by this fix.
- `classify_bootstrap_state` already treats `resource_has_assigned_node`
  **alone** as sufficient for `KnownBoundNode` — no dependency on
  `node_known`.
- `resolved_node_id` already falls back to `registry_assigned_node_id`
  whenever `node_known` is `false`.
- The requested-vs-assigned conflict check (`bootstrap.rs`, unchanged)
  still runs **before** any degradation logic and still fails closed on a
  mismatch, regardless of Firebase availability.

Nothing in this fix changes *who* is authoritative for anything. It only
changes what happens when a non-authoritative, already-redundant-or-unused
call fails.

## 3. Graceful-degradation policy — exact implementation

Two small, pure, directly-testable helper functions in
`DATUM/src/agent/bootstrap.rs`:

```rust
fn resolve_node_context_enrichment(
    registry_binding_present: bool,
    query_result: Result<(bool, Option<Value>)>,
) -> Result<(bool, Option<Value>)> {
    match query_result {
        Ok(result) => Ok(result),
        Err(_) if registry_binding_present => Ok((false, None)),
        Err(error) => Err(error),
    }
}

fn capability_enrichment_or_none(query_result: Result<Option<Value>>) -> Option<Value> {
    query_result.unwrap_or(None)
}
```

Wired at the two call sites, replacing bare `?`:

```rust
let node_context_query = match node_candidate.as_deref() {
    Some(node_id) => query_node_context(&client, base, project_id, node_id),
    None => Ok((false, None)),
};
let (node_known, latest_snapshot) =
    resolve_node_context_enrichment(registry_assigned_node_id.is_some(), node_context_query)?;
```

```rust
let (expected_capabilities, sentinel_registration) = match resolved_node_id.as_deref() {
    Some(node_id) => (
        capability_enrichment_or_none(query_optional_capabilities(&client, base, project_id, node_id)),
        query_optional_registration(&client, base, node_id)?,   // UNCHANGED
    ),
    None => (None, None),
};
```

## 4. Load-bearing vs. enrichment — the precise rule

| Call | Registry binding present | Registry binding absent |
|---|---|---|
| `query_node_context` (node/latest) | **Enrichment.** Failure degrades to `(false, None)`; bootstrap continues; `KnownBoundNode` is still reached via the binding alone. | **Load-bearing.** Failure propagates exactly as before — this remains the only Firebase-free-or-not signal available for a not-yet-bound resource. |
| `query_optional_capabilities` (capabilities) | **Never load-bearing, in either binding state.** Failure always degrades to `None`; `expected_capabilities` is confirmed unconsumed by reconciliation in every case (Phase 119H0 §6). | Same — always degrades to `None`. |
| `query_optional_registration` (Sentinel registration) | **Unchanged.** Still `?`. Phase 119H0 found this endpoint already Firebase-free (`registration_store()`); Phase 119H1 does not broaden scope to it, per its own task instruction. No compelling reason to change it was found during this fix — its failure mode is unrelated to the Firebase decoupling this phase targets. | Unchanged. |
| `GET /api/v1/resources/:resource_id` (resource lookup) | **Always load-bearing**, unconditionally, in every case — untouched by this fix. | Same. |

## 5. No-binding case remains fail-closed

When `registry_assigned_node_id` is `None` (a not-yet-bound resource,
whether or not `--node-id` was supplied), `resolve_node_context_enrichment`'s
`registry_binding_present` argument is `false`, so any `query_node_context`
failure propagates via `Err(error) => Err(error)` exactly as before — the
whole bootstrap call fails. **A requested `--node-id` alone, combined with
a failed or absent node-context lookup and no Resource Registry binding,
can never become `KnownBoundNode`** — verified directly by
`no_binding_plus_firebase_backed_node_lookup_failure_still_propagates_as_an_error`.
A plain 404 (node genuinely unknown to the legacy snapshot mechanism) still
correctly yields `KnownUnboundResource` rather than `KnownBoundNode`,
unchanged (`b5_no_binding_plus_404_context_never_reaches_known_bound_node_from_requested_node_alone`).

## 6. Requested-vs-assigned conflict — unchanged, re-verified

The existing conflict check (`if requested != assigned { bail!(...) }`)
runs **before** either new helper is ever reached, and is completely
untouched by this fix. Re-verified end-to-end with a mock DServer
(`b7_requested_node_id_conflicting_with_registry_binding_still_fails_closed`):
a Resource Registry binding to `fog-119h1` plus a requested `--node-id
rpi-119h1` still fails closed with the exact existing "conflicting role"
message, even when the mock server would otherwise happily answer every
other endpoint.

## 7. Stage107A — unaffected

No file outside `DATUM/src/agent/bootstrap.rs` was touched. No DServer
source was touched. `resolve_stage107a_cycle_gate`'s resource-identity
check and `authorize_operational_reconciliation`/`authorize_operational_cleanup`'s
fresh `active_resource_binding_for_node` re-fetch (both downstream of
bootstrap, both already independent of anything bootstrap enriches) are
byte-for-byte unchanged. No authorization operation, lease, pinning mode,
clock-skew, or M1-M8 gate was touched.

## 8. Reconcile/rollback/cleanup parity

All three still share the identical `run_operational_reconciliation` →
`run_bootstrap` → `run_bootstrap_with_resource_discovery` →
`run_operational_reconciliation_from_bootstrap` chain, unmodified in
structure. The fix lives entirely inside the one shared bootstrap function,
so it benefits — and is exercised identically by — all three modes with no
mode-specific branching added anywhere.

## 9. Tests

12 new tests added to `DATUM/src/agent/bootstrap.rs`, `mod tests`:

- **Pure policy tests** (B1, B3, B4, B6 for `resolve_node_context_enrichment`;
  B8, B9, B10 for `capability_enrichment_or_none`): each Ok/Err ×
  binding-present/absent combination, isolated from any network I/O.
- **End-to-end classification tests** (B2, B5): the degraded tuple threaded
  through the real, unmodified `classify_bootstrap_state`, proving the
  final `BootstrapState` outcome, not merely the intermediate value.
- **Mock-DServer integration tests** (task §21/§22), using a small,
  path-routed extension of this crate's existing established
  `TcpListener`-based mock-server pattern (already used by
  `outputs::dmonitor_http::tests` / `dcode::client::tests` — no new test
  framework or dependency added):
  - `task_section_21_binding_present_with_firebase_backed_endpoints_failing_still_yields_known_bound_node`
    — a real call to `run_bootstrap_with_resource_discovery` against a mock
    server that returns a valid Resource Registry binding but **500s both**
    `/nodes/:id/latest` and `/nodes/:id/capabilities`, still yields
    `BootstrapState::KnownBoundNode`, the correct `resolved_node_id` and
    `resource_id`, `node_known == false`, both enrichment fields `None`,
    and `side_effects_performed == false`.
  - `b7_requested_node_id_conflicting_with_registry_binding_still_fails_closed`.
  - `no_binding_plus_firebase_backed_node_lookup_failure_still_propagates_as_an_error`.

**Results**: focused (`bootstrap` filter): 13/13 pass. Full DATUM lib suite:
1251/1251 pass (1239 baseline + 12 new), zero regressions, 1 pre-existing
ignore. Full DATUM test suite (all binaries/integration tests): all pass,
zero failures. `--release` build succeeds. `git diff --check`: clean.

## 10. Live prerequisites for Phase 119H (unchanged from Phase 119H0 §14, now item 1 complete)

1. ~~Implement the two-call graceful-degradation fix~~ — **done, this
   phase.**
2. Confirm the RPi (`rpi-119d`)/fog-01 (`fog-119d`) resources from
   Phase 119D each already have (or can trivially be given, via the
   existing `POST /api/v1/resources/:resource_id/binding` endpoint) a
   Resource Registry binding.
3. Re-run `smartsentinel-operational-reconcile --execute` and
   `smartsentinel-operational-reconcile --cleanup --execute` against the
   same isolated DServer topology Phase 119D used, now expecting bootstrap
   to succeed past the point that previously blocked it.
4. Only then power on RPi/fog for the full live A→B→A migration + governed
   source-cleanup re-proof.
