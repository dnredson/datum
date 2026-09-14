# ADR-0020 — Canonical deterministic placement v1

- Status: **Accepted and implemented, v3 (independent review fix).**
- Decision refs: ADR-0002 (single placement authority), ADR-0006 (canonical
  digest), ADR-0017 (canonical `datum.dgraph/1`/`datum.dcontinuum/1`),
  ADR-0018 (D-Deploy proposal/acceptance/projection), ADR-0019 (D-Node
  structural authority and authoritative D-Continuum derivation).
- Scope: implements conceptual roadmap phase 120 (deterministic/heuristic
  placement) at its smallest meaningful baseline — a reproducible,
  server-side placement PRODUCER that feeds the UNCHANGED D-Deploy
  proposal/acceptance path. It does **not** implement Stage/phase 121 (GNN),
  122 (closed feedback loop), 123 (AI/RL/LLM policy), D-Code/WASM execution
  (111), D-Call (112), or redeployment/migration (119) — none of those are
  touched here, and no scoring/latency/energy/locality/QoS heuristic is
  introduced (deferred to an explicitly later, explicitly heuristic policy
  version).

## Context

Conceptual phase 109 (canonical D-Deploy) and the D-Node/D-Continuum
authority foundation (ADR-0019) together give DServer a governed,
relationally sound, single-authority way to VALIDATE and ACCEPT an
already-decided placement. Nothing in the repository, however, could
PRODUCE a candidate placement from a `datum.dgraph/1` and an authoritative
`datum.dcontinuum/1` alone — every existing proposal, in every prior stage's
tests and every real caller, supplied `placements` by hand. The legacy
`core::dgraph_planner` module (Controlled WASM Deployment phase, C59-I44)
does produce placement decisions, but exclusively over the LEGACY composite
`core::dgraph::DGraph` (application + infrastructure + placement + health
folded into one structure) — a type this repository's own module
documentation (`canonical_dgraph.rs`) explicitly distinguishes from
`CanonicalDGraphV1` and never reinterprets as it. Routing a canonical
D-Graph through that legacy composite type merely to reuse its code would
require either lossy translation or reintroducing exactly the coupling
Stage 110's canonical contracts were introduced to remove.

## Decision

### 1. A new, minimal, pure module: `core::canonical_placement`

`plan_canonical_placements(dgraph: &CanonicalDGraphV1, dcontinuum:
&CanonicalDContinuumV1) -> Result<CanonicalPlacementPlanV1, Vec<CanonicalValidationFinding>>`
is the sole entry point. It is PURE: no I/O, no database handle, no `&mut`
parameter of any kind, and — verified structurally, not merely by
convention — no import anywhere in the module of `accept_proposal`,
anything from `core::service_artifact_registry`, or anything from the
legacy `core::dgraph`/`core::dgraph_planner` modules. The output,
`CanonicalPlacementPlanV1`, is an ephemeral, non-canonical, never-persisted,
never-digested handoff value — exactly like `core::ddeploy`'s own
`ProposalValidationOutcome` — never a competing placement authority. The
single authoritative placement carrier remains, unchanged, the accepted
`CanonicalDMapV1.placements` (ADR-0002).

### 2. Placement policy v1: deterministic minimum-residual-after, first-fit

Static inputs used, both of which already exist on the canonical contracts
(Stage 110 P0 fixes, independent review v3) — **no new field was invented**:
`CanonicalDServV1.execution_requirement.dnode_abi` /
`.resource_requirement.{cpu_cores, memory_bytes}`, and
`CanonicalDNodeV1.capabilities.dnode_abi_versions` /
`.capacity.{cpu_cores, memory_bytes}`.

D-Servs are processed in ascending `service_id` order (the one order with no
dependency on caller-supplied array position — ADR-0017 documents `services`
as a canonically unordered set keyed by `service_id`). For each, D-Nodes are
filtered to those (a) declaring the required `dnode_abi` and (b) having
enough PLANNER-LOCAL RESIDUAL capacity — the node's declared `capacity`,
decremented by every earlier decision in the *same* plan, never a live/
observed value — for both `cpu_cores` and `memory_bytes`. Among the eligible
set, the node minimizing residual `cpu_cores` remaining AFTER this placement
is chosen; ties broken by minimum residual `memory_bytes` after; remaining
ties broken by the lexicographically smallest `node_id`. Both `dcontinuum`'s
nodes and `dgraph`'s services are internally re-sorted before planning, so
reversing either input array is proven (regression test) to produce a
byte-identical result.

