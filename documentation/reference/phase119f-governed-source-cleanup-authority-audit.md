# Phase 119F — Governed Source Cleanup Authority Audit

- Status: Focused design/audit only. No production code was added or
  changed by this phase.
- Branch: `agent/phase119-migration`, continuing from Phase 119E closure
  HEAD `3a823bfda408208c7b6c867e5e1a29ea3816258e`.
- Scope: select the smallest safe mechanism by which a D-Node that
  previously hosted a DATUM-managed service can learn, after that service
  migrates away, that the old runtime is no longer desired and may be
  removed through the existing Stage107A-governed mutation path.

## 1. Executive conclusion

**The missing piece is narrower than it first appears.** Independent
tracing in this phase found that both runtime kinds already possess strong,
restart-safe, DATUM-verifiable local identity evidence: native-process
runtimes already persist a rich historical record
(`ManagedProcessRuntimeStateV1`, including the exact `dmap_ref` a process
was spawned under, PID+start-time for reuse detection, and a full
`/proc/<pid>/cmdline` digest); container runtimes already carry
`docker run --label datum.artifact_id=...` ownership labels, verifiable via
`docker inspect` independent of the CLI process. **Fact B (exact removable
local runtime identity) is therefore largely already solved for both
runtime kinds — no new local inventory (F5) needs to be invented from
scratch.** What genuinely does not exist is **Fact A**: an authoritative,
per-service, server-derived signal that a specific service is no longer
desired on a specific node. The recommended design (§24) supplies exactly
that signal — a small, **stateless, freshly-derived-every-request**
`OperationalCleanupDirective` list, computed purely from already-persisted
DDeploy acceptance history compared against current authority, served
**structurally separate** from `desired_services` (never overloading
`desired_state`, never becoming a second placement authority) — and lets
the agent apply it using the identity evidence that already exists.
**F2 alone is proven insufficient** by direct trace of the agent's own
per-service loop, not merely asserted. No production code was written in
this phase; the design below is for Phase 119G's future implementation.

## 2. Closure blocker restated

Phase 119E (§10/§13/§18/§21, re-read in full this phase) established: once
a service's last placement leaves a node, that node drops entirely out of
the project's `OperationalDGraph`, `operational-reconciliation-context`
returns `"node slice not found"` rather than an empty-but-known slice, and
no orphan-sweep or dropped-service handling exists anywhere in the DATUM
agent crate. M16 ("cleanup cannot be guessed from missing/unreachable
server state") and M17 ("absent desired state must be authoritative before
removal") must both be preserved by any fix.

## 3. Current OperationalDGraph behavior (re-verified fresh)

Re-read `union_project_operational_graph` (`dserver/src/core/ddeploy.rs`)
directly this phase: `merged_by_node` is built **exclusively** by iterating
`app.active_dmap.placements` for every currently-active application in a
project (line ~1991: `for placement in &app.active_dmap.placements`). A
node that is not named by any current placement receives no key in
`merged_by_node` and therefore no entry in the final
`OperationalDGraph.nodes` vector. No historical node participation is
preserved anywhere in this function — confirmed by reading its complete
body, not merely its signature.

## 4. Current agent behavior (re-verified fresh)

Re-read `operational_dgraph.rs` (dserver core) and
`operational_reconciliation.rs` (DATUM agent) directly:

- `OperationalNodeDesiredState.desired_services` is structurally allowed to
  be an empty `Vec` (`#[serde(default)]`, no non-empty check in
  `validate_operational_dgraph`) — **F2 is not blocked by validation.**
- `build_operational_slice` looks up the requested node in `graph.nodes`
  via `.find(...).ok_or_else(...)`, producing a hard error when the node is
  entirely absent from the graph — never an empty-but-present slice
  (confirmed by direct re-read, `operational_dgraph.rs` line ~342–353).
- `desired_state` validation (`validate_desired_service`) accepts
  **exactly** `"running" | "completed"` — no `"absent"`/`"stopped"` value
  exists anywhere in the current contract.
- The agent's per-service dispatch (`operational_reconciliation.rs`,
  `for service in &context.desired_services`) only ever visits entries
  **present** in the served list. **Confirmed directly: even if a node's
  slice were served with an authoritative, present-but-empty
  `desired_services: []` (i.e., F2 implemented exactly as described), this
  loop would iterate zero times and never reach any mutation logic for any
  previously-running service.** F2 alone cannot drive removal — this is a
  structural fact of the current loop, not a hypothesis.

