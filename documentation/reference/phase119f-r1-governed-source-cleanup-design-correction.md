# Phase 119F-R1 — Independent Correction of the Governed Source-Cleanup Design

- Status: Focused design/audit correction only. No production or test code was
  added or changed by this phase.
- Branch: `agent/phase119-migration`, continuing from the Phase 119F evidence
  bundle, HEAD `1015aa586ba3edc2aa8baaaa3e8db83f0288382e`.
- Scope: independently attempt to disprove two concerns raised about Phase
  119F's *recommended future design* (§22, §24 of
  `phase119f-governed-source-cleanup-authority-audit.md`), and, since both
  concerns survive disproof attempts, correct the design before any Phase
  119G implementation begins.
- This document does **not** retract any of Phase 119F's evidence about the
  codebase as it exists today (E01–E14, re-confirmed unchanged in this pass).
  It corrects a forward-looking recommendation that was under-specified.

## 1. What Phase 119F got right

Re-verified in this pass, unchanged:

- **The closure blocker itself (§2–§6).** A node that loses its last
  placement disappears entirely from `OperationalDGraph`
  (`union_project_operational_graph`, E01); a missing node hard-errors
  rather than receiving an empty slice (`build_operational_slice`, E02); an
  empty `desired_services` is structurally legal (E03) but the agent's
  per-service loop iterates zero times over it (E05) — so an authoritative
  empty slice alone (F2) cannot drive removal. This is still correct and is
  the correct framing of the problem.
- **Fact A / Fact B separation (§7).** Splitting "is S still desired on N"
  (Fact A, a server-authority question) from "what concrete
  process/container was DATUM's" (Fact B, a local-identity question) remains
  the right decomposition and is retained unchanged in the corrected design
  below.
- **F5 is largely already solved (§8, §9, E07, E08).** Native-process
  identity (`ManagedProcessRuntimeStateV1`, including `dmap_ref`,
  `cmdline_digest`, `pid`+`proc_start_time_ticks`) and container identity
  (`datum.artifact_id`/`datum.execution_generation`/
  `datum.execution_projection_digest` labels) both already exist and both
  survive their respective process's restart. This remains true.
- **F2+F4, never persisted, is the right shape (§12–§15, §18–§20).**
  Deriving a cleanup signal fresh on every request, from already-persisted
  DDeploy history compared against current authority, rather than
  persisting any new cleanup-workflow state, is still the correct choice —
  it is what makes node-offline, DServer-restart, and agent-restart
  semantics trivially correct by construction, and the corrected derivation
  in §5 below preserves this property exactly.
- **Reusing `operation = "rollback"` at the authorization level (§22,
  partially).** Not inventing a new authorization *operation* value is still
  correct — see §6 below for the one necessary refinement to what that
  authorization is allowed to certify.
- **M16/M17 preservation reasoning (§23).** The principle that only a
  specific prior acceptance plus a genuine current-absence check may ever
  produce a directive, and that no transport failure/missing binding/
  unknown node may ever produce one, is correct and is re-verified against
  the corrected derivation in §7 below.
- **D-Engine unaffected (E12, §16 of this document's evidence).**
  `derive_active_dengine_nodes` takes only a `PlacementAuthorityView` and
  cannot be affected by any cleanup mechanism that doesn't also change
  actual accepted placements — unaffected by anything in this correction.

## 2. What independent evidence review found incomplete

Two problems were found in Phase 119F's §22/§24 *recommendation* — not in its
description of current code:

- **R1-A**: the recommended derivation compares CURRENT acceptance against
  only the *immediately-prior* accepted revision for an application. This
  loses any node that was displaced two or more revisions ago and has not
  reconnected since — an old DATUM-managed runtime on such a node could
  survive forever, contradicting Phase 119F's own claimed offline-node
  property.
- **R1-B**: the recommendation states "every existing gate (lease, resource
  identity, generation/projection) remains required unchanged" for a
  cleanup authorization, but no current server mechanism can reconstruct
  the HISTORICAL artifact identity (`execution_generation`/
  `execution_projection_digest`) a vacated service actually ran under, once
  the `ServiceArtifact` registry has since advanced — and the recommended
  `OperationalCleanupDirective` shape itself carries no artifact/generation
  field to check in the first place. The generation/projection claim is
  therefore not achievable as literally stated.

