# Phase 119G — Governed Source Cleanup Implementation

- Status: Production implementation. Local (non-live) closure only — no
  RPi/fog/cloud validation performed in this phase.
- Branch: `agent/phase119-migration`, continuing from Phase 119F-R1's design
  correction, HEAD `7623adaa93b5ceb51f914431769fedc0e801f721`.
- Scope: implement the corrected governed source-cleanup mechanism
  (`documentation/reference/phase119f-r1-governed-source-cleanup-design-correction.md`)
  plus four additional implementation clarifications from independent
  review: acceptance→proposal (not `materialized_dmap_ref` alone) historical
  reconstruction; a distinct `operation = "cleanup"` authorization; a
  cleanup-only context path for a source node absent from
  `OperationalDGraph`; and an explicit fail-closed rule for the previously
  unguarded v2→v1 downgrade direction.

## 1. v2→v1 downgrade hardening (fail-closed, not implemented)

Independent review found a genuine, real asymmetry: `accept_v2_proposal`
(`dserver/src/core/ddeploy.rs`) already retires the v1 `active_by_application`
pointer when an application transitions to v2 authority, but the reverse
direction had no symmetric guard — `create_proposal`/`accept_proposal` (both
v1) never consulted `state.phase113_v2.active` at all. A v1 proposal could
therefore be created and accepted for an application currently v2-active,
producing simultaneous `active_by_application` + `phase113_v2.active`
entries for the same key — exactly the "dual active DMap authority"
corruption `active_placement_authority` already fails closed on for every
read, and `validate_persisted_state_integrity` refuses to ever reload.

**Fix, not a redesign**: a new finding code,
`F_DDEPLOY_V1_DOWNGRADE_FROM_V2_UNSUPPORTED` (`ddeploy.rs:129`), checked in
**both**:
- `create_proposal` — refuses to even create a v1 proposal for an
  application with active v2 authority.
- `accept_proposal` — re-checked at acceptance time (a CAS-race case
  creation's own guard cannot catch: the proposal was created before the
  application became v2-active, and only became v2-active afterward).

Both checks fail closed **before any mutation**. Reverse-version-transition
support (safely retiring v2 authority to accept a v1 proposal) remains an
explicit non-goal — this phase makes the unsupported direction loud and
safe, not silently corrupting.

Regression: `v1_create_proposal_fails_closed_when_application_is_currently_v2_active`
and `v1_accept_proposal_fails_closed_when_application_becomes_v2_active_after_proposal_creation`
(`dserver/src/core/ddeploy.rs`, `mod tests`) both assert the refusal AND that
v2 active authority remains byte-identical afterward, AND (the second test)
that `active_placement_authority` never observes dual-authority corruption
as a result.

## 2. Historical placement reconstruction (acceptance → proposal, not `materialized_dmap_ref` alone)

`materialized_dmap_ref` is only an `{id, revision, digest}` reference — it
never stores a placement payload of its own. The corrected reconstruction
goes:

```
v1: acceptance.proposal_id -> state.proposals[proposal_id] -> record.proposal.placements
v2: acceptance.proposal_id -> state.phase113_v2.proposals[proposal_id] -> record.proposal.placements
```

mirroring the reconstruction principle `accept_proposal`'s own
idempotent-replay branch already establishes (`build_candidate_dmap` +
digest recomputation). Before any reconstructed placement is trusted, the
new derivation validates, per acceptance:

1. The referenced proposal exists.
2. Its lifecycle state is `Accepted` (v1: `ProposalLifecycleState::Accepted`;
   v2: `ProposalV2State::Accepted`).
3. Its own `accepted_acceptance_id` names this exact acceptance (never the
   reverse).
4. Its digest recomputes correctly (`compute_proposal_digest`/
   `DDeployStateV2::proposal_digest`) and matches BOTH the proposal record's
   own stored digest and the acceptance's stored `proposal_digest`.
5. Project/application identity agree between proposal and acceptance.
6. (v1 only, additional defense-in-depth reusing existing tooling) the
   historical D-Map itself reconstructs via `build_candidate_dmap` and its
   digest matches `acceptance.materialized_dmap_ref.digest`.