## 5. Existing M8/rollback prerequisites (traced end to end)

Server side (`dserver/src/core/operational_dgraph.rs`,
`authorize_operational_reconciliation`, re-confirmed): a durable-scope
authorization request must supply `dgraph_id`, `dgraph_revision`,
`operation` (`"reconcile"` or `"rollback"`), `allow_host_mutation: true`,
and is checked against an **active Resource Registry binding**
(`active_resource_binding_for_node`) — refusing with 409 otherwise. The
issued `OperationalReconciliationAuthorization` carries a `snapshot`
(the artifact bindings the server believed governed the node at issuance)
and a finite `expires_at_utc`.

Agent side (`operational_reconciliation.rs`, re-confirmed): `control_plane_authorized`
requires the served authorization's `operation`/`project_id`/`dgraph_id`/
`dgraph_revision`/`node_id` to **exactly match** the currently-served
context (line ~1299–1310) — this is already the existing revision-binding
guard that would also protect any future cleanup authorization (§17). For
each service in `context.desired_services`, the runtime-kind dispatch
resolves a `DispatchContainerMapping` (local `config.toml`) or routes to
`native_process_runtime::reconcile_native_service`; **only then** does
`check_boundary(lease, boundary_context, "M8", ...)` (the Stage107A
finite-lease gate) run, immediately before `rollback_trusted_container_intent`'s
`docker rm -f` or the native-process rollback path's stop/state-removal.

**Exact minimum inputs M8/removal require, proven from code:**
1. A `desired_services` entry for that exact `service_id` (even under
   `--rollback`) — without one, the per-service loop never reaches this
   service at all.
2. A matched artifact assignment (`context.artifacts`) for that service_id.
3. A valid, current, non-expired authorization whose
   `operation`/`dgraph_id`/`dgraph_revision`/`node_id`/`project_id` all
   match the served context exactly.
4. A local trusted-identity resolution: `DispatchContainerMapping`
   (container) or `native_process_state`'s persisted record
   (native_process) for that exact service.
5. A passed Stage107A lease/generation/projection gate
   (`check_boundary`/`stage107a_service_check`).
6. Correct verified resource identity (`resource_id` match,
   `BLOCKED_WRONG_RESOURCE_IDENTITY` otherwise).

## 6. Why F2 alone is insufficient

Directly proven in §4: the per-service loop has nothing to iterate when
`desired_services` is empty, so items 1–2 and 4 of §5's list can never be
satisfied for a vacated service under F2 alone, regardless of how
authoritative the empty slice is. F2 answers a different, narrower question
("is this node structurally known and currently zero-desired, vs.
genuinely unreachable/unknown") — a useful distinction (§11, §30) but not,
by itself, a removal signal.

## 7. Authoritative absence vs. runtime identity

Kept strictly separate throughout this audit, per the task's own framing:

- **Fact A** (authoritative absence): "service S is no longer desired on
  node A under CURRENT accepted placement authority." Derivable **purely**
  from already-persisted DDeploy acceptance history (§10) — no new
  persistence required.
- **Fact B** (exact removable local identity): "this concrete
  process/container was previously created/governed by DATUM as service S
  on node A." Already substantially satisfied by existing mechanisms for
  both runtime kinds (§8/§9) — no new inventory required from scratch.

A generic 404/timeout/auth-failure proves **neither** fact, and no part of
this audit's recommendation treats it as doing so (M16, preserved
throughout).

## 8. Container identity evidence

