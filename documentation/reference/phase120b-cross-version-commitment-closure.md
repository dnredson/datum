# Phase 120B — Cross-Version Structural Commitment Accounting Closure

- Status: **CLOSED. No known Phase 120 blocker remains.**
- Branch: `agent/phase120-deterministic-placement`, continuing from Phase
  120A's closure-audit commit `c2df55a5a978e24fd42dfac9293c0c109f2f7503`.
  `origin/main` confirmed unchanged at `ff72b2b6401b37564a4fe949eb19a9c15c0b4326`
  throughout.
- Scope: fix exactly the narrow gap Phase 120A identified — two call
  sites (`post_plan_proposal`, DMap/1 `accept_proposal`'s R3C capacity
  gate) using the DMap/1-only `compute_project_node_commitments` instead
  of the correct, already-existing, already-tested
  `compute_project_node_commitments_versioned`. No new placement policy,
  no DMap/2 planner, no DIoT planner, no live infrastructure.

## 1. Phase 120A blocker (recap)

Two commitment-aggregation functions exist in `core::ddeploy`:
`compute_project_node_commitments` (DMap/1-only, structurally incapable
of reflecting DMap/2 authority) and `compute_project_node_commitments_versioned`
(the correct aggregator, adding `state.phase113_v2.active` commitments on
top). The DMap/2 acceptance path already used the versioned aggregator
correctly. Two call sites did not: `post_plan_proposal`'s own
residual-capacity baseline, and DMap/1 `accept_proposal`'s own R3C
capacity gate. Both were structurally blind to any active DMap/2
application's structural footprint on a shared node — a v1 proposal
(planned or otherwise) could be accepted against a node whose capacity
was already fully consumed by an active v2 application.

## 2. Root cause

Not a logic bug in the versioned function itself (which was already
correct and already proven safe for the reverse direction) — a wiring
gap. Two call sites were written against the DMap/1-only sibling
function, likely because the versioned function did not exist (or DMap/2
did not yet exist) when those call sites were first written, and no
later phase updated them when DMap/2's parallel acceptance path was
introduced. `PlannerProvenanceV1.committed_by_node`'s own doc comment
explicitly named the non-versioned function as its documented source —
confirming this was a consistent, load-bearing (if incorrect) assumption
across the planner's own persisted evidence, not an isolated slip.

## 3. Planner fix

`dserver/src/api/ddeploy.rs`, `post_plan_proposal`:

Before:
```rust
let commitments = compute_project_node_commitments(&project_id, &ddeploy_state.active_by_application, &ddeploy_state.proposals, Some(&payload.application_id))
    .map_err(internal_error)?;
```

After:
```rust
let commitments = compute_project_node_commitments_versioned(&project_id, &ddeploy_state, Some(&payload.application_id))
    .map_err(internal_error)?;
```

This is a direct, semantically-equivalent-but-correct swap: the versioned
function subsumes the non-versioned one's DMap/1-only behavior and adds
DMap/2 awareness on top, with the same `exclude_application_id` semantics
(this application's own prior authority, under either version, excluded
from the "other applications" baseline). The import list in
`api/ddeploy.rs` was updated accordingly (the non-versioned function was
otherwise unused in that file after this change).

## 4. v1 acceptance fix

`dserver/src/core/ddeploy.rs`, `accept_proposal`'s R3C gate.

Before:
```rust
let candidate_commitments = compute_project_node_commitments(&record.proposal.project_id, &updated_active, &state.proposals, None)
    .map_err(AcceptError::Internal)?;
```

After (abbreviated; full code mirrors the DMap/2 acceptance path's own
established pattern at `accept_with_previous`):
```rust
let mut candidate_commitments = compute_project_node_commitments_versioned(
    &record.proposal.project_id, state, Some(&record.proposal.application_id),
).map_err(AcceptError::Internal)?;
let candidate_services: BTreeMap<&str, &CanonicalDServV1> = record.dgraph_snapshot.services
    .iter().map(|s| (s.service_id.as_str(), s)).collect();
for placement in &record.proposal.placements {
    let service = candidate_services.get(placement.service_id.as_str())
        .ok_or_else(|| AcceptError::Internal(/* ... */))?;
    let entry = candidate_commitments.entry(placement.node_id.clone()).or_default();
    entry.cpu_cores = entry.cpu_cores.checked_add(service.resource_requirement.cpu_cores as u64)
        .ok_or_else(|| AcceptError::Internal(/* ... */))?;
    entry.memory_bytes = entry.memory_bytes.checked_add(service.resource_requirement.memory_bytes)
        .ok_or_else(|| AcceptError::Internal(/* ... */))?;
}
```
The subsequent per-node capacity comparison against
`authoritative_dcontinuum.nodes`, and the `F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED`/
`F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED` findings it raises, are
unchanged.

