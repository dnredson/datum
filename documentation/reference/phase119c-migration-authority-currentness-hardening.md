# Phase 119C — Migration Authority Currentness Hardening

- Status: Small, local hardening of the Phase 119B pure migration core.
- Branch: `agent/phase119-migration`, continuing from Phase 119B HEAD
  `2928c83dc77d4cf32560f72d984888489f735951`.
- Scope: distinguish historical target acceptance from current canonical
  authority, relationally validate accepted-target evidence, and fail
  closed on v1/v2 persisted-identity ambiguity — without adding a migration
  controller, `MigrationRecord`, persistence, API, target selection,
  D-Monitor dependency, or host mutation.

## 1. Problem discovered after 119B

Phase 119B's `TargetAlreadyAccepted { materialized_dmap_ref }` was reached
whenever the exact target proposal had ever been accepted, with no
distinction from whether that materialized D-Map was still the current
active authority. Concretely:

```
A current -> intent A->B -> B accepted -> C accepted later -> evaluate old intent A->B
```

119B reported `TargetAlreadyAccepted` here — historically true, but
insufficient as a *current* migration-authority relationship, since Phase
119A's own recovery principle is that the CURRENT active D-Map is canonical
truth, not a proposal's own historical acceptance record in isolation.

## 2. Historical acceptance vs. current authority

These are now explicitly two different facts, never collapsed:

- **Historical, immutable fact**: "proposal B was accepted and materialized
  D-Map X." This can never become false once true — acceptance records are
  never mutated or removed anywhere in `ddeploy.rs`/`ddeploy_v2.rs`.
- **Current, mutable fact**: "D-Map X is / is not the CURRENT accepted
  placement authority right now." This can change the instant a later
  proposal is accepted for the same application.

Phase 119C's relation model exposes both, by deriving the second directly
from `active_placement_authority` at evaluation time and comparing it
against the first (§8 of Phase 119A's own already-existing seam, reused
unmodified).

## 3. Relation-model refinement

`MigrationAuthorityRelation::TargetAlreadyAccepted` was replaced by two
variants:

```rust
TargetAcceptedAndCurrent {
    materialized_dmap_ref: CanonicalContractReferenceV1,
},
TargetAcceptedButSuperseded {
    materialized_dmap_ref: CanonicalContractReferenceV1,
    current_authority: Option<CanonicalContractReferenceV1>,
},
```

`ReadyForAcceptance`, `NoActiveSource`, `SourceStale`, and
`TargetInvalidForSource` are unchanged from 119B — their semantics were not
in question.

## 4. Accepted-and-current semantics

Reached when the target proposal's acceptance evidence relationally
validates (§8 below) AND the live `active_placement_authority` result is
byte-identical (`id`/`revision`/`digest`) to that acceptance's own
`materialized_dmap_ref`. It means ONLY that canonical placement cutover
occurred and remains the current authority right now — never that bytes
were installed, a runtime started, the target is healthy, source cleanup
completed, or the application is operationally converged. This is
documented directly on the variant itself, not left to prose elsewhere.

## 5. Accepted-but-superseded semantics