`operational_reconciliation.rs` (re-confirmed, `ExecutionIdentityLabels`,
lines ~1095–1122): every container M1 creates is labeled
`datum.artifact_id`, `datum.execution_generation`,
`datum.execution_projection_digest` via `docker run --label`. Convergence
(`baseline_inspect`/P1-3) already re-checks a running container's
`Config.Labels` against these exact expected values, treating a container
with **no** `datum.*` labels at all as pre-Stage-107A/externally-created
(never touched). **A container can already be independently proven
DATUM-created, for a specific artifact/generation/digest, by `docker
inspect` alone — surviving agent-process restart entirely, since Docker
itself (not the CLI) is the persistence layer.** The one gap: these labels
do not carry `service_id`/`application_id`/`dmap_ref` directly — that
association currently lives only in the node-local `config.toml`
`DispatchContainerMapping` (`service_name`/`container_name`/`artifact`),
which is static configuration, not per-migration state, and therefore
already survives restarts and migrations by construction (it is written
once by deployment tooling, not per-placement).

## 9. Native-process identity evidence

`DATUM/src/agent/native_process_state.rs`'s `ManagedProcessRuntimeStateV1`
(re-read in full this phase) already persists, keyed by
`stable_service_key(project_id, node_id, service_id)` and surviving agent
restart by design (a file under the configured `native_process_root`):
`service_id`, `artifact_id`, `execution_generation`,
`execution_projection_digest`, `application_id`, **`dmap_ref: Option<CanonicalContractReferenceV1>`**
(the exact accepted DMap `{id, revision, digest}` this process was spawned
under), `content_sha256`, `pid`, `proc_start_time_ticks`,
`observed_executable`, `cmdline_digest` (a hash of the complete observed
`/proc/<pid>/cmdline`, not the desired argv), `started_at_utc`. This
**already** defends against exactly the risks the task raised: PID reuse
(via `pid`+`proc_start_time_ticks` together), executable replacement (via
`cmdline_digest`/`observed_executable`/`content_sha256` comparison in
`observe_persisted_identity`), and — most valuably — **already carries the
exact historical `dmap_ref`** needed to disambiguate the "service returns
to source" scenario (§16) without any new field.

## 10. Historical server evidence

`OperationalDGraphPersistedState` (`dserver/src/core/operational_dgraph_persistence.rs`,
re-confirmed) persists `authorizations: BTreeMap<...,
OperationalReconciliationAuthorization>` durably. However, an
`OperationalReconciliationAuthorization` is created **at issuance time**,
before the agent attempts anything — it proves an authorization *could
have been used*, never that reconciliation *actually succeeded*. DServer
has no reliable, structural feedback channel for "the agent's mutation
succeeded" (the only optional feedback path,
`POST /api/v1/snapshots`, is the same unrelated legacy telemetry mechanism
Phase 119D/119E already found Firebase-coupled and unreliable). **Server-side
history can identify candidate past authorizations but cannot prove actual
local success — only the agent's own local evidence (§8/§9) can.** This
confirms the task's own suspicion in §18 directly.

## 11. Current ServiceArtifact safety

**Unsafe to assume equal to historical identity.** A `ServiceArtifact`'s
`execution_generation`/`execution_projection_digest` can change after an
old runtime was spawned (a normal DCode/config update, unrelated to
migration). Any cleanup mechanism that authorized removal using the
**current** ServiceArtifact identity, rather than the **historical** one
the old runtime actually ran under, could target the wrong generation.
This is exactly why `ManagedProcessRuntimeStateV1`'s own
`execution_generation`/`execution_projection_digest`/`dmap_ref` fields are
valuable: they are the actual-spawn-time identity, never recomputed,
exactly parallel to how `DispatchContainerMapping` already ties container
removal to a fixed, per-service `service_id`/`container_name`/`artifact`
config triple rather than "whatever the registry currently says."

## 12. F2 evaluation

**Structurally legal, cleanly implementable, but insufficient alone (§6).**
Its real value is narrower and more specific: distinguishing "structurally
known node, project-relevant, currently zero desired services" from
"unknown node/wrong project/unreachable" (§30). Recommended as a
**supporting, not sufficient,** element of the final design — see §24.

## 13. F4 evaluation

**Necessary.** An explicit, per-service, authoritative directive is the
only way to give the agent's existing per-service loop something to act
on for a vacated service. Key design questions answered:

1. Does a tombstone become a second placement authority? **Must not, and
   the recommended shape (§24) structurally cannot** — it carries no
   `desired_state: "running"`-equivalent claim and is served in a field
   entirely separate from `desired_services`, never contributing to
   D-Engine (D-Engine is defined purely from *current* DMap placements,
   unchanged, §16).