## 3. R1-A: independent verdict — **VALID**

### 3.1 The exact mechanism that fails

Phase 119F §24 states the derivation as: "find its immediately-prior
acceptance (the accepted record with the next-lower revision for the same
application ... ) ... If that prior acceptance placed `service_id` on
`node_id`, and the current active DMap does not, emit one
`OperationalCleanupDirective`."

This compares **current (revision N) vs. exactly revision N−1**. It never
looks at revision N−2, N−3, etc.

### 3.2 The counterexample from the task, traced precisely

```
rev1: S -> A     (governed runtime actually starts on A)
      A goes offline
rev2: S -> B
      (A still offline; never sees rev2)
rev3: S -> C
      A reconnects only now, after rev3
```

At revision 3, "current vs. immediately-prior" compares rev3 (current: `S ->
C`) against rev2 (immediately-prior: `S -> B`). Node A appears in neither
side of that one comparison — it only appears in rev1, which is never
consulted. **No cleanup directive is ever produced for A.** The rev1 runtime
on A survives indefinitely, even though A is online and asking. This
directly contradicts §18's claimed "an offline node simply sees the correct
directive the moment it next successfully fetches its context" — that claim
is only true for a node displaced by the *single most recent* transition,
not for one displaced two or more revisions ago.

**Verdict: R1-A is valid.** This is a real, reachable gap in the recommended
derivation, not a hypothetical edge case — two acceptances is an ordinary
occurrence (any application that migrates twice while a node happens to be
offline for both).

## 4. Multi-hop and duplicate-placement analysis (H1–H6)

| Case | Scenario | Accepted placement history | Runtime that could actually exist | Current placement | Cleanup obligation |
|---|---|---|---|---|---|
| H1 | A→B→C, A offline throughout | rev1:A, rev2:B, rev3:C | rev1 runtime on A (never touched) | C | A must be cleaned (rev1 `dmap_ref`); "immediately-prior" (§3) misses this — **this is R1-A itself, generalized** |
| H2 | A→B→A→C, A offline during 2nd A placement | rev1:A, rev2:B, rev3:A, rev4:C | Could be the rev1 runtime (if A never actually reconciled rev3) OR the rev3 runtime (if it did) | C | A must be cleaned; the directive must not assume it knows WHICH of two possible historical `dmap_ref`s is actually running — see §5.3 |
| H3 | A→B→C→D, several old nodes reconnect late | rev1:A, rev2:B, rev3:C, rev4:D | possibly A's rev1, B's rev2, C's rev3 runtimes, independently | D | A, B, and C must each independently be cleanable whenever each reconnects — requires the FULL history, not a fixed lookback of one revision |
| H4 | A→B, cleanup on A succeeds, later B→C | rev1:A, rev2:B, rev3:C | none on A (already cleaned); possibly B's rev2 runtime | C | Repeated derivation must keep re-emitting "A had rev1" candidates harmlessly forever (idempotent no-op once A's local evidence says absent) — see §8 |
| H5 | A→B→A | rev1:A, rev2:B, rev3:A | current A runtime, created under rev3 | A (again) | **No candidate may ever be emitted for A** while current placement is A — the current-authority check must be re-evaluated fresh, never cached from an earlier derivation (§7) |
| H6 | Placement AND ServiceArtifact generation both change repeatedly | any of the above, plus `execution_generation` advancing on every acceptance | runtime frozen at whatever generation/digest it was actually spawned under | any | Cleanup identity must never substitute CURRENT `ServiceArtifact` generation/digest for the runtime's actual historical one (R1-B, §6) — placement history and artifact-generation history are two independent axes and must not be conflated |

## 5. Corrected history derivation

### 5.1 Chosen shape: **D2, generalized and cross-version**

Not D1 (immediately-prior only — proven insufficient, §3). Not solved by
persisting new cleanup-workflow state (no evidence in E15/E16 requires it,
and the "derive fresh" property that makes offline/restart semantics
trivially correct is too valuable to give up). The corrected derivation is:

For a given `(project_id, node_id)`, for every `application_id` that has
**any** historical acceptance in `state.acceptances` (v1) **or**
`state.phase113_v2.acceptances` (v2) for `project_id` (a full scan of both
maps, filtered by `project_id`/`application_id` — not limited to
currently-active applications, and not limited to one prior revision):

