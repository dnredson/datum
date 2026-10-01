# Phase 119E — Independent Migration A→B Closure Review

## 1. Verdict

**C — FAIL. PHASE 119 REMAINS OPEN.**

The authority/cutover half of migration (placement transition, D-Engine
derivation, DCode authorization/execution following the accepted target) is
fully designed, hardened, and live-proven in both directions. It is sound
and requires no further core changes. **Phase 119 as a whole cannot close**
because Phase 119A's own definition of "migration A→B" explicitly included
governed cleanup of no-longer-desired runtime on the source, and this
review independently confirms — by direct source tracing, not merely by
trusting Phase 119D's own account — that **no governed production mechanism
exists today by which a node can learn a service has been migrated away and
must be removed.** This is a genuine capability gap, not an untested-but-
present capability, and a real operator relying only on governed DATUM
mechanisms cannot complete a full migration to a converged, cleaned-up end
state.

## 2. Reviewed range

`43d8ab5..39972b8` (Phase 118 closure → Phase 119D commit), 4 commits:

| Commit | Classification | Files |
|---|---|---|
| `e8783b5` | 119A architecture | `phase119a-...-audit.md` (new) |
| `2928c83` | 119B pure migration core | `ddeploy_migration.rs` (new), `mod.rs` (+1 registration) |
| `189de1b` | 119C currentness/integrity hardening | `ddeploy_migration.rs` (rewritten) |
| `39972b8` | 119D live evidence | `phase119d-...-proof.md` (new) |

`git diff --stat`/`--name-status` confirm exactly 6 files touched across the
whole range, all squarely Phase-119-scoped. **No unrelated production
change found.** `git diff --check` over the range flags one cosmetic
trailing-whitespace byte in the already-committed 119D document (a markdown
table row) — trivial, non-blocking, and left untouched per this review's own
"stage only the new document" instruction.

## 3. Phase119 responsibility reconstructed

Independently re-derived from DATUM's own architectural layering (D-Map =
desired placement authority; DMonitor = observed reality; reconciliation =
the convergence mechanism connecting them — a separation enforced
consistently since Phase 111) and from Phase 119A's own text (re-read in
full this phase, not merely cited): **119A's own definition of "migration
A→B" is explicitly the union of both candidate definitions the task poses**,
not the minimal one:

> "Migration A→B means: transition one application's sole canonical
> accepted placement authority from its current state (A) to an explicit,
> caller-supplied target state (B), through exactly one existing governed
> DDeploy acceptance, with legal (content-only) preparation permitted
> beforehand **and governed cleanup of no-longer-desired runtime happening
> afterward through the existing reconciliation loop**." (119A §4, quoted
> verbatim)

This settles the central ambiguity directly: Phase 119's own founding
document did not scope itself to authority-transition alone — it explicitly
promised reuse of an "existing reconciliation loop" for cleanup, treating
that as an already-solved, reusable capability. This review's own
independent tracing (§10) found that promise to rest on a false premise.

## 4. A/B authority model

Re-confirmed unchanged and coherent:

- **A** = `PlacementAuthorityView` (`dmap_id`/`revision`/`digest`), exactly
  `active_placement_authority`'s own output — never a hostname, container,
  or caller-invented reference.
- **B** = an ordinary, pending `DDeployProposalV1`/`V2` — never a fabricated
  DMap (DMap identity is server-owned, materializing only at acceptance).
- B is never chosen by Phase 119's own code — `ddeploy_migration.rs`
  imports no placement-scoring, no D-Node ranking, no capability
  comparison; it only references an already-existing `target_proposal_id`.
- Acceptance remains exclusively server-owned:
  `evaluate_migration_authority_relation` never calls `accept_proposal`/
  `accept_with_previous`/`accept_v2_proposal` anywhere (confirmed by direct
  read of every line of the function, §5 below) — it only *observes*
  whether acceptance has already happened.
- No second authority was created: `MigrationIntentV1`/
  `MigrationAuthorityRelation` carry no `dmap_id`-shaped identity of their
  own; every reference is to `CanonicalContractReferenceV1` values that
  themselves point back at DDeploy's own DMap/proposal identities.

