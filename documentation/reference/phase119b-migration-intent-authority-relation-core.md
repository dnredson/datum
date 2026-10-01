# Phase 119B — Migration Intent & Authority-Relation Core

- Status: Production implementation, small and non-authoritative.
- Branch: `agent/phase119-migration`, continuing from Phase 119A audit HEAD
  `e8783b55b7b1c0cabaf2cb95b6ee6fce3a105d81`.
- Scope: implement the smallest pure core that can represent "migrate this
  exact current placement authority A to this exact already-existing target
  D-Deploy proposal B" and derive their current relationship, without
  preparation, acceptance, mutation, or placement selection.

## 1. Scope

This phase adds exactly one new production module,
`dserver/src/core/ddeploy_migration.rs`, containing two public types
(`MigrationIntentV1`, `MigrationAuthorityRelation`) and one public function
(`evaluate_migration_authority_relation`), plus its own focused test suite.
No API route, no persistence, no host mutation, no D-Monitor dependency, no
target-selection logic, and no D-Deploy/D-Map mutation were added anywhere.

## 2. 119A refinement

Phase 119A (`documentation/reference/phase119a-migration-a-b-architecture-lifecycle-audit.md`)
recommended, as its minimum 119B slice, a persisted-state-free core plus a
"pure function/table implementing the §25 state machine transitions." This
phase's own task framing correctly refined that: without a persisted
`MigrationRecord`, facts such as "preparing," "content-prepared," or
"cleaning-up" cannot be honestly derived from canonical D-Deploy state alone
— nothing in `DDeployStateV1` records whether bytes were installed on a
target node or whether an operator started a migration attempt. This phase
therefore implements no eight-state workflow machine. It implements only
what is genuinely, deterministically derivable from already-persisted
D-Deploy proposal/acceptance records: the relationship between an exact
source authority reference and an exact target proposal reference, right
now.

## 3. Why no workflow state machine

`ProposalRecord`/`DDeployProposalV2Record` already carry everything needed
to answer "is this exact target proposal still pending, or has it already
been accepted, and if so what did it materialize" — `state:
ProposalLifecycleState`/`ProposalV2State` and `accepted_acceptance_id:
Option<String>`, cross-referenced against `DDeployAcceptanceV1`/
`DDeployAcceptanceV2`'s own `materialized_dmap_ref`. Nothing else about a
migration's progress (content installed, preparation started/finished,
operator abandoned, target healthy, cleanup finished, operationally
converged) has any persisted representation anywhere in the current
codebase. Inventing states for those facts would mean the "state machine"
lies the instant reality diverges from whatever a caller last reported —
exactly the failure mode Phase 119A's crash-recovery principle (§28) warns
against. This module instead always re-derives its answer fresh from
current state, which is naturally idempotent and naturally crash-safe with
zero persistence.

## 4. MigrationIntentV1

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationIntentV1 {
    pub project_id: String,
    pub application_id: String,
    pub source_dmap_ref: CanonicalContractReferenceV1,
    pub target_proposal_id: String,
    pub target_proposal_digest: String,
}
```