1. Collect every acceptance for that `(project_id, application_id)` from
   **both** maps (E17: a v1-to-v2 transition removes only the
   `active_by_application` pointer, §5.2 below — it never removes the old
   v1 acceptance records, so both maps must be scanned to see the full
   history of one application).
2. For each such acceptance, inspect its `materialized_dmap_ref`-identified
   D-Map's placements (v1: `CanonicalContractReferenceV1`; v2:
   `ContractRefV2`, normalized via the same field-copy pattern
   `ddeploy_migration.rs::locate_acceptance` already established, E14) for
   any placement of a service onto `node_id`.
3. For each **distinct** `(service_id, materialized_dmap_ref)` pair found
   this way that names `node_id`, check the **current** active authority
   for that application (`active_placement_authority`, the same seam
   Phase 118/119 already use, re-read fresh — never cached, §7). If current
   authority does not place that `service_id` on `node_id` (or there is no
   current active authority for that application at all), this pair is a
   cleanup candidate.
4. Emit one `OperationalCleanupDirective` per surviving candidate pair.

This is a full historical scan, not a single-hop lookback — it directly
closes H1/H3 (every past node is found, regardless of how many revisions
ago it was displaced) while remaining a pure, stateless function of
already-persisted data (no new persistence, preserving §18–§20's
restart/offline reasoning unchanged).

### 5.2 Why the v1-to-v2 boundary does not lose history

`accept_v2_proposal`'s transition path (`dserver/src/core/ddeploy.rs:677`,
E17) does exactly one thing to v1 state when an application moves from v1
to v2 authority: `self.active_by_application.remove(&key)` — it removes
only the "current pointer" entry, never touching `state.acceptances`. The
old v1 `DDeployAcceptanceV1` records for that application remain in
`state.acceptances` indefinitely (E13's finding that no production code
prunes `acceptances`/`proposals` still holds). The corrected derivation
(§5.1, step 1) therefore recovers full cross-version history by scanning
both maps by `application_id`, never by following a single "current
active" pointer.

The reverse direction (a v2-active application later accepting a v1
proposal) was not confirmed to be guarded symmetrically in this pass — no
`self.phase113_v2.active.remove(&key)` call was found near `accept_proposal`
(v1)'s definition. This is orthogonal to the history-reconstruction
question above (which only requires that past records remain *readable*,
regardless of transition direction) and is noted here as an open question
for whoever next touches v1/v2 transition safety — it is **not** a blocker
for this correction.

### 5.3 Duplicate historical placements on the same node (H2, task §17)

