# Phase 136A — Placement Authority Foundation

**Status:** backend foundation only. Not committed at time of writing. The Placement Workspace (Console UI) is Phase 136 proper and has **not** started — this phase hardens and extends the backend so that UI can be built on correct authority semantics. Acceptance/review workflow remains Phase 137.

## Purpose

Phase 135's independent-review audit found a real, pre-existing authority gap: physical D-Nodes are global (ADR-0019 — a D-Node is keyed only by `node_id`, never by project), but the structural CPU/memory *commitment* accounting used by the deterministic planner and by D-Deploy's acceptance-time capacity gate (R3C, Stage 111/ADR-0018) was **scoped to one project**. Two different projects could each independently get an accepted proposal that, combined, committed more of one shared physical node's declared capacity than it has. This phase corrects that, adds a preview-only placement-candidate endpoint Phase 136's UI needs, and leaves everything else (acceptance, review, policy) untouched.

## Authority boundary — seven states, never collapsed

| State | Source | Authority |
|---|---|---|
| planned | `core::canonical_placement::plan_canonical_placements` → `CanonicalPlacementPlanV1` | Advisory only — never calls `accept_proposal`, never mutates anything |
| proposed | `core::ddeploy::DDeployProposalV1` with `ProposalRecord.state == Pending` | Persisted, immutable, but not active authority |
| accepted | `core::ddeploy::ActiveApplicationDeployment` / `CanonicalDMapV1` via `accept_proposal` | The sole governed mutation that activates anything |
| desired runtime | `core::ddeploy::OperationalDesiredService` via `derive_application_operational_services` | Derived from the accepted D-Map only |
| observed | DMonitor's raw evidence (`core::dmonitor_state`) | Observation authority (ADR-0024), fully decoupled from placement eligibility — see below |
| healthy | DMonitor's per-evidence `health` field | Observation only |
| converged | DMonitor's `placement_convergence` (`evaluate_application_dserv_convergence`) | Observation-derived verdict, never placement authority |

Today, "planned" and "proposed" collapse into one step for the deterministic path (`POST .../plan` always immediately persists a Pending proposal) — Part C below adds a genuinely separate "planned" preview that persists nothing, so a caller can inspect a candidate without it becoming a proposed object.

## Global physical capacity accounting (the fix)

### The gap, concretely proven

`core::ddeploy::compute_project_node_commitments`/`compute_project_node_commitments_versioned` (as they existed before this phase) filtered every active application by `project_id == <the caller's project>` before summing its structural CPU/memory commitment per D-Node. The acceptance-time R3C gate (`accept_proposal`, `accept_v2_proposal`) and the planner's own commitment baseline (`post_plan_proposal`) both called this project-scoped function. Concretely: a D-Node declaring 8 CPU cores, with Project A's accepted application committing 6, would let Project B's proposal commit 6 *more* on the exact same physical node — 12 > 8 — because Project B's commitment baseline literally never saw Project A's commitment at all. This was proven directly (not assumed) by constructing exactly this scenario against the pre-fix code and observing an `Ok` acceptance; the regression tests below pin the corrected, rejecting behavior permanently.

### The fix

`compute_project_node_commitments`/`compute_project_node_commitments_versioned` are renamed to `compute_global_node_commitments`/`compute_global_node_commitments_versioned` and no longer take a `project_id` filter at all — they aggregate every currently active application's structural commitment, across **every** project, for every D-Node. The `exclude` parameter changed from a bare `application_id` to a `(project_id, application_id)` pair, since `application_id` alone is only unique *within* a project (`application_key`) — excluding by the full pair is what correctly lets a redeployment of one application exclude only its own prior allocation while still counting every other project's commitment on the same node.

A new shared function, `core::ddeploy::evaluate_global_node_capacity`, is the single source of the "does this candidate, added on top of the baseline, exceed any D-Node's declared capacity" arithmetic — reused, never duplicated, by:

- the v1 R3C acceptance gate (`accept_proposal`)
- the v2 R3C acceptance gate (`accept_v2_proposal`)
- `POST /api/v1/ddeploy/proposals/validate` (additive, see below)
- `POST /api/v1/ddeploy/proposals` (additive, see below)

The finding codes (`project_node_cpu_capacity_exceeded`/`project_node_memory_capacity_exceeded`) are **unchanged** — only their scope changed, not their wire meaning ("this D-Node's declared capacity would be exceeded"). No new finding code, no new canonical type, no new persistence.

### What this explicitly does NOT mean

**Shared ≠ unlimited, but shared ≠ exclusive either.** A node already hosting one project's accepted workload is not thereby unavailable to another project — occupancy only *consumes capacity*, it never blocks placement outright. Three different projects may simultaneously and legitimately share one physical node as long as their combined declared CPU/memory fits (proven: `three_different_projects_sharing_a_node_within_its_total_capacity_are_all_accepted`). No `ProjectResourceBinding`, `ResourceAllocation`, ownership, or tenant-allocation concept was introduced — this is exactly "global physical capacity vs. accepted structural resource requirements," nothing more.

### Commitment source

Global commitment is derived **only** from already-accepted authoritative placements (`ActiveApplicationDeployment.active_dmap.placements`, both v1 and the v2 `phase113_v2.active` store) and their accepted proposal's own immutable `dgraph_snapshot`'s declared `resource_requirement`. It never reads DMonitor, observed CPU/memory, `core::resource_observation`, or any OS/runtime utilization signal — this remains pure structural capacity accounting, exactly like Phase 135's Continuum View never invented an "available capacity" figure.

### v1/v2 duality