Reached when the target proposal's acceptance evidence relationally
validates, but the live active authority differs from what it materialized.
`current_authority` carries the live replacement (`Some`, the overwhelmingly
common case) or `None` in the case — not known to be reachable through any
existing legitimate D-Deploy path, since there is no "unaccept"/removal API
anywhere in `ddeploy.rs`/`ddeploy_v2.rs` — where no active authority exists
at all despite this target having once been accepted. That sub-case is
handled defensively (per this phase's own instruction: "represent honestly
... unless repository invariants prove this state is corruption") rather
than assumed impossible or promoted to a hard error.

## 6. Crash-after-acceptance

Preserved and independently re-tested (`h2_crash_after_acceptance_recognizes_b_as_accepted_and_current`):
target-acceptance status is still checked strictly before source currency,
so re-evaluating an untouched `MigrationIntentV1` immediately after its
target's acceptance committed correctly yields `TargetAcceptedAndCurrent`,
never `SourceStale`, even though the captured source reference no longer
matches live authority.

## 7. Unrelated-C-before-B semantics

Preserved exactly (`h5_unrelated_c_accepted_before_pending_b_is_still_source_stale`):
if an unrelated proposal C is accepted while B is still `Pending`, B itself
never won, so the relation remains `SourceStale` — never any
accepted-target variant. This is the case Phase 119C's task explicitly
required to remain distinct from §9's later-supersession case, and the two
are exercised by separate tests reaching different code paths (the
"target.accepted" branch is never entered for this scenario at all, since
B's own `state` never changes).

## 8. Acceptance relational integrity

The accepted-target branch no longer merely dereferences
`accepted_acceptance_id` into an acceptance record's
`materialized_dmap_ref`. It now additionally verifies, using each
acceptance record's own `proposal_id`/`proposal_digest`/`project_id`/
`application_id` fields (already present on both `DDeployAcceptanceV1` and
`DDeployAcceptanceV2`, unmodified):

```
acceptance.proposal_id      == intent.target_proposal_id
acceptance.proposal_digest  == target's own stored/recomputed proposal_digest
acceptance.project_id       == intent.project_id
acceptance.application_id   == intent.application_id
```

Any mismatch fails closed with the new `F_MIGRATION_TARGET_ACCEPTANCE_INCONSISTENT`
finding, before `materialized_dmap_ref` is ever trusted for anything. Four
dedicated tests (`h7`–`h10`) construct each mismatch directly by tampering a
legitimately-created, legitimately-accepted persisted acceptance record in
place (the same "tamper the stored record, prove fail-closed" technique
already used elsewhere in `ddeploy.rs`'s own v2 integrity tests), since no
normal API path can produce an internally inconsistent acceptance record —
these are defense-in-depth tests against persisted-state corruption, not
reachable-via-API scenarios.

## 9. v1/v2 identity ambiguity hardening

Neither `create_proposal`/`DDeployStateV2::create_proposal` nor
`accept_proposal`/`accept_v2_proposal` check the OTHER version's store
before inserting a proposal or acceptance record — confirmed by direct
re-read this phase — so no existing repository invariant proves a
`proposal_id` or `acceptance_id` collision between the v1 and v2 stores is
impossible. `locate_target_proposal` and the new `locate_acceptance` helper
therefore both check BOTH stores and return `Err(())` (mapped to
`F_MIGRATION_TARGET_PROPOSAL_AMBIGUOUS`/`F_MIGRATION_TARGET_ACCEPTANCE_AMBIGUOUS`
respectively) if the same ID exists in both, rather than silently preferring
v1 as 119B originally did. Two dedicated tests (`h11`, `h12`) construct each
collision directly by inserting a minimal, structurally-complete stub record
into the "other" store under a colliding ID — bypassing normal v2 creation
entirely, since the point is to prove the evaluator's own defense, not to
claim this is reachable through any current legitimate API.

## 10. Source-reference validation

`validate_intent_shape` now additionally rejects `source_dmap_ref.revision
== 0`, mirroring the existing, unmodified canonical D-Map contract exactly:
both `CanonicalDMapV1::validate()` and `CanonicalDMapV2::validate()` already
reject `revision < 1` (`canonical_dmap.rs`/`canonical_dmap_v2.rs`, confirmed
unchanged this phase), and `accept_proposal`/`accept_with_previous` always
produce revision `>= 1`. No incompatible rule was invented.

## 11. Source exactness vs. revision CAS

Restated precisely, since this distinction is easy to blur and this phase's
own task required documenting it accurately: `intent.source_dmap_ref`'s
exact `{id, revision, digest}` comparison against the live
`PlacementAuthorityView` is this module's OWN pure preflight check, answering
"which exact authority did this migration intend to leave?" The existing,
completely unmodified `expected_active_dmap_revision` field — checked inside
`accept_proposal`/`accept_with_previous`/`accept_v2_proposal` — remains the
ONLY actual transactional race guard, answering "did any acceptance happen
after this proposal was created?" Phase 119C added no
`expected_active_dmap_digest`, no migration-specific lock, and no second
acceptance guard of any kind — `git diff` against `ddeploy.rs`/`ddeploy_v2.rs`
for this phase is empty, confirming zero DDeploy transaction-semantics
changes.

## 12. Negative-space authority boundaries

Unchanged from 119B, re-confirmed: `evaluate_migration_authority_relation`'s
only parameters remain `&DDeployStateV1` and `&MigrationIntentV1`; no
D-Monitor, D-Code registry, D-Call, D-Node registry, or Stage 107A type is
imported anywhere in the module (confirmed by its own `use` statements —
only `canonical_dgraph`, `canonical_dmap`, `ddeploy`, and `ddeploy_v2` types
are referenced). `MigrationIntentV1`'s shape is completely unchanged from
119B (`project_id`, `application_id`, `source_dmap_ref`,
`target_proposal_id`, `target_proposal_digest` — no `migration_id`, no
`schema`, no timestamps, no state, no health fields). The module's own
top-of-file doc comment was corrected to no longer claim
`Serialize`/`Deserialize` derives that were never actually present — it now
accurately states the type derives only `Debug`/`Clone`/`PartialEq`/`Eq`
and deliberately carries no serde implementation, matching
`PlacementAuthorityView`'s own precedent.

## 13. Tests

All 22 tests carried forward from Phase 119B (renumbered/consolidated where
119B's own `TargetAlreadyAccepted`-based tests needed updating for the new
variant names — `T13`/`T14` became `H1`/`H2`, `T15`/`T21` became `H5`,
`T25`/`T26` became `H18`–`H20`, with their original assertions preserved
verbatim in spirit). Ten new tests added for this phase's own hardening:
`H3` (accepted-then-superseded), `H4` (deterministic re-evaluation of a
superseded target), `H6`–`H10` (acceptance relational-integrity failures:
missing, wrong proposal_id, wrong digest, wrong project, wrong application),
`H11`/`H12` (v1/v2 identity ambiguity for proposals and acceptances), `H13`
(source revision zero rejected), `H17` (accepted-but-superseded across a
real v1→v2 D-Map-generation transition, using the actual production
`create_v2_proposal`/`accept_v2_proposal` entry points — not the inner
`DDeployStateV2` methods alone, since only the outer wrapper correctly
retires the stale v1 active record as part of the same transition), and one
additional explicit read-only proof specifically on the new accepted-target
currentness path. **32 tests total, all passing.**

## 14. Remaining Phase 119 work

Phase 119 (migration A→B) remains open. What exists now: an exact source
reference, an exact target proposal reference, and a fully-hardened,
currentness-aware, relationally-validated authority relation between them —
still purely read-only, still with zero persistence, zero API, and zero
target-selection logic. A future 119D/orchestration phase would need to
decide whether and how to wrap this core in real workflow persistence
(explicitly not warranted yet, per both 119A's and 119B's own reasoning,
unrevisited by this phase), and eventually perform the live cross-node proof
Phase 119A §39 describes.

## 15. Live-proof readiness

Unchanged from 119A/119B: this phase performed no live validation and needed
none — every property hardened here (currentness comparison, relational
acceptance-evidence validation, v1/v2 ambiguity detection, revision
validation) is a pure, deterministic, in-memory fact provable entirely from
already-exercised `DDeployStateV1` fixtures. The eventual live proof
(RPi + fog-01, per Phase 119A §39) remains gated on future orchestration/
preparation work, not on anything this phase touched.