**Emit every distinct historical `dmap_ref`, never collapse to "latest
only."** In H2 (A→B→A→C), both rev1's and rev3's `dmap_ref` placed `S` on
`A`; the actual local runtime could match either one, depending on whether
A was online for rev3's reconciliation. The corrected derivation (§5.1,
step 3) treats `(service_id, dmap_ref)` as the dedup key, not
`(service_id, node_id)` — so both rev1 and rev3 candidates are emitted
independently. Safety does not come from the server guessing which one is
real; it comes from the agent's own local identity evidence (§6.2)
matching at most one of them (or none, if the node was never actually
reconciled under either). This is the same reasoning the task's §18
anticipates ("over-generating safe, idempotent cleanup candidates may be
preferable to losing cleanup obligations") and is adopted here explicitly
as the chosen answer to task §17's three options — it requires **multiple
historical refs**, not a single latest one and not a broader
identity-independent rule.

## 6. R1-B: independent verdict — **VALID**

### 6.1 What the evidence shows

Three independent facts, each separately confirmed (E15/E16/E19), combine
to make Phase 119F §22's claim ("generation/projection remains required
unchanged") unachievable as stated:

1. **DServer does not retain historical authorization snapshots.**
   `authorization_key(project_id, node_id)` (E15) has no revision or
   service component; a new durable authorization for a node
   *overwrites* the previous one in both the in-memory store and the
   persisted candidate. There is no durable trail of what any earlier
   authorization certified.
2. **DServer does not retain historical ServiceArtifact content.**
   `artifacts_by_project: BTreeMap<String, Vec<ServiceArtifact>>` (E16)
   holds exactly one live entry per `(project_id, artifact_id)`, updated
   in place; only a bare `u64` high-water generation counter survives past
   generations, never their actual content.
3. **Every artifact-to-service join resolves against CURRENT registry
   state, by design.** `resolve_assignments_from_registry_with_authority`'s
   own doc comment (E19) says it joins "the CURRENT, mutable `artifacts`
   list." There is no code path anywhere in this codebase that resolves an
   artifact assignment against anything but the live registry.

Given (1)+(2)+(3): if a service's `ServiceArtifact` generation/digest has
advanced since the old runtime was spawned (H6), there is **no current
server-side mechanism** that can construct a fresh authorization's
`artifact_bindings` bound to the OLD (historical, actually-running)
identity rather than the NEW (current registry) identity. A cleanup
authorization built the same way ordinary rollback authorizations are built
today would inevitably bind to current identity, not historical identity.

### 6.2 A second, independent problem the same evidence surfaces

Phase 119F's own conceptual `OperationalCleanupDirective` (§24) carries
`service_id`, `application_id`, `previous_dmap_ref` — **no artifact_id,
generation, or digest field at all.** There was never a plan to resolve a
current-vs-historical artifact identity for it in the first place. This
means §22's claim that the generation/projection gate
(`stage107a_service_check`/`check_generation_and_projection`) "remains
required unchanged" cannot even be evaluated for a cleanup-directive
service, because that gate requires an `ArtifactReconciliationAssignment`
matched by `service_id` (E19) — and a service present only in
`cleanup_directives`, never in `desired_services`, has no such assignment
resolved for it anywhere in the current context-building pipeline.

Attempting to fix this by routing cleanup service_ids through the SAME
`resolve_assignments_from_registry_with_authority` join used for
`desired_services` would (a) still only recover CURRENT, not historical,
identity (problem 6.1 again) and (b) introduce a new failure mode: a single
unmatched service — e.g. a since-deleted historical artifact — makes that
function `return Err(...)` for the **entire node's context**, not just the
cleanup candidate (E19), meaning a stale cleanup candidate could silently
break ordinary reconciliation for every other, currently-desired service on
that same node. This was not identified or guarded against anywhere in the
original §24/§25.

### 6.3 Why this is narrower than it sounds

Independently, this pass also found (E20) that **today's ordinary M8
rollback already does not verify a container's actual labels before
removal.** `rollback_service_block_from` deliberately excludes
image/target-identity verification from blocking M8 ("a container whose
target image is mismatching ... is PRECISELY a container that may need
removing" — an existing, already-shipped design decision, not something
this correction introduces), and `rollback_trusted_container_intent` itself
only ever inspects `ContainerState` (Missing/Running/Stopped), never label
content, before `docker rm -f`. Ordinary rollback's actual removal safety
today rests on: trusted `container_name` identity (from the static
`DispatchContainerMapping`) + authorization/lease/revision freshness — NOT
on proving the physical container's generation/digest labels match
anything. This materially narrows the new work required: cleanup does not
need to invent a way to prove historical artifact identity to reach parity
with today's rollback, because today's rollback does not prove it either.

**Verdict: R1-B is valid**, but the corrected design in §6.4 does not need
new server-side historical-identity reconstruction to close it — it needs
an explicit, honest carve-out plus one small, additive local-identity
extension for containers (§6.4, point 3).

### 6.4 Corrected authorization model

Evaluated against the task's options A–G:

| Option | Verdict |
|---|---|
| A — reuse ordinary rollback authorization unchanged | Rejected as stated: silently implies the generation/projection gate applies, which is unevaluable for a directive-only service (§6.2) |
| B — rollback + new cleanup-binding section | Close, but the "binding" cannot honestly carry a generation/digest guarantee (§6.1) — see D below |
| C — brand new `operation = "cleanup"` | Rejected: throws away the free revision-freshness/lease/resource-identity protection `"rollback"` already has proven correct (§17 of the original audit), for no benefit — the semantic gap is in the CONTEXT payload, not the authorization operation |
| **D — fresh authorization proves only CURRENT ABSENCE; agent independently proves LOCAL HISTORICAL OWNERSHIP** | **Adopted**, with the container extension below |
| E — authorization carries a set of allowed historical execution identities from durable server evidence | Rejected: no durable historical execution-identity evidence exists to draw from (§6.1) — would require inventing new persistence this correction found unnecessary |
| F — persist new server-side historical execution identity going forward | Rejected as a *required* step (adds new persisted state where none is needed today); left as an optional, later hardening idea, same status Phase 119F's own Option 6 already gave it |
| G | Not needed; D covers the requirement |

**Corrected model (Option D, refined):**

1. Authorization continues to reuse `operation = "rollback"` (§22 partially
   correct) — unchanged issuance gates: active Resource Registry binding,
   finite lease, `allow_host_mutation`, exact
   `project_id`/`dgraph_id`/`dgraph_revision`/`node_id` match at use time
   (§17's protection, unchanged and still sufficient for task §19's
   return-to-source requirement — see §7 below).
2. A cleanup-directive service is **explicitly exempted** from
   `stage107a_service_block_before_image` (generation/projection/
   completeness) — documented as intentional, for the same underlying
   reason M8 already exempts image-identity verification (§6.3): no
   trustworthy historical artifact binding can be constructed server-side
   (§6.1), so pretending to check one would be security theater, not
   security. This is a **documented, explicit carve-out**, not a silent
   weakening — it must be written into the code as its own named
   condition, never inherited implicitly by an `Option::None` artifact
   match falling through the existing gate.
3. In place of the generation/projection gate, the agent MUST independently
   verify **local historical ownership** by comparing the directive's
   `previous_dmap_ref` against its own already-persisted local identity
   before any M8 mutation:
   - **Native process**: already fully supported today —
     `ManagedProcessRuntimeStateV1.dmap_ref` (E08) is compared directly
     against the directive's `previous_dmap_ref`; no new field needed.
   - **Container**: **not yet supported** (E18 — current `datum.*` labels
     carry only `artifact_id`/`execution_generation`/
     `execution_projection_digest`, never `dmap_ref`). This correction
     recommends adding three small, additive labels at M1 creation time,
     following the exact existing pattern:
     `datum.dmap_id`, `datum.dmap_revision`, `datum.dmap_digest` — written
     alongside the three labels that already exist, checked via the same
     `docker inspect` mechanism `inspected_container_labels_match` already
     uses. This is new work, but it is small, purely additive, requires no
     schema migration, and brings container cleanup to the same local-proof
     strength native process already has, rather than falling back to
     "trust `container_name` alone," which is a materially weaker
     guarantee than the native-process side gets for the identical
     operation.
4. If no local match is found (neither runtime kind), the directive is a
   safe no-op — exactly as today's rollback already treats
   `ContainerState::Missing` (E06) and an absent native-process
   `persisted_state` (E06).

This preserves Stage107A's strength for the parts of it that still apply
(lease, resource identity, revision freshness) and replaces the part that
cannot honestly apply (generation/projection against a historical artifact)
with an equally strong, differently-shaped local-ownership check — never
silently dropping a gate without naming its replacement, per task §14.

## 7. Current return-to-source must win (task §19)

Unaffected by either R1-A or R1-B's fixes — re-verified against the
corrected derivation: step 3 of §5.1 evaluates current authority **fresh,
at derivation time, every request** (never cached from an earlier
derivation), so H5 (A→B→A) never emits a candidate for A while current
placement is A. The remaining race the task describes — a cleanup context
generated, then a new acceptance returns S to A, then the old cleanup
context reaches the agent — is closed exactly as Phase 119F §17 already
established: the authorization is bound to `dgraph_revision` at issuance,
`control_plane_authorized`'s exact-match check (E06/E09) rejects a
now-stale `dgraph_revision`, and the mutation never reaches `check_boundary`.
**No new mechanism is needed for this specific race; the existing
revision-freshness check, unchanged, already closes it — provided the
cleanup authorization is bound to `dgraph_revision` exactly like ordinary
rollback already is (§6.4, point 1).**

## 8. Idempotent over-generation (task §18)

Adopted, per §5.3's reasoning: repeatedly deriving a cleanup candidate for
an already-clean historical node is safe, because (a) the destructive
mutation remains gated by fresh authorization + lease + local-identity
match (§6.4) on every attempt, and (b) a local-identity lookup that finds
nothing is already a proven safe no-op today for both runtime kinds (E06).
No persisted acknowledgment/ack state is introduced — H4's requirement
("repeated cleanup directives for A must remain harmless") is satisfied by
construction, not by new bookkeeping.

## 9. v1/v2 history (task §20)

Answered in §5.1–§5.2 above: v2's `DDeployStateV2` (`proposals`/
`acceptances`/`active`) mirrors v1's shape field-for-field (E17); a v1→v2
transition preserves old v1 acceptance records (only the active pointer
moves, §5.2); `DDeployAcceptanceV2.materialized_dmap_ref` is a
`ContractRefV2`, field-identical to v1's `CanonicalContractReferenceV1`
(E14), normalized the same way `locate_acceptance` already normalizes it
for migration purposes. The corrected derivation scans **both** maps by
`application_id` rather than relying on field-shape parity alone (E14's own
caveat: "this proves parity here is achieved by hand-written, per-call-site
field copying... does not characterize every other v1/v2 boundary") — this
correction supplies the missing v1↔v2 boundary characterization E14 flagged
as out of its own scope: v1→v2 forward history is provably intact (§5.2);
v2→v1 (reverse) was not confirmed either way and is called out as an open
question, not assumed safe.

## 10. Which original Phase 119F sections are superseded

| Section | Status |
|---|---|
| §1–§21, §23, §27, §28 | **Not superseded** — re-verified, still correct |
| §22 | **Superseded in part**: reusing `operation = "rollback"` stands; the claim that "generation/projection... remains required unchanged" is corrected by §6.4 of this document |
| §24 | **Superseded**: "immediately-prior acceptance" replaced by the full historical scan in §5.1; the conceptual `OperationalCleanupDirective` struct shape is unchanged, but derivation and consumption semantics are corrected |
| §25 | **Superseded**: minimum implementation slice updated, §11 below |
| §26 | **Extended, not replaced**: new required tests added, §11 below, alongside all originally-listed tests which remain valid |

## 11. Corrected minimum Phase 119G implementation slice

1. `OperationalCleanupDirective { service_id, application_id, previous_dmap_ref: CanonicalContractReferenceV1 }`
   — unchanged shape from Phase 119F §24 (no artifact/generation field —
   §6.2 established this is deliberate, not an oversight).
2. A pure derivation function performing the **full historical scan**
   described in §5.1 (both `state.acceptances` and
   `state.phase113_v2.acceptances`, filtered by `project_id`+
   `application_id`, deduplicated by `(service_id, dmap_ref)`, checked
   against `active_placement_authority` fresh) — replacing the
   "immediately-prior" derivation Phase 119F §24 originally sketched.
3. `ArtifactReconciliationContext`/`OperationalDGraphSlice` gain the new
   `cleanup_directives` field exactly as Phase 119F §25 proposed — but
   cleanup directives must **not** be routed through
   `resolve_assignments_from_registry_with_authority` or otherwise
   contribute an entry to `context.artifacts` (§6.2's failure-isolation
   finding).
4. Agent-side: a new, narrow handling branch that, for each served
   `cleanup_directive`:
   - explicitly skips `stage107a_service_block_before_image` for this
     service (documented carve-out, §6.4 point 2);
   - resolves local identity (native-process persisted state, or trusted
     container mapping + the new `datum.dmap_id`/`datum.dmap_revision`/
     `datum.dmap_digest` labels) and compares it against
     `previous_dmap_ref`;
   - proceeds through the **existing, unmodified** `check_boundary`/M8 path
     only on a positive local match; is a safe no-op otherwise.
5. **New, small, additive agent-side change**: extend `ExecutionIdentityLabels`/
   `as_docker_run_args()` with the three new `datum.dmap_id`/
   `datum.dmap_revision`/`datum.dmap_digest` labels at M1, and extend
   `inspected_container_labels_match`'s sibling (a new
   `inspected_container_dmap_ref_matches`-shaped check) for the cleanup
   path specifically.
