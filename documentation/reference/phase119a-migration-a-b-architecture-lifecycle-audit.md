# Phase 119A — Migration A→B Architecture & Lifecycle Audit

- Status: Audit / design only. No production code was added or changed by
  this phase.
- Branch: `agent/phase119-migration`, created from Phase-118 closure HEAD
  `43d8ab58727a0a2484d3a656432a2a164e7f946f` (`origin/agent/phase118-dengine`).
- Scope: define the minimum architecture for a controlled placement
  transition A→B without inventing a second placement authority, deployment
  lifecycle, host-mutation path, placement planner, DMonitor-owned desired
  state, or dual-active DMap model. Recommend (do not enact) the minimum
  Phase 119B slice.

## 1. Executive conclusion

**The central hypothesis in Section 0 of the task survives challenge and is
adopted as this audit's recommendation, with one important refinement forced
by evidence: "prepare-first" preparation is legitimately limited to
content/byte staging, not warm-runtime staging.**

- **A** = the exact current accepted placement authority for one application,
  identified by `{project_id, application_id, dmap_id, revision, digest}` —
  precisely what `ddeploy::active_placement_authority` already returns as a
  `PlacementAuthorityView`. Nothing new.
- **B** = an explicit, externally supplied **DDeploy proposal** (v1 or v2,
  whole-application, CAS-bound to A's exact revision via the *already
  existing* `expected_active_dmap_revision` field) — never a fabricated DMap,
  since DMap identity/revision are server-owned and only materialize at
  acceptance.
- **Cutover** is a single, ordinary, already-existing `accept_proposal` /
  `DDeployStateV2::accept_with_previous` call. This repository's own code
  already atomically bundles, in that one transaction: the new active DMap,
  the recomputed digest, and — critically, independently confirmed this
  phase — the new **operational projection** (`derive_application_operational_services`,
  called synchronously inside `accept_proposal` and stored directly on
  `ActiveApplicationDeployment`, then unioned project-wide and validated
  *before* anything commits). This single fact answers most of the audit's
  hardest questions at once: cutover is atomic, D-Engine flips exactly then
  (Phase 118, unchanged), and the per-node desired-state feed that drives
  Stage 107A-governed reconciliation flips exactly then too — meaning
  **source cleanup and target startup are already naturally expressed by the
  existing reconciliation loop**, with zero new mutation mechanism required.
- **Legal pre-cutover preparation is real and already exists**, but only at
  content granularity: `smartsentinel-dcode-install` (confirmed this phase to
  be a pure, DDeploy/DMap-independent, hash-verified local content-store
  write, ADR-0021) and Phase 117K's own live proof (D2 registered + H2 bytes
  installed while D1/P1 remained sole active authority) generalize directly.
  **Starting a warm, running-but-unauthorized runtime for B, or authorizing
  governed DCode invocation on B, is NOT supported by any existing mechanism**
  — `authorize_dcode_invocation` is sourced exclusively from
  `active_placement_authority`, confirmed by direct read this phase — and
  this audit does **not** recommend inventing one for Phase 119 v1. This
  removes the double-resource-commitment/reservation problem from v1's scope
  almost entirely: pre-cutover preparation costs disk, not a second running
  service.
- **Dual-active authority (Option C) is rejected**, confirmed by direct
  re-inspection of `active_placement_authority`'s existing fail-closed
  "dual active DMap authority" guard — migration gives no legitimate reason
  to violate it, and nothing in the evidence gathered this phase suggests
  otherwise.
- **Rollback is not a special mechanism.** Before cutover it is simply
  abandoning preparation (A never moved). After cutover it is a **new**,
  ordinary governed DDeploy proposal + acceptance whose target restores a
  previous (freshly revalidated) placement — the exact same mechanism as any
  other migration, not a distinct code path. The pre-existing, differently-
  scoped "rollback" in `DATUM/src/agent/operational_reconciliation.rs`
  (`AUTHORIZATION_OPERATION_ROLLBACK`, a node-local, remove-only container
  cleanup, "M8") is a different concept at a different scope and must not be
  confused with placement-authority rollback — this audit treats it as the
  existing mechanism that (for free) performs source-A cleanup after cutover,
  not as a migration-level rollback primitive.

## 2. Evidence base