## 5. Why `accept_proposal` was not a simple function swap

`compute_project_node_commitments_versioned(project_id, state, None)`
could not directly replace the old call, because `state` at this point in
`accept_proposal` is still the **pre-acceptance** state — it does not yet
contain this candidate's own new active entry (that only happens later,
at `state.active_by_application = updated_active` on the commit path).
The previous code worked around this by deriving its baseline from
`updated_active` (a clone of `state.active_by_application` with this
candidate's new entry already inserted, replacing any prior entry for the
same application) — which correctly counted the candidate exactly once,
but was DMap/1-only by construction, since `updated_active`'s type cannot
hold a DMap/2 entry. Simply pointing the versioned function at `state`
with `None` excluded would have double-omitted the candidate (it isn't in
`state` yet) rather than including it once. The correct fix — compute the
"every OTHER application, across both versions" baseline via the
versioned function with this application explicitly excluded, then add
the candidate's own placements on top exactly once — mirrors the pattern
the DMap/2 acceptance path already used correctly, and is the pattern now
shared (though not code-deduplicated; each path adds its own candidate's
placements from its own candidate source, `record.proposal.placements`).

## 6. Same-application replacement semantics

`exclude_application_id = Some(&record.proposal.application_id)` remains
load-bearing in the corrected gate: a redeployment of the SAME
application (whether previously active under DMap/1 or, structurally,
DMap/2 — though a DMap/2-to-DMap/1 transition is separately blocked by
Phase 119G's downgrade guard before this gate is ever reached) excludes
that application's own prior commitment before its new candidate
commitment is added once. The pre-existing regression
`redeploying_the_same_application_does_not_double_count_its_own_prior_active_allocation`
was re-run unmodified and still passes, confirming no double-counting was
introduced by this fix.

## 7. Cross-version capacity matrix

| Active (other app) | Candidate (this app) | Coverage |
|---|---|---|
| DMap/1 | DMap/1 | Pre-existing (`second_acceptance_exceeding_project_cpu_capacity_fails_closed`, `two_active_applications_whose_summed_demand_fits_are_both_legal`) — unaffected |
| DMap/2 | DMap/1 | **New this phase** (`v2_active_app_plus_v1_candidate_exceeding_cpu_capacity_is_rejected`, `..._memory_...`, `..._within_capacity_is_accepted`) — this was the blocker |
| DMap/1 | DMap/2 | Pre-existing (`v1_active_app_plus_v2_candidate_exceeding_capacity_is_rejected`) — already correct, unaffected |
| DMap/2 | DMap/2 | Pre-existing (`v2_active_app_plus_v2_candidate_exceeding_capacity_is_rejected`) — already correct, unaffected |

All four combinations now have direct, passing regression coverage.

## 8. Provenance correction

`PlannerProvenanceV1.committed_by_node`'s doc comment, which previously
named `compute_project_node_commitments` as its source, now correctly
names `compute_project_node_commitments_versioned` and describes the
field as the exact version-aware baseline supplied to the planner. This
is proven, not merely asserted: `planner_sees_active_v2_commitment_and_persists_it_in_provenance`
persists a real `ProposalRecord` via `create_proposal` with a genuine
DMap/2 active commitment in its baseline, then reads back
`record.planner_provenance.committed_by_node["a-node"].cpu_cores == 3`
from the persisted result, not from the in-memory plan alone.

## 9. Tests

**New (`core::ddeploy`, 5):**
- `v2_active_app_plus_v1_candidate_exceeding_cpu_capacity_is_rejected` —
  the exact Phase 120A counterexample; also asserts the candidate
  proposal remains Pending, v2 active authority is untouched, and no v1
  active authority is created.
- `v2_active_app_plus_v1_candidate_exceeding_memory_capacity_is_rejected`
  — memory-dimension equivalent, CPU deliberately non-limiting.
- `v2_active_app_plus_v1_candidate_within_capacity_is_accepted` —
  positive control: v2 occupancy is capacity consumption, not an
  absolute block.
- `planner_sees_active_v2_commitment_and_persists_it_in_provenance` —
  exercises the real `compute_project_node_commitments_versioned` ->
  `plan_canonical_placements` -> `create_proposal` sequence
  `post_plan_proposal` itself performs (two nodes, a DMap/2 app consuming
  3/4 cores on `a-node`, a new 2-core service correctly lands on
  `b-node`), then proves the PERSISTED provenance carries the DMap/2
  commitment.
- `planner_fails_closed_and_creates_no_proposal_when_v2_commitment_exhausts_node`
  — a fully-DMap/2-consumed node correctly yields `unsatisfied_static_capacity`
  and no proposal is ever created.

**Unmodified, re-run, still green:** all 286 pre-existing `core::ddeploy`
tests, all 23 `core::canonical_placement` tests (the pure planner itself
required no change), all 24 `api::ddeploy` tests, and specifically
`redeploying_the_same_application_does_not_double_count_its_own_prior_active_allocation`
and `v1_create_proposal_fails_closed_when_application_is_currently_v2_active`
(same-application and self-downgrade semantics, explicitly re-confirmed
unaffected).

## 10. Remaining explicit limitations (unchanged from Phase 120A, not regressions)

- An application currently active under DMap/2 still cannot plan or
  accept a DMap/1 proposal for itself (`F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED`,
  Phase 119G, inherited by `post_plan_proposal` via `create_proposal`).
  This is an intentional, separate scope boundary — a v2-active
  application's relationship to its OWN prior authority — not addressed
  or relaxed by this phase, and not required to be per Phase 120A §13.
- No DMap/2-aware deterministic proposal PRODUCER was added — the
  planner still only ever produces DMap/1 candidate proposals. Only its
  CAPACITY AWARENESS of active DMap/2 commitments (from other
  applications) was fixed.
- No DIoT placement was added.
- No telemetry/AI/GNN placement was added; the seam for a future
  learned proposal producer (`ProposalSource::Ai`) remains as documented
  in Phase 120A, unchanged.
- No migration-specific planner input (`exclude_node`/`source_node`/
  `force_move`) was added; the planner remains an ordinary,
  non-migration-aware proposal producer.

## 11. Whether any Phase 120 blocker remains

**No.** Re-assessed directly against Phase 120A's own closure criteria:

- Can an active DMap/2 application remain invisible to `POST /ddeploy/plan`
  structural capacity? **No** — fixed, proven by
  `planner_sees_active_v2_commitment_and_persists_it_in_provenance` and
  `planner_fails_closed_and_creates_no_proposal_when_v2_commitment_exhausts_node`.
- Can an active DMap/2 application remain invisible to a DMap/1
  acceptance-time capacity gate? **No** — fixed, proven by
  `v2_active_app_plus_v1_candidate_exceeding_cpu_capacity_is_rejected`
  and its memory equivalent.
- Can a DMap/1 candidate be double-counted against its own previous
  active revision? **No** — `exclude_application_id` remains load-bearing
  in both corrected call sites; pre-existing redeploy regression still
  passes unmodified.
- Are v1->v1, v2->v1, v1->v2, and v2->v2 combinations all safe? **Yes**
  — all four now have direct, passing regression coverage (§7).

No further Phase 120 closure-critical gap was identified. Phase 120 —
deterministic placement — is closed.

---

PHASE 120B CROSS-VERSION COMMITMENT CLOSURE COMPLETE.

PHASE 120 — DETERMINISTIC PLACEMENT — CLOSED.

DETERMINISTIC PLANNING AND ACCEPTANCE NOW ACCOUNT FOR
ACTIVE STRUCTURAL COMMITMENTS ACROSS BOTH DMAP/1 AND DMAP/2.

THE PLANNER REMAINS PURE AND NON-AUTHORITATIVE;
ONLY EXPLICIT DDEPLOY ACCEPTANCE CREATES ACTIVE PLACEMENT AUTHORITY.

PHASE 121 — GNN — IS NEXT.