6. No new authorization operation value (still `"rollback"`, §6.4 point 1).
7. No new server-side persisted state (derivation remains stateless, §5.1).
8. No change to `desired_state`'s two-value vocabulary, to
   `union_project_operational_graph`, or to D-Engine — all confirmed
   unaffected (§1).

## 12. Required tests (extends Phase 119F §26, does not replace it)

All tests originally listed in §26 remain required. Add:

- Full-history derivation: H1 (two-hop offline reconnect emits a directive
  for the two-revisions-back node); H2/H3 (three- and four-hop cases,
  multiple simultaneously-reconnecting historical nodes each independently
  produce their own directive); H4 (repeated derivation for an
  already-cleaned node remains stable/harmless); H5 (return-to-source
  produces zero directives for that node, re-evaluated fresh each request,
  not cached); H6 (a `ServiceArtifact` generation change between the
  original placement and derivation time changes nothing about the
  emitted `dmap_ref`, and the directive never carries or implies an
  artifact/generation field).
- Duplicate historical placement (§5.3): a node placed twice non-adjacently
  for the same service produces two distinct directives (one per historical
  `dmap_ref`), not one collapsed/most-recent-only directive.
- Cross-version history: an application accepted first under v1 then under
  v2 still surfaces its v1-era historical placements in the scan (§5.2);
  the reverse direction is left as a documented open question, not silently
  assumed and not tested as "safe" without further investigation.