This is a deterministic best-fit-by-minimum-slack bin-packing heuristic.
It is explicitly **not** latency/energy/locality/QoS-aware — those remain
future, explicitly-named heuristic policy versions — and explicitly **not**
learned: no GNN/RL/LLM weight of any kind participates, and none can, since
`CanonicalDContinuumV1` structurally carries no live telemetry field to
learn from in the first place (`canonical_dcontinuum`'s own
`no_telemetry_field_exists_on_the_type` proof; re-verified for this module
by `canonical_placement::tests::plan_output_never_contains_a_capacity_or_platform_string_leak`).

### 3. Capacity accounting is real, in-plan, checked, and honestly scoped

Because static `cpu_cores`/`memory_bytes` requirements and capacities
already exist canonically, this v1 baseline performs REAL capacity
accounting, not a placeholder: a running per-node residual is maintained for
the duration of one `plan_canonical_placements` call (never persisted,
never shared across calls), decremented by `checked` arithmetic that can
never underflow (a node is only ever chosen after its residual is proven
sufficient), and an unsatisfiable D-Graph fails closed
(`unsatisfied_static_capacity`) rather than silently oversubscribing. A
positive-control regression cross-validates every successful plan's
placements against the EXISTING, independently-reviewed
`CanonicalDMapV1::validate_against` aggregate feasibility check (never
re-implementing that arithmetic a second time to check itself).

### 4. Deliberately excluded from this module: `ServiceArtifact.control_plane.target_nodes`

This module never reads `ServiceArtifact` at all. `target_nodes` is an
ELIGIBILITY constraint on an already-decided placement (ADR-0002's own
architectural framing: "eligibility constraints, not canonical placement
authority"), not an input to canonical placement SELECTION. A plan this
module accepts can still be rejected downstream by `create_proposal`'s
unchanged `validate_full` (`node_not_eligible`) if a `ServiceArtifact`
happens to restrict a service to a node this policy did not choose — that is
correct, expected behavior, not a bug: the legacy artifact-eligibility gate
is not duplicated or shadowed here, it simply still runs, unmodified, at its
existing point in the pipeline.

### 5. Structured failure codes

Three new stable finding codes, distinct from `CanonicalDGraphV1`/
`CanonicalDContinuumV1`'s own structural-validation codes (which this module
reuses verbatim by calling `dgraph.validate()`/`dcontinuum.validate()`
first, rather than wrapping them in a generic `invalid_canonical_input`):

- `no_nodes` — the D-Continuum declares zero D-Nodes while the D-Graph
  requires placing at least one D-Serv.