Any failure anywhere in this chain fails the **whole call** closed
(`Err(String)`) — this reads already-committed state that should never be
corrupt; a failure here is corruption, never silently skipped.

The v2 path performs checks 1-5 but not the extra v1-only check 6, since
there is no v2 analog of `build_candidate_dmap`/`recompute_dmap_digest`
readily available. This is not a safety gap: the actual placements consumed
for cleanup purposes come from the already-integrity-checked
`proposal_record.proposal.placements`, never from `materialized_dmap_ref`
itself — a corrupted `materialized_dmap_ref.digest` value would only ever
make the emitted directive's `previous_dmap_ref` fail to match a real local
runtime's actual historical identity (a missed match, never a false
positive).

## 3. Full-history derivation

`dserver/src/core/ddeploy.rs:2133`, `derive_cleanup_directives_for_node(state, project_id, node_id)`:

1. Cross-version ambiguity guard: the same `acceptance_id` naming two
   different records across `state.acceptances`/`state.phase113_v2.acceptances`
   fails the whole call closed (mirrors `ddeploy_migration::locate_acceptance`'s
   own precedent).
2. Full scan of **both** v1 and v2 acceptance maps, filtered by
   `project_id`, each validated per §2 above.
3. Every historical placement naming `node_id` becomes a candidate
   `(application_id, service_id, dmap_id, dmap_revision, dmap_digest)`
   tuple, collected into a `BTreeSet` — deduplicating identical tuples while
   preserving **every distinct** historical `dmap_ref` (never collapsed to
   "latest per service").
4. For each distinct `application_id` encountered, current authority is
   read **fresh** via `active_placement_authority` (never cached across
   calls). A candidate is dropped if current authority places that exact
   `service_id` on `node_id`; current authority always wins.
5. Surviving candidates become `OperationalCleanupDirective` values, sorted
   deterministically.

`cleanup_directives_for_node(project_id, node_id)` (`ddeploy.rs:2297`) is the
read-accessor: loads persisted state fresh from disk and derives — no
caching, mirroring `active_operational_projection`'s own established
pattern. **No new persisted state anywhere** — this remains a pure function
of already-persisted DDeploy history.

### Multi-hop / duplicate-placement behavior (tests H1–H10)

All in `dserver/src/core/ddeploy.rs`, `mod cleanup_directive_derivation`:

- **H1** (single-hop): directive for the historical source only.
- **H2** (A→B→C): querying either historical node independently yields its
  own directive — no fixed one-hop lookback.
- **H3** (duplicate placement, A→B→A on the same node): two **distinct**
  directives (different `previous_dmap_ref`), never collapsed.
- **H4** (return-to-source): zero directives for a node CURRENT authority
  places again, regardless of history; a *different*, still-vacated node
  still shows its own directive.
- **H5**: repeated derivation over unchanged state is byte-identical.
- **H6**: a `ServiceArtifact`'s current `execution_generation` changing
  never affects derived history — `derive_cleanup_directives_for_node`'s
  signature has no `ServiceArtifactRegistryState` parameter at all, so this
  is structurally guaranteed, and the test makes that guarantee explicit.
- **H7**: a v1-era historical placement survives a later v1→v2 transition
  for the same application (scanning `state.acceptances` directly, never
  through the retired `active_by_application` pointer).
- **H8/H9**: corrupt acceptance↔proposal linkage / proposal digest
  corruption both fail the whole call closed.
- **H10**: the same `acceptance_id` in both v1 and v2 maps fails closed.

## 4. v1/v2 cross-version behavior