- Failure isolation (§6.2): a cleanup candidate referencing a `service_id`
  no longer present at all in the current `ServiceArtifact` registry must
  not prevent the SAME node's ordinary `desired_services` from resolving
  and reconciling normally.
- Agent-side carve-out: a served cleanup directive is processed without
  ever evaluating `stage107a_service_block_before_image` for that service;
  a native-process local `dmap_ref` mismatch blocks removal (existing PID
  reuse/executable-mismatch tests already cover this shape); a container
  whose new `datum.dmap_id`/`datum.dmap_revision`/`datum.dmap_digest`
  labels do not match `previous_dmap_ref` blocks removal; a container with
  no such labels at all (pre-this-change container, or externally created)
  is never treated as a match.
- Negative tests (M16/M17, unchanged from §26): 404/missing-node context,
  transport error, missing Resource Registry binding each still produce
  the existing error path, never a synthesized directive.

## 13. Summary answers to the twelve required architecture questions

1. **A→B→C while A offline**: full historical scan (§5.1) finds rev1's `A`
   placement directly; no longer depends on which revision is
   "immediately-prior."
2. **Multiple historical placements on the same source**: emit one
   directive per distinct `(service_id, dmap_ref)` pair, never collapse to
   latest-only (§5.3).
3. **Current return-to-source invalidates cleanup**: current-authority
   check is re-evaluated fresh at every derivation (§7); any
   already-issued authorization is additionally protected by unchanged
   `dgraph_revision` freshness (§7).