No `schema` field: this is deliberately not a canonical `datum.*` wire
contract (no `datum.migration/1` was created — Phase 119A §4/§19 and this
phase's own explicit prohibition). No `migration_id`: without persisted
workflow state, the exact `(source_dmap_ref, target_proposal_id)` pair is
already sufficient identity for the intent being evaluated — a synthetic ID
would have no consumer. No `created_at`, `status`, `policy`, health fields,
node IDs, or placements: those belong to the target proposal itself (never
duplicated) or to future, evidenced needs, not fabricated now. `Debug`,
`Clone`, `PartialEq`, `Eq` are derived purely for internal/test ergonomics —
`Serialize`/`Deserialize` were deliberately NOT derived, matching
`PlacementAuthorityView`'s own precedent (Phase 117/118): this type is never
sent over the wire and never persisted, so there is nothing to serialize.

## 5. Source A exact reference

`source_dmap_ref: CanonicalContractReferenceV1` reuses the existing
`{id, revision, digest}` type verbatim — no new struct was introduced. It is
populated from `PlacementAuthorityView.{dmap_id, revision, digest}`, exactly
as Phase 119A §5/§6 specified.

## 6. Target B exact proposal reference

`target_proposal_id`/`target_proposal_digest`: two plain `String` fields,
not `CanonicalContractReferenceV1`, because a D-Deploy proposal has no
`revision` field of its own (only a materialized D-Map does, and only after
acceptance). No placements, D-Code bindings, D-Forward ref, D-IoT
placements, D-Call realizations, or external port bindings are duplicated —
all of that remains owned entirely by the target proposal record itself,
looked up by `target_proposal_id` when needed.

## 7. Source-exactness vs. DDeploy revision CAS

These answer two different questions and are not in tension:

- **`intent.source_dmap_ref` (exact `id`+`revision`+`digest`) answers**:
  "which exact authority did this migration intend to leave?" — checked by
  this module's own preflight comparison against the live
  `PlacementAuthorityView` (`current_source != intent.source_dmap_ref` →
  `SourceStale`).
- **The target proposal's own `expected_active_dmap_revision` (existing,
  unmodified DDeploy field, checked inside `accept_proposal`/
  `accept_with_previous`) answers**: "did any acceptance happen after this
  proposal was created?" — the actual transactional race guard, unchanged
  by this phase.

This module never adds `expected_active_dmap_digest`, a migration-specific
CAS database, locks, or a second acceptance guard. It only additionally
checks, as a pure preflight fact (`TargetInvalidForSource`), that the
target proposal's own `expected_active_dmap_revision` (captured
automatically by `create_proposal` at proposal-creation time, from
`state.active_for(project_id, application_id)`) actually equals
`intent.source_dmap_ref.revision` — i.e., that the intent and its own named
target proposal were constructed consistently with each other, before ever
attempting acceptance.

## 8. V1/V2 target normalization

`locate_target_proposal` checks `state.proposals` (v1) first, then
`state.phase113_v2.proposals` (v2), and normalizes either hit into one
private `NormalizedTargetProposal { project_id, application_id,
stored_digest, recomputed_digest, expected_active_dmap_revision, accepted,
accepted_acceptance_id }`. `evaluate_migration_authority_relation`'s
decision tree runs exactly once, against this one normalized shape,
regardless of which D-Map generation the target proposal belongs to — no
separate v1/v2 migration semantics exist. `lookup_materialized_dmap_ref`
performs the equivalent normalization on the acceptance side, converting
`ContractRefV2` into `CanonicalContractReferenceV1` field-by-field (both are
already field-for-field identical `{id, revision, digest}` shapes).

## 9. Validation contract