A v1→v2 transition (`accept_v2_proposal`'s wrapper in `ddeploy.rs`) removes
only the v1 `active_by_application` **pointer** — it never touches
`state.acceptances` (the history map). The corrected derivation therefore
recovers full cross-version history by scanning both acceptance maps by
`application_id`, never by following a single "current active" pointer
(H7). The reverse direction (v2→v1) is explicitly hardened fail-closed
(§1) rather than assumed safe.

## 5. `OperationalCleanupDirective` — exact shape

Unchanged from Phase 119F/119F-R1's own recommendation (`dserver/src/core/operational_dgraph.rs:121`):

```rust
pub struct OperationalCleanupDirective {
    pub service_id: String,
    pub application_id: String,
    pub previous_dmap_ref: CanonicalContractReferenceV1,
}
```

Server-derived, **never persisted**, **never a placement authority**, no
`desired_state` field, no artifact/generation/digest field (§9 explains
why). DATUM carries its own mirror copy
(`DATUM/src/agent/operational_reconciliation.rs`), exactly matching the
existing convention every other wire type in this codebase already follows.
Wired additively (`#[serde(default)]`) through `OperationalDGraphSlice`,
`ArtifactReconciliationContext`, and DATUM's own
`OperationalReconciliationContext` — a DServer/agent predating this field
sees an empty list, never an error.

**Never routed through `resolve_assignments_from_registry_with_authority`**
or otherwise given an entry in `context.artifacts`: a since-deleted or
renamed historical artifact for one cleanup candidate must never be able to
fail the whole node's context for every other, currently-desired service.

## 6. Cleanup-only context (source node absent from `OperationalDGraph`)

`operational_dgraph_slice_snapshot` (`dserver/src/api/operational_dgraph.rs`)
now distinguishes, when the requested node is absent from `graph.nodes`:

- **No genuine active Resource Registry binding** for this
  `(project_id, node_id)` → `None` (existing fail-closed behavior,
  unchanged).
- **A binding exists, but `cleanup_directives_for_node` derives nothing** →
  `None` (existing fail-closed behavior, unchanged) — cleanup is never
  synthesized from mere graph absence.
- **A binding exists AND at least one genuine directive is derived** → a
  valid cleanup-only `OperationalDGraphSlice`: `desired_services: []`,
  `cleanup_directives: [...]`, and (downstream) `artifacts: []`. The source
  is never added back to any current DMap, and D-Engine (a pure projection
  of current accepted placements) is structurally unaffected.

A node with **both** current work and cleanup obligations (task's required
multi-app scenario) is handled uniformly: `cleanup_directives_for_node` is
called for every context build, whether or not the node is present in
`graph.nodes` — a present node's slice gains its cleanup directives
alongside its unchanged `desired_services`.

## 7. `operation = "cleanup"` — a distinct authorization

Later independent review found reusing `"rollback"` (Phase 119F-R1's own
original recommendation) unsafe for multi-application nodes: ordinary
rollback mode applies to the agent's **entire** per-service dispatch loop
over `desired_services`. An authorization merely saying `"rollback"` would
carry no way to scope itself to only the vacated service's cleanup — a
currently-legitimate co-located service (app B / B1) on the same node could
risk entering rollback semantics it was never meant to.

`AUTHORIZATION_OPERATION_CLEANUP = "cleanup"` (`dserver/src/core/operational_dgraph.rs:41`,
mirrored in DATUM) is a genuinely distinct operation value, issued via a new
route:

```
POST /api/v1/nodes/:node_id/operational-reconciliation/cleanup-authorize
```

(`post_operational_reconciliation_cleanup_authorization` → `authorize_operational_cleanup`,
`dserver/src/api/operational_dgraph.rs`). It reuses **every** applicable
Stage107A issuance gate ordinary reconcile/rollback already enforce: active
Resource Registry binding, exact `project_id`/`dgraph_id`/`dgraph_revision`
match, durable-scope-only issuance, finite lease, `allow_host_mutation`,
resource identity. The **one** structural difference:
`build_reconciliation_authorization` tolerates a target node absent from
`graph.nodes` **only** for `operation == "cleanup"` (§6) — every other
operation still requires node presence, unchanged (regression-tested:
`reconcile_authorization_still_refuses_a_node_absent_from_the_graph`).

A new operation value does **not** imply new authority — it scopes which
agent-side dispatch path the resulting authorization may drive, not what
kind of authority DServer grants.

### Existing `reconcile`/`rollback` unchanged

Neither operation's semantics changed. `rollback` still means "roll back a
currently-desired service," `reconcile` still means ordinary convergence.
Cleanup is independently distinguishable by construction: a single
authorization's `operation` field can be exactly one of the three values,
never more than one, so the ordinary `desired_services` loop and the new
cleanup loop can never both be authorized by the same served authorization.

## 8. Agent-side: `--cleanup` mode, mutual exclusion, non-interference

`smartsentinel-operational-reconcile --cleanup [--execute]` is a new,
independent CLI flag (`DATUM/src/bin/smartsentinel-operational-reconcile.rs`),
mutually exclusive with `--rollback` — checked both at the CLI's own
entrypoint (before any bootstrap/network I/O) and again, defensively, inside
`run_operational_reconciliation_from_bootstrap`.

**Two explicit, unconditional gates**, computed once per cycle in
`operational_reconciliation.rs`:

```rust
let control_plane_authorized = !cleanup_requested && context.authorization.as_ref().is_some_and(|authorization| {
    authorization.operation == expected_authorization_operation /* "reconcile" or "rollback" */
    && /* ...unchanged project/dgraph/revision/node/allow_host_mutation checks... */
});
let cleanup_control_plane_authorized = cleanup_requested && context.authorization.as_ref().is_some_and(|authorization| {
    authorization.operation == AUTHORIZATION_OPERATION_CLEANUP
    && /* ...the SAME project/dgraph/revision/node/allow_host_mutation checks... */
});
```

The `!cleanup_requested` prefix on `control_plane_authorized` is
**deliberate and load-bearing**, not incidental: it guarantees the ordinary
`desired_services` loop can never mutate while `--cleanup` mode is active,
**regardless of what authorization happens to be attached to the served
context** — not merely because `expected_authorization_operation` never
equals `"cleanup"`. This directly satisfies the required multi-app
non-interference property: app B's B1 on the same node is validated/
reported by the unchanged, unmodified per-service loop, but can never be
mutated during a cleanup cycle.

`process_cleanup_directives` (`DATUM/src/agent/operational_reconciliation.rs:4382`)
is the new, independent entry point — called **only** when `--cleanup` was
requested, iterating `context.cleanup_directives` exclusively. It never
touches `desired_services`.

## 9. The explicit generation/projection carve-out

A cleanup directive carries no artifact/generation/digest — no current
server mechanism can reconstruct the HISTORICAL artifact identity a vacated
service actually ran under once the `ServiceArtifact` registry has
advanced, and every existing artifact-join path resolves only against
CURRENT registry state (Phase 119F-R1 §6.1/§6.2). `process_cleanup_directives`
therefore **never calls** `stage107a_service_block_before_image`/
`check_generation_and_projection` for a cleanup directive — this is an
explicit, named, separate code path, never an accidental fall-through of an
`Option::None` artifact match. The replacement safety proof, in full:

```
current authoritative absence (Fact A, §3)
+ exact historical local dmap identity (Fact B, §10/§11)
+ trusted local runtime identity (container_name / stable_service_key)
+ current cleanup authorization (operation == "cleanup", §7)
+ exact operational revision match (unchanged dgraph_revision check)
+ resource identity (unchanged)
+ finite lease (unchanged)
+ the M8 boundary (§12, unchanged)
```

This mirrors, and is narrower work than it first appears next to, an
already-existing precedent: ordinary M8 rollback already deliberately
excludes image/target-identity verification from blocking removal ("a
container whose target image is mismatching is precisely a container that
may need removing" — pre-existing code, `rollback_service_block_from`).
Cleanup's local-identity check (§10/§11) is **additional**, not a
replacement for something rollback already had.

## 10. Container local ownership — new `datum.dmap_*` labels

Three new, additive Docker labels, written **only** at M1 creation time,
from the exact `ArtifactReconciliationAssignment.dmap_ref` that governed
that creation (never recomputed from hostname/container/image):

```
datum.dmap_id
datum.dmap_revision
datum.dmap_digest
```

alongside the existing, unchanged `datum.artifact_id` /
`datum.execution_generation` / `datum.execution_projection_digest`.
Cleanup requires **all** of:

1. `inspected_container_has_execution_identity_labels` — the existing three
   execution-identity labels are **present** (a DATUM-owned container of
   SOME generation) — presence only, never a specific expected value
   (cleanup has no historical generation/digest to compare against, §9).
2. `inspected_container_dmap_ref_matches` — the three new labels **exactly**
   equal `directive.previous_dmap_ref`.
3. A trusted local `DispatchContainerMapping` resolves `directive.service_id`
   to a `container_name` (`trusted_container_mapping_for_service`, resolved
   by `service_id` alone — cleanup has no delivered artifact to
   additionally cross-check `container_name`/`image` against).

`container_cleanup_local_ownership_confirmed` combines checks 1-2; if
either check fails, or the container is absent, cleanup is a safe no-op —
**no `docker rm` occurs**. The removal step itself reuses
`rollback_trusted_container_intent` **completely unmodified** (it already
performs `check_boundary(..., "M8", ...)` internally).

### Compatibility limitation, stated honestly (task section 19)

A container created **before** this change ships lacks `datum.dmap_*`
labels entirely. It is **never** automatically cleanup-matched — cleanup
never falls back to trusting `container_name` alone. Such a container
remains cleanable only once ordinary, unmodified governed reconciliation
(M1/M3 convergence) recreates it under the new labels, should existing
convergence rules ever determine its identity is stale for an unrelated
reason. This is a disclosed, accepted limitation of this phase, not a
silently-patched gap.

## 11. Native-process local ownership

Reuses `ManagedProcessRuntimeStateV1` — already-existing, already-persisted
evidence, **no new fields, no new persistence**.
`process_native_cleanup_directive` (`DATUM/src/agent/native_process_runtime.rs:1619`)
requires an **exact** match of both `application_id` and `dmap_ref` against
`directive.previous_dmap_ref` before proceeding — a matching `dmap_ref` for
a different `application_id` (or vice versa) is never treated as a match.
Once matched, it reuses the **exact same** `governed_stop`/
`governed_remove_state` primitives the existing
`NativeReconciliationAction::Rollback` arm already uses, so every existing
identity defense applies unchanged: PID+`proc_start_time_ticks` reuse
detection, `cmdline_digest`/`observed_executable` mismatch detection,
"never remove state unless stop is positively confirmed" (Section AP fault
matrix). Cleanup adds an **earlier**, additional gate (the dmap_ref match)
before either primitive is ever reached — it relaxes nothing either
primitive already does.

## 12. Stage107A / M8 boundary — unchanged

Both runtime kinds' actual host mutation is gated by the **same,
unmodified** `check_boundary(lease, boundary_context, "M8", records)` call
immediately before the destructive action (`rollback_trusted_container_intent`
for containers; `governed_stop`+`governed_remove_state`'s own N4/N6
boundaries for native processes). No new mutation primitive was invented;
cleanup reuses the existing M8 boundary and its existing lease/freshness
semantics exactly.

## 13. Stale cleanup authorization

Unchanged mechanism, re-verified: a cleanup authorization is bound to
`dgraph_revision` at issuance exactly like ordinary reconcile/rollback.
`cleanup_control_plane_authorized`'s exact-match check on
`authorization.dgraph_revision == Some(context.dgraph_revision)` rejects a
now-stale authorization the moment a later acceptance changes the served
context's revision — the scenario "cleanup authorization issued for A, then
B→A returns S to A before A acts" fails closed via this pre-existing
revision-freshness gate, with zero new code. Regression:
`cleanup_authorization_still_enforces_dgraph_revision_freshness`.

## 14. Negative proofs (M16/M17)

Every one of the following remains non-authoritative, unchanged: a 404/
missing-node context, a transport error, a missing Resource Registry
binding, malformed state, an expired lease, wrong resource identity, a
stale revision. Each produces the existing error path (or, for cleanup
specifically, `None` from `operational_dgraph_slice_snapshot`) — never a
synthesized directive, never an authorized mutation. `derive_cleanup_directives_for_node`
itself can only ever produce a directive from a genuinely reconstructed,
integrity-checked historical acceptance plus a genuine current-absence
check (§3) — this is a structural property of the function, re-verified
against H8/H9/H10's fail-closed corruption tests.

## 15. D-Engine / DMonitor — unaffected

`derive_active_dengine_nodes` (unchanged) takes only a
`PlacementAuthorityView` and cannot be affected by anything cleanup
computes, since cleanup never changes actual accepted placements. No
cleanup-related code path writes to, or is read by, any D-Monitor
mechanism; cleanup directives carry no D-Monitor identity and are not a
health/observability signal.

## 16. Tests

- **Server** (`dserver/src/core/ddeploy.rs`): H1-H10 (derivation, §3), 2
  v1-downgrade regression tests (§1), 4 new `AUTHORIZATION_OPERATION_CLEANUP`
  tests in `operational_dgraph.rs` (node-absence carve-out, revision
  freshness, `allow_host_mutation` requirement, unsupported-operation
  rejection).
- **Agent** (`DATUM/src/agent/operational_reconciliation.rs`): 6 pure
  container-cleanup comparison tests (label presence/absence/mismatch,
  `container_state_from_inspected`, `trusted_container_mapping_for_service`).
- **Agent** (`DATUM/src/agent/native_process_runtime.rs`): 4
  `process_native_cleanup_directive` tests (no root configured, no
  persisted state, dmap_ref mismatch, application_id mismatch) — a
  "successful removal" end-to-end test is not attempted on this
  (non-Linux) development machine, consistent with this file's own
  pre-existing, disclosed testing convention (`governed_stop`'s real
  signal-based confirmation is Linux-only; every other rollback-path test
  in this file accepts the same limitation).
- **Full regression**: 1921 dserver tests pass (1920 baseline + this
  phase's new H6 test), zero failures. 1239 DATUM lib tests pass (1229
  baseline + 10 new), zero failures. Full DATUM test suite (all binaries)
  passes with zero failures. Both `--release` builds succeed.

### Disclosed testing gap: cleanup-only context (C1-C8)

`operational_dgraph_slice_snapshot`'s new cleanup-only branch is **not**
integration-tested in `dserver/src/api/operational_dgraph.rs`, unlike every
other function in that module's test suite. Verified directly in this
phase: `cleanup_directives_for_node` reads the real, process-shared SQLite
control-state database via `resolved_db_path()` → `default_control_state_db_path()`,
which has no per-test override or injection point anywhere in this
codebase — confirmed by reading `resolved_db_path`'s body, which hardcodes
the real path unconditionally. Every existing test in this module instead
operates purely on the in-memory `graph_store()`/`authorization_store()`
statics, never on this shared file. Running new tests against the real
shared database was confirmed, in this phase, to intermittently fail with
"database is locked" under this test binary's default parallel execution.

The underlying LOGIC this branch composes is independently, thoroughly
tested: `derive_cleanup_directives_for_node` (H1-H10, above) and
`active_resource_binding_for_node`/`authorization_is_valid_for_context`/
`build_operational_slice` (pre-existing coverage elsewhere in this file).
Recommended follow-up, not attempted in this phase: either a dedicated,
injectable per-test SQLite path, or refactoring `operational_dgraph_slice_snapshot`
to accept `DDeployStateV1` by parameter rather than loading it internally.

## 17. Remaining non-blocking debt

- The Firebase-bootstrap coupling (Phase 119E Finding #2, restated in
  Phase 119F §28) remains out of scope — relevant only to live physical
  re-validation, not to this implementation's correctness.
- The C1-C8 integration-testing gap (§16) is a disclosed, accepted
  limitation, not a correctness gap in the underlying logic.
- v2→v1 downgrade remains explicitly unsupported (§1) — a dedicated
  reverse-version-transition design remains a separate, future piece of
  work if ever needed.

## 18. What Phase 119G deliberately did NOT do

- No new server-side persisted state anywhere (cleanup directives remain
  fully stateless/recomputed, per Phase 119F-R1 §5.1/§8).
- No change to `desired_state`'s two-value vocabulary, to
  `union_project_operational_graph`, or to D-Engine.
- No v2→v1 downgrade support (explicitly hardened fail-closed instead).
- No live RPi/fog/cloud validation — recommended as Phase 119H, once this
  local implementation has been reviewed.