4. **Server facts establishing CURRENT absence**: `active_placement_authority`
   over `DDeployStateV1`'s already-persisted `active_by_application`/
   `phase113_v2.active`, unchanged, same seam as Phase 118/119 already use.
5. **Local facts establishing DATUM ownership**: native process —
   `ManagedProcessRuntimeStateV1.dmap_ref` match, already exists; container
   — new `datum.dmap_id`/`datum.dmap_revision`/`datum.dmap_digest` labels
   (§6.4 point 3), a small additive extension.
6. **What a fresh cleanup authorization binds**: `operation="rollback"`,
   exact `project_id`/`dgraph_id`/`dgraph_revision`/`node_id` match
   (unchanged) — explicitly **not** a generation/projection/artifact
   binding for the cleanup-directive service (§6.4 point 2).
7. **Container cleanup preserves Stage107A strength**: via trusted
   `container_name` (unchanged, same as ordinary rollback already relies
   on, E20) plus the new local `dmap_ref` label check (§6.4 point 3) —
   strictly stronger than ordinary rollback's existing container-removal
   guarantee, not weaker.
8. **Native-process cleanup preserves Stage107A strength**: via the
   already-existing `dmap_ref` field, no new work required.
9. **Must historical artifact identity be newly persisted server-side?**
   **No** — confirmed unnecessary; the corrected design never needs it
   (§6.1–§6.4).
10. **Is a new authorization operation/binding shape necessary?** No new
    *operation* value; the *context payload* needs the explicit
    generation/projection carve-out described in §6.4 point 2 — a semantic
    change, not a new wire shape.
11. **v1/v2 history normalization**: full union scan of both acceptance
    maps by `application_id`, using the existing `locate_acceptance`-style
    field-copy normalization for `ContractRefV2` → `CanonicalContractReferenceV1`
    (§5.1, §9).
12. **No generic 404/timeout becomes deletion authority**: unchanged from
    Phase 119F §23 — a directive is only ever produced from a genuine,
    specific historical acceptance record plus a genuine current-absence
    check; re-verified against the corrected derivation in §5.1 and holds
    by the same construction argument, extended to cover the full scan
    rather than a single-hop lookback.