`validate_intent_shape` checks V1–V5 (non-empty `project_id`/
`application_id`/`source_dmap_ref.id`, canonical `sha256:`-format digests
for `source_dmap_ref.digest` and `target_proposal_digest`, non-empty
`target_proposal_id`). `evaluate_migration_authority_relation` then checks
V6 (target proposal exists), V7 (recomputed digest matches both the
proposal's own stored digest AND the intent's asserted digest — reusing
`ddeploy::compute_proposal_digest`/`DDeployStateV2::proposal_digest`
verbatim, never a reimplemented digest algorithm), V8/V9 (target
project/application match intent), V15 (target's own
`expected_active_dmap_revision` equals the intent's source revision), and
V10–V14 via the existing `active_placement_authority` call (scoped
correctly by construction, since it is called with `intent.project_id`/
`intent.application_id`). V16 (target must be `Pending` for a pre-cutover
intent) is structurally enforced by the branch order itself — see §10. V17
(v1/v2 parity) is enforced by construction — see §8. This module never
re-runs D-Continuum eligibility, capacity, D-Code registry, ServiceArtifact,
D-Forward, or D-IoT validation — those remain exclusively DDeploy proposal/
acceptance responsibilities, untouched here.

## 10. Authority-relation derivation

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum MigrationAuthorityRelation {
    ReadyForAcceptance,
    NoActiveSource,
    SourceStale { current_source: CanonicalContractReferenceV1, expected_source: CanonicalContractReferenceV1 },
    TargetInvalidForSource { target_expected_active_dmap_revision: Option<u64>, source_revision: u64 },
    TargetAlreadyAccepted { materialized_dmap_ref: CanonicalContractReferenceV1 },
}
```

Decision order, deliberately: (1) intent shape, (2) target proposal exists
and its digest is verified, (3) target scope matches, (4) **target
acceptance status is checked before source currency** — this ordering is
the single most important design decision in this module (§14).

## 11. Pending target semantics

If the target proposal is still `Pending`: the live
`active_placement_authority` result is compared against
`intent.source_dmap_ref`. An exact match plus a matching
`expected_active_dmap_revision` yields `ReadyForAcceptance`; a matching
active authority but a target whose own `expected_active_dmap_revision`
disagrees yields `TargetInvalidForSource`; no active authority at all yields
`NoActiveSource`; any other mismatch yields `SourceStale`.

## 12. Accepted target semantics

If the target proposal's own record says `Accepted`, this module looks up
its `accepted_acceptance_id` and that acceptance's `materialized_dmap_ref`,
and returns `TargetAlreadyAccepted { materialized_dmap_ref }` — **without
ever consulting the current active authority**. This is correct because
proposal acceptance records are immutable in this system (confirmed
unchanged, Phase 111/113 design): once true, "this exact proposal was
accepted, and materialized this exact D-Map" never stops being true, even
if a later proposal has since superseded it as the *current* authority.
Whether `materialized_dmap_ref` is *still* the current active authority is
a distinct, additional fact this relation does not itself assert — Phase
119A §32 keeps "cutover succeeded" and "still current"/"operationally
converged" as separate milestones, and this module reflects only the
former.

## 13. Superseded-source semantics

If the target proposal is `Pending` but the live active authority no longer
matches `intent.source_dmap_ref`, the relation is `SourceStale { current_source,
expected_source }`. Because acceptance status is checked first (§12), this
relation is only ever reached when the target proposal itself was **not**
the cause of the change — i.e., some other, unrelated proposal won. Test
`t15_unrelated_proposal_c_winning_is_source_stale_not_target_accepted`
proves exactly this distinction directly.

## 14. Crash-after-acceptance behavior

Because target-acceptance status is checked strictly before source
currency, re-evaluating the exact same `MigrationIntentV1` object after an
unrelated process crash — with zero surviving workflow state, only the
intent value itself and current `DDeployStateV1` — correctly returns
`TargetAlreadyAccepted` if (and only if) this exact target proposal won,
even though `intent.source_dmap_ref` (captured before cutover) no longer
matches the live active authority at all. Test
`t14_crash_after_acceptance_recognizes_b_as_winner_even_though_a_is_stale`
proves this directly: the intent is captured while A is current, B is then
accepted (simulating a crash occurring the instant after), and re-evaluating
the untouched intent value still correctly reports `TargetAlreadyAccepted`,
never `SourceStale`.

## 15. D-Engine non-effect

Test `t26_and_t28_pending_b_cannot_alter_dengine_membership_and_evaluation_is_read_only`
directly re-derives D-Engine membership (via the unmodified Phase 118
`ddeploy::derive_active_dengine_nodes`) both before and after evaluating a
fully valid, `ReadyForAcceptance` migration intent, and asserts it is
unchanged (`["node-a"]` in both cases) — a pending, structurally valid
target proposal has zero effect on D-Engine, exactly as Phase 118 already
guarantees and as this module never attempts to touch.

## 16. DCode/DCall/DMonitor boundaries

`evaluate_migration_authority_relation` imports no `dcode_registry`,
`dcode_authorization`, `dcall`, or D-Monitor (`dmonitor_*`) type anywhere —
confirmed both by the module's own `use` statements (§structural fact, not
merely asserted) and by test `t25_evaluation_touches_no_dmonitor_type`. Its
only inputs are `&DDeployStateV1` and `&MigrationIntentV1`; there is nothing
D-Code/D-Call/D-Monitor-shaped for it to read even if it wanted to.

## 17. Phase120 interface

Phase 120 can later produce an ordinary target D-Deploy proposal through the
existing, unmodified `create_proposal`/`DDeployStateV2::create_proposal`
path (using whatever deterministic placement logic it implements), and then
supply only `intent.source_dmap_ref` (from `active_placement_authority`) and
the resulting `proposal_id`/`proposal_digest` to
`evaluate_migration_authority_relation`. Nothing in this module inspects
*why* a target proposal contains the placements it does — confirmed by
`evaluate_migration_authority_relation`'s own body, which reads only
`target.project_id`/`application_id`/digest/`expected_active_dmap_revision`/
acceptance status, never `target.placements` or any D-Code/D-Forward/D-IoT
content. Phase 120 needs no awareness of D-Monitor, runtime state,
reconciliation, migration internal workflow, or host preparation to use this
interface.

## 18. Tests

22 focused tests in `dserver/src/core/ddeploy_migration.rs`'s own `tests`
submodule, built against real production types (`DDeployStateV1`,
`CreateProposalInput`, `CanonicalDGraphV1`, `CanonicalDContinuumV1`,
`ServiceArtifact`, etc. — no shadow structs), covering: `ReadyForAcceptance`
(T1), `NoActiveSource` (T2), intent project/application mismatch (T3/T4),
source id/revision/digest mismatch → `SourceStale` (T5/T6/T7), unknown
target proposal (T8), target digest mismatch (T9), target belonging to
another project/application (T10/T11), target's own
`expected_active_dmap_revision` mismatch → `TargetInvalidForSource` (T12),
target already accepted (T13), crash-after-acceptance (T14), unrelated
proposal superseding source (T15/T21), idempotent re-evaluation (T16),
unrelated state content cannot affect the result (T23), structurally
malformed intents (T24), no D-Monitor dependency (T25), and read-only/
D-Engine non-effect (T26/T28/T29 combined into one test). All 22 pass.

## 19. Deferred work

Explicitly NOT built in this phase, per its own scope: any persisted
`MigrationRecord`, any HTTP API, any content/byte preparation trigger, any
host mutation, any target-selection/scoring logic, any migration ID, any
canonical wire schema, and any change to `accept_proposal`/
`accept_with_previous`/`active_placement_authority` (all confirmed
unmodified — only one new call site, `derive_active_dengine_nodes` +
`active_placement_authority`, both read-only, was added anywhere in
existing files). A future 119C may wrap this core in real persistence and
orchestration once a concrete, evidenced need for surviving a multi-minute,
restart-spanning migration attempt is demonstrated — not before.

## 20. Closure assessment

Every closure criterion (C1–C25) in this phase's own task is satisfied: one
small non-authoritative intent type exists (C1) that references rather than
duplicates source authority (C2) and target proposal (C3); both v1/v2 target
paths are covered by one shared decision tree (C4); exact source
id/revision/digest mismatches fail preflight (C5) as `SourceStale`; target
digest mismatch fails (C6) as an error; project/application mismatch fails
(C7); target `expected_active_dmap_revision` must match source revision
(C8); pending B against current A is identified deterministically (C9) as
`ReadyForAcceptance`; already-accepted B is identified deterministically
(C10) as `TargetAlreadyAccepted`; an unrelated proposal C winning is
distinguished from B's own acceptance (C11); crash-after-acceptance
re-evaluation recognizes B as winner (C12); no workflow state was invented
(C13); no `MigrationRecord` is persisted (C14); no migration API was added
(C15); no D-Map/D-Deploy mutation occurs (C16); no D-Monitor dependency
exists (C17); no target-selection logic exists (C18); no host mutation
exists (C19); pending B cannot alter D-Engine membership (C20); the existing
D-Deploy CAS is reused unchanged (C21); the source-exactness-vs-revision-CAS
distinction is documented accurately (C22, §7 above); Phase 120 can supply a
normal target proposal cleanly (C23, §17 above); focused (22/22) and full
crate (1893/1893) tests pass (C24); `git diff --check` passes cleanly (C25).
