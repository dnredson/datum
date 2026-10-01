# Phase 120A — Independent Deterministic Placement Current-State and Closure Audit

- Status: **VERDICT B — MINOR CLOSURE WORK REMAINS.**
- Branch: `agent/phase120-deterministic-placement`, created from
  `agent/phase119-migration` HEAD `c08ccc4d715d9eb76644e3f1bd7d80a7945265c4`
  (Phase 119I's independent final migration closure commit). `origin/main`
  confirmed unchanged at `ff72b2b6401b37564a4fe949eb19a9c15c0b4326` throughout.
  This document is the only change on this branch; no production source was
  modified, per this task's own "AUDIT / REVIEW FIRST" instruction.
- Scope: independently determine whether roadmap Phase 120 (deterministic
  placement) is already implemented and closable, given that
  `core::canonical_placement`, ADR-0020, `POST /api/v1/ddeploy/plan`,
  planner provenance, inter-application commitment accounting, and
  ServiceArtifact eligibility all already exist in the repository — or
  whether later architectural evolution (DMap/2 in particular) left a
  specific remaining gap.

## 1. Executive verdict

**VERDICT B.** The deterministic placement foundation — the pure policy
(`plan_canonical_placements`), its four genuine inputs, its provenance,
its HTTP hardening (source/dcontinuum/placements not caller-suppliable,
`deny_unknown_fields`), its eligibility integration, and its inheritance of
Phase 119G's v1-downgrade-from-v2 guard — is correct, deterministic, pure,
and well-tested. However, this audit found one concrete, source-confirmed,
uncompensated gap in **cross-version (DMap/1 vs DMap/2) structural
commitment accounting**, present at exactly two call sites, that allows a
v1 acceptance (whether deterministically planned or not) to be granted
against a node whose capacity is already consumed by an *active v2*
application, invisibly. This is a real safety gap under this task's own
explicit closure criteria (§16/§18 of the task), not a hypothetical —
but the correct, already-tested fix already exists in the codebase
(`compute_project_node_commitments_versioned`) and is not architecturally
novel: closing it is a small, well-bounded wiring change at two call
sites, not new design work. That combination — a real but narrow,
already-solved-in-principle gap in an otherwise sound and complete
foundation — is exactly VERDICT B's definition, not VERDICT C's.

## 2. Reconstructed Phase 120 responsibility

Given canonical D-Graph + authoritative structural D-Continuum + current
project-wide structural commitments + current ServiceArtifact placement
eligibility, deterministically produce non-authoritative candidate D-Serv
placements feeding the ordinary DDeploy proposal path. DDeploy validation
remains authoritative; explicit acceptance remains required. This matches
the task's own candidate definition, and matches what
`core::canonical_placement`'s own module doc comment (lines 1–50) already
states almost verbatim, independently arrived at before re-reading that
comment.

## 3. Inventory — `core::canonical_placement` (read in full, 839 lines)

- `NodeCommitmentV1 { cpu_cores: u64, memory_bytes: u64 }` — plain data,
  never derived by this module itself.
- `PlacementEligibilityView` — `BTreeMap<String, BTreeSet<String>>`
  wrapper preserving the absent-vs-present-empty distinction; `unrestricted()`,
  `insert`, `eligible_nodes_for`.
- `PlacementDecisionRecord` — one per placement, records
  service_id/node_id/required ABI/eligible_node_count/residual
  before-and-after for both CPU and memory.
- `PlacementProvenance` — planner/policy identity, dgraph/dcontinuum
  id/revision/digest, `decisions`. Explicitly documented (C1E correction)
  as insufficient alone to *reproduce* a past decision, since `committed`/
  `eligibility` are not retained here (only in the persisted
  `PlannerProvenanceV1` superset).
- `CanonicalPlacementPlanV1 { placements, provenance }` — ephemeral
  handoff value, never itself a canonical wire contract or digested object.
- `plan_canonical_placements(dgraph, dcontinuum, committed, eligibility) ->
  Result<CanonicalPlacementPlanV1, Vec<CanonicalValidationFinding>>` — the
  sole entry point.

## 4. Planner purity — structurally confirmed

`canonical_placement.rs`'s only imports are `canonical_dcontinuum`,
`canonical_dgraph`, `canonical_dmap`, `std::collections`, `serde`. There is
no import of `ddeploy`, `service_artifact_registry`, `dnode_registry`,
`dmonitor`, any HTTP/network/filesystem type, or `accept_proposal`
anywhere in the file. Every function takes only shared references and
returns a value; there is no `&mut` parameter, no database handle, no
`Instant`/`SystemTime`/RNG use. This is a structural guarantee, not merely
a documented one — re-confirmed by direct read of the full file, not
inherited from ADR-0020's prose.

## 5. Deterministic policy — re-verified against current code

`PLACEMENT_POLICY_NAME = "deterministic-min-residual-first-fit"`. Services
processed in ascending `service_id` order (never caller array order);
nodes filtered to ABI-compatible AND eligible (per `PlacementEligibilityView`)
AND sufficient planner-local residual capacity (declared capacity minus
`committed` minus every earlier decision in this same plan, via
`saturating_sub` — never a panic); among survivors, minimum residual CPU
after placement wins, ties broken by minimum residual memory after, then
lexicographically smallest `node_id`. No RNG, no hashmap-iteration
dependence (a `Vec` is explicitly sorted by `node_id` before use), no
wall-clock, no telemetry. Confirmed live by the four determinism tests
(`reversed_service_order_produces_identical_output`,
`reversed_node_order_produces_identical_output`,
`same_canonical_inputs_produce_the_same_plan_every_time`,
`min_residual_after_wins_over_lexicographically_smaller_but_worse_fit_node`),
all passing.

## 6. The four planning inputs — complete, no hidden fifth

`CanonicalDGraphV1`, authoritative `CanonicalDContinuumV1` (built
server-side via `dnode_registry::build_canonical_dcontinuum`, never a
caller claim — `PlanRequestPayload` carries no `dcontinuum` field at all),
`committed: &BTreeMap<String, NodeCommitmentV1>`, `eligibility:
&PlacementEligibilityView`. No fifth input exists anywhere in the
function signature or its call sites.

## 7. HTTP hardening — re-verified fresh against current source

`PlanRequestPayload` (`api/ddeploy.rs:230`) has exactly two fields,
`application_id` and `dgraph`, under `#[serde(deny_unknown_fields)]` —
confirmed both by direct read and by the existing regression
(`api::ddeploy::tests`, the test asserting the struct's own source text
contains `deny_unknown_fields`, at line 1504). `post_plan_proposal` never
reads a `source` field from any payload; `create_proposal` is always
called with `source: ProposalSource::Deterministic` unconditionally
(`api/ddeploy.rs:~490`). There is no `auto_accept`/`activate`/`deploy`/
`execute` field anywhere on `PlanRequestPayload` or `PlanResponse` — the
plan endpoint always returns state `"pending"` (line 541) and never calls
`accept_proposal`/`accept_v2_proposal` itself. Grep for
`auto_accept`/`auto_activate` across `api/ddeploy.rs` and
`canonical_placement.rs` returns zero matches.

## 8. `PlannerProvenanceV1` — reviewed fresh, reload-integrity confirmed

Persisted superset of `PlacementProvenance`: adds `dgraph_ref`/
`dcontinuum_ref` (C1C: must relationally match the proposal's own refs,
enforced by `validate_persisted_state_integrity`), `committed_by_node`
(C1A — but see §11, this is the exact field implicated in the central
finding), and `eligibility` (C1B, preserving the absent/present-empty
distinction on reload — proven by
`eligibility_snapshot_preserves_absent_vs_present_empty_distinction`).

## 9. Eligibility semantics — `derive_placement_eligibility_view` vs `check_node_eligibility`

`check_node_eligibility(artifact, node_id)` (`ddeploy.rs:1643`) returns
`None` (eligible) only if `target_nodes` contains `node_id`; an empty
`target_nodes` therefore fails every node. `derive_placement_eligibility_view`
mirrors this exactly: absent service_id in the view = unrestricted-by-this-view
(real enforcement deferred to `create_proposal`'s own
`resolve_artifact_for_service`/`check_node_eligibility` call downstream);
present-with-empty-set = zero eligible nodes, matching the real per-node
semantics. Both distinctions are directly tested
(`service_with_no_resolved_artifact_is_unrestricted_by_eligibility`,
`an_artifact_with_an_explicitly_empty_target_nodes_set_still_means_zero_eligible_nodes`).
No divergence found between the planner's advisory view and the real
enforcement semantics.

## 10. TOCTOU / lock ordering — unchanged after Phase 119

`post_plan_proposal` acquires `ddeploy_transaction()` then
`dnode_registry::dnode_transaction()`, then reads the D-Node registry,
builds the authoritative D-Continuum, loads DDeploy state, computes
commitments, then enters `service_artifacts.with_stable_view(...)` before
calling `create_proposal` and persisting — the same order ADR-0020's R1/P2
description and Phase 119's own acceptance-path lock ordering use. No
phase since (119A–119I) touched this ordering; re-confirmed by direct
read, not by trusting the ADR's prose.

## 11. **Central finding — cross-version (v1/v2) structural commitment accounting asymmetry**

Two commitment-aggregation functions exist in `core::ddeploy`:

- `compute_project_node_commitments` (line 1740) — **v1-only**. Its
  `active_set: &BTreeMap<String, ActiveApplicationDeployment>` parameter is
  a type that structurally cannot hold v2 (`ActiveDMapV2`) entries.
- `compute_project_node_commitments_versioned` (line 1781) — **the
  correct, complete v1+v2-aware aggregator**. It wraps the v1-only
  function and additionally iterates `state.phase113_v2.active.values()`,
  adding v2 commitments on top.

Three call sites were traced to their exact source context (all read
directly, not inferred):

1. **v2 acceptance path** (`ddeploy.rs:625`, inside the v2 acceptance
   method) — uses `compute_project_node_commitments_versioned`. **Correct.**
   Proven safe by the existing regression
   `v1_active_app_plus_v2_candidate_exceeding_capacity_is_rejected`: a v1
   app already consuming a shared node's full capacity correctly blocks a
   v2 candidate from being accepted on that node.
2. **v1 acceptance path** (`accept_proposal`, `ddeploy.rs:2712`, at its
   R3C `F_PROJECT_NODE_CPU_CAPACITY_EXCEEDED`/
   `F_PROJECT_NODE_MEMORY_CAPACITY_EXCEEDED` gate) — uses the **non-versioned**
   `compute_project_node_commitments`, called against `updated_active`,
   which is constructed at line 2666 as
   `state.active_by_application.clone()` — purely v1 data, structurally
   incapable of reflecting any active v2 application. **This path is
   blind to active v2 commitments from other applications.**
3. **Deterministic planner's own HTTP handler** (`post_plan_proposal`,
   `api/ddeploy.rs:~467`) — also uses the **non-versioned** function for
   its `commitments` baseline passed into `plan_canonical_placements`.
   **The planner itself is blind to active v2 commitments** from other
   applications when computing residual capacity for a new plan.