- `no_abi_compatible_node` — a D-Serv's required `dnode_abi` is declared by
  zero D-Nodes (distinct from a malformed/unsupported ABI STRING, which
  `dgraph.validate()` already catches as `invalid_dnode_abi` before this
  module's own selection logic ever runs).
- `unsatisfied_static_capacity` — at least one ABI-compatible node exists,
  but none has sufficient residual capacity at that point in planning.

An empty `dgraph.services` is explicitly **not** an error, independent of
how many nodes `dcontinuum` declares: there is nothing to place, so a
deterministic empty plan is returned.

### 6. Provenance

`PlacementProvenance` records planner/policy identity and version, the
exact `dgraph`/`dcontinuum` id/revision/digest planned against, and a
`PlacementDecisionRecord` per service (chosen node, required ABI, how many
nodes were eligible, and residual capacity before/after) in decision order
— sufficient to reconstruct and audit *why* a given node was chosen, without
re-running the (deterministic, so reproducible anyway) policy. Changing any
part of `dcontinuum`'s content — even a field irrelevant to placement
selection itself, like `platform.os` — changes `dcontinuum_digest` and is
therefore visible in provenance even when the resulting placement decision
happens to be unaffected (regression-tested).

### 7. D-Deploy integration: reuse, never duplicate

`architecture: canonical DGraph + authoritative structural DContinuum
(ADR-0019) + deterministic placement policy -> candidate placements ->
UNCHANGED core::ddeploy::create_proposal -> UNCHANGED validate_full ->
explicit human/operator acceptance later (UNCHANGED accept_proposal)`.

The new `POST /api/v1/ddeploy/plan` handler (`api::ddeploy::post_plan_proposal`)
derives the authoritative D-Continuum exactly the way `post_create_proposal`
already does (`dnode_registry::build_canonical_dcontinuum`), calls
`plan_canonical_placements`, and on success calls the SAME `create_proposal`
function `post_create_proposal` calls, with zero special-cased validation or
persistence logic of its own. `ProposalSource::Deterministic` — an enum
variant that already existed, unused, in `core::ddeploy` before this pass —
is stamped on the resulting proposal. The request body carries no
`dcontinuum` field and no `placements` field at all: both are entirely
server-derived, so there is no caller-suppliable field this endpoint could
use to fabricate a node, a capacity, or a placement decision. A failed plan
(bad D-Graph, no ABI-compatible node, exhausted capacity) fails BEFORE the
`ServiceArtifact` registry, the D-Deploy transaction lock, or persisted
state are touched at all — no partial proposal is ever created.

This handler, like `post_create_proposal`, never calls `accept_proposal`.
The resulting proposal is `Pending` and requires the exact same explicit
`POST .../:proposal_id/accept` call, by a human/operator audit label, that
every other Stage 111 proposal requires — confirmed by an integration
regression that separately calls `accept_proposal` after planning and
proposal creation, proving the two steps are genuinely decoupled, not
merely documented as such.

### 8. Relationship to future GNN/RL/LLM placement work

Per the architectural principle established before this pass ("AI must
never bypass DATUM governance"): a future GNN/RL/LLM placement producer is
expected to become an ALTERNATIVE PROPOSAL PRODUCER competing with this
module for the same `CreateProposalInput.placements` slot and the same
`ProposalSource::Ai` provenance tag that already exists in `core::ddeploy` —
never an alternative to, or bypass of, `create_proposal`/`validate_full`/
explicit `accept_proposal`. This module's own architecture (pure function,
canonical inputs in, candidate placements out, zero authority of its own)
is the template that relationship is expected to follow.

## What was reused from the legacy planner, and what was not

`core::dgraph_planner::DGraphPlannerDryRun` and its supporting types operate
exclusively over `core::dgraph::DGraph` (the Controlled WASM Deployment
phase's composite application+infrastructure+placement+health graph) via
`validate_dgraph`, `DGraphNode`, `DGraphPlacementDecision`, and
`DGraphReconfigurationDecision` — none of which have any canonical-contract
equivalent or conversion path, and reusing them would require constructing
a legacy `DGraph` from canonical inputs, exactly the composite-graph
coupling `canonical_dgraph.rs`'s own module documentation identifies as a
DIFFERENT, NOT-reinterpreted structure. **Nothing was reused at the code
level.** The only thing carried forward conceptually is the general shape of
recording a `planner_version` alongside decision evidence — the legacy
module's own `PLANNER_VERSION` constant and `DGraphPlannerBlockedItem`/
`DGraphPlannerWarning` provenance pattern — reimplemented from scratch
against the canonical types, not copied. The legacy planner is untouched and
not removed; it continues to serve its own, separate, composite-graph phase.

## Testing evidence

`core::canonical_placement`: 16 unit tests (determinism under reversed
service/node order, exact tie-break resolution, multi-service placement,
ABI-incompatibility and capacity-exhaustion rejection, explicit empty-graph
and empty-continuum behavior, provenance-digest responsiveness to an
unrelated continuum field change, a JSON-text scan proving no live-telemetry-
shaped key ever appears in plan output). `core::ddeploy`: 3 new integration
tests proving a planner-produced proposal (a) is accepted by the unchanged
`create_proposal` validation path, (b) can be explicitly accepted afterward
through the unchanged `accept_proposal` path with no auto-acceptance, and
(c) that planning alone, without a separate acceptance call, activates
nothing.

## Known limitations (honestly scoped, not hidden)

- **No resource-aware tie-break beyond CPU/memory.** No disk, network
  bandwidth, GPU, or other resource dimension is modeled — none exists on
  the canonical contracts today.
- **No latency/locality/energy/QoS awareness.** Deferred to an explicitly
  later, explicitly heuristic policy version (`PLACEMENT_POLICY_VERSION`
  exists precisely so a future policy can be distinguished from this one in
  provenance).
- **No cross-application awareness at plan time.** The planner considers
  only the one D-Graph/D-Continuum pair it is given; it does not know about
  other applications' already-accepted placements in the same project when
  choosing residual capacity. (`create_proposal`/`accept_proposal`'s own
  existing project-wide union/collision checks, unchanged by this pass,
  remain the actual safety net — see ADR-0018.)
- **`ServiceArtifact.control_plane.target_nodes` is not planner input** —
  see Decision §4. A generated plan can still be rejected downstream for
  this reason; that is intentional, not a gap this ADR leaves ambiguous.
- **No self-digest of the plan itself.** The ephemeral, HTTP-response-only
  `PlacementProvenance` records the digests of the two canonical D-Graph/
  D-Continuum INPUTS, not a digest of the plan/decision output; this was
  judged unnecessary scope (the plan is never persisted or re-verified
  against a stored digest anywhere) and can be added later without changing
  any existing field. Independent review v3 (C1) note: this bullet is about
  `PlacementProvenance` specifically -- the PERSISTED
  `core::ddeploy::PlannerProvenanceV1` is a strict superset that also
  retains the `committed`/`eligibility` context (see the v3 section below),
  so this limitation does not extend to what is actually persisted.

## Independent review v2 corrections

Independent review of the v1 package found three closure blockers (R1, R2,
R3) and one honest-disclosure gap (P2). All four are fixed in this pass.
None reverses a v1 architectural decision: the planner is still pure, still
never calls `accept_proposal`, still never becomes a competing placement
authority, and the accepted `CanonicalDMapV1` remains the sole placement
authority throughout.

### R1 — proposal source is no longer caller-controlled

v1's `PlanRequestPayload` carried a `source: ProposalSource` field
(`#[serde(default = "default_source")]`, defaulting to `Operator`), and
`post_plan_proposal` passed `payload.source` straight into
`CreateProposalInput`. A caller could therefore claim `Operator` or `Ai`
provenance for a proposal the deterministic planner actually produced.

Fixed: `PlanRequestPayload` now carries only `application_id` and `dgraph`
— no `source` field exists anywhere on it, so there is no deserializable
value that could ever reach `proposal.source`. `post_plan_proposal` sets
`ProposalSource::Deterministic` unconditionally, in the function body, not
from any payload field. `PlanRequestPayload` additionally derives
`#[serde(deny_unknown_fields)]` — unique to this DTO in this module,
deliberately, since an explicit `source`/`dcontinuum`/`placements` field
from a caller now produces an immediate 400 rather than being silently
ignored (which could otherwise look, to that caller, like it had taken
effect). Regression evidence: `api::ddeploy::tests::
plan_request_payload_has_no_caller_controllable_source_field`,
`plan_request_payload_rejects_unknown_fields`,
`post_plan_proposal_hardcodes_deterministic_source_and_never_reads_a_payload_source`
(all structural/source-scan tests, following this file's own established
`source_contains_no_host_mutation_calls` pattern, since no precedent exists
anywhere in this codebase for invoking an Axum handler directly from a unit
test), plus `core::ddeploy::tests::planner_output_is_accepted_by_create_proposal_unmodified`,
which independently proves a proposal built the way this handler builds one
really does end up `ProposalSource::Deterministic`.

### R2 — the planner now considers real ServiceArtifact node eligibility

v1's Decision §4 concluded the planner should ignore `target_nodes`
entirely, reasoning correctly that `target_nodes` is an eligibility
constraint, never placement authority (ADR-0002) — but incorrectly
concluding from that, that the planner must therefore never CONSIDER it
while generating candidates. That conclusion was too strong: a service
eligible only for `cloud-01`, with `fog-01` and `cloud-01` both otherwise
ABI/capacity-compatible, would have the deterministic best-fit policy select
`fog-01` — which `create_proposal`'s unchanged `validate_full` then rejects
as `node_not_eligible`, even though a valid deterministic placement on
`cloud-01` existed all along. That is a false-negative planning failure, not
correct fail-closed behavior.

Fixed: a new `core::canonical_placement::PlacementEligibilityView` (ephemeral,
never persisted, never part of provenance) and
`core::ddeploy::derive_placement_eligibility_view` (built from the CURRENT,
project-scoped `ServiceArtifact` stable-view snapshot, reusing
`resolve_artifact_for_service`'s exact resolution semantics). A service_id
present in the view restricts candidates to EXACTLY its recorded
`target_nodes` set, mirroring `check_node_eligibility`'s real per-node
semantics byte-for-byte — including that an artifact with an explicitly
EMPTY `target_nodes` still means ZERO eligible nodes for that service, never
"unrestricted" (this is the one place the original independent review
request's own wording ["empty target_nodes continues to mean unrestricted
eligibility"] conflicted with the actual, tested, ADR-0002-documented Stage
111 semantics — `check_node_eligibility` and its own regression
`empty_target_nodes_means_no_eligible_node` are unambiguous that an empty
list means NO node is eligible; this fix reuses that real interpretation
rather than the paraphrase, per the request's own, more authoritative
instruction to "reuse the same interpretation used by check_node_eligibility").
A service_id ABSENT from the view (no `ServiceArtifact` resolves yet, e.g. a
brand-new application) is unrestricted BY THIS VIEW — `create_proposal`'s own
`resolve_artifact_for_service` call still fails closed on that condition,
unconditionally, downstream; this view exists only to avoid the
false-negative, never to grant an eligibility exception the real check
would not also grant. A new finding code, `no_eligible_node`, distinguishes
"ABI-compatible nodes exist but none is artifact-eligible" from
`no_abi_compatible_node` (no ABI match at all) and
`unsatisfied_static_capacity` (eligible AND ABI-compatible, but insufficient
residual). This module still never imports `ServiceArtifact` or the
artifact registry itself — eligibility is fed in as an opaque, pre-derived
value, keeping the selection machinery pure. TOCTOU safety: see "Locking"
below. Regression evidence: `canonical_placement::tests::
eligibility_view_skips_a_tighter_fit_ineligible_node_for_a_larger_eligible_one`,
`all_abi_and_capacity_compatible_nodes_ineligible_fails_closed`,
`service_with_no_resolved_artifact_is_unrestricted_by_eligibility`,
`an_artifact_with_an_explicitly_empty_target_nodes_set_still_means_zero_eligible_nodes`.
Post-acceptance placement authority is unaffected and unchanged — see the
pre-existing, still-green `core::ddeploy::tests::
mutating_target_nodes_after_acceptance_does_not_move_active_dmap` and
`stage111_never_writes_target_nodes_equal_to_accepted_node`.

### R3 — project-wide structural resource commitment (the most important finding)

v1's capacity accounting was correct only WITHIN one candidate D-Graph.
Nothing aggregated committed CPU/memory across ALL active applications in a
project, so a second application could be planned AND accepted against
structural capacity a first, already-active application had already fully
committed — real overcommit, not merely a documented limitation. Fixed with
four parts:

- **R3A — commitment derivation.** `core::ddeploy::compute_project_node_commitments`
  aggregates per-node structural (never live) CPU/memory committed by every
  active application in a project, reading each active deployment's own
  ACCEPTED, immutable `ProposalRecord.dgraph_snapshot` (never a live/current
  D-Graph) for each placed service's declared `resource_requirement`, and
  `active_dmap.placements` for which node it committed to. Fails closed on
  internally-inconsistent retained state (a dangling `proposal_id`, a
  placement's `service_id` absent from its own snapshot, or checked-add
  overflow) rather than assuming `validate_persisted_state_integrity`'s
  invariants already hold.
- **R3B — planning residual awareness.** `plan_canonical_placements` gained
  a `committed: &BTreeMap<String, NodeCommitmentV1>` parameter (a plain,
  dependency-free value type — this module still imports nothing from
  `core::ddeploy`), subtracted from each node's declared capacity (via
  `saturating_sub`, never a panic) BEFORE this plan's own in-plan residual
  accounting starts. `post_plan_proposal` computes it from the CURRENT
  `DDeployStateV1.active_by_application`, excluding the application being
  (re)planned — a redeployment REPLACES that application's own current
  active D-Map, it does not add a second, simultaneously-authoritative one
  (transient dual-running migration headroom remains an explicitly deferred
  phase-119 concern, not modeled here; this is a POST-ACCEPTANCE steady-state
  model only).
- **R3C — acceptance-time project capacity gate.** Planning-time residual
  accounting alone is insufficient: two proposals can each be legitimately
  planned/created against the same currently-free capacity (R3D). Before
  `accept_proposal` commits, it now forms the candidate `updated_active` set
  exactly as it already did, derives total commitments across every active
  application in that candidate set via `compute_project_node_commitments`,
  and compares against the CURRENT authoritative D-Continuum's per-node
  capacity — failing closed with new stable finding codes
  `project_node_cpu_capacity_exceeded` / `project_node_memory_capacity_exceeded`
  (surfaced as `AcceptError::Invalid`, HTTP 422) if either is exceeded. This
  gate is in `core::ddeploy::accept_proposal` itself, not the planner — it
  applies uniformly to deterministic, operator, and any future AI-sourced
  proposal alike, since a stale-relative-to-other-apps proposal is a D-Deploy
  acceptance-safety concern generally, not something specific to how a
  proposal's placements were originally produced. Because `updated_active`
  REPLACES (never adds to) the accepting application's own map entry, a
  same-application redeployment is never double-counted against its own
  prior allocation.
- **R3D — regression coverage.** The exact concurrent-pending-proposals
  scenario is regression-tested: two Pending proposals, each independently
  requesting all of a 4-core node's capacity, may legitimately coexist (D-Graph-scoped
  creation-time validation cannot see the other), but only the FIRST
  acceptance may actually commit it — `core::ddeploy::tests::
  second_acceptance_exceeding_project_cpu_capacity_fails_closed` (plus its
  memory equivalent, a same-app-redeploy non-double-counting test, a
  two-apps-whose-sum-fits positive control, an overflow-fails-closed test
  against `compute_project_node_commitments` directly, and a real-active-app
  residual-reduction test at the planner level).
- **R3E — corrected claim.** The original Decision §7/Known-limitations text
  described `create_proposal`/`accept_proposal`'s pre-existing project-wide
  union/collision checks as "the actual safety net" for the planner's lack of
  cross-application awareness. That was incomplete: those checks (service_id
  collision, `OperationalDGraph` validity) never enforced aggregate resource
  capacity — R3's counterexample was possible specifically because no check
  anywhere did. The corrected model: PLANNING accounts for other active
  applications' committed capacity (R3B, a residual ESTIMATE, advisory only);
  ACCEPTANCE atomically revalidates the true candidate active set's
  aggregate capacity against current authoritative structural capacity
  (R3C, the actual enforcement); D-Continuum remains structural capacity
  throughout, never live utilization; transient dual-running migration
  headroom remains an unmodeled, explicitly future phase-119 concern.

### P2 — planner provenance now persists (implemented, not deferred)

v1's `PlacementProvenance` was response-only: a historical deterministic
proposal could not later prove which planner/policy version produced it
without an external client having preserved the original HTTP response.
Fixed without changing the public `datum.ddeploy-proposal/1` wire schema:
`ProposalRecord` (an internal, non-wire-schema persisted wrapper — it
already carried `dgraph_snapshot`/`dcontinuum_snapshot` beyond the public
proposal, exactly like this) gained an optional `planner_provenance:
Option<PlannerProvenanceV1>` field (`#[serde(default)]`, so a pre-existing
persisted proposal reloads as `None`, not corruption). `PlannerProvenanceV1`
records `planner_version`/`policy_name`/`policy_version` and the full
`Vec<PlacementDecisionRecord>`. `validate_persisted_state_integrity` gained
a matching relational check: a `Some` provenance is invalid unless the
proposal's own `source` is `Deterministic`, and unless the provenance's
decisions agree EXACTLY (by `service_id -> node_id`) with the proposal's own
persisted `placements` — catching a tampered/rewritten placements list with
stale provenance left behind. Regression evidence: `core::ddeploy::tests::
persisted_deterministic_provenance_round_trips_a_restart`,
`tampered_persisted_provenance_disagreeing_with_placements_fails_closed_on_reload`,
`legacy_operator_proposal_with_no_planner_provenance_reloads_normally`,
`a_planner_provenance_record_claiming_a_non_deterministic_source_fails_closed_on_reload`.

### Locking / consistency (R2 TOCTOU + R3B)

Planning now genuinely depends on live state — the current D-Node registry
(as in v1), the current `DDeployStateV1` (R3B), and the current
`ServiceArtifact` snapshot (R2) — so `post_plan_proposal` now holds the same
lock order `post_accept_proposal` already established: `DDEPLOY_TRANSACTION
-> DNODE_TRANSACTION -> ServiceArtifact stable view`, held across planning
itself, not merely across the final persist. This closes the TOCTOU gap a
detached, pre-lock snapshot of either state would otherwise leave: the facts
a plan is built from cannot change out from under it before its own
resulting proposal is persisted. No lock is ever held across network I/O —
there is none in this path. `PUT`/`DELETE` D-Node registry handlers and
`ServiceArtifact` mutation handlers only ever acquire their own single lock,
never `DDEPLOY_TRANSACTION`, so no reverse-acquisition deadlock path exists
— the same property already established and regression-tested for
acceptance (`api::ddeploy::tests::
post_plan_proposal_holds_the_same_lock_order_accept_proposal_uses`;
mechanism-level concurrency proof for the underlying
`ServiceArtifactRegistry::with_stable_view` primitive:
`service_artifact_registry::tests::with_stable_view_blocks_a_concurrent_mutate_until_the_view_completes`).

## Superseded statements

- Decision §4's title and body ("Deliberately excluded from this module:
  `ServiceArtifact.control_plane.target_nodes`" / "This module never reads
  `ServiceArtifact` at all") are corrected by R2 above: the module still
  never imports `ServiceArtifact`, but it now receives, and filters
  candidates by, an ephemeral eligibility view DERIVED from it. The
  underlying architectural point — `target_nodes` is eligibility, never
  placement authority, and this module never duplicates or shadows the real
  check — remains true and unchanged.
- Decision §7's claim that a failed plan "fails BEFORE the `ServiceArtifact`
  registry, the D-Deploy transaction lock, or persisted state are touched at
  all" is superseded by the Locking section above: planning now correctly
  MUST touch (read-only) that state to plan against it accurately; what
  remains true and load-bearing is that a failed plan still creates NO
  persisted proposal.
- Known limitations' "No cross-application awareness at plan time" bullet
  and its "the actual safety net" characterization of the pre-existing
  union/collision checks are corrected by R3E above.
- Testing evidence's "16 unit tests" / "3 new integration tests" counts are
  superseded by the cumulative totals in the updated Testing evidence
  section at the end of this document (now v3).

## Independent review v3 corrections

Independent review of the v2 package approved R1, R2, and R3 outright (no
redesign) and found one remaining closure blocker, C1 (with five parts,
C1A-C1E), plus one documentation-precision item, C2. Both are fixed in this
pass. Neither reopens or redesigns any v1/v2 decision.

### C1 — provenance no longer captured all four genuine planner inputs

After v2's R2/R3 fixes, `plan_canonical_placements` genuinely depends on
FOUR inputs -- `CanonicalDGraphV1`, `CanonicalDContinuumV1`, the project-wide
structural commitment map, and the `ServiceArtifact`-derived eligibility view
-- not merely the two canonical contracts. `core::ddeploy::PlannerProvenanceV1`
(the PERSISTED record; the ephemeral, HTTP-response-only
`canonical_placement::PlacementProvenance` is deliberately left unchanged --
see Decision §6/Known-limitations above) still only recorded
`planner_version`/`policy_name`/`policy_version`/`decisions` -- the same
D-Graph and D-Continuum could now legitimately produce a DIFFERENT placement
depending on other active applications' commitments or on `ServiceArtifact`
eligibility at plan time, and neither fact was retained anywhere.

Fixed: `PlannerProvenanceV1` gained four fields, populated in
`api::ddeploy::post_plan_proposal` directly from the EXACT values that
successful `plan_canonical_placements` call itself received (never
reconstructed afterward):

- **C1A** `committed_by_node: BTreeMap<String, NodeCommitmentV1>` -- the
  exact per-node structural commitment baseline
  (`compute_project_node_commitments`'s own output) supplied to that call.
  `NodeCommitmentV1` gained `Deserialize` so it can round-trip through
  persisted state.
- **C1B** `eligibility: PlacementEligibilityView` -- the exact eligibility
  view supplied to that call. `PlacementEligibilityView` gained
  `Serialize`/`Deserialize`/`PartialEq`/`Eq`; its one field (a plain
  `BTreeMap<String, BTreeSet<String>>`) serializes with the
  absent-vs-present-empty distinction the type's own semantics depend on
  preserved automatically -- a service_id key absent from the map is
  genuinely absent from the serialized JSON object, never conflated with a
  key present with an empty array value.
- **C1C** `dgraph_ref`/`dcontinuum_ref: CanonicalContractReferenceV1` --
  built from `plan.provenance`'s own already-recomputed id/revision/digest
  fields (identical to what `create_proposal` independently recomputes for
  `proposal.dgraph_ref`/`dcontinuum_ref`, since both derive from the same
  deterministic digest functions over the same inputs). `validate_persisted_
  state_integrity` gained a relational check: `planner_provenance.dgraph_ref`/
  `dcontinuum_ref` must equal `proposal.dgraph_ref`/`dcontinuum_ref` exactly.
  No public wire-schema change: this remains internal `ProposalRecord`
  metadata, and no committed deterministic proposal existed anywhere yet on
  this branch, so no migration semantics were needed for an unshipped
  earlier shape.
- **C1D** strengthened decision-integrity reload validation, replacing v2's
  `decisions -> BTreeMap<service_id, node_id>` comparison (which could
  silently collapse a duplicate decision service_id via a duplicate map key)
  with an explicit check that: no `service_id` appears more than once among
  `decisions`; the decision service_id SET exactly equals the proposal's own
  placement service_id set (catching both a missing and an extra decision);
  each decision's `node_id` matches that service's own persisted placement;
  `required_dnode_abi` matches that service's own immutable `dgraph_snapshot`
  requirement; and each decision's residual-before/after arithmetic agrees
  exactly with that service's own declared `resource_requirement` (`before -
  required == after`, via `checked_sub`). This is internal historical
  self-consistency validation only -- it never attempts to validate against
  live/dynamic state.
- **C1E** corrected every doc comment that claimed a decision was "a pure
  function of the two canonical inputs' own declared content" (module doc,
  `PlacementProvenance`'s own doc comment) -- both now accurately describe
  the real four-input dependency, and `PlacementProvenance`'s doc comment
  now explicitly distinguishes itself (two-input digests only, ephemeral,
  HTTP-response-only) from `PlannerProvenanceV1` (the persisted superset that
  retains all four). The corrected claim, stated once here for reference: a
  deterministic proposal's PERSISTED provenance records policy identity/
  version, canonical D-Graph identity, canonical D-Continuum identity, the
  other-active-application structural commitment snapshot, the
  `ServiceArtifact`-derived eligibility snapshot, and ordered placement
  decision evidence -- sufficient for relational self-consistency and
  deterministic reproducibility, and explicitly NOT a claim of cryptographic
  authenticity against an attacker capable of rewriting the entire SQLite
  database and recomputing every digest (no such signing/backup mechanism
  exists anywhere in this repository).

Regression evidence: `core::ddeploy::tests::
persisted_commitment_and_eligibility_snapshots_round_trip_a_restart`,
`eligibility_snapshot_distinguishes_absent_service_from_present_but_empty`,
`different_committed_input_changes_recorded_provenance_even_when_final_placement_is_identical`,
`different_eligibility_input_changes_recorded_provenance_even_when_final_placement_is_identical`,
`duplicate_decision_service_id_fails_closed_on_reload`,
`missing_decision_fails_closed_on_reload`,
`extra_decision_fails_closed_on_reload`,
`decision_abi_mismatch_fails_closed_on_reload`,
`impossible_residual_arithmetic_fails_closed_on_reload`,
`tampered_provenance_dgraph_ref_digest_fails_closed_on_reload`,
`tampered_provenance_dcontinuum_ref_digest_fails_closed_on_reload`;
`persisted_deterministic_provenance_round_trips_a_restart` (v2, extended with
new ref-relational assertions); `canonical_placement::tests::
eligibility_snapshot_preserves_absent_vs_present_empty_distinction`.

### C2 — HTTP-status-code documentation precision

`MANUAL_VALIDATION_COMMANDS.md`'s R1 spoofing-attempt section asserted the
pinned Axum version necessarily maps a `deny_unknown_fields` `serde` error to
exactly HTTP 400 -- this pass never executed an end-to-end Axum integration
test to confirm that exact status code. Corrected to state the actual
required product property (rejected with SOME 4xx JSON-extraction error, no
proposal ever created, never a 201 with spoofed provenance) without asserting
an unverified exact code, and backed with a genuinely direct
`serde_json::from_str::<PlanRequestPayload>` deserialization regression
(`api::ddeploy::tests::plan_request_payload_serde_rejects_an_explicit_source_field`,
plus its `dcontinuum`/`placements` siblings, plus a positive-control sanity
test) rather than relying solely on the pre-existing text-scan-style
structural tests.

## Testing evidence (v3, cumulative)

`core::canonical_placement`: 23 unit tests (16 v1 + 6 v2 + 1 v3:
`eligibility_snapshot_preserves_absent_vs_present_empty_distinction`, a
direct JSON-serialization proof of the absent-vs-present-empty distinction
C1B's persisted snapshot depends on).
`core::ddeploy`: 28 tests beyond v1's 3 (10 v2 project-capacity + 4 v2
provenance round-trip/tamper/legacy/source-rejection + 11 v3: commitment/
eligibility snapshot restart round-trip, two distinctness-changes-provenance
tests, and 7 C1D decision-integrity/tamper reload-failure tests — duplicate,
missing, extra, ABI mismatch, impossible residual arithmetic, tampered
`dgraph_ref`, tampered `dcontinuum_ref`).
`api::ddeploy`: 8 tests (4 v2 structural/source-scan + 4 v3 direct
`serde_json::from_str` deserialization proofs for `source`/`dcontinuum`/
`placements` rejection plus a positive-control acceptance sanity check).
All pre-existing Stage 111/foundation/v1/v2-placement tests remain green and
unmodified in substance (`dnode_registry`: 27/27 unaffected;
`artifact_reconciliation_context`: 37/37 unaffected; `cargo check --locked`
and `cargo check --locked --tests` both clean).