2. Derived from previous vs. current placement — **yes, exclusively**
   (§24's derivation).
3. Persists how long — **not persisted at all; recomputed fresh every
   request** (§20/§26) — the simplest, most restart-safe answer available.
4. Ack/removal — **none needed**; re-derivation naturally stops once the
   historical diff no longer shows the service as removed, and re-applying
   an already-satisfied directive against an already-absent local runtime
   is already a safe no-op (`ContainerState::Missing` → `"rollback_already_absent"`,
   `false` mutation; the native-process equivalent is symmetric).
5. Multi-application scope — **naturally correct**, since the diff is
   computed per application, per node (§16).
6. Exact artifact/runtime identity for M8 — **supplied by the historical
   `dmap_ref`/prior placement record**, cross-checked against the agent's
   own already-existing local identity evidence (§8/§9), never invented.

## 14. F5 evaluation

**Not required as new work.** The search in §8/§9 found the F5-shaped
capability essentially already exists for both runtime kinds (native
process: full persisted historical record; container: Docker-level
ownership labels plus static config mapping). No new local
inventory/tracking mechanism needs to be built from scratch — the
recommended design (§24) is explicitly F4 + *reuse* of existing identity,
not F4 + a new F5.

## 15. Combined-option comparison

| Option | Removal authority clarity | Local target identity | Restart safety | Offline-node safety | Multi-app correctness | New persistence |
|---|---|---|---|---|---|---|
| 1 — F2 only | Clear (empty ≠ unknown) | **None** | N/A | N/A | N/A | None |
| 2 — F4 only | Clear | Relies on agent already having it | Yes (stateless server derivation) | Yes (derived fresh on next fetch, whenever it happens) | Yes (per-app diff) | None |
| 3 — F5 only | Unclear (no authority signal) | New, redundant with what exists | Depends on new design | Unclear | Unclear | New (unnecessary) |
| 4 — F2+F5 | Partial | Redundant | Depends | Unclear | Unclear | New (unnecessary) |
| **5 — F2+F4 (recommended)** | **Clear** | **Reuses existing evidence** | **Yes** | **Yes** | **Yes** | **None** |
| 6 — F4 + explicit new local trusted-identity contract | Clear | Reuses + formalizes | Yes | Yes | Yes | Small (optional hardening, not required for v1) |

**Recommended: Option 5 (F2+F4), with Option 6's "formalize the contract"
idea deferred as an optional 119G/119H hardening step, not required for
correctness.**

## 16. Multi-application safety

Confirmed by construction (§24's derivation is per-application): the
required scenario (app A's `A1` moves N→M while app B's `B1` stays on N)
produces a cleanup directive scoped to `{project, application: A,
service: A1, node: N}` only — app B's own previous-vs-current diff shows no
change, so no directive is ever generated for `B1`. The derivation never
inspects "is the node now empty," only "did *this application's* placement
for *this service* change."

## 17. Stale-cleanup race

Directly protected by the **existing, unmodified** authorization-freshness
check (`control_plane_authorized`'s exact-match requirement on
`dgraph_revision`, §5). If S returns to A in a later acceptance
(revision N+1) after a cleanup authorization was issued against revision N,
any re-fetch of the context after N+1 carries `dgraph_revision: N+1`; the
old authorization's `dgraph_revision: N` fails the equality check and is
rejected — **zero new code required for this protection**, since cleanup
authorizations would be bound to `dgraph_revision` exactly like ordinary
reconcile/rollback authorizations already are.

## 18. Node-offline semantics

Since cleanup directives are **never persisted, only recomputed fresh from
already-persisted DDeploy history on every request** (§13, point 3), an
offline node simply sees the same (correct) directive the moment it next
successfully fetches its context — no expiry, no loss, no special-case
handling needed. This is a direct consequence of the "derive, don't
persist" design choice, not an incidental benefit.

## 19. DServer restart semantics

Same reasoning as §18: nothing new is persisted server-side, so a DServer
restart loses nothing new — the directive is recomputed identically from
`DDeployStateV1.acceptances` (already durably persisted, unmodified) the
next time it is requested.

## 20. Agent restart semantics

Native-process identity (`ManagedProcessRuntimeStateV1`) is file-persisted
under `native_process_root` and explicitly designed to survive agent
restart (its whole purpose, per its own module doc, re-confirmed this
phase). Container identity (`docker.*` labels) survives at the Docker
daemon level, entirely independent of the CLI process. **Neither identity
source depends on any new volatile in-memory state this design would add**
— consistent with the task's own warning that volatile in-memory inventory
alone would be insufficient, and correctly avoided by not needing to add
any.

## 21. DMap v1/v2 parity

The recommended derivation (§24) operates on `CanonicalContractReferenceV1`
values and DDeploy's already-existing acceptance history
(`DDeployAcceptanceV1`/`DDeployAcceptanceV2`, both already exposing
`materialized_dmap_ref` in this same normalized shape, per Phase 119C's own
`locate_acceptance` normalization pattern, reused conceptually here). No
new v1-only or v2-only logic is introduced; the same seam Phase 119B/C
already established for normalizing v1/v2 acceptance identity is the
correct model to reuse for finding "the immediately-prior acceptance for
this application," regardless of which DMap generation either acceptance
belongs to.

## 22. Stage107A authorization model

**Recommend C1-shaped: reuse `operation = "rollback"` with a cleanup-scoped
snapshot**, rather than inventing a new `operation = "cleanup"` value
(C2) or overloading the existing `"rollback"` context with implicit
semantics (C3). Rationale: `"rollback"`'s existing semantic — "remove a
previously-created, now-unwanted governed runtime, under a fresh,
explicitly-scoped authorization" — is already an exact match for source
cleanup; the *authorization request* already carries `dgraph_id`/
`dgraph_revision`/`node_id`/`operation`, and only the **context's**
`desired_services`/new cleanup-directive list needs to change to carry the
one new piece of information (which exact service, under which historical
`dmap_ref`, is being cleaned up) — this does not weaken Stage107A in any
way; every existing gate (lease, resource identity, generation/projection)
remains required unchanged.

## 23. M16/M17 preservation

Explicitly re-checked against the recommended design: a directive is
**only** ever produced when (a) a **specific prior acceptance** for the
exact `(project, application, service)` tuple is found in already-persisted
history, AND (b) the **current** active authority (from the same
`active_placement_authority` seam Phase 118/119 already use, never a new
read) genuinely does not place that service on that node. No transport
failure, timeout, missing Firebase, missing Resource Registry binding,
corrupted state, or unknown node can produce a directive — each of those
conditions instead produces an ordinary error (matching current behavior
exactly), never a synthesized directive. **M16 and M17 both hold by
construction of the derivation itself**, not merely by policy.

## 24. Recommended architecture

**A pure, server-derived, per-node `Vec<OperationalCleanupDirective>`,
served structurally alongside (never merged into) `desired_services`,
computed fresh on every request from already-persisted DDeploy acceptance
history compared against current active authority, consumed by the agent
through its own already-existing local identity evidence.**

Conceptual shape (not implemented in this phase):

```rust
/// Server-derived, NEVER PERSISTED, NEVER a placement authority. Recomputed
/// fresh on every operational-reconciliation-context request from
/// already-persisted DDeploy acceptance history. Structurally separate from
/// `desired_services` -- a directive here is never confused with "this
/// service is desired running here."
pub struct OperationalCleanupDirective {
    pub service_id: String,
    pub application_id: String,
    /// The exact accepted D-Map this service was placed under the LAST
    /// time it was desired on this node -- never independently recomputed,
    /// copied verbatim from the historical acceptance record.
    pub previous_dmap_ref: CanonicalContractReferenceV1,
}
```

Derivation (conceptual, for a given `project_id`/`node_id`): for each
application with a current `ActiveApplicationDeployment`, find its
immediately-prior acceptance (the accepted record with the next-lower
revision for the same application, from the already-persisted
`acceptances` map — no new persistence). If that prior acceptance placed
`service_id` on `node_id`, and the **current** active DMap does not, emit
one `OperationalCleanupDirective`. This is a pure function over existing
state, symmetric across DMap/1 and DMap/2 (§21), and requires no schema
migration to any persisted structure.

The reconciliation context served to the agent gains one new, optional
field (`cleanup_directives: Vec<OperationalCleanupDirective>`), separate
from `desired_services`. The agent, for each directive, resolves the exact
same `DispatchContainerMapping`/`native_process_state` lookup it already
performs for ordinary rollback (§8/§9), and only proceeds through the
**unchanged** Stage107A `check_boundary`/M8 gate if a matching local
identity is actually found — if no local trusted mapping/persisted record
exists for that service, the directive is a safe no-op (there was nothing
DATUM-owned to remove).

## 25. Minimum Phase119G slice

1. A new, non-persisted `OperationalCleanupDirective` type (`dserver/src/core/operational_dgraph.rs`
   or `ddeploy.rs`, colocated with the existing `OperationalDesiredService`).
2. A pure derivation function (likely in `ddeploy.rs`, beside
   `union_project_operational_graph`, e.g.
   `derive_cleanup_directives_for_node`), operating only on
   `DDeployStateV1`'s already-persisted `acceptances`/`active_by_application`.
3. Extend `ArtifactReconciliationContext`/`OperationalDGraphSlice`
   (`dserver/src/core/operational_dgraph.rs`, `artifact_reconciliation_context.rs`)
   with the new field, additive only (`#[serde(default)]`), never changing
   `desired_services`'s own shape.
4. Agent-side: a new, narrow handling branch in
   `operational_reconciliation.rs` that, for each served
   `cleanup_directive`, resolves existing local identity and — only if
   found — routes through the **existing, unmodified**
   `rollback_trusted_container_intent`/native-process rollback path.
5. No new authorization operation value; reuse `"rollback"` (§22).
6. No new persisted state anywhere, server or agent.

## 26. Required tests

- Pure unit tests for the derivation function: single-app move (directive
  emitted); multi-app co-location (only the moved service's directive
  emitted, §16); service returns to source before the directive is ever
  served (no directive emitted, since current placement now matches);
  stale directive superseded by a later acceptance moving the service back
  (existing `dgraph_revision` check rejects the stale authorization, §17);
  v1-sourced and v2-sourced prior acceptances both produce equivalent
  directives (§21); no directive for a node that was never structurally
  known to the project; no directive for a project mismatch.
- Agent-side unit tests: a served cleanup directive with no matching local
  identity (container mapping absent, no persisted native-process state) is
  a safe no-op; a served directive with matching local identity, under a
  valid fresh rollback authorization, reaches the exact same `check_boundary`/M8
  path ordinary rollback already uses; a served directive under an
  authorization whose `dgraph_revision` does not match the current context
  is rejected exactly like ordinary reconcile/rollback already is; PID
  reuse/executable-mismatch cases (already-existing native-process identity
  tests) continue to correctly block removal for an unrelated process
  wearing the same name.
- Explicit negative tests proving M16/M17: a 404/missing-node context, a
  transport error, and a missing Resource Registry binding each produce the
  existing error path, never a synthesized directive or a mutation.

## 27. Future live re-proof

Once implemented, the eventual live proof should reuse Phase 119D's exact
topology (RPi `rpi-119d` + fog-01 `fog-119d`) and repeat the same A→B→A
sequence, this time additionally showing: after cutover, node A is served a
`cleanup_directives` entry for `probe`; a fresh `"rollback"`-operation
authorization is obtained for A; `smartsentinel-operational-reconcile
--rollback --execute` removes the real native-process runtime on A; a
repeat run is a safe no-op; and the same sequence in reverse after the
rollback-to-A transition removes the real runtime on B. This live proof
still depends on resolving the separate Firebase-bootstrap finding (§28) or
working around it, whichever is chosen when 119G/live-proof work begins —
not decided in this audit.

## 28. Remaining non-blocking debt

- The Firebase-bootstrap coupling (Phase 119E's Finding #2) remains
  explicitly out of this phase's scope, per instruction — it is separate
  foundation-hardening debt, relevant only to the eventual live
  re-validation environment, not to this design's correctness.
- The one cosmetic trailing-whitespace byte already noted in Phase 119E's
  own review (in the already-committed 119D document) remains untouched,
  consistent with this phase's own "stage only the new document"
  instruction.