No compensating check exists elsewhere: `PlannerProvenanceV1.committed_by_node`'s
own doc comment (`ddeploy.rs:368-374`) explicitly documents it as
"`compute_project_node_commitments`'s own return value" — i.e. the
non-versioned function is the *documented, intended* source, confirming
this is an unaddressed gap rather than an intentionally-accepted
limitation with a stated rationale. A grep for any test covering the
reverse direction (v2 active + v1 candidate, or v2 active + a
deterministically-planned v1 candidate) found none — only the
already-safe direction (§ above) is tested.

**Concrete failure scenario:** project P has a shared node `n1` with 4
CPU cores. Application `app-x` is active under DMap/2 consuming all 4
cores. A caller plans (`POST /api/v1/ddeploy/plan`) or otherwise submits
a v1 proposal for a *different* application `app-y` requiring cores on
`n1`. Both the planner's residual-capacity computation and
`accept_proposal`'s own R3C capacity gate compute `app-x`'s commitment as
zero (since `compute_project_node_commitments` only iterates
`active_by_application`, which `app-x`'s v2 activation never populates).
`app-y`'s proposal can be planned onto, and accepted at, `n1`, even
though `n1` has no actual residual capacity — an aggregate capacity
violation exactly matching this task's own closure criterion (§18):
"If one version can overcommit against the other: Phase120 closure
blocker, even if planner itself is deterministic."

**This is a confirmed Phase 120 closure blocker per the task's own
framing — not fixed in this audit, per the task's explicit
"AUDIT / REVIEW FIRST... do NOT fix it" instruction.**

## 12. Minimum recommended Phase 120B implementation

Swap both call sites from `compute_project_node_commitments` to
`compute_project_node_commitments_versioned`:

- `dserver/src/core/ddeploy.rs:2712` (inside `accept_proposal`'s R3C
  gate) — change the call from
  `compute_project_node_commitments(&record.proposal.project_id, &updated_active, &state.proposals, None)`
  to a `compute_project_node_commitments_versioned(&record.proposal.project_id, state, None)`-shaped
  call, then re-add the current proposal's own placements on top exactly
  as today (the accumulation logic after the call is otherwise unchanged).
  Note `state` here is borrowed immutably elsewhere in the same function
  for `state.proposals`/`authoritative_dcontinuum`, so the exact borrow
  shape needs care, but no new data dependency is introduced — `state`
  already contains `phase113_v2` at this call site.
- `dserver/src/api/ddeploy.rs:~467` (inside `post_plan_proposal`) — change
  from `compute_project_node_commitments(&project_id, &ddeploy_state.active_by_application, &ddeploy_state.proposals, Some(&payload.application_id))`
  to `compute_project_node_commitments_versioned(&project_id, &ddeploy_state, Some(&payload.application_id))`.

Both replacement functions already exist, are already used correctly
elsewhere (the v2 acceptance path), and are already covered by a passing
regression proving their correctness for the v1-active/v2-candidate
direction — Phase 120B's job is symmetric test coverage for the reverse
direction (v2-active + v1-candidate-acceptance, and v2-active +
deterministic-plan-then-v1-accept) plus the two-line call-site swap. This
is why the finding supports VERDICT B rather than C: the fix requires no
new type, no new function, no new architectural decision — only wiring
an existing, proven-correct function into two call sites that currently
use the wrong (but structurally similar, easy-to-have-missed) sibling
function.

## 13. Scenario §19 — planning for an application currently active under DMap/2

Traced directly: `post_plan_proposal` always calls the **v1**
`create_proposal` (never a v2 creation path). Phase 119G's fail-closed
`F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED` guard lives inside
`create_proposal` itself, so it is inherited by the planner endpoint for
free — attempting to plan for an application that currently has active
DMap/2 correctly fails closed with a 422/`unprocessable` response
(`api/ddeploy.rs:528`, `.map_err(|findings| unprocessable(&findings))?`).
**This is answer (B) of the task's own three options: yes, rejection is
correct, and this is acceptable Phase 120 scope** — no
versioned/v2-aware deterministic proposal producer is required merely to
handle an application's own self-downgrade case, since the existing guard
already covers it structurally. This is a separate, already-closed
question from §11's cross-*application* commitment-accounting gap.

## 14. DServ-vs-DIoT placement scope boundary

Neither `canonical_placement.rs` nor ADR-0020 mentions D-IoT anywhere
(confirmed by direct grep, zero matches in both files). Phase 120's
scope, as implemented, is D-Serv placement only. This is a legitimate,
documented boundary (D-IoT devices are not schedulable D-Node targets in
the current architecture) rather than an unaddressed gap — no evidence
was found that D-IoT placement was ever intended to be part of this
planner's responsibility.

## 15. Relationship to Phase 119 migration

`ddeploy_migration.rs` contains zero references to
`canonical_placement`/`plan_canonical_placements` and zero
migration-specific placement inputs (`exclude_node`/`source_node`/
`force_move` do not exist anywhere in the repository). This confirms
Phase 119A's own stated boundary (§8/§40 of that document): Phase 120
produces an explicit target proposal; Phase 119 executes A→supplied-B
without caring how B was chosen. The existing planner can already serve
as an ordinary proposal producer feeding a migration's target-B proposal
with no migration-specific input required — inventing an `exclude_node`-style
input would wrongly turn planner policy into imperative caller placement
authority, which this audit does not recommend.

## 16. DMap authority / auto-accept / DMonitor boundary

- DMap remains sole authority: no type in `canonical_placement.rs` is
  ever written to persisted DDeploy state directly; every plan is only
  ever a candidate `CreateProposalInput.placements` value, and acceptance
  (`accept_proposal`/`accept_v2_proposal`) remains the only path to an
  active `CanonicalDMapV1`.
- No auto-accept: confirmed in §7, no such field or call exists.
- DMonitor/telemetry boundary: `CanonicalDContinuumV1` structurally
  carries no telemetry field (re-confirmed by `canonical_placement.rs`'s
  own `plan_output_never_contains_a_capacity_or_platform_string_leak`
  test, which greps the full serialized plan for
  `cpu_usage`/`available`/`load_average`/`temperature`/`rtt`/etc. and
  finds none).
- Future GNN/RL/LLM seam: confirmed clean by the module's own doc
  comment (lines 47-50) and by the fact that `ProposalSource::Ai` already
  exists as a sibling to `ProposalSource::Deterministic` — any future
  learned proposal producer feeds the same `CreateProposalInput.placements`
  slot, never bypassing `create_proposal`/`accept_proposal`.

## 17. ADR-0020 staleness

ADR-0020's full text (all three revisions, v1/v2/v3, 610 lines) never
mentions "v2", "DMap/2", "versioned", or
`compute_project_node_commitments_versioned` anywhere. This is itself
independent documentary evidence supporting §11's finding: the ADR was
written and twice revised without the author(s) apparently being aware
that a second, parallel commitment-tracking system (DMap/2, introduced by
a later phase) existed and needed the same accounting. **Recommendation
for Phase 120B: add an ADR-0020 addendum (not a silent rewrite of
existing text) documenting the v1/v2 commitment-accounting requirement
and, once fixed, the resolution — preserving the existing R1/R2/R3/C1/C2
history as-is.**

## 18. Feature classification

| Feature | Classification |
|---|---|
| Deterministic D-Serv placement | REQUIRED FOR 120 — DONE |
| ABI filtering | REQUIRED FOR 120 — DONE |
| Structural capacity accounting (single-version) | REQUIRED FOR 120 — DONE |
| Inter-application commitment accounting (v1-only) | REQUIRED FOR 120 — DONE |
| Inter-application commitment accounting (v1+v2 uniform) | REQUIRED FOR 120 — **GAP, §11** |
| ServiceArtifact eligibility | REQUIRED FOR 120 — DONE |
| Deterministic tie-breaking | REQUIRED FOR 120 — DONE |
| Provenance (ephemeral + persisted) | REQUIRED FOR 120 — DONE |
| Proposal persistence | REQUIRED FOR 120 — DONE (unchanged `create_proposal`) |
| Explicit acceptance | REQUIRED FOR 120 — DONE |
| DMap/2 self-downgrade fail-closed handling | REQUIRED FOR 120 — DONE (inherited, §13) |
| D-IoT placement | OUT OF SCOPE (§14) |
| Latency/locality/energy/network-bandwidth-aware policy | DEFERRED (explicit future heuristic policy version, ADR-0020) |
| GPU-aware placement | DEFERRED |
| Live telemetry / DMonitor feedback | OUT OF SCOPE for this planner (ADR-0002 boundary) |
| GNN/RL/LLM placement | DEFERRED, clean seam confirmed (§16) |
| Automatic migration trigger | OUT OF SCOPE (Phase 119/121 concern, not 120) |
| Migration execution | OUT OF SCOPE (Phase 119's own responsibility) |

## 19. Q1–Q12 special closure decisions

- **Q (is Phase 120 already fully implemented)?** No — one confirmed gap (§11).
- **Q (does the gap require a new planner)?** No — the planner itself is
  correct; only two capacity-accounting call sites outside the planner
  module need to use the already-existing versioned aggregator.
- **Q (self-downgrade for v2-active applications, §19/§13)?** Correctly
  fail-closed already, acceptable Phase 120 scope, no fix needed.
- **Q (DServ-vs-DIoT boundary)?** DServ-only, legitimate, documented (§14).
- **Q (migration relationship)?** Clean, no coupling needed or found (§15).
- **Q (auto-accept)?** Does not exist (§7/§16).
- **Q (DMap authority)?** Unchanged, sole authority (§16).
- **Q (GNN/RL/LLM seam)?** Clean (§16).
- **Q (ADR-0020 stale text)?** Yes, missing v1/v2 awareness entirely;
  addendum recommended, not a rewrite (§17).
- **Q (is the found gap "future heuristics" that should not count against
  closure)?** No — it is a present-tense, structurally-confirmed capacity
  violation possible today between two already-shipped acceptance paths,
  not a missing future capability.
- **Q (verdict A eligible)?** No — a real gap exists.
- **Q (verdict C required)?** No — the gap's fix is small, uses an
  already-existing and already-tested function, and requires no new
  design; VERDICT B's own definition ("small, well-bounded... gap must be
  closed") fits exactly.

## 20. Regression / build results (this audit, fresh)

- `cargo test --manifest-path dserver/Cargo.toml canonical_placement --locked`: **23 passed, 0 failed.**
- `cargo test --manifest-path dserver/Cargo.toml ddeploy --locked`: **286 passed, 0 failed.**
- `cargo test --manifest-path dserver/Cargo.toml api::ddeploy --locked`: **24 passed, 0 failed.**
- `cargo test --manifest-path dserver/Cargo.toml --locked` (full suite): **1921 passed, 0 failed, 0 ignored.**
- `cargo build --manifest-path dserver/Cargo.toml --release --locked`: **succeeded** (pre-existing dead-code warnings only, no new warnings introduced by this audit since no source changed).
- `git diff --check`: **clean.**
- No DATUM-agent suite run — Phase 120 is entirely server-side; no
  agent-side dependency was discovered during this audit.

## 21. Remaining Phase 120B work (not performed here, per task instruction)

1. Swap the two call sites identified in §12 to
   `compute_project_node_commitments_versioned`.
2. Add regression coverage for the reverse direction: v2-active +
   v1-candidate acceptance rejected; v2-active + deterministic-plan
   correctly reduces planner residual capacity and/or the resulting v1
   acceptance is correctly rejected if it would overcommit.
3. Add an ADR-0020 addendum documenting the v1/v2 commitment-accounting
   requirement and its resolution.
4. Re-run the full regression suite and both release builds to confirm
   no regression, then close Phase 120 with a follow-up independent
   review (Phase 120B or a Phase 120-closing review), analogous to how
   Phase 119E's blocker was closed via 119F→119F-R1→119G→119H→119I.

---

PHASE 120A INDEPENDENT AUDIT CLOSED.
PHASE 120 — DETERMINISTIC PLACEMENT — MINOR CLOSURE WORK REMAINS.
THE DETERMINISTIC PLANNER FOUNDATION IS CORRECT, PURE, AND DETERMINISTIC.
A NARROW, SOURCE-CONFIRMED CROSS-VERSION (DMAP/1 VS DMAP/2) STRUCTURAL COMMITMENT-ACCOUNTING GAP REMAINS AT TWO CALL SITES.
THE CORRECT FIX ALREADY EXISTS IN THE CODEBASE AND REQUIRES NO NEW DESIGN.
PHASE 120B SHOULD WIRE IT IN AND CLOSE.