`compute_global_node_commitments_versioned` aggregates both `DDeployStateV1.active_by_application` (v1) and `phase113_v2.active` (v2) into **one** combined map — never double-counted (each active entry contributes exactly once, from exactly one of the two stores) and never silently blind to either (a dedicated regression, `v2_active_app_in_one_project_plus_v1_candidate_in_a_different_project_exceeding_capacity_is_rejected`, proves the v1/v2 unification fix from Phase 120B and this phase's cross-project fix compose correctly together).

## Planner

`post_plan_proposal`'s commitment baseline now calls the global function. Since the deterministic planner (`plan_canonical_placements`) already treats its `committed` parameter as "capacity already spoken for, subtract before choosing," this one change is sufficient to stop the planner from recommending a node another project has already structurally exhausted — no change to the planner's own policy/tie-break logic was needed or made.

## Manual placement draft validation (`POST .../proposals/validate`)

Previously PURE with respect to other applications' commitments — it validated a candidate's own internal shape/aggregate/eligibility only. It now **additionally** runs the shared `evaluate_global_node_capacity` check, so a caller sees capacity conflicts that are already knowable *right now*, before deciding whether to create a real proposal. The response gained one new field, `capacity_check_is_momentary`, **always present** (regardless of `valid`): this result is a read of currently accepted state, not a reservation or lease — a later `/proposals` or `/accept` call independently re-checks whatever is current at that later moment, and may still reject an already-"valid" candidate if another proposal is accepted first (R3C/R3D). No lock is held across this handler; it remains as cheap and side-effect-free as before.

## Proposal creation (`POST .../proposals`)

A proposal already known, right now, to violate global capacity is no longer persisted: after the existing `create_proposal`/`validate_full` succeeds structurally, the SAME shared capacity check runs once more against the newly-created proposal's own placements; a violation returns `422` and **`persist_state` is never reached** — nothing is written. This is advisory, not a guarantee: a proposal that passes this check can still legitimately fail the identical check again at acceptance time if a race occurs (another proposal accepted in between) — that later rejection is correct behavior, not a bug this handler could have prevented.

## Preview-only placement candidate endpoint

**`POST /api/v1/ddeploy/candidate`** — deterministic auto-placement generation using the exact same authoritative inputs `POST .../plan` uses (authoritative D-Node registry/D-Continuum, the global commitment baseline, current `ServiceArtifact` eligibility, the same deterministic planner) — with **zero persistence**: `create_proposal` is never called, nothing is written, no authority is mutated. Read-only; acquires no `DDEPLOY_TRANSACTION`/`DNODE_TRANSACTION` write lock (same unlocked-read convention as `GET .../active`).

**Request** reuses `PlanRequestPayload` verbatim (`{application_id, dgraph}`, `deny_unknown_fields`) — the caller cannot supply `placements`/`dcontinuum`/`source`/accepted authority; every one of those remains entirely server-derived, identical to `/plan`'s own contract.

**Response** (`datum.placement-candidate/1` — a descriptive label only, never a canonical wire schema, never persisted, never itself digested):

```json
{
  "schema": "datum.placement-candidate/1",
  "project_id": "...",
  "application_id": "...",
  "dgraph_ref": { "id": "...", "revision": 1, "digest": "sha256:..." },
  "dcontinuum_ref": { "id": "...", "revision": 1, "digest": "sha256:..." },
  "expected_active_dmap_revision": null,
  "placements": [{ "service_id": "...", "node_id": "..." }],
  "planner_provenance": { "planner_version": "...", "policy_name": "...", "decisions": [...] }
}
```

Deliberately carries **no `proposal_id`** (and does not reuse `ProposalRecord` identity) — nothing here was persisted, so nothing here has a proposal identity to report. `expected_active_dmap_revision` is informational context only (the CAS basis a real proposal would capture *right now*), not itself a guarantee.

**Candidate vs. `/plan` equivalence**: both handlers make the identical core call sequence — `compute_global_node_commitments_versioned` → `derive_placement_eligibility_view` → `plan_canonical_placements` — against the same authoritative state; `/plan` additionally calls `create_proposal`/`persist_state` afterward, `/candidate` never does. Proven at the core level by `candidate_preview_and_plan_produce_identical_placements_for_identical_authoritative_state` (this crate has no established precedent for invoking a full Axum handler needing `AppState`'s Firebase/GraphQL dependencies inside a unit test — see `api::ddeploy::tests`' own `production_source` doc comment — so this is the strongest automated proof available; see "Manual HTTP-level verification" below for the human-run equivalent).

## v1-only Phase 136 MVP (explicit)

Phase 136's Placement Workspace MVP operates on `datum.dmap/1` / DServ placements only. `POST /api/v1/ddeploy/candidate` and the global commitment fix are both v1/v2-*aware* (they correctly see v2 commitments so a v1 candidate is never miscalculated against a v2-occupied node), but **no v2 planner/candidate endpoint exists or is added here** — `/plan` and `/candidate` remain v1-only, matching the pre-existing gap that no `/api/v1/ddeploy/v2/plan` has ever existed. Existing v2 authority support (`GET .../v2/authority`, `POST .../v2/proposals`, `.../accept`) is completely unchanged. Phase 135's Continuum View display of existing accepted placement authority (both v1 and v2) is unaffected — Phase 135's own backend (`core::continuum_view`, `api::continuum_view`) was not touched by this phase.

## Phase 137 boundary (unchanged, restated)

This phase touches **zero** acceptance logic's outcome — `accept_proposal`/`accept_v2_proposal` still perform the identical governed mutation they always did, now simply checking against a corrected (global, not project-scoped) baseline. No review workflow, no approval policy, no new way to call `.../accept` was added. Phase 137 (Proposal/Review/Acceptance) remains entirely unaffected in scope; this phase only makes the capacity *check* those existing functions already performed honest.

## Non-goals (explicit)

No `ProjectResourceBinding`/`ResourceAllocation`/ownership/tenant-allocation model. No node exclusivity. No new canonical contract or persisted type. No DMonitor/observed-utilization input to placement. No v2 planner. No Placement Workspace UI, no Console navigation change, no drag-and-drop, no proposal-creation button — all of that remains Phase 136 proper, not started.

## Tests

- **Core (`dserver/src/core/ddeploy.rs`)**: all pre-existing R3C/same-project capacity tests remain green, proving the global rewrite preserves every existing same-project invariant exactly. Five new mandatory Phase 136A regressions: `cross_project_oversubscription_is_rejected_after_the_global_capacity_fix`, `three_different_projects_sharing_a_node_within_its_total_capacity_are_all_accepted`, `a_fourth_different_project_exceeding_the_remaining_shared_capacity_is_rejected`, `redeploying_an_application_excludes_only_its_own_old_allocation_from_the_global_baseline`, `v2_active_app_in_one_project_plus_v1_candidate_in_a_different_project_exceeding_capacity_is_rejected`. Plus `candidate_preview_and_plan_produce_identical_placements_for_identical_authoritative_state`.
- **API (`dserver/src/api/ddeploy.rs`)**: structural/text-scan tests (this crate's established pattern for HTTP handlers that need a full `AppState`) proving: the candidate endpoint reuses `PlanRequestPayload` verbatim, never calls `create_proposal`/`persist_state`/`accept_proposal`/`accept_v2_proposal`, acquires no write lock, uses the same shared global commitment function `/plan` uses, and that its response carries no proposal identity; plus that `/validate` runs the shared capacity check and always discloses the momentary-notice field, and that `/create` runs the same check *before* `persist_state` in its body.

## Manual HTTP-level verification

This crate has no established precedent for constructing a real `AppState` (Firebase client + GraphQL schema) inside an automated test, so true end-to-end HTTP verification of the new route is manual, exactly like every other Axum handler in this module (see `MANUAL_VALIDATION_COMMANDS.md`):

```bash
# 1. Point at a throwaway DB and start the server (separate shell):
DATUM_CONTROL_STATE_DB_PATH=/tmp/phase136a-manual.db cargo run --bin DServer

# 2. Declare one D-Node (8 cores):
curl -X PUT http://127.0.0.1:8080/api/v1/dnodes/shared-01 -H 'Content-Type: application/json' -d '{
  "node_id": "shared-01", "platform": {"os": "linux", "architecture": "amd64"},
  "capacity": {"cpu_cores": 8, "memory_bytes": 8589934592},
  "capabilities": {"dnode_abi_versions": ["datum-dnode/0"]}
}'

# 3. Snapshot the DB before calling /candidate:
sha256sum /tmp/phase136a-manual.db > /tmp/before.sha256

# 4. Call the preview-only candidate endpoint (project-a, a 2-core service):
curl -X POST http://127.0.0.1:8080/api/v1/ddeploy/candidate -H 'X-Datum-Project: project-a' -H 'Content-Type: application/json' -d '{
  "application_id": "app-1",
  "dgraph": {"schema": "datum.dgraph/1", "application_id": "app-1", "dgraph_id": "dgraph:app-1", "revision": 1,
    "services": [{"service_id": "svc-1", "execution_requirement": {"dnode_abi": "datum-dnode/0"}, "resource_requirement": {"cpu_cores": 2, "memory_bytes": 1}}],
    "calls": []}
}'
# Expect: 200, placements: [{"service_id":"svc-1","node_id":"shared-01"}], no proposal_id field anywhere.

# 5. Zero-persistence proof -- the DB must be byte-identical after:
sha256sum /tmp/phase136a-manual.db > /tmp/after.sha256
diff /tmp/before.sha256 /tmp/after.sha256   # must be empty

# 6. GET /api/v1/ddeploy/proposals/:id for any made-up id -- confirm no
#    proposal was created as a side effect of step 4 (404 not_found).

# 7. Candidate == /plan equivalence: call POST .../plan with the identical
#    body/header and confirm its "placements" array is byte-identical to
#    step 4's (ignoring the extra proposal_id/proposal_digest/state fields
#    /plan alone carries). /plan this time DOES persist -- the DB hash
#    will legitimately change after this call, unlike step 4.
```