## 5. 119B core review

Read in full, fresh, this phase (`dserver/src/core/ddeploy_migration.rs`).
Confirmed directly (not merely cited from 119B's own report):

- `MigrationIntentV1` derives only `Debug, Clone, PartialEq, Eq` — no
  `Serialize`/`Deserialize`, no `schema` field. Non-wire, confirmed.
- `evaluate_migration_authority_relation(state: &DDeployStateV1, intent:
  &MigrationIntentV1) -> Result<MigrationAuthorityRelation,
  CanonicalValidationFinding>` — takes `state` by **shared reference only**;
  no `&mut` anywhere in the function signature or body; no call to any
  mutating DDeploy function. **Pure and read-only, confirmed by direct
  inspection of every branch.**
- No persistence: the function is never called from any API handler or
  storage path anywhere in the diff range (confirmed — `grep` for its name
  outside its own module and tests returns nothing).
- References, never duplicates: `source_dmap_ref` is a
  `CanonicalContractReferenceV1` (the same shape `PlacementAuthorityView`
  exposes); the target is `target_proposal_id`/`target_proposal_digest`
  only — no placements, D-Code bindings, or D-Forward/D-IoT content is
  copied into the type anywhere.

## 6. 119C currentness review

Confirmed live in the current file, all six final relation variants exist
exactly as documented: `ReadyForAcceptance`, `NoActiveSource`,
`SourceStale`, `TargetInvalidForSource`, `TargetAcceptedAndCurrent`,
`TargetAcceptedButSuperseded`. Directly re-verified the decision order:
target-acceptance status is checked **before** source-currency evaluation
(lines ~408–469), which is precisely what makes crash-after-acceptance
recovery and the "unrelated proposal won" distinction correct. Relational
integrity (`acceptance.proposal_id`/`proposal_digest`/`project_id`/
`application_id` all cross-checked against the target proposal, not merely
dereferenced) is present exactly as documented. v1/v2 ambiguity is detected
for **both** proposal identity (`locate_target_proposal`) and acceptance
identity (`locate_acceptance`), each returning `Err(())` — mapped to
`F_MIGRATION_TARGET_PROPOSAL_AMBIGUOUS`/`F_MIGRATION_TARGET_ACCEPTANCE_AMBIGUOUS`
— rather than silently preferring v1. Source revision `< 1` is rejected in
`validate_intent_shape`. **All fail-closed paths confirmed present in the
current source, not merely asserted in prose.**

## 7. 119D live evidence review

Cross-checked every numeric claim in the 119D document against itself for
internal consistency (task's own "do not accept prose alone if the document
contains contradictory values" instruction): DMap revision sequence
**1 → 2 → 3** is consistent throughout every table and every prose
reference; module digest `98340f04...dea2b7d` and descriptor digest
`sha256:9e2555cc...7d6d51` are identical across §17/§20/§24/§28's tables
with no contradiction found anywhere; D-Engine membership values
(`{rpi-119d}` → `{fog-119d}` → `{rpi-119d}`) are consistent with the
placements shown in each corresponding DMap. No internal contradiction was
found in the document.

## 8. Proven authority lifecycle (L1–L13)

| # | Claim | Verdict |
|---|---|---|
| L1 | Target content may exist before activation | **PROVEN** — DCode installed on B while proposal B remained `Pending` |
| L2 | Pending B does not alter active DMap | **PROVEN** — re-verified live immediately pre-cutover |
| L3 | Pending B does not alter D-Engine | **PROVEN** — same re-verification |
| L4 | Pending B does not authorize B execution | **PROVEN** — HTTP 422 confirmed twice (pre-migration and pre-cutover) |
| L5 | A remains executable pre-cutover | **PROVEN** — real invocation succeeded with matching output hash |
| L6 | DDeploy acceptance is the sole canonical cutover | **PROVEN** — no other mutation path was used anywhere in the run |
| L7 | DMap revision increments monotonically | **PROVEN** — 1 → 2 → 3, never rewound |
| L8 | D-Engine flips A→B discretely | **PROVEN** — verified immediately post-acceptance |
| L9 | DCode authorization flips A→B | **PROVEN** — symmetric 422/200 flip confirmed |
| L10 | Identical DCode bytes execute successfully on both real nodes | **PROVEN** — byte-identical module/descriptor/output-reason hashes across aarch64 and x86_64 |
| L11 | Rollback is a new proposal + acceptance | **PROVEN** — new `proposal_id`/`acceptance_id`, `expected_active_dmap_revision: 2` (not a reuse of revision 1's old values) |
| L12 | Rollback produces a new monotonic revision | **PROVEN** — revision 3, never a rewind to revision 1 |
| L13 | Governed execution returns to A | **PROVEN** — matching module/descriptor/reason hashes, new `dmap_revision: 3` |

**All 13 authority-layer properties: PROVEN**, independently re-verified
against the document's own recorded values, not merely accepted as
narrated.

## 9. Physical reconciliation evidence

Confirmed directly against the 119D document's own §26: physical
Stage107A-governed runtime start on A, start on B, cleanup on A, and
runtime convergence after either cutover or rollback are **all explicitly
recorded as "not exercised,"** never silently upgraded from desired-state
evidence into actual-runtime evidence anywhere in the document. This
statement is accurate and independently confirmed — no table in the
document claims physical convergence that the prose disclaims.

## 10. Source-cleanup signal gap — independent trace

Traced fresh, independently, in `dserver/src/core/ddeploy.rs` and
`DATUM/src/agent/operational_reconciliation.rs`:

1. `union_project_operational_graph` (`ddeploy.rs`) builds
   `merged_by_node` by iterating **only** `app.active_dmap.placements` for
   every currently-active application in the project. A node with zero
   current placements across every active application receives **no
   entry** in the resulting `OperationalDGraph.nodes` vector — confirmed by
   reading the full function body, not merely its signature.
2. `build_operational_slice` (`operational_dgraph.rs`) looks up a
   requested `node_id` in `graph.nodes` via `.find(...).ok_or_else(...)`,
   returning a hard `Err("node '{}' is not present in operational
   D-Graph")` when absent — never an empty-but-present slice.
3. The reconciliation agent's own per-service loop
   (`operational_reconciliation.rs`, `for service in
   &context.desired_services`) only ever visits services **currently**
   present in the served list; a node whose slice is missing entirely
   never reaches this loop at all.
4. No orphan-sweep, no locally-persisted-state comparison against current
   authority, and no "stopped"/"absent" sentinel exists anywhere in the
   DATUM agent crate (confirmed by grep — zero matches for `orphan`,
   `sweep`, `prune`, or any dropped-service handling).

**Answer: NO.** Node A cannot currently learn, through any supported
governed production mechanism, that a migrated-away service must be
removed. This was independently re-confirmed at the code level in this
review (not merely re-cited from 119D), and 119D's own live behavior
(`"operational D-Graph node slice not found"`) is the direct, expected
consequence of exactly this code path — the two are consistent, not merely
coincidental.

**Classification: this is a Phase 119 closure blocker, not merely a
downstream reconciliation limitation** (full reasoning in §21/§23).

## 11. Firebase bootstrap limitation — independent assessment

Confirmed: `DATUM/src/agent/bootstrap.rs`'s `query_node_context`
unconditionally calls `GET /api/v1/nodes/{id}/latest` and treats any
non-200/404 response as a hard failure (`bail!`). `dserver/src/storage/firebase.rs`
hardcodes `base_url` to `https://firestore.googleapis.com/...` with no
emulator/override configuration point found anywhere in the struct or its
constructor. **This is both (A) an environment-specific gap** — the
isolated test DServer used a placeholder, unreachable Firebase project,
while a real production deployment (per the repository's own checked-in
`dserver/config.toml`, which names a real project `monitor-f49c8`) would
not hit this at all — **and (B) a genuine architectural production
dependency** — the reconciliation CLI's bootstrap step is unconditionally
coupled to a legacy, unrelated Firestore-backed telemetry subsystem, with
no way to configure it out even in a legitimate lightweight/CI/emulator
deployment. **Neither alone blocks Phase 119 closure**: even if fully
fixed, Finding #1 (§10) would still prevent source cleanup. This finding is
kept explicitly separate from Finding #1 (§16 below) and is not a Phase 119
closure blocker.

## 12. Target realization

Confirmed directly in the 119D document (§25): after cutover,
`GET /api/v1/nodes/fog-119d/operational-reconciliation-context` returned a
real, non-empty `desired_services` entry for `probe` at revision 2. This is
a genuine **desired-state signal**, independently distinguished from an
**actual-runtime-started** claim, which the document does not make. The
absence of a live-started target process is squarely attributable to
Finding #2 (§11) — an environment-validation/coupling issue, not a
mechanism blocker in its own right, since the desired-state signal itself
(the actual thing Phase 119's placement transition needs to produce) is
correctly present and correct.

## 13. Source removal

`"operational D-Graph node slice not found"` is categorically different
from `desired_services: []`: the former means the node has been dropped
from the OperationalDGraph entirely (§10, point 1–2); the latter would mean
"you are a known, current participant with zero desired services right
now" — a state the current architecture never produces for a node whose
placements went to zero. **The existing M8/"rollback"-named remove-only
native-process path (`operational_reconciliation.rs`) can only be reached
via the per-service loop over `context.desired_services`** — since a
dropped node's context does not exist at all, that loop is never entered,
and M8 is unreachable for this scenario specifically. M8's mere existence
does **not** imply source cleanup is reachable — this review does not make
that inferential leap.

## 14. Rollback

**Authority decision: fully proven.** New `proposal_id`, new
`expected_active_dmap_revision` (bound to the *current* authority at
rollback-proposal-creation time, not a stale reused value), new
`acceptance_id`, new monotonic DMap revision (3, never 1). This satisfies
Phase 119's rollback responsibility **at the authority layer completely** —
confirmed independently, no gap found here.

**Physical-convergence decision: not proven**, for the same reason as §9 —
explicitly disclosed, not fabricated.

## 15. DCode

Confirmed: module digest and descriptor digest are byte-identical across
every stage in 119D's own DCode table (§31 of that document) — A
pre-migration, A pre-cutover recheck, B post-cutover, and A again after
rollback. This is architecturally distinct from Phase 117's DCode
*revision replacement* (same node, new descriptor) — Phase 119 exercised
the orthogonal case (same descriptor, different node), and the two were
never conflated anywhere in the evidence.

## 16. D-Engine

Confirmed discrete, never-unioned membership at every stage: `{rpi-119d}`
→ `{fog-119d}` → `{rpi-119d}`. Physically stale runtime remaining on A
after cutover (a real possibility given Finding #1) does **not** and
structurally **cannot** make A a D-Engine member again — `derive_active_dengine_nodes`
(re-confirmed unchanged, §"verify starting state" work in this review)
reads only the currently active DMap's own placements, never any
node-local runtime-presence fact. This is an important, independently
reaffirmed architectural strength: **the physical-cleanup gap found in §10
is a reconciliation-layer problem, not an authority-layer problem** —
D-Engine, DCode authorization, and DMap all remain correct and
un-contaminated by it.

## 17. DCall/DMonitor/DNode boundaries

- **DCall**: not required and not exercised; 119A's own stale-authority
  revalidation at grant redemption (already proven, unmodified by Phase
  119) remains sufficient. No additional Phase 119 work needed here.
- **DMonitor**: not used as migration authority anywhere in the diff range
  (`ddeploy_migration.rs` imports no DMonitor type). Its absence from live
  evidence is irrelevant to authority closure — DMonitor was never part of
  what Phase 119 needed to prove at the authority layer.
- **DNode/D-Continuum**: migration owned no structural node identity,
  capability, or capacity authority anywhere — both 119D's DNode
  declarations and the whole DCompile/DCode/DDeploy chain used the
  existing, unmodified `PUT /api/v1/dnodes/{id}` and D-Continuum-derivation
  paths exclusively.

## 18. Security invariants M1–M20

| # | Invariant | Result |
|---|---|---|
| M1 | Target selection not owned by Phase 119 | **HOLDS BY CODE** — no scoring/ranking logic anywhere in `ddeploy_migration.rs` |
| M2 | One current placement authority | **PROVEN** live + holds by code (dual-active guard unchanged) |
| M3 | Pending B does not alter current authority | **PROVEN** (L2) |
| M4 | Content staging does not activate B | **PROVEN** (L1, L4) |
| M5 | Cutover only through DDeploy acceptance | **PROVEN** (L6) |
| M6 | D-Engine follows accepted placement only | **PROVEN** (L8) + **HOLDS BY CODE** (§16) |
| M7 | DCode authorization follows accepted placement only | **PROVEN** (L9) |
| M8 | Exact executable identity can remain constant | **PROVEN** (L10, §15) |
| M9 | Stale source fails closed | **HOLDS BY CODE** (`SourceStale` variant, §6) |
| M10 | B historical vs current distinguished | **HOLDS BY CODE** (`TargetAcceptedAndCurrent`/`TargetAcceptedButSuperseded`, §6) |
| M11 | Rollback is new governed transition | **PROVEN** (L11, L12) |
| M12 | Current authority wins after restart/re-evaluation | **HOLDS BY CODE** (119C's own crash-recovery ordering, §6) |
| M13 | Migration does not own DMonitor health | **HOLDS BY CODE** (§17) |
| M14 | Migration does not own DNode structure | **HOLDS BY CODE** (§17) |
| M15 | Host mutation remains Stage107A-governed | **HOLDS BY CODE** — no mutation path added anywhere; ServiceArtifact/native-process reconciliation reused unmodified |
| M16 | Source cleanup cannot be guessed from missing/unreachable server state | **HOLDS BY CODE** — confirmed no code anywhere treats a 404/missing context as "remove everything"; this is precisely *why* Finding #1 is a real gap rather than a solved-but-untested path |
| M17 | Absent desired state must be authoritative before removal | **N/A today** — no removal mechanism exists at all yet to evaluate this against; the invariant is correctly *not violated* only because nothing acts on the absence |
| M18 | No dual-active canonical DMap | **PROVEN** live + **HOLDS BY CODE** |
| M19 | No hostname/container inference determines placement | **HOLDS BY CODE** — placement is always `service_id`/`node_id` from an accepted DMap |
| M20 | Phase 120 can supply B without becoming migration authority | **HOLDS BY CODE** (§4) |

**19 of 20 fully hold; M17 is vacuously satisfied only because no removal
path exists to test it against — recorded honestly rather than marked
simply "PROVEN."**

## 19. Phase 120 dependency

**Decision: D3 — Phase 120 may begin in parallel; Phase 119 remains open.**
Phase 120's own responsibility (deterministic placement *selection*,
producing a target proposal) is independent of whether physical
convergence/cleanup can complete — target-selection logic can be built and
validated entirely against D-Node/D-Continuum/ServiceArtifact eligibility
data without any dependency on runtime reconciliation. Blocking Phase 120
entirely (D2) would be an unjustified schedule cost with no architectural
basis; declaring Phase 119 fully closed merely because Phase 120 doesn't
need to wait (D1) would misrepresent Phase 119's own unmet promise (§3).
D3 lets both proceed on their own true dependencies.

## 20. Test/build evidence (independently re-run this phase)

- `cargo test ddeploy_migration --locked` → **32 passed, 0 failed**.
- `cargo test ddeploy:: --locked` → **240 passed, 0 failed** (unchanged
  baseline).
- `cargo test --locked` (full `dserver`) → **1903 passed, 0 failed**, plus
  1/1 in `control-state-inspect`.
- `cargo build --release --locked` → succeeded, warnings only (pre-existing).
- `git diff --check` → clean (current working tree; the one cosmetic
  whitespace byte noted in §2 is in an already-committed 119D file, not
  reintroduced by this review).
- **DATUM regression** (re-run for closure confidence, since 119D exercised
  DATUM binaries live): `cargo test --locked` in `DATUM/` →
  **main unit-test binary: 447 passed, 0 failed**; every CLI binary target:
  0 tests (expected, no embedded tests); every integration test file: all
  passed, with some infrastructure-gated tests correctly `ignored` (5, 1,
  7, 1, 1, 1, 1 across the live/hardware-dependent suites) rather than
  run — **zero failures anywhere in the DATUM crate.**

## 21. Closure blockers

**Blocker 1 (the only blocker): the source-cleanup signal gap (§10).**
Exact statement: no governed production mechanism today allows a D-Node to
learn that a service which has migrated away must be removed, once that
node drops out of the project's `OperationalDGraph` entirely. This directly
contradicts Phase 119A's own stated migration definition (§3) and leaves a
real operator with no governed path to a converged end state after any
migration — the old copy can persist on the source node indefinitely with
zero governed remediation.

## 22. Non-blocking limitations

- Finding #2, the Firebase-bootstrap coupling (§11) — real but separable;
  does not by itself prevent closure and should not be bundled with
  Blocker 1's fix (§39-equivalent reasoning: source-cleanup semantics can
  be unit/integration-proven entirely independently of Firebase).
- No warm/zero-downtime migration, no state transfer, no `MigrationRecord`
  persistence, no DMonitor telemetry in this run — all correctly
  non-essential to what "migration" must mean at minimum.
- The one cosmetic trailing-whitespace byte in the 119D document (§2).

## 23. Phase119 final decision

Phase 119D is **correctly closed as a qualified live-proof subphase** — it
honestly disclosed exactly what it did and did not prove, and this review
found no misrepresentation in it. **Phase 119 as a whole remains open**,
specifically and only because of Blocker 1. Phase 119B/119C's production
code required no changes and is confirmed to provide real, non-redundant
value (exact A/B relation, stale-source detection, historical-vs-current
distinction, restart-safe re-evaluation, v1/v2 fail-closed ambiguity
handling) — none of this is unnecessary abstraction, and none of it needs
further work to close Phase 119; the remaining work is entirely in the
reconciliation/OperationalDGraph layer, not in the migration core.

## 24. Recommended next step

**Recommend Phase 119F**, scoped narrowly to Blocker 1 only (Finding #2
kept separate, per §22, as independent foundation-hardening debt for a
later phase — not combined with 119F).

Likely goal (to be finalized by 119F's own design work, not decided here):
a D-Node that previously hosted a DATUM-managed service must receive an
authoritative, explicit empty/removal desired-state signal after that
service migrates away — **without** ever treating a generic failure,
timeout, authorization error, or corrupted-state condition as an implicit
"remove everything" instruction (M16/M17 must both remain strictly
enforced by any fix). Compared possibilities, for 119F's own future
judgment, not implemented here:

- **F2-shaped** (reconciliation context API returns a valid, empty desired
  slice for a node DServer still structurally knows about, rather than a
  missing-node error) appears the most architecturally conservative
  option: it only changes what counts as "known" vs. "not found," and lets
  the existing per-service loop naturally see zero desired services rather
  than needing new tombstone semantics — but this still requires the
  server to distinguish "structurally known node, zero placements" from
  "node never declared"/"unreachable," which 119F must design carefully
  against M16/M17.
- **F4-shaped** (explicit tombstone/`desired_state = "absent"` entries)
  would let the agent's existing per-service dispatch loop handle removal
  through its ALREADY-EXISTING native-process/container stop path with the
  smallest agent-side change, at the cost of the server needing to track
  "previously placed, now removed" history somewhere.
- **F1/F3/F5** were not favored on first read (F1 risks conflating
  structural D-Node history with placement history; F3's "no desired
  services = remove all" is exactly the pattern §21 of the task warns
  against unless extremely carefully scoped; F5's orphan-sweep needs its
  own new local-inventory tracking) — 119F's own review should weigh all
  five properly before choosing, this review does not decide.

Minimum test suite for 119F: unit tests proving (a) a node with zero
current placements is still correctly distinguished from an unknown node,
(b) the chosen removal signal never fires from a transport/auth/corruption
failure, (c) multi-application projects behave correctly when only one
application's placement moves away, (d) restart-safety. An eventual live
re-proof (RPi + fog-01, reusing 119D's exact topology) would then close the
physical-convergence half of Phase 119's own definition.