Read/inspected directly this phase (fresh reads, file paths and line
anchors recorded in each section below, not carried over from any prior
phase's own citations):

- `DATUM/src/agent/operational_reconciliation.rs` (module doc, rollback
  constants, `rollback_trusted_container_intent`, "M1"–"M8" mutation
  boundary references).
- `DATUM/src/agent/stage107a_enforcement.rs` (module doc; ADR-0011/ADR-0012
  lease-gated mutation boundary; `BLOCKED_*` reason codes).
- `dserver/src/core/operational_dgraph.rs` (`OperationalDGraph`,
  `OperationalNodeDesiredState`, `OperationalDesiredService`,
  `build_operational_slice`).
- `dserver/src/api/operational_dgraph.rs` (how the slice is served;
  `build_reconciliation_authorization`; recovery wiring).
- `dserver/src/core/operational_dgraph_recovery.rs` (module doc: "Read-only
  recovery... A missing or invalid state is never repaired by publishing a
  new graph").
- `dserver/src/core/ddeploy.rs` (`accept_proposal`'s full commit sequence,
  lines ~2360–2410; `expected_active_dmap_revision` CAS, lines 279/2361/3975;
  `active_placement_authority`, re-confirmed unchanged from Phase 118).
- `dserver/src/core/ddeploy_v2.rs` (`DDeployProposalV2`'s full field list;
  `accept`/`accept_with_previous`, including its own doc comment: "This is
  what makes a v1 -> v2 transition retain the logical identity and monotonic
  revision without creating a second authority").
- `dserver/src/core/dcall.rs` (grant redemption's source+destination
  stale-authority recheck against current `dmap_ref`, lines ~873–923).
- `dserver/src/core/dcode_authorization.rs` (`authorize_dcode_invocation`,
  sourced exclusively from `active_placement_authority`).
- `DATUM/src/bin/smartsentinel-dcode-install.rs` and
  `dserver`'s `dcode_registry.rs`/ADR-0021 (content-addressed module store,
  hash-verified, no DMap/DDeploy dependency).
- `dserver/src/core/dmonitor_freshness.rs` (module doc: explicit health /
  freshness / current-head / convergence separation), `dmonitor_readiness.rs`
  (`DMonitorReadinessV1::{Ready, NotReady}`), `dmonitor_interpretation.rs`.
- `documentation/reference/phase113b-live-multinode-closure.md` (topology,
  scope, conclusion sections).
- `documentation/reference/phase117k-live-production-dcompile-h1-h2-proof.md`
  §17 ("H2 bytes installed before activation (still D1 active)").
- `documentation/reference/phase118a-dengine-architecture-authority-audit.md`
  and `phase118c-independent-dengine-closure-review.md` (frozen D-Engine
  authority, reused not re-derived).
- Repository-wide greps for `migrat`, `reconcil`, `rollback`, `drain`,
  `quiesce`, `recover` across `dserver/` and `DATUM/`.

## 3. Existing reconciliation/recovery inventory

| Component | File | Classification |
|---|---|---|
| Per-node desired-state projection | `dserver/src/core/operational_dgraph.rs` | CURRENT CANONICAL ARCHITECTURE — the exact mechanism Phase 119 must reuse |
| Operational-dgraph serving/authorization | `dserver/src/api/operational_dgraph.rs`, `build_reconciliation_authorization` | CURRENT PRODUCTION |
| Operational-dgraph crash recovery | `dserver/src/core/operational_dgraph_recovery.rs` | CURRENT PRODUCTION — direct precedent for migration-record crash recovery (Section 27) |
| Node-local reconciliation/host mutation | `DATUM/src/agent/operational_reconciliation.rs` | CURRENT PRODUCTION |
| Stage 107A lease-gated mutation boundary | `DATUM/src/agent/stage107a_enforcement.rs` | CURRENT PRODUCTION — the mutation authority Phase 119 must not bypass |
| Node-local "rollback" (`AUTHORIZATION_OPERATION_ROLLBACK`, M8) | `DATUM/src/agent/operational_reconciliation.rs` | CURRENT PRODUCTION but **narrow scope**: remove-only host cleanup for a service no longer desired on one node — reused as source-cleanup mechanism (Section 29), NOT as migration/placement rollback |
| DCode content-addressed module store | `DATUM/src/bin/smartsentinel-dcode-install.rs`, ADR-0021 | CURRENT PRODUCTION — the legal pre-cutover content-preparation primitive |
| DGraph rollback/recovery (`dgraph_rollback_audit.rs`, `operational_dgraph_recovery.rs`'s sibling module) | `dserver/src/core/dgraph_rollback_audit.rs` | UNRELATED to placement migration — this is a D-Graph *structural-validation* audit trail (Stage 110-era), not a runtime rollback mechanism; excluded from further consideration |
| Phase 113B "multinode" | `documentation/reference/phase113b-live-multinode-closure.md` | HISTORICAL BUT RELEVANT — proved *concurrent* multi-node D-Call delivery (fog-01/cloud-01), not node-to-node *migration*; relevant only as live-topology precedent for Section 39's future proof, not as migration-semantics precedent |
| DCall grant redemption stale-authority recheck | `dserver/src/core/dcall.rs` | CURRENT PRODUCTION — already answers the "old grant after cutover" question (Section 17) |
| DMonitor readiness/freshness/interpretation | `dserver/src/core/dmonitor_readiness.rs`, `dmonitor_freshness.rs`, `dmonitor_interpretation.rs` | CURRENT PRODUCTION — evidence source only, never authority (Section 19) |

No SUPERSEDED or PROTOTYPE migration-specific code was found anywhere — there
is currently **no dedicated migration mechanism of any kind** in the
repository. Everything usable was found by generalizing existing,
independently-purposed mechanisms.

## 4. Definition of migration A→B

**Migration A→B means: transition one application's sole canonical accepted
placement authority from its current state (A) to an explicit, caller-
supplied target state (B), through exactly one existing governed DDeploy
acceptance, with legal (content-only) preparation permitted beforehand and
governed cleanup of no-longer-desired runtime happening afterward through
the existing reconciliation loop.** It is not a new deployment concept — it
is a *name for a particular way of using the existing DDeploy proposal/accept
lifecycle*, where the proposal happens to differ from the currently active
DMap only in placement (and optionally D-Code binding), rather than being an
initial deployment from nothing.

## 5. Source A identity

**A = `PlacementAuthorityView`**, exactly as returned by
`ddeploy::active_placement_authority(state, project_id, application_id)`
(unchanged since Phase 117/118). The exact identity fields a migration must
bind to are `dmap_id`, `revision`, `digest` (plus `project_id`/
`application_id` for scoping). This is **not** a hostname, container, process,
module path, or caller-provided node — it is the same accepted-authority
reference every other Phase 117/118 production consumer already uses
(D-Code authorization, D-Call route derivation, grant issuance/redemption,
D-Engine derivation).

**Fail-closed requirement confirmed already implemented, not merely
desired**: `DDeployProposalV1`/`DDeployProposalV2` already carry
`expected_active_dmap_revision: Option<u64>`, checked at acceptance
(`ddeploy.rs` line ~2361: `if record.proposal.expected_active_dmap_revision
!= current_active_revision { ... "another proposal was accepted first" }`,
and the v2 equivalent in `accept_with_previous`). A migration target proposal
that names A's exact revision as its `expected_active_dmap_revision`
therefore **already** fails closed if A changed before cutover — no new CAS
mechanism is needed (Section 26/M9/M10).

## 6. Target B representation

**Recommended: B1 refined into "B2" — an explicit, concrete, pending DDeploy
proposal (`proposal_id` + `proposal_digest`), not a fabricated DMap-like
object.** Rationale, directly from source: `CanonicalDMapV1`/`CanonicalDMapV2`
identity (`dmap_id`, `revision`, `digest`) is server-owned and materializes
only inside `accept_proposal`/`accept_with_previous` — pretending a DMap
already exists for an unaccepted target would either require inventing a
placeholder identity (a second, fake authority) or leaving the target
un-identified until it's too late to reference it precisely. A `DDeployProposalV2`
(re-read in full this phase, `ddeploy_v2.rs` lines 19–70) already bundles
**everything** a target needs in one object: `dgraph_ref`, `dcontinuum_ref`,
`dforward_ref`, `placements`, `diot_placements`, `dcall_realizations`,
`external_port_bindings`, `expected_active_dmap_revision`,
`diot_artifact_authority`, and (Phase 117G) D-Code artifact-authority
bindings — i.e. B3 ("a complete prospective DMap-like object") is already
subsumed by B2, because a DDeploy proposal already *is* the complete
prospective bundle, just not yet accepted. B4 (a bare target `node_id` for
one service) is rejected as underspecified — see Section 7. B5 was
considered (a bespoke `MigrationIntentV1` distinct from a proposal) and
rejected for the same reason B3 was: it would duplicate structure the
proposal type already owns.

## 7. Migration granularity

**Migration intent may concern one service or a subset of services, but
authority cutover is always one complete application DMap acceptance.**
Confirmed structurally: `accept_proposal` (v1) and `accept`/`accept_with_previous`
(v2) each operate on exactly one `CanonicalDMapV1`/`CanonicalDMapV2` per
`(project_id, application_id)` key — there is no per-service acceptance path
anywhere in `ddeploy.rs`/`ddeploy_v2.rs`. A target proposal that moves one
service must therefore carry every other service's *unchanged* placement
forward explicitly (the caller's/orchestration's responsibility to construct,
by starting from A's current placements and editing only the migrating
service(s) — Section 21/46). No independent per-service canonical placement
authority may be created; this matches the task's explicit prohibition and
the evidence agrees with it completely.

## 8. Phase 120 target-selection boundary

Phase 119 may **validate** an explicit B and **orchestrate transition to**
it; it must never **decide** B. No current code was found to blur this
boundary — `derive_active_dengine_nodes` (Phase 118) is read-only and
membership-only; `active_placement_authority` never ranks or selects
anything; nothing in `ddeploy.rs`/`ddeploy_v2.rs` scores or optimizes a
placement (the existing module doc in `ddeploy.rs` itself already states:
"No Stage 112 placement algorithm exists here: every function in this module
either VALIDATES a caller-supplied candidate placement or MATERIALIZES it
verbatim. Nothing selects, scores, or optimizes a node."). This exact
sentence, already present in the codebase before Phase 119 began, is the
strongest possible confirmation that the boundary Phase 119 must respect is
already the codebase's own stated design principle, not a new constraint
this audit invents. **The clean interface to Phase 120: Phase 120 produces an
explicit target proposal (or the placement content of one); Phase 119
executes `A -> supplied B` without caring how B was chosen.**

## 9. Lifecycle alternatives

See Section 10 (recommendation) and Section 33 (comparison table) for the
full evaluation. Three options were evaluated per the task's framing
(accept-first, prepare-first/accept-as-cutover, dual-active), plus this
audit's own refinement of prepare-first (content-only preparation, no warm
runtime). No fourth fundamentally distinct option was found in the evidence.

## 10. Recommended lifecycle

**Option B, refined: prepare-first (content-only) / accept-as-cutover.**

1. Caller/operator submits an explicit target proposal B (v1 or v2, matching
   A's version or intentionally version-transitioning — Section 49) whose
   `expected_active_dmap_revision` names A's exact current revision. Standard
   proposal creation/validation (existing, unmodified).
2. **Legal preparation** may occur against B's target node(s): D-Code content
   installation via the existing content-addressed module store
   (`smartsentinel-dcode-install`-equivalent path), governed independently by
   ADR-0021 hash verification, with zero dependency on B being accepted.
   **Nothing else may legally happen on B before cutover** — see Section 11.
3. A (not B) remains the sole canonical placement authority throughout
   preparation. D-Engine, DCode authorization, DCall routing, and the
   operational projection all continue to reflect A exclusively — none of
   this requires new code; it is the existing behavior of every consumer of
   `active_placement_authority`, confirmed unchanged.
4. **Cutover** = one ordinary `accept_proposal`/`accept_with_previous` call
   on B. This single existing transaction already, atomically: materializes
   B's DMap identity/digest, replaces the active record, re-derives the
   operational projection for the whole project (pre-commit-validated), and
   (Phase 118) instantly flips D-Engine membership. No new mutation is
   introduced by migration at this step — it is the pre-existing DDeploy
   acceptance path, used exactly as designed.
5. **Cleanup/convergence** happens automatically: the next reconciliation
   cycle on A's former node(s) observes a `desired_services` list that no
   longer includes the migrated service and performs the existing,
   Stage-107A-governed removal (the "M8"/rollback-named container-cleanup
   path already present in `operational_reconciliation.rs`). The next cycle
   on B's node observes the service now present and performs ordinary
   provisioning/start through the existing mutation boundary. **No new host-
   mutation code is required.**

This lifecycle preserves single-authority at every instant (A, then B, never
both — Section 25), reuses 100% of existing DDeploy/reconciliation machinery,
and requires no reservation/capacity-duplication concept because no warm
runtime exists on B before cutover.

## 11. Pre-cutover preparation semantics

| Action | Classification | Evidence |
|---|---|---|
| Copy/install D-Code bytes to B's content store | **LEGAL PREPARATION** | `smartsentinel-dcode-install` is a pure, hash-verified local write with zero DMap/DDeploy/DCode-registry dependency (ADR-0021); Phase 117K proved this live at production scale, before P2 existed |
| Pre-pull container/image on B | **LEGAL PREPARATION**, by the same reasoning, if/when that mechanism exists independently of desired-state reconciliation (not confirmed as a separate current primitive this phase, but architecturally identical in kind to D-Code install — content presence, not runtime activation) | Analogous to D-Code install; no direct source confirming an image-pre-pull-only primitive exists today, so treat as **CURRENTLY UNSUPPORTED unless/until confirmed** rather than assumed |
| Materialize immutable configuration on B | **LEGAL PREPARATION** in principle (content, not activation) — no current dedicated primitive found; treat as a 119B-scope question, not fabricated here | — |
| Validate ABI/capabilities against B's structural declaration | **LEGAL PREPARATION** — this is exactly what `check_dcontinuum_authority` already does at proposal-acceptance-adjacent validation time; can be invoked read-only before cutover too | `dnode_registry.rs` (frozen authority, Phase 118A) |
| Verify secrets/artifact availability for B | **LEGAL PREPARATION** — `secret_preflight.rs`/`secret_resolver_contract.rs` exist as read-only preflight checks already | file presence confirmed by grep (Section 2) |
| Create a **stopped** runtime on B | **CURRENTLY UNSUPPORTED** — no primitive found that creates a stopped-but-present runtime independent of the active desired-state loop | — |
| Create a **running-but-isolated** runtime on B | **ARCHITECTURALLY UNSAFE for v1** — would require inventing a reservation/warm-capacity authority (Section 24) not currently modeled anywhere; explicitly not recommended | — |
| Invoke DCode on B | **REQUIRES ACTIVE AUTHORITY** — `authorize_dcode_invocation` is sourced exclusively from `active_placement_authority`; confirmed this phase by direct read, fails closed for a node not part of the active DMap | `dcode_authorization.rs` lines 121–143 |
| Expose traffic to B | **REQUIRES ACTIVE AUTHORITY** — same reasoning; no route/grant can be issued for a service/node pair not present in the active DMap's D-Graph/placements | `dcall.rs` |
| Grant a D-Call route to B | **REQUIRES ACTIVE AUTHORITY** — same as above | `dcall.rs` |

## 12. Target readiness

Given Section 11's finding (no warm runtime exists pre-cutover), "B is ready
for cutover" in Phase 119 v1 is necessarily narrower than a full
health/convergence definition:

- **structural target still exists/capable** — re-check against the current
  D-Node registry/D-Continuum (already-existing `check_dcontinuum_authority`,
  reusable read-only) — PROVABLE today.
- **exact artifact/content available** — content-store presence check
  (already exists via the module store's own lookup) — PROVABLE today.
- **required dependencies available (secrets, artifacts)** — existing
  preflight modules (`secret_preflight.rs`, `service_artifact_registry.rs`
  eligibility checks) — PROVABLE today, read-only.
- **desired runtime materialized** — **NOT PROVABLE today** in the sense of
  "a warm runtime is up and healthy," because no such runtime is permitted to
  exist before cutover (Section 11). This is not a gap to fix; it is a
  consequence of the content-only preparation scope this audit recommends.
- **process/runtime healthy if pre-start is legal** — **N/A for v1** (pre-
  start is not legal).
- **observation sufficiently fresh** — N/A for v1, for the same reason;
  DMonitor has nothing to observe on B before cutover since nothing runs
  there yet.
- **no unresolved mutation/reconciliation failure elsewhere** — could be
  checked as an operator-facing sanity signal, but is not something the
  current architecture ties to acceptance eligibility.

**Conclusion: "B is ready" in v1 reduces to "B's structural/content/
eligibility prerequisites are satisfiable," not "B is observed healthy."**
This is honest given the evidence, not an invented shortcut — a fuller
readiness contract (warm-runtime health before cutover) would require the
architecturally-unsafe capability this audit declines to invent, and is
recorded as a 119B/C-or-later implementation question only if a future phase
decides warm-overlap migration is actually required.

## 13. Atomic cutover semantics

**Confirmed: `accept_proposal`/`accept_with_previous` is, and should remain,
the unique canonical cutover event.** Direct evidence: within one call, the
codebase already (a) revalidates the proposal against current registries,
(b) computes the new DMap digest, (c) derives the new operational projection
via `derive_application_operational_services`, (d) unions it project-wide and
validates the union *before* committing anything, and (e) only then inserts
the new active record. This means DMap identity, D-Code binding (via the
proposal's own `dcode_artifact_authority`/`diot_artifact_authority`), DMap
revision, the operational projection, D-Call currentness (grants reference
`dmap_ref`, re-verified at redemption — Section 17), and D-Engine's
derivation input all transition **together, atomically, under one existing
authority transition**. Migration does not need to, and must not, mutate
active placement itself outside this path — it only prepares and requests
acceptance through it.

## 14. DDeploy/DMap relationship

Unchanged and reused verbatim (Section 3 freeze). DMap remains the only
canonical placement/realization authority; migration is purely a *usage
pattern* of DDeploy's existing proposal/accept lifecycle, never a parallel
authority.

## 15. D-Engine before/after semantics

Frozen per Phase 118 (Section 3). Re-confirmed consistent with this audit's
findings, not re-derived: **pre-cutover, D-Engine == A's membership** exactly,
even while B's proposal exists and B's content is fully installed — because
`derive_active_dengine_nodes` only ever reads an already-*active*
`PlacementAuthorityView`, and pending proposals are never consulted by
`active_placement_authority` (Phase 118A Section 22, re-verified true here
since `ddeploy.rs`'s acceptance-reading code is unchanged). **Post-acceptance,
D-Engine == B's membership**, instantly, at the same moment cutover commits
(Section 13). No intermediate dual-membership state exists or is proposed.
If a future presentation wants "active engine + prepared target footprint,"
those must be two distinct fields — this audit does not design that view and
does not need to; it is out of Phase 119A's scope (no wire type is proposed
here at all, mechanism-only per Section 45).

## 16. DCode relationship

For a placement-only migration (identical D-Code descriptor, node changes):
confirmed via `authorize_dcode_invocation`'s dependency on
`active_placement_authority` — before cutover, authorization resolves the
service on A only (B has no placement entry in the active DGraph/DMap yet);
after cutover, on B only. This directly generalizes Phase 117K's precedent
(content presence ≠ active executable authority) from "same node, new
D-Code revision" to "new node, same D-Code revision" — the underlying
mechanism (`active_placement_authority`-gated authorization) is identical in
both cases; only which field of the accepted authority changed differs.

**Migration may also change placement AND D-Code descriptor simultaneously**
— this audit recommends **permitting** it, because the target proposal
already commits to an exact D-Code binding via its own
`dcode_artifact_authority` field (Phase 117G, unchanged); there is no reason
to forbid a proposal from changing both placement and binding at once, and
no separate migration-only D-Code-currentness mechanism should be created —
the existing one already handles this correctly by construction (it doesn't
distinguish "why" the binding changed, only validates that it's correct now).

## 17. DCall relationship

Re-confirmed by direct read of `dcall.rs`'s grant redemption path (lines
~873–923, unchanged, not modified this phase): redemption re-derives BOTH
source and destination authority via `authorize_dcode_invocation` and
compares `dmap_ref`/`dgraph_ref`/artifact digests against what the grant
bound at issuance, failing closed with `RedeemError::StaleAuthority` on any
mismatch. **This means: a grant issued before cutover but redeemed after
cutover already, correctly, fails as stale — no new hardening is required.**
A grant both issued *and* redeemed before cutover completes its (already
in-flight) delivery normally under the authority valid at redemption time —
this is an expected, benign race inherent to any live cutover (an in-flight
call cannot be retroactively invalidated, and D-Code v1's stateless/
fresh-instance execution model means no long-lived session could straddle
the cutover in a way that matters). **Conclusion: current DCall currentness
machinery is sufficient; no 119B/C hardening is required for this specific
property.** No explicit grant-revocation step is needed — staleness is
detected reactively, at redemption, which is the correct and already-existing
enforcement point (not modified in 119A).

## 18. DMap/2 / DForward / DIoT relationship

Confirmed structurally (Section 6): `DDeployProposalV2` already bundles
`placements`, `diot_placements`, `dcall_realizations`, `external_port_bindings`,
and `dforward_ref` as one atomic object — there is no way to submit "just a
placement change" to the existing DDeploy v2 API; the caller must always
supply the complete intended bundle. **This means Phase 119 cannot and must
not "silently redesign DForward"** — it simply requires that whoever
constructs migration target B (an operator, tooling, or eventually Phase 120)
carries forward every DForward/DIoT/DCall-realization/external-port-binding
value unchanged unless the migration is intentionally also changing them.
This is already-enforced by the existing type, not a new rule Phase 119
invents.

## 19. DMonitor relationship

DMonitor may be **read** for evidence relevant to a human/policy decision
about migration timing (source degraded, target-adjacent structural facts),
but confirmed (Section 3 freeze, re-verified via `dmonitor_readiness.rs`/
`dmonitor_interpretation.rs`'s existing read-only consumption of
`PlacementAuthorityView`) that DMonitor **cannot** select B, mutate DMap,
make B active, or rewrite D-Engine membership — no code path anywhere grants
it write access to any of those. Given Section 11/12's finding that no warm
runtime exists on B pre-cutover, DMonitor's practical role in v1's migration
mechanism is minimal (there is nothing running on B yet to observe); its
larger relevance is post-cutover convergence observation (Section 32) and as
policy-input evidence for a human/operator deciding *when* to migrate — which
is explicitly POLICY, not MECHANISM (Section 45), and therefore out of Phase
119's own implementation scope.

## 20. Freshness/health/convergence/readiness

Directly reconstructed from `dmonitor_freshness.rs`'s own module doc, which
already draws exactly the distinction the task requires, independent of this
audit:

| Fact | Current owner | Authoritative? | Can block cutover? | Observed/desired |
|---|---|---|---|---|
| Structural eligibility (node exists, capable) | D-Node Registry / `check_dcontinuum_authority` | Yes (acceptance-time) | Yes — already blocks proposal acceptance today | Desired-side structural fact |
| Content/module presence | DCode content store (ADR-0021) | Yes, for its own narrow claim (bytes match hash) | Not currently wired to acceptance; could inform an operator's readiness judgment | Neither — a preparation-side fact |
| Runtime existence | Node-local reconciliation (Stage 107A) | Yes, for "is a container/process present" | N/A pre-cutover (none should exist on B) | Observed |
| Health | `DMonitorHealthV1` (collector-reported, at `observed_at_utc`) | Yes, for "what the collector believed then" | No, not alone — see freshness | Observed |
| Freshness | `dmonitor_freshness.rs`, `evaluate_freshness` (pure, policy-parameterized) | Yes — explicitly a *server policy judgment*, kept separate from health | Should gate whether a health/convergence claim can be trusted at all | Meta — evaluates observed evidence's age |
| Convergence (agrees with active DMap/DForward authority) | Explicitly out of `dmonitor_freshness.rs`'s own scope per its module doc; owned by whichever module compares observed vs. desired (`dmonitor_interpretation.rs`) | Yes, for its own narrow claim | Relevant post-cutover (did B actually converge), not pre-cutover (nothing to converge yet) | Comparison of observed vs. desired |
| Readiness (`DMonitorReadinessV1::{Ready,NotReady}`) | `dmonitor_readiness.rs` | Yes, for its own defined policy (`Ready` requires `Healthy` + fresh, v1) | Could inform, but does not by itself decide, a migration cutover decision in v1 (no warm runtime exists to be "ready" pre-cutover) | Derived from observed + freshness |

These five concepts are never collapsed anywhere in the current codebase, and
this audit does not collapse them either.

## 21. Capacity and temporary double-resource problem

**Largely avoided by construction in the recommended lifecycle.** Because
pre-cutover preparation is limited to content/byte staging (Section 11), no
second running service exists on B before cutover — the only "double
resource usage" is disk space for content that may end up unused if
migration is aborted, which is already an accepted, already-proven-safe fact
(Phase 117K: D1 and D2 bytes coexist in the same content-addressed store with
no capacity-authority implications). **No reservation concept is required
for v1.** If a future phase genuinely requires warm-runtime overlap (running
B before cutover while A still serves), that would require inventing a
capacity-reservation authority not currently modeled anywhere in this
repository — this audit explicitly classifies that as a **deferred advanced
migration capability**, not a Phase 119 v1 requirement, and does not design
it.

## 22. Stage107A reconciliation/host-mutation relationship

Confirmed (Section 3 freeze, re-verified via direct read of
`stage107a_enforcement.rs`'s module doc and constant list this phase): all
host mutation is gated by a lease (`OperationalReconciliationAuthorization`,
scoped `process`/`durable`, bound to `dgraph_id`+`dgraph_revision`+`node_id`)
checked at mutation boundaries "M1"–"M8". Migration introduces **zero** new
mutation boundary and **zero** bypass: target-B content preparation uses the
existing content-store path (independent of the lease system entirely, since
it never mutates a "runtime," only a local file store); source-A cleanup uses
the existing M8-equivalent lease-gated removal, triggered naturally once the
post-cutover operational projection no longer lists the service for that
node. **Migration requires no new authorization kind.**

## 23. Migration persistence decision

See Section 27 for the full derivation. Conclusion: a lightweight, **non-
authoritative** persisted record is likely warranted for operational
workflow tracking across a potentially multi-minute, restart-spanning
process (M2-flavored, not M1/M3 alone), but it must never itself become or
duplicate placement authority (Section 52).

## 24. Migration workflow identity

If a `MigrationRecord` is recommended (Section 27), it should carry its own
`migration_id` purely as workflow/progress identity, clearly distinct from
DMap identity (canonical placement authority) and proposal identity
(candidate authority transition). No migration ID may become or influence
placement authority — this mirrors exactly how `acceptance_id` is already
kept distinct from `proposal_id` and `dmap_id` in the existing DDeploy model,
a pattern this audit's recommendation reuses rather than invents.

## 25. Lifecycle states

If persistence is adopted (Section 27), the minimum useful state set,
derived from the recommended lifecycle (Section 10) and failure matrix
(Section 33), is:

| State | Active canonical DMap | Allowed mutations | Authoritative facts | Observed evidence | Valid next states | Crash/restart recovery rule |
|---|---|---|---|---|---|---|
| `preparing` | A | Content installation on B only | A remains sole authority | Content-store presence | `ready_to_cutover`, `aborted` | Re-derive from A's `active_placement_authority` + content-store state; record is advisory only |
| `ready_to_cutover` | A | None (waiting for operator/caller cutover request) | A remains sole authority | Structural/content preflight all pass | `cutover_requested`, `aborted` | Same as above |
| `cutover_requested` | A (until acceptance completes) | Exactly one `accept_proposal`/`accept_with_previous` call in flight | A until the instant acceptance commits | — | `cutover_accepted`, `cutover_failed` | On restart: re-check `active_placement_authority` directly — if it already names B, treat as `cutover_accepted` regardless of the record's own last-known state (Section 28/M12) |
| `cutover_accepted` | B | Reconciliation cleanup on A, provisioning on B (both via existing mechanisms) | B is now sole authority | Operational projection now lists B | `converging`, `degraded` | Trust `active_placement_authority` (already B); never "restore" A |
| `converging` | B | Same as above, in progress | B | Reconciliation/DMonitor evidence of A cleanup + B startup | `completed`, `degraded` | Same as above |
| `completed` | B | None | B | A fully absent from desired state; B observed as expected | (terminal) | Same as above |
| `degraded` | B | None new from migration; ordinary operator remediation | B (still canonical) | Cleanup or convergence failure evidence | `completed` (once resolved), or a new rollback migration | B remains authoritative; never automatically reactivate A (Section 39/M13) |
| `aborted` | A | None (any partial B content may be cleaned up per ordinary governed rules) | A | — | (terminal) | A was never authoritative for anything but itself; nothing to recover |

No state named `active_target` or equivalent is introduced — there is no
state in which B is "partially active"; it is either not-yet-authoritative
(`preparing`/`ready_to_cutover`/`cutover_requested`) or fully authoritative
(`cutover_accepted` onward). This directly avoids the task's named anti-
pattern.

## 26. Concurrency/CAS

**Existing guards are sufficient; no parallel CAS system should be invented.**
Confirmed (Section 5): `expected_active_dmap_revision`, checked at
acceptance for both v1 and v2, already fails closed exactly as required —
"another proposal was accepted first" (v1) / "stale active DMap revision"
(v2). A migration target proposal naming A's exact revision automatically
inherits this guarantee with zero new code.

## 27. Idempotency

Migration should reuse DDeploy's existing idempotent-acceptance semantics
(confirmed present in `accept_with_previous`: an acceptance_id that already
maps to the same proposal/digest returns the existing acceptance rather than
erroring or double-committing; a proposal that already has *any* acceptance
returns that acceptance rather than creating a second one). A migration
orchestration layer retrying a cutover request after an ambiguous failure
(e.g. a network timeout after the server actually committed) should reuse
this idempotency rather than re-derive its own — repeating a `prepare`,
`observe`, or `cleanup` request should likewise be naturally idempotent
because each of those reads current state rather than applying a delta.

## 28. Crash recovery

**M12 (task): crash recovery must trust active DMap over migration progress
state.** This is not a new principle Phase 119 invents — it is the exact
principle `operational_dgraph_recovery.rs` already states for a structurally
adjacent problem ("A missing or invalid state is never repaired by publishing
a new graph" — i.e., recovery reads the persisted authoritative source, never
reconstructs or guesses). Applied to migration: on restart, any
`MigrationRecord`'s own `state` field must be treated as advisory only; the
first recovery action must be `active_placement_authority(project_id,
application_id)` — if it already names B, the record must be reconciled
forward to `cutover_accepted` (or later) regardless of what it last recorded,
and must **never** attempt to "restore" A. This directly satisfies the
task's critical crash-after-acceptance case (Section 38 of the task).

## 29. Source cleanup

**Reused, not reinvented.** Once B is accepted, A's former node(s) simply no
longer appear in the newly-derived operational projection for that service;
the existing per-node reconciliation loop, on its next ordinary cycle,
observes this and performs the existing lease-gated removal path already
named "M8"/`AUTHORIZATION_OPERATION_ROLLBACK` in
`operational_reconciliation.rs`. No ssh-based, docker-name-matching, or other
direct host cleanup is introduced or would be legal — all mutation remains
Stage-107A-governed exactly as today.

## 30. Drain/stateful limitations

No `drain`/`quiesce`/graceful-shutdown/in-flight-invocation-tracking concept
was found anywhere in current production source (the grep in Section 2 found
`drain`/`quiesce` only in unrelated files — `controlled_dgraph_deployment.rs`,
MQTT delivery internals, an event compactor — none of which implement a
service-level drain). **Classification: graceful drain is a deferred
advanced migration behavior, not required for Phase 119 v1.** This is
consistent with, not merely convenient for, D-Code v1's stateless/
fresh-instance execution constraint (Section 31 below) — a stateless,
fresh-instance-per-invocation service has no meaningful "in-flight session"
to drain beyond the single invocation DCall's own redemption/delivery
already correctly bounds (Section 17). **Stateful services**: current DCode
v1 is explicitly stateless/fresh-instance constrained (established, frozen
architecture); this audit does not invent checkpointing, memory migration,
filesystem replication, or session transfer, and records live migration of
stateful services as out of scope, deferred to whatever future phase relaxes
the stateless constraint.

## 31. Rollback before cutover

Before B's acceptance, rollback/abort is simply **abandoning preparation**:
A remains active throughout (it was never touched), and any prepared B
content is cleaned up according to ordinary governed rules (the content
store itself, or an explicit cleanup call — no DMap involvement whatsoever,
since no DMap-level change ever occurred). **No DMap rollback is needed
because authority never moved.**

## 32. Rollback after cutover

**After B's acceptance, rollback must be a NEW explicit DDeploy proposal
targeting the restored placement, followed by a NEW acceptance — never a
rewrite of DMap history or the prior acceptance record.** This is directly
supported by existing DDeploy semantics: acceptances are immutable, keyed
records (Phase 111/113, unchanged); there is no "unaccept" or history-mutation
API anywhere in `ddeploy.rs`/`ddeploy_v2.rs`. Reusing "previous A" as the
target's input requires fresh revalidation, not blind reuse — confirmed by
the same acceptance-time revalidation logic (`validate_full`,
`check_dcontinuum_authority`) that already treats *any* proposal, historical
or not, identically: D-Node existence/capability, D-Code binding
resolvability, ServiceArtifact eligibility, and DForward/DIoT relationships
must all be re-checked against *current* state. **Previous A is historical
evidence, never automatically safe rollback authority** — this is not a
new rule; it is what `validate_full`/`check_dcontinuum_authority` would
already enforce on any proposal, including one whose content happens to
match a historical DMap.

**CUTOVER_COMPLETE vs. OPERATIONALLY_CONVERGED**: kept distinct, per Section
25's state table (`cutover_accepted` vs. `converging`/`completed`).
Acceptance establishes B as desired authority instantly; runtime
cleanup/startup convergence is a separate, observable-but-not-authoritative
milestone that may lag behind it — collapsing the two would misrepresent
what DDeploy acceptance actually guarantees (authority, not runtime state).

## 33. Failure matrix

| # | Scenario | Canonical authority | Migration state consequence | Retry/rollback behavior | Human/policy input required? |
|---|---|---|---|---|---|
| F1 | B preparation fails | A | `preparing` → `aborted` | Retry preparation or abort | No (mechanism can fail closed automatically) |
| F2 | B node disappears (structurally removed) | A | `ready_to_cutover`/`preparing` → blocked; re-validation at cutover would fail closed | Must choose a new target or abort | Yes — target choice is policy |
| F3 | B capability changes | A | Same as F2 — acceptance-time `check_dcontinuum_authority` would reject | Retarget or abort | Yes |
| F4 | Source A changes during preparation (another operator accepts a different proposal) | Whatever the other operator accepted | `expected_active_dmap_revision` mismatch fails B's eventual acceptance closed | Migration must be re-derived from the new A | No — mechanism fails closed automatically; policy decides whether to retry |
| F5 | Target proposal becomes stale (DContinuum/DNode/ServiceArtifact/DCode/DForward drift) | A | Acceptance-time revalidation (existing, unmodified) rejects | Recreate/revise proposal | No, mechanism-only |
| F6 | DMonitor evidence goes stale | A | Advisory only in v1 (no warm-runtime readiness depends on it) — does not block mechanism | N/A in v1 | Yes, if used as an operator signal |
| F7 | Acceptance itself fails (any `AcceptError`) | A (unchanged — DDeploy transactions are all-or-nothing) | `cutover_requested` → `cutover_failed`; A remains authoritative | Retry cutover or abort | No |
| F8 | Crash before acceptance | A | On restart, record (if any) is advisory; re-derive from `active_placement_authority` (still A) | Resume preparation or abort | No |
| F9 | Crash immediately after acceptance | B (already committed) | On restart, record must be reconciled forward to reflect B — never "restore" A (Section 28/M12) | N/A — B already won | No |
| F10 | Target runtime fails after cutover | B (still canonical) | `converging` → `degraded` | Operator remediation, or a new rollback migration | Yes |
| F11 | Source cleanup fails | B (still canonical) | `converging`/`completed` → `degraded`; B never reverts | Operator remediation of the stuck A-side runtime | Yes |
| F12 | Rollback target (former A) is no longer valid | B (still canonical until a new acceptance succeeds) | Rollback proposal fails acceptance-time revalidation, exactly like any other invalid proposal | Choose a different rollback target | Yes |
| F13 | Old D-Call grant used after cutover | B (routing-wise) | Redemption fails closed with `StaleAuthority` (existing, unmodified — Section 17) | Caller must re-request a fresh grant | No |
| F14 | D-Code bytes absent at B | A (pre-cutover) or B (post-cutover, degraded) | Pre-cutover: preparation incomplete, cutover should not be requested. Post-cutover: authorization/execution on B fails per existing D-Code descriptor-currentness rules (unchanged) | Install bytes, retry | No, mechanism-only (policy decides urgency) |
| F15 | Multi-service partial preparation | A | Cutover remains blocked until the *entire* target proposal's prerequisites are satisfied (Section 39 of the task) — a partial DMap must never be accepted merely to make progress | Wait, or revise the target proposal's scope | Yes, if scope needs to shrink |

## 34. Scenario matrix

| # | Scenario | Assessment |
|---|---|---|
| S1 | Single service N1→N2 | Directly supported: target proposal changes one placement, carries all others forward unchanged |
| S2 | Multiple services all move N1→N2 | Directly supported: one target proposal, multiple placement changes, one acceptance |
| S3 | Only one of three services moves | Directly supported: target proposal carries the two unchanged placements forward explicitly |
| S4 | Two target nodes (fan-out from one source) | Directly supported: target proposal's placements simply name different nodes per service |
| S5 | Same placement, D-Code-only change | **Explicitly NOT a placement migration** — this is ordinary Phase 117-governed redeploy; Phase 119 mechanism is unnecessary (though not harmful) for this case |
| S6 | D-Code + placement change together | Supported — Section 16; no separate mechanism needed, the proposal already commits both |
| S7 | Pending B while A remains healthy | The expected/normal case for the recommended lifecycle |
| S8 | A unhealthy while B preparing | Mechanism does not automatically accelerate cutover (Section 23 of the task) — that is a policy decision left to the operator/caller; DMonitor evidence may inform but never triggers it automatically |
| S9 | B prepared but A changed by another operator | F4 — target's `expected_active_dmap_revision` no longer matches; acceptance fails closed; migration must be re-derived |
| S10 | B accepted, cleanup of A fails | F11 — `degraded`, B remains canonical, no automatic reactivation of A |
| S11 | Rollback B→former A | Section 32 — a new, freshly-revalidated proposal/acceptance, never a history rewrite |
| S12 | Former A unavailable during rollback | F12 — rollback proposal fails acceptance-time revalidation like any invalid proposal; a different target must be chosen |
| S13 | DMap/2 with DIoT/external bindings | Section 18 — the whole v2 bundle must be supplied explicitly; migration does not regenerate DForward/DIoT/DCall-realization data on its own |
| S14 | v1 source → v2 target | **Legal, and already explicitly supported** — `accept_with_previous`'s own doc comment: "This is what makes a v1 -> v2 transition retain the logical identity and monotonic revision without creating a second authority" (Section 49) |
| S15 | DServer restart at each lifecycle stage | Section 28 — recovery always re-derives from `active_placement_authority`, never trusts stale migration-record state alone |

## 35. Security invariants M1–M20

All evaluated directly against the evidence gathered this phase (this is an
audit — no code exists yet to test against, so each is evaluated as "does
the RECOMMENDED architecture, as specified, satisfy this," not as a
compiled-and-tested claim):

| # | Invariant | Assessment |
|---|---|---|
| M1 | Phase 119 never selects target B | HOLDS by design — B is always caller/operator/Phase-120-supplied; no scoring/ranking logic is proposed anywhere in this audit |
| M2 | Exactly one canonical placement authority exists per application | HOLDS — Section 25's state table never has two simultaneously-authoritative DMaps; dual-active remains rejected (Section 3/Option C) |
| M3 | Pending/prepared B cannot change D-Engine membership | HOLDS — Section 15, inherited unmodified from Phase 118 |
| M4 | Installing target bytes cannot change active DCode authority | HOLDS — Section 16/11, content store is independent of DCode registry/authorization |
| M5 | Preparing target runtime cannot redirect governed traffic before cutover | HOLDS — Section 11, no warm runtime is permitted to exist, so nothing could receive traffic |
| M6 | DMonitor cannot activate B | HOLDS — Section 19, DMonitor remains read-only evidence |
| M7 | Migration workflow cannot mutate DMap directly | HOLDS — Section 13, only `accept_proposal`/`accept_with_previous` ever mutates active placement, and migration only calls it exactly as any other caller would |
| M8 | Cutover uses normal DDeploy acceptance | HOLDS — Section 10/13, by design, no alternate path |
| M9 | Source A is bound by exact authority identity/CAS | HOLDS — Section 5/26, via the existing `expected_active_dmap_revision` |
| M10 | Stale source prevents cutover | HOLDS — same mechanism, already fail-closed |
| M11 | Target proposal is revalidated using current canonical authorities | HOLDS — `validate_full`/`check_dcontinuum_authority` at acceptance, unmodified |
| M12 | Crash recovery trusts active DMap over migration progress state | HOLDS by recommendation — Section 28, explicitly designed this way, mirroring `operational_dgraph_recovery.rs`'s own stated principle |
| M13 | Post-cutover cleanup failure cannot reactivate A | HOLDS by recommendation — Section 25/33 (F11), `degraded` state never reverts automatically |
| M14 | Post-cutover rollback is a NEW governed acceptance | HOLDS by recommendation — Section 32 |
| M15 | Old DCall route/grant cannot remain silently authoritative after cutover | HOLDS — Section 17, existing redemption-time stale-authority recheck, unmodified |
| M16 | Host mutation remains Stage107A-governed | HOLDS — Section 22, no new boundary or bypass introduced |
| M17 | DCode descriptor identity remains exact | HOLDS — Section 16, unchanged descriptor-currentness mechanism |
| M18 | DMap/2 extras are not silently regenerated by migration | HOLDS — Section 18, the proposal always carries the caller's explicit bundle |
| M19 | Migration progress never becomes placement authority | HOLDS by recommendation — Section 24/52, `migration_id` is workflow identity only |
| M20 | No runtime/hostname/container inference determines A or B | HOLDS — Section 5/6, both are exact canonical-authority/proposal references, never inferred from infrastructure facts |

All 20 assessed as satisfiable by the recommended architecture. None requires
new authority to satisfy; all are inherited from, or directly reuse, existing
Phase 111–118 mechanisms.

## 36. Authority matrix

| Question | Current owner | Migration may read? | Migration may mutate? |
|---|---|---|---|
| Current placement (A) | DDeploy active authority (`active_placement_authority`) | Yes | No |
| Target proposal (B) | DDeploy proposal store | Yes (its own submitted proposal) | Only through ordinary proposal creation, exactly like any caller |
| D-Node structural identity | D-Node Registry | Yes (read-only validation) | No |
| D-Code registry | D-Code registry/authorization | Yes (read-only) | No — install writes only to the independent content store, never the registry |
| Installed bytes | Local content-addressed module store (ADR-0021) | Yes — this is the one thing migration preparation may legitimately write | Yes, for content installation only, via the existing hash-verified write path |
| Runtime desired state | `OperationalDGraph` (server-derived from active DMap) | Yes (read) | No — never written directly; it is *always* a pure derivation of the active DMap, recomputed automatically at acceptance |
| Observed health | DMonitor | Yes (read, advisory) | No |
| Readiness | DMonitor (`dmonitor_readiness.rs`) | Yes (read, advisory in v1) | No |
| DCall routing | D-Call grant/route machinery | Yes (read, e.g. to understand currentness) | No — migration never issues/revokes grants directly; staleness is handled reactively by the existing redemption check |
| DIoT realization | DForward/DMap-v2 extras | Yes (read; must carry forward in the target proposal if unchanged) | No independent authority — only through the proposal, like any caller |
| Host mutation | Stage 107A-governed node-local reconciliation | No direct read/write — migration never mutates a host; it only causes the desired-state projection to change, which the existing loop then acts on | No |
| Active DMap | DDeploy | Yes (read) | Only via ordinary `accept_proposal`/`accept_with_previous`, exactly like any caller — never a special migration-only mutation path |
| Migration progress (if persisted) | The migration record itself, if 119B adopts one | Yes | Yes — this is the ONLY state migration may own outright, and even then only as advisory workflow tracking, never as a duplicate of any authority above (Section 52/53/54/55) |

## 37. Phase 115 relationship

Phase 115's own closure documents
(`phase115-dserv-observation-producer-closure.md`,
`phase115a-dserv-observation-authority-audit.md`,
`phase115h-stage107a-lease-cycle-hardening.md`) concern D-Serv **observation**
production and the Stage 107A **lease-cycle** hardening this audit already
reuses directly (Section 22). Phase 113B's "multinode" work
(`phase113b-live-multinode-closure.md`) proved **concurrent** multi-node
D-Call delivery (fog-01/cloud-01 simultaneously running different services),
which is a different concept from node-to-node **migration** — no placement
ever moved in that phase; two independent, permanently-placed nodes called
each other. **Phase 119 directly and substantially reuses**: the Stage 107A
lease-gated mutation boundary (Section 22), the per-node desired-state
reconciliation loop (Sections 3/10/29), and the `operational_dgraph_recovery.rs`
"never repair from a guess" recovery principle (Section 28). **Phase 119 does
NOT reuse**: anything from Phase 113B specifically, beyond it being useful
live-topology precedent for the eventual live proof (Section 39). The
node-local "rollback" (`AUTHORIZATION_OPERATION_ROLLBACK`, M8) found in
`operational_reconciliation.rs` is real, current, and reused — but only as
the *mechanism* for source-A cleanup, never as a template for what
migration-level "rollback" means (Section 1/32) — these are genuinely
different concepts that happen to share an English word, and this audit
does not treat the name match as semantic compatibility.

## 38. Minimum Phase 119B slice

Derived from evidence, not assumed to require a large controller:

**Recommended**: a small, **non-authoritative** set of pure types and a pure
validation/state-machine core, mirroring the shape Phase 117I/117J took for
DCompile submission:

1. `MigrationIntentV1` (or similarly named) — a thin, non-canonical,
   non-wire-required type that references (never duplicates) `project_id`,
   `application_id`, `source_dmap_ref` (`{id, revision, digest}`, matching
   `PlacementAuthorityView`'s own shape), and `target_proposal_id`/
   `target_proposal_digest`.
2. A pure function to validate that a supplied target proposal's
   `expected_active_dmap_revision` matches the caller-asserted source
   `dmap_ref` (a thin wrapper making the existing CAS check's precondition
   explicit and checkable before attempting acceptance, not a new CAS
   mechanism).
3. A pure function/table implementing the Section 25 state machine
   transitions, taking `active_placement_authority`'s current output plus a
   (possibly absent) persisted migration record, and returning the correct
   *derived* current state — exactly the M12 crash-recovery principle,
   implemented as code rather than merely documented.
4. **Persistence decision for 119B specifically**: recommend starting with
   **no persisted `MigrationRecord` at all** (an even smaller slice than
   full M1 in the task's own M1–M4 comparison) — i.e., stateless
   orchestration first (M1), with the caller/operator responsible for
   remembering source/target/progress across a single migration attempt,
   and the pure validation/state-derivation core (item 3 above) available
   for a future 119C to wrap in real persistence once a concrete need for
   surviving a multi-minute, restart-spanning migration is demonstrated
   live — this mirrors Phase 118's own "Option A first, upgrade only when a
   concrete consumer need appears" reasoning, which this audit adopts by the
   same logic.

**No host mutation, no new API route, no DMap/DDeploy/DMonitor/DCall change,
and no migration test suite belongs in 119A** — all deferred to 119B, and
even 119B's own footprint should start with the smallest slice above.

## 39. Future live-proof plan

**Not performed now.** The two required capabilities not exercised by Phase
117K's single-node proof are (a) a second, independently addressable,
runtime-capable D-Node, and (b) observing the *same* service transition its
authorized/executing node identity across two real hosts. Per the task's
known infrastructure list, **RPi (`dener@192.168.10.109`) + fog-01
(`192.168.10.103`)** is the preferred eventual pairing (cloud-01's historical
instability noted, not re-verified now; availability of either host is not
assumed or checked in this audit). The eventual live proof should
demonstrate, in order: service executing on A (real governed invocation,
Phase 117K-style) → explicit target B proposal created, naming A's exact
revision → D-Code content installed on B while A remains sole active
authority and a fresh authorization/invocation still resolves A (direct
generalization of Phase 117K §17's central proof) → cutover acceptance →
fresh authorization/invocation now resolves B, with real execution proof
(a distinguishable `output_reason_sha256` or equivalent, as 117K used) →
observed cleanup/convergence removing A's runtime → (if exercised) rollback
as a genuinely new transition, with its own fresh authorization proof. Only
this live run can prove: real cross-node D-Code content transfer at
production scale, real Stage-107A-governed cleanup on a real second host,
and the true end-to-end latency/window of a cutover. Everything else in this
audit's conclusions is provable from source/unit-level evidence alone.

## 40. Phase 120 boundary

Restated precisely (Section 8): Phase 120 must be able to produce an explicit
target placement (content matching what Section 6 calls B — a DDeploy
proposal or the placement content of one) using deterministic placement
logic; Phase 119 must be able to execute `A -> supplied B` without caring how
B was chosen. This audit's recommended architecture (Sections 6, 10, 38)
already satisfies this cleanly: nothing in the recommended `MigrationIntentV1`/
validation core inspects *why* a target proposal contains the placements it
does — it only validates and orchestrates.

## 41. ADR recommendation

**Recommend: reference documentation only for 119A/119B; no new ADR, and no
change to the existing DDeploy/reconciliation ADRs, is warranted at this
audit's evidence level.** Reasoning: every mechanism this audit recommends
reusing is already ADR-governed (DDeploy's proposal/accept lifecycle,
ADR-0002/ADR-0018; Stage 107A's lease boundary, ADR-0011/ADR-0012; D-Code
content addressing, ADR-0021) — migration does not introduce a new decision
requiring its own ADR-level durability, only a new *composition* of existing,
already-decided mechanisms. If 119B's implementation later reveals a genuine
new architectural decision (e.g., if evidence eventually forces adopting a
reservation/warm-capacity authority, Section 21), that decision — not
migration in general — would be the one meriting a focused ADR at that time.

## 42. Open questions

1. **Image/config pre-pull primitive.** Section 11 could not confirm whether
   a content-only, DMap-independent image-pre-pull mechanism analogous to
   D-Code's content store already exists for container images specifically.
   If it does not, Phase 119B preparation may be narrower in practice than
   Section 11's table optimistically allows — left for 119B's own source
   verification, not assumed here.
2. **Whether any future consumer will require warm-runtime overlap
   migration.** This audit declines to design it (Section 21/12), but does
   not rule out a future phase revisiting this if a concrete requirement
   (e.g. near-zero-downtime stateful migration) emerges.
3. **Exact `MigrationIntentV1` field list.** Deliberately not frozen in this
   audit (per the task's own instruction) — Section 38 names the concepts it
   must reference, not its final Rust shape.
4. **DMonitor's precise role once/if warm-overlap migration is ever
   supported.** Not designed here since warm overlap itself is deferred.
5. **Whether 119B should expose any API at all**, versus remaining pure
   library code consumed by a CLI in the `DATUM` crate (mirroring how
   `smartsentinel-dcode-install` is a CLI, not a DServer API route) — this
   audit leans toward "small library core first, API only if a concrete
   consumer need emerges" (Section 20's decision tree from Phase 118,
   analogously applied), but does not decide it definitively.
