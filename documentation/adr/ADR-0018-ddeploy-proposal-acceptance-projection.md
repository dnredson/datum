# ADR-0018 — D-Deploy proposal / explicit acceptance / D-Map -> OperationalDGraph projection (Stage 111)

- Status: Accepted and implemented (Stage 111, corrected by independent review v2, v3, and v4).
- Decision refs: ADR-0002 (D-Map single placement carrier -- the central
  invariant this ADR operationalizes), ADR-0006 (canonical digest, JCS+SHA-256),
  ADR-0007 (no new shared model crate, reaffirmed), ADR-0009 (`application_id`
  distinct from `project_id`), ADR-0011/ADR-0012/ADR-0013 (existing execution
  authorization machinery, left untouched), ADR-0015 (`RuntimeKind`, untouched),
  ADR-0017 (Stage 110 canonical `datum.dgraph/1`/`datum.dcontinuum/1`/
  `datum.dmap/1` -- reused verbatim, never redesigned).
- Scope: the D-Deploy proposal/validation/acceptance lifecycle, canonical
  `datum.dmap/1` materialization, the `target_nodes` placement-authority
  transition, and the derived `OperationalDGraph` projection this workflow
  produces. **Not** Stage 112 (no placement algorithm), **not** Stage 113 (no
  live telemetry), **not** a canonical D-Code/artifact registry redesign.

## Independent review v2 corrections (read this first)

The v1 version of this ADR (and the code it described) contained five real
defects, found by an independent review that traced the ACTUAL execution
paths rather than only the persisted `DDeployStateV1` object. **The v1
architectural decisions listed in "Non-goals" and throughout this document
remain correct and were NOT redesigned** -- these are implementation
corrections to how those decisions actually reach code, not a change of
decision. Nothing in this section is deleted or hidden; earlier sections of
this ADR describe the CORRECTED behavior directly (they were themselves
edited where the correction changes what is true), and this section
records what was wrong and why.

1. **v1 changed the effective `OperationalDGraph` authority but left the
   real artifact-reconciliation/authorization resolver on legacy
   `target_nodes` semantics unconditionally.** `resolve_assignments_from_registry`
   (`dserver/src/core/artifact_reconciliation_context.rs`) and its two real
   callers (`get_operational_reconciliation_context`,
   `authorize_operational_reconciliation`) always required
   `artifact.control_plane.target_nodes` to contain the current node --
   correct for a legacy project, but WRONG once a canonical D-Map is
   authoritative: a later `target_nodes` edit could make an
   already-accepted service silently fail to resolve on its own accepted
   node, even though `DDeployStateV1.active_by_application` still correctly
   recorded the placement. **Fix:** an explicit `PlacementAuthority` enum
   (`LegacyTargetNodes` / `CanonicalDMap`) now threads through
   `resolve_assignments_from_registry_with_authority`/
   `build_artifact_reconciliation_context_with_authority`; both real
   callers select the authority from the SAME `effective_canonical_operational_graph`
   decision already used for the graph itself, so a canonical-authority-active
   project's artifact resolution never re-checks `target_nodes` as current
   placement truth, while a legacy project's behavior is byte-for-byte
   unchanged. A repository-wide grep performed while fixing this ALSO found
   THREE MORE production call sites of the same authority-blind resolver
   that the review's own "critical callers" list had not named --
   `api::governed_deployment_preflight::get_governed_deployment_preflight`,
   `api::secret_preflight::get_secret_preflight`, and
   `api::secret_resolver_contract::get_secret_resolver_contract` -- fixed
   identically and autonomously, without waiting to be asked. See "The
   `target_nodes` placement-authority transition" and the review package's
   `real-artifact-authority-audit.txt`/`legacy-endpoint-isolation-audit-v2.txt`
   for the full, traced call path and the mandatory end-to-end regression
   test (`canonical_authority_reaches_the_real_artifact_resolution_path`,
   `artifact_reconciliation_context.rs`).

2. **Acceptance-time artifact-eligibility revalidation used a
   point-in-time cloned snapshot, leaving a TOCTOU window before the
   commit.** `post_accept_proposal` (`dserver/src/api/ddeploy.rs`) used to
   call `ServiceArtifactRegistry::list` (clone, release the lock), THEN
   `accept_proposal`, THEN `persist_state` -- a concurrent artifact write
   could commit strictly between the clone and the persisted acceptance,
   so the accepted result could reflect eligibility that was already stale
   at the moment it became durable. **Fix:** the acceptance-critical
   sequence (resolve artifacts, `accept_proposal`, `persist_state`) now
   runs entirely inside ONE `ServiceArtifactRegistry::with_stable_view`
   call, holding the SAME read-lock mechanism Stage 107A's own
   `authorize_operational_reconciliation` already relies on, for the exact
   same reason. Lock order is `DDEPLOY_TRANSACTION` -> registry stable view
   -> SQLite persistence -- audited against the rest of the codebase for a
   reverse acquisition (none found). See "Acceptance is one atomic
   transaction" and `acceptance-artifact-toctou-audit.txt`.

3. **Acceptance trusted `ProposalRecord.proposal_digest` without
   recomputing it from the actual persisted `proposal` field.** A
   hand-corrupted or buggily-mutated persisted proposal (e.g. `placements`
   changed after creation, without updating `proposal_digest`) could pass
   every existing check -- including a caller-supplied
   `expected_proposal_digest` that happened to equal the STALE recorded
   value -- and be accepted with the mutated content. **Fix:**
   `accept_proposal` now recomputes `compute_proposal_digest(&record.proposal)`
   and requires it to equal the recorded `proposal_digest` BEFORE any other
   check, including the idempotent-replay branch -- an already-Accepted
   proposal record must also pass this check before it can be replayed.
   See "D-Map revision ownership, CAS, and idempotency" and
   `proposal-integrity-audit.txt`.

4. **Idempotent replay of an already-accepted proposal could pair that
   proposal's OWN acceptance record with a DIFFERENT, more-recently-accepted
   proposal's active D-Map**, because the replay branch read
   `state.active_by_application` (the CURRENT active entry for the
   application) instead of reconstructing the REPLAYED proposal's own
   historical materialization. Replaying proposal P1 after proposal P2 had
   superseded it could return `{acceptance: P1's acceptance, active_dmap:
   P2's current D-Map}` -- neither P1's nor P2's own consistent result.
   **Fix:** replay now deterministically reconstructs the replayed
   proposal's OWN materialized D-Map via `build_candidate_dmap` (pure,
   using only that proposal's own immutable content plus its own
   acceptance's recorded `materialized_dmap_ref`), verifies it against that
   ref's digest, and reports an explicit `currently_active: bool` alongside
   the renamed `materialized_dmap` field (previously the misleading
   `active_dmap`) so a caller can tell "this proposal's own historical
   result" apart from "the current authoritative state" (the latter
   remains `GET /api/v1/ddeploy/active`'s job). See "D-Map revision
   ownership, CAS, and idempotency" and `historical-idempotency-audit.txt`.

5. **The v1 review ZIP was not actually standalone-reproducible.** Its
   Python checker/selftest resolve fixtures via
   `Path(__file__).resolve().parents[1] / "scripts" / "fixtures" / ...`,
   which only works when the checker sits at `scripts/check_ddeploy_contracts_v1.py`
   relative to a repository root -- the v1 package instead placed it at
   `checkers/check_ddeploy_contracts_v1.py` with fixtures at
   `fixtures/ddeploy-v1/...`, so extracting the ZIP alone and running the
   checker failed. **Fix:** the v2 review package
   (`stage111-ddeploy-acceptance-projection-review-v2-final.zip`) uses
   repository-relative paths throughout (`scripts/check_ddeploy_contracts_v1.py`,
   `scripts/fixtures/datum-contracts/ddeploy-v1/...`) and was verified by
   extracting it to a fresh temporary directory and running both scripts
   from there -- see `package-reproducibility-audit.txt` for the exact
   commands and output.

Two further, smaller findings from the same review pass, fixed alongside
the above:

- **Identifier-composition audit:** `canonical_dmap_id`/`application_key`
  previously joined `project_id`/`application_id` with a plain
  `":"`-separated (or, for `application_key`, a non-printable-separator)
  concatenation. Since these are arbitrary caller-supplied strings only
  checked non-empty (never restricted from containing `:` -- and this
  repository's OWN real fixtures already use colon-containing application
  ids, e.g. `"app:smart-sentinel-lab"`, ADR-0017), two DIFFERENT
  `(project_id, application_id)` pairs could genuinely produce the SAME
  composite string (e.g. `("p", "a:b")` and `("p:a", "b")` both joined to
  `"dmap:p:a:b"`). Both functions now use a length-prefixed encoding
  (`"<len>:<value>"` per component, concatenated with no separator between
  components), which is unambiguous regardless of what characters either
  component contains. `dmap_id` is DServer-owned and opaque (never required
  to be human-parseable), so this change has no external compatibility
  impact; a repository-wide grep confirmed no other code depended on the
  old string shape. See `identifier_composition_is_collision_free_across_many_adversarial_pairs`
  and `naive_colon_join_would_have_collided_but_length_prefixed_encoding_does_not`
  (`dserver/src/core/ddeploy.rs`).
- **Persisted-state integrity audit:** `load_state` now runs a bounded,
  fail-closed consistency check (`validate_persisted_state_integrity`) over
  a successfully-deserialized `DDeployStateV1` blob before returning it --
  proposal digests, acceptance-implies-record, materialized-D-Map digests,
  and active-application-map-key correspondence are all independently
  re-verified. This is deliberately NOT a migration or repair framework:
  any inconsistency fails the load outright. See "Persistence, restart, and
  versioning" and `persisted-state-integrity-audit.txt`.

Two audited-and-concluded (not code-changed) questions, per the review's
explicit request that these be resolved with evidence rather than
overengineered:

- **Single-process CAS scope:** `DDEPLOY_TRANSACTION` is a process-local
  `std::sync::Mutex`, exactly like `api::operational_dgraph`'s pre-existing
  `state_transaction()`. **Stage 111 v0.1, like the rest of this
  repository's control-plane state (the in-memory `OperationalDGraph`
  store, the `ServiceArtifactRegistry` in-memory cache), assumes exactly
  ONE active DServer controller process writing to a given control-state
  SQLite database at a time.** No multi-writer-process deployment model
  exists anywhere in this codebase today; introducing one is out of scope
  for Stage 111 and would require a SQLite-transactional CAS, not merely
  documentation. See `atomic-acceptance-audit-v2.txt`.
- **Legacy write-guard race (`put_operational_dgraph` vs. concurrent
  acceptance):** `put_operational_dgraph` checks `canonical_authority_active`
  before entering its own `state_transaction()`, which is a DIFFERENT lock
  from Stage 111's `DDEPLOY_TRANSACTION`; in principle a legacy PUT could
  observe "no canonical authority" and then a concurrent acceptance could
  activate authority before the PUT's write lands. **Concluded harmless,
  not fixed with additional cross-module locking:** every read path
  (`operational_dgraph_snapshot`/`graph_and_authorizations_snapshot`)
  checks canonical authority FIRST and only falls back to the legacy store
  when authority is NOT active; once active, canonical authority is never
  revoked in Stage 111 (no such endpoint exists), so a legacy write that
  slips through this race becomes permanently dormant, unread storage --
  never a placement-authority violation, never observed by any consumer.
  See `legacy-endpoint-isolation-audit-v2.txt` for the full trace proving
  no consumer is affected.

## Independent review v3 corrections (read this second)

Independent review v3 traced the v2 implementation further and found one
more P0 defect (a second, independently-persisted placement carrier) and
one more P1 defect (persisted-state integrity checks were per-record, not
relationally closed across records), plus a related acceptance-time gap
(no pre-commit validation of the derived operational projection against
the existing `OperationalDGraph` contract). **No v1 or v2 decision was
reversed** -- both fixes are, again, implementation corrections, not
redesigns.

1. **v2 stored a derived, node-keyed operational cache
   (`desired_services_by_node: BTreeMap<node_id, Vec<OperationalDesiredService>>`)
   inside `ActiveApplicationDeployment`, and `union_project_operational_graph`
   read NODE ASSIGNMENT from that cache's own keys, not from
   `active_dmap.placements`.** This created a SECOND, independently-persisted
   placement carrier alongside the canonical `active_dmap` -- both were
   populated consistently by the normal acceptance path, but the v2
   persisted-state integrity validator never checked they stayed in
   agreement, so a corrupted/hand-edited `desired_services_by_node` could
   silently report a service on a DIFFERENT node than the authoritative
   D-Map, violating ADR-0002's "D-Map is the SINGLE authoritative placement
   carrier." **Fix:** `ActiveApplicationDeployment.operational_services` is
   now `BTreeMap<service_id, OperationalDesiredService>` -- keyed by
   `service_id`, and `OperationalDesiredService` itself has no node-bearing
   field at all. `union_project_operational_graph` derives EVERY node
   assignment by iterating `active_dmap.placements` directly and using
   `operational_services` ONLY to look up the runtime/desired_state payload
   for a service the D-Map has already placed; a service present in
   `active_dmap.placements` but absent from `operational_services` is now a
   fail-closed finding (`operational_projection_invalid`), never a silent
   omission. See "D-Map -> `OperationalDGraph` projection" below and the
   review package's `single-placement-authority-audit.txt`.

2. **Persisted-state integrity (the v2 bounded audit) checked each
   proposal/acceptance/active record against ITSELF (its own recomputed
   digest) but never checked the RELATIONSHIPS between records.** Two
   independently self-consistent objects -- an `active_dmap` that correctly
   recomputes to its own recorded `dmap_digest` -- could still jointly lie
   about which accepted proposal actually produced them (e.g. a tampered
   `active_dmap` with a matching, freshly-recomputed `dmap_digest`, but
   never actually materialized by the proposal/acceptance it claims to
   reference). **Fix:** `validate_persisted_state_integrity` is now a full
   relational closure check across five categories (proposal records,
   acceptance records, active application deployments, the operational
   service snapshot, and project operational revision bookkeeping) --
   every map key, cross-reference, and reconstructed historical value is
   checked against every other relevant record, not merely against itself.
   The check that actually catches the example above: every active
   application's `active_dmap` (id/revision/digest) must match its OWN
   referenced acceptance's recorded `materialized_dmap_ref` -- a tampered
   `active_dmap` re-digested to be self-consistent with itself still
   disagrees with the UNTOUCHED, originally-recorded `materialized_dmap_ref`
   on the acceptance it claims to belong to. A SEPARATE, additional check
   goes one step further: the historically-reconstructed D-Map (built ONLY
   from the referenced proposal's immutable content plus the referenced
   acceptance's own recorded ref) must be EXACTLY, STRUCTURALLY equal to
   the persisted `active_dmap`. Independent review v4 correction: this
   second check is, by construction, logically implied by the first check
   together with category B's own reconstruction-vs-digest check (both are
   digest comparisons; SHA-256 collision resistance means two different
   values sharing both digests is not constructible) -- it is kept as
   defense-in-depth against a future change that might weaken one check
   without weakening the other, not because any currently-known corruption
   shape reaches it as the FIRST failure. `self_consistent_tamper_is_still_detected`
   (`dserver/src/core/ddeploy.rs`) asserts on the ACTUAL error text to
   verify it is the first (acceptance-cross-reference) check that fires,
   correcting a v3 test-description error that had credited the second
   check instead. A dangling `acceptance.proposal_id`
   (tolerated in v2 as "not fatal, proposals are never deleted so this can't
   happen from normal code") is now treated as corruption and fails closed,
   per this review's correction that a reference type this module itself
   guarantees stable should never be silently tolerated as danglable. See
   "Persistence, restart, and versioning" below and the review package's
   `persisted-relational-integrity-audit.txt`.

3. **Acceptance never validated the derived operational projection against
   the existing `OperationalDGraph` contract before committing.** Stage
   110's `CanonicalDGraphV1` legally permits zero services (ADR-0017 places
   no minimum), so a structurally valid D-Graph/D-Map pair with zero
   services/placements could be accepted, producing a candidate
   `OperationalDGraph` with zero nodes at a non-bootstrap revision -- a
   shape the PRE-EXISTING `validate_operational_dgraph` (used by every
   OTHER path that publishes an operational graph) already rejects. Without
   checking this at acceptance time, Stage 111 could activate canonical
   state that its own downstream `OperationalDGraph` consumers could not
   represent. **Fix:** `accept_proposal` now builds the CANDIDATE
   project-wide projection (via `union_project_operational_graph`, which
   also performs cross-application collision detection) and runs
   `validate_operational_dgraph` against it BEFORE any state mutation;
   failure returns `AcceptError::Invalid` with the stable finding
   `operational_projection_invalid` and mutates nothing. This is a Stage
   111 BRIDGE validation rule -- it does NOT change Stage 110's own
   canonical D-Graph contract (an empty-service D-Graph remains legal at
   that layer); it only means such an application cannot currently be
   ACTIVATED through this v0.1 operational bridge. See "D-Map ->
   `OperationalDGraph` projection" below and
   `operational-projection-validity-audit.txt`.

## Independent review v4 corrections (read this third)

Independent review v4 found that v3's persisted-state integrity audit,
while a genuine improvement, still had two gaps -- both about what happens
on RELOAD, never about the real-time acceptance path -- plus several
executed-test-versus-claimed-test discrepancies in the v3 review package's
own documentation. **No v1/v2/v3 decision was reversed.**

1. **Reload never reconstructed and validated the project operational
   union (R1).** v3's `validate_persisted_state_integrity` checked every
   proposal/acceptance/active record's relationships to every OTHER
   record, but never reconstructed the actual project-wide
   `OperationalDGraph` union and validated it -- so two INDIVIDUALLY
   self-consistent, correctly-cross-referenced accepted applications that
   jointly claim the same `service_id` would pass every per-record check
   yet produce an invalid union on read. **Fix:** `validate_persisted_state_integrity`
   now reconstructs `union_project_operational_graph` for every project
   with accepted history and runs `validate_operational_dgraph` against it,
   exactly mirroring the check `accept_proposal` already runs pre-commit.
   See "D-Map -> `OperationalDGraph` projection" above and
   `single-placement-authority-audit.txt`/`persisted-relational-integrity-audit.txt`.

2. **Relational closure derived its "which projects/applications matter"
   sets from `active_by_application`'s own key set, which a corrupted
   blob can simply not contain entries for.** Category E's project-revision
   check, and the (v3) absence of any "every accepted application has an
   active entry" check, both iterated `active_by_application` -- so
   DELETING an active entry (or restoring an OLDER one after a newer
   acceptance) could silently skip validation of exactly the thing that
   deletion/restoration corrupted. Concretely: removing a project's ONLY
   active entry made category E's project-loop iterate zero projects, and
   nothing else noticed the application's canonical authority had vanished;
   restoring an entry from an earlier acceptance (after a later one had
   superseded it) left every per-record check individually satisfied
   (the restored entry still matches ITS OWN real accepted history) while
   silently resurrecting stale placement. A related gap: an Accepted
   proposal only had to name SOME existing acceptance, never verified that
   THAT acceptance's own `proposal_id` pointed back to it -- so a falsely-
   Accepted proposal could claim another proposal's real acceptance.
   **Fix:** every project/application set this function iterates is now
   derived from `state.acceptances` (never deleted by this module), never
   from `active_by_application`. A new check requires every application
   with accepted history to have an active entry whose D-Map revision and
   acceptance_id match the LATEST accepted materialization for that
   application (ordered by the server-owned `materialized_dmap_ref.revision`,
   never by acceptance_id or any other caller-controlled/lexical label);
   an ambiguous history (two different acceptances claiming the same
   revision) is itself now a fail-closed finding. Category A's Accepted-
   proposal check is now bidirectional: the referenced acceptance's own
   `proposal_id` must equal the proposal being checked. See
   `persisted-relational-integrity-audit.txt`.

3. **The v3 review package's corruption-matrix audit substituted
   "symmetric"/"same code path"/"transitively" reasoning for several
   scenarios the original gate required as actual executed tests, and two
   EXISTING tests were miscategorized: they passed, but via an EARLIER,
   unrelated check, not the LATER check their names/descriptions claimed to
   exercise.** `proposal_dgraph_ref_mismatch_fails_closed_on_reload`
   mutated `proposal.dgraph_ref` without recomputing `proposal_digest`, so
   it actually tripped the proposal-digest guard, never the dgraph_ref-vs-
   snapshot cross-check its name claims.
   `self_consistent_tamper_is_still_detected` is caught by the acceptance-
   cross-reference check (`active_dmap` vs its own acceptance's
   `materialized_dmap_ref`), not by the separate historical-structural-
   equality check the v3 audit credited. **Fix:** the dgraph_ref test now
   ALSO recomputes `proposal_digest` after mutating the reference
   (explicitly documented as two deliberate changes), isolating the
   intended check; a SECOND, independent test mutates the SNAPSHOT instead
   or the reference, reaching the same check via a different corruption
   vector. Symmetric dcontinuum_ref/dcontinuum_snapshot tests were added.
   The self-consistent-tamper test now asserts on the actual error text to
   verify which check fires, and its comment/the ADR text above were
   corrected. Every remaining originally-requested corruption scenario now
   has a dedicated executed test asserting a specific, checked error
   reason (never merely `is_err()`) -- see
   `persisted-relational-integrity-audit.txt`'s case-to-test mapping and
   `source-vs-audit-closure-v4.txt` for the verification method used to
   catch these two discrepancies.

4. **Documentation precision.** An earlier version of this ADR said "an
   empty application is rejected," overstating the rule -- acceptance
   validates the PROJECT-WIDE UNION, and an empty application is accepted
   when a sibling application already supplies nodes to that union; it is
   rejected only when it is the project's sole active application. See
   "D-Map -> `OperationalDGraph` projection" above for the corrected text
   and the paired regression tests. Separately: this ADR and its audits now
   describe the persisted-state integrity checks precisely as consistency
   of RETAINED accepted history against the specific tested corruption
   shapes -- never as protection against an arbitrary, wholesale rewrite of
   the underlying database (no such guarantee exists or is claimed; this
   module has no cryptographic signing or backup/restore mechanism).

## Why a proposal/acceptance lifecycle, not a direct D-Map write

Stage 110 (ADR-0017) gave `datum.dmap/1` a type and a validator, but nothing
in this repository yet decided WHO gets to construct one, WHEN it becomes
active, or how a second concurrent attempt is prevented from silently
clobbering the first. Stage 111's brief was explicit that this gap could not
be closed by simply exposing a `PUT` for `CanonicalDMapV1`: a caller-supplied
placement must be independently re-validated by the server, must never
activate on the same request that proposed it, and must be safe against two
callers racing to deploy the same application. The result is a two-phase
workflow with its own persisted lifecycle state, implemented in
`dserver/src/core/ddeploy.rs` (pure logic, 58 unit tests) and exposed via
`dserver/src/api/ddeploy.rs` (5 HTTP endpoints).

## Proposal lifecycle: propose -> validate -> pending -> accept -> materialize -> project

1. **Propose** (`POST /api/v1/ddeploy/proposals`): a caller submits a
   `CanonicalDGraphV1` snapshot, a `CanonicalDContinuumV1` snapshot, and a
   candidate `placements: Vec<CanonicalDServPlacementV1>`. DServer validates
   both snapshots structurally, **recomputes their digests itself**
   (`recompute_dgraph_digest`/`recompute_dcontinuum_digest`, Stage 110,
   never trusting a caller-supplied digest), and constructs the proposal's
   `dgraph_ref`/`dcontinuum_ref` from that recomputation. `create_proposal`
   (`dserver/src/core/ddeploy.rs:634`) then runs the full validator
   (`validate_full`) against the candidate placement and, only if it finds
   zero problems, inserts a `ProposalRecord` in state `Pending`.
2. **Validate, standalone** (`POST /api/v1/ddeploy/proposals/validate`): the
   identical `validate_full` logic, exposed as a pure preview with **no
   persistence** -- see "Validation is pure" below.
3. **Fetch** (`GET /api/v1/ddeploy/proposals/:proposal_id`): read-only,
   project-scoped (a proposal belonging to another project reports 404, not
   a differentiated mismatch code -- see "Cross-tenant proposal isolation"
   below).
4. **Accept** (`POST /api/v1/ddeploy/proposals/:proposal_id/accept`): the
   only place a canonical `datum.dmap/1` is ever constructed with an
   authoritative `dmap_id`/`revision` and made active. See "Acceptance is
   one atomic transaction" below.
5. **Read active** (`GET /api/v1/ddeploy/active`): "what is currently active
   for this project (optionally filtered to one application)" --
   project_id, application_id, the active D-Map, its digest, the originating
   proposal_id/acceptance_id, and the current operational-projection
   revision.

## A proposal is never a D-Map

`DDeployProposalV1` (schema `datum.ddeploy-proposal/1`,
`dserver/src/core/ddeploy.rs:108`) deliberately carries no `dmap_id` and no
`dmap_revision` field at all -- there is no field to accidentally treat as
authoritative. It never self-serializes with `"schema": "datum.dmap/1"`; no
function in this module sets a proposal's schema to anything but
`DDEPLOY_PROPOSAL_SCHEMA`. `build_candidate_dmap` does construct an
EPHEMERAL `CanonicalDMapV1` from a proposal during validation, but that value
is returned inside `ProposalValidationOutcome.candidate_dmap`, is never
inserted into `DDeployStateV1.active_by_application`, and is documented at
its call site as "never persisted or activated by this function." Only
`accept_proposal`'s single commit block (`dserver/src/core/ddeploy.rs:848-857`)
ever writes an entry into `active_by_application`.

## Explicit acceptance, and why `accepted_by` is an audit label, not authentication

`DDeployAcceptanceV1` (schema `datum.ddeploy-acceptance/1`) requires a
caller-supplied `accepted_by` string. Exactly like `ResolvedImageIdentityV1.resolved_by`
(Stage 107B-A) and per ADR-0013 §6, DServer has no application-layer
authentication in this prototype -- `accepted_by` is provenance for a human
reader of the acceptance record later, never a cryptographically verified or
authenticated operator identity. The API layer (`post_accept_proposal`)
rejects an empty/whitespace-only label with 400, but performs no identity
verification beyond that. Any future authentication layer would populate
this same field from a verified principal without changing its shape.

## D-Map revision ownership, CAS, and idempotency

DServer, never the caller, owns `dmap_id` (`canonical_dmap_id(project_id,
application_id)`, stable per `(project_id, application_id)`) and `revision`
(monotonically increasing PER APPLICATION: first acceptance = 1, each
subsequent accepted proposal for the same application = previous + 1).

**Stale-proposal compare-and-swap:** `create_proposal` snapshots
`expected_active_dmap_revision` from the CURRENT active revision at proposal
creation time (`None` for a first deployment). `accept_proposal`
(`dserver/src/core/ddeploy.rs:730`) re-checks this snapshot against the
revision active AT ACCEPTANCE TIME; a mismatch fails closed with
`AcceptError::Stale` (`F_STALE_ACTIVE_DMAP_REVISION`), mapped to HTTP 409.
The worked example from the brief is exactly what
`two_proposals_same_active_revision_first_succeeds_second_is_stale` proves:
proposals A and B are both created expecting revision 3; A is accepted first
and becomes revision 4; B's acceptance is rejected as stale rather than
silently becoming revision 5.

**Idempotent re-acceptance:** if `accept_proposal` is called again for a
proposal already in state `Accepted`, it replays the EXISTING
`AcceptOutcome` (same `acceptance_id`, same D-Map revision,
`idempotent_replay: true`) without touching `state` at all -- proven by
`accepting_same_proposal_twice_is_idempotent`. The API layer skips its
`persist_state` call entirely on an idempotent replay, since nothing
changed.

## Acceptance is one atomic transaction

`accept_proposal` mutates its `&mut DDeployStateV1` argument **in memory
only** -- every validation step (digest check, CAS, revalidation via
`validate_full`, digest recomputation, desired-services derivation,
cross-application collision check) runs against reads and clones; `state`'s
fields are written in exactly one block, at the very end, only after every
prior check has passed (`dserver/src/core/ddeploy.rs:848-857`). The function
itself never touches disk. The API layer (`post_accept_proposal`) is
responsible for exactly one `persist_state` call after a successful return,
writing the proposal lifecycle, the acceptance record, the active D-Map, and
the derived operational-projection revision counter as **one JSON blob**
under `DOMAIN_DDEPLOY_STATE` inside `persist_domain_json`'s single SQLite
transaction (`dserver/src/storage/control_state.rs`) -- there is no
multi-step "write A, then write B, then write C" sequence to observe half of.

**Crash-consistency proof:** `crash_during_acceptance_leaves_previous_active_deployment_unchanged`
accepts revision 1 and persists it, then accepts revision 2 IN MEMORY ONLY
(never calling `persist_state` for that mutation, simulating exactly the
failure mode a real disk-I/O error produces) and drops the in-memory copy.
Reloading state from disk shows the active D-Map is still revision 1, the
second proposal is still `Pending`, and the derived operational projection
still reflects revision 1's placements.

## The `target_nodes` placement-authority transition

Before Stage 111, `resolve_assignments_from_registry` (the pre-existing
artifact-reconciliation join) filters candidate assignments by
`artifact.control_plane.target_nodes.contains(node_id)` -- this is how a
node currently learns whether it may run a given service at all. Stage 111
does **not** rewrite this filter to something like `target_nodes = [chosen_node]`
(that would move placement truth back into `ServiceArtifact`, directly
violating ADR-0002). Instead:

- **Before an accepted D-Map exists** for a project, every legacy path
  (`operational_dgraph_snapshot` with no canonical authority active,
  `authorize_operational_reconciliation`, `resolve_assignments_from_registry`
  itself) uses `target_nodes` exactly as it always has.
- **`target_nodes` is ELIGIBILITY, checked only at proposal-validation and
  acceptance time** (`check_node_eligibility`, `dserver/src/core/ddeploy.rs:393`),
  mirroring the existing empty-list-means-no-eligible-node semantics
  exactly. It answers "may this service be placed on this node," never
  "where is this service currently placed."
- **After acceptance, the accepted `CanonicalDMapV1` is placement authority
  and `target_nodes` is never consulted again for CURRENT placement --
  including on the REAL artifact-reconciliation/authorization path, not
  merely inside the persisted `DDeployStateV1` object (independent review
  v2, correction 1).** `resolve_assignments_from_registry_with_authority`
  (`dserver/src/core/artifact_reconciliation_context.rs`) takes an explicit
  `PlacementAuthority` parameter; `get_operational_reconciliation_context`
  (`dserver/src/api/artifact_reconciliation.rs`) and
  `authorize_operational_reconciliation` (`dserver/src/api/operational_dgraph.rs`)
  both derive it from the SAME `effective_canonical_operational_graph`
  decision already used to select the graph itself, and pass
  `PlacementAuthority::CanonicalDMap` whenever canonical authority is
  active -- under that mode, `target_nodes` is not read as current
  placement truth AT ALL, so a later edit to it cannot silently relocate
  (or stop resolving) an already-accepted service. `mutating_target_nodes_after_acceptance_does_not_move_active_dmap`
  and `a_new_proposal_targeting_a_now_ineligible_node_is_rejected`
  (`dserver/src/core/ddeploy.rs`) prove the persisted-state half of this
  snapshot semantics; `canonical_authority_reaches_the_real_artifact_resolution_path`
  (`dserver/src/core/artifact_reconciliation_context.rs`) is the mandatory
  end-to-end proof that the REAL resolution path behaves identically.
- **This module never writes `target_nodes`.** `stage111_never_writes_target_nodes_equal_to_accepted_node`
  is a source-level regression test asserting no code path in this file
  assigns into `control_plane.target_nodes` at all -- the transition is
  achieved entirely by WHEN `target_nodes` is read (proposal/acceptance
  time only), never by rewriting it to reflect a decision.

## `target_stages`: fail-closed, not silently ignored

**Audit conducted before implementing any behavior, per the brief's explicit
requirement.** `datum.dcontinuum/1` (Stage 110, ADR-0017) has no D-Stage
concept at all -- no field on `CanonicalDNodeV1` names a stage.
`dserver::core::capabilities::NodeCapabilities.stage` exists as a
self-reported, Firebase-backed field defaulting to `"unknown"` for any node
that never explicitly reported one; it is neither authoritative nor
guaranteed complete for every candidate node. `resolve_assignments_from_registry`
itself never enforces `target_stages` today -- it is a declared field with
no consuming code. Every real, checked-in `ServiceArtifact` fixture in this
repository declares `target_stages: []`.

**Conclusion:** Stage 111 does not extend `datum.dcontinuum/1` with D-Stage,
and does not invent a mapping from `NodeCapabilities.stage` to eligibility.
`check_target_stages` (`dserver/src/core/ddeploy.rs:416`) fails closed with
`target_stage_constraint_unverifiable` for ANY non-empty `target_stages` on
a resolved artifact, rather than silently ignoring the constraint. If a
future stage introduces an authoritative D-Stage source, this is the single
call site that would be extended to actually verify it.

## D-Map -> `OperationalDGraph` projection

**Independent review v3, single-persisted-placement-authority fix.**
`derive_application_operational_services` (`dserver/src/core/ddeploy.rs`,
formerly `derive_application_desired_services`) turns an already-VALID
accepted `CanonicalDMapV1` into a PLACEMENT-INDEPENDENT operational
semantics map, `BTreeMap<service_id, OperationalDesiredService>`: for each
placement, it resolves the service's `ServiceArtifact` (the same join
`validate_full` already proved unambiguous) and constructs an
`OperationalDesiredService { service_id, runtime: artifact.runtime.kind,
desired_state: "running" }` -- keyed by `service_id`, carrying NO node
identity anywhere. **The operational `runtime` value comes from the
transitional `ServiceArtifact` binding, never from the canonical
D-Graph** -- `CanonicalDServV1` has no runtime field at all (ADR-0004: the
D-Serv execution contract is host-independent); "how" a service actually
executes remains an operational-plane fact, exactly as ADR-0015 already
established.

This computation happens **once, at acceptance time**, and is stored
verbatim in `ActiveApplicationDeployment.operational_services` -- it is
never re-resolved live when a projection is later read. This mirrors how the
pre-existing legacy `OperationalDGraph.runtime` field already behaves (a
value supplied once by its publisher, not dynamically re-joined on every
read): a later, unrelated content edit to the same artifact (its image tag,
its execution generation) cannot silently change what an already-accepted
deployment reports as its desired runtime. A NEW accepted proposal is
required to pick up new artifact content for placement purposes -- restated
below under "Post-acceptance artifact changes."

**Node placement is read EXCLUSIVELY from `active_dmap.placements` at
projection time -- there is no second, independently-persisted
service->node index anywhere in this module (independent review v3
correction; v2 had stored one, `desired_services_by_node`, keyed by
`node_id`, which duplicated placement authority).**
`union_project_operational_graph` iterates each active application's OWN
`active_dmap.placements` directly and uses `operational_services` ONLY to
fetch the runtime/desired_state payload for a service the D-Map has
already placed; a placement whose service is missing from
`operational_services` is a fail-closed finding
(`operational_projection_invalid`), never a silently-dropped entry. This
holds both at acceptance time (the candidate projection is built and
validated before any commit -- see below) and at read/reload time (the
SAME function is used by `active_operational_projection` and by the
persisted-state integrity audit's category-D checks).
`active_application_deployment_carries_no_persisted_node_placement_outside_active_dmap`
and `moving_a_service_via_new_dmap_is_the_only_way_node_placement_changes`
(`dserver/src/core/ddeploy.rs`) are the structural and functional proofs.

**Acceptance validates the derived PROJECT-WIDE projection before
committing (independent review v3; precision-corrected in v4).**
`accept_proposal` builds the candidate project-wide `OperationalDGraph` via
`union_project_operational_graph` -- the UNION of every active application
in the project, not the accepting application in isolation -- and runs the
PRE-EXISTING `core::operational_dgraph::validate_operational_dgraph`
against that union before any state mutation. Only when the UNION itself
would have zero nodes at a non-bootstrap revision (exactly the shape that
validator already rejects for every OTHER operational-graph-publishing
path) does acceptance fail with `AcceptError::Invalid` carrying the stable
finding `operational_projection_invalid`, mutating nothing.

**Independent review v4 documentation-precision correction:** an earlier
draft of this section said "an empty application is rejected," which
overstates the rule -- the check is a property of the UNION, not of any one
application. A structurally-legal-at-Stage-110 but zero-service/zero-
placement application is rejected ONLY when it is the project's sole
active application (the union is then necessarily empty too); the SAME
empty application is ACCEPTED when a sibling application already active in
the same project supplies at least one node, because the union it
contributes to is then non-empty. This is a Stage 111 BRIDGE limitation,
not a change to Stage 110's own canonical D-Graph contract (an empty-
service D-Graph remains legal there in isolation).
`empty_application_in_an_otherwise_empty_project_is_rejected_as_operational_projection_invalid`
and `empty_application_with_a_sibling_application_supplying_nodes_is_accepted`
(dserver/src/core/ddeploy.rs) are the paired regression tests proving both
halves of this distinction.

**Reload re-validates the SAME union (independent review v4, R1).** The
acceptance-time check above only ever ran once, at the moment of
acceptance -- nothing re-ran it on a later `load_state`. Persisted-record
checks alone (categories A-F below) cannot detect, for example, two
individually self-consistent accepted applications whose combined
placements collide on `service_id`, or a hand-corrupted state whose
individual records are each internally valid but whose reconstructed union
is operationally invalid. `validate_persisted_state_integrity` (called
from `load_state`) now reconstructs `union_project_operational_graph` for
every project with accepted history and runs the SAME
`validate_operational_dgraph` check against it, failing the load closed on
either a union error (e.g. the collision case) or a validator error (e.g.
the empty-union case). `cross_application_collision_undetectable_per_record_is_caught_by_reload_union_reconstruction`
and `operationally_invalid_reconstructed_projection_is_caught_at_reload_even_when_records_are_individually_valid`
are the regression tests; both hand-construct a state whose individual
proposal/acceptance/active records ALL pass categories A-F, specifically to
prove the union-reconstruction step is what catches the corruption, not a
per-record check. `positive_multi_application_multi_revision_state_reloads_cleanly`
is the accompanying positive control.

**Deterministic, and identity-distinct from the canonical D-Graph.**
`union_project_operational_graph` sorts nodes by `node_id` and each node's
services by `service_id`, so the same accepted deployment always reproduces
byte-identical projected content. The projected graph's own identity is
`operational_dgraph_id_for_project(project_id)` = `"ddeploy-operational:{project_id}"`
-- **deliberately never equal to** the canonical D-Graph's own `dgraph_id`
(which an application-plane `CanonicalDGraphV1` owns independently) or to
the legacy operational graph's conventional `"dgraph:{project_id}"` id. This
distinct identity is what makes the authorization-separation guarantee below
hold with zero additional matching-logic changes. Its `revision` is a
project-level counter (`project_operational_revision`) that increments only
when an ACCEPTED deployment changes the EFFECTIVE projection; an idempotent
re-acceptance never increments it.

**Provenance recorded in `metadata`** (least-disruptive integration point,
per the brief): `placement_authority: "canonical_dmap"` plus, per active
application, `application_id`, `dmap_id`, `dmap_revision`, `dmap_digest`,
`dgraph_ref`, `dcontinuum_ref`, `proposal_id`, `acceptance_id`.

## The transitional `ServiceArtifact` bridge

`ServiceArtifactRegistry` (Stage 107A/107B/108/109) predates Stage 111 and
remains the operational bridge from a canonical `service_id` to executable
content (a container image or a native-process definition). Stage 111 reuses
its EXISTING join semantics (`artifact_id`/`service_id`/
`metadata.service_aliases`, mirrored by `resolve_artifact_for_service`
without the `target_nodes` filter, which is proposal/acceptance-time
eligibility, not a join key) rather than inventing a parallel resolution
path. **This bridge is explicitly NOT a canonical D-Code registry.**
`ServiceArtifact` remains a transitional operational-plane concept; solving
D-Code's real canonical artifact/content-addressing model is out of scope
for Stage 111 and is not attempted here. Exactly one eligible artifact is
required per canonical service; zero (`artifact_not_found`) or more than one
(`artifact_ambiguous`) both fail closed -- never a silent "pick the first
match."

## Effective operational graph authority: one accessor, audited at every consumer

`dserver/src/api/ddeploy.rs` exposes two thin, read-only wrappers over
`core::ddeploy`: `effective_canonical_operational_graph(project_id)` (the
canonical-derived `OperationalDGraph` if canonical authority is active for
that project, else `None`) and `canonical_authority_active(project_id)`
(a boolean check used by the legacy write guard below).
`dserver/src/api/operational_dgraph.rs` centralizes the authority decision
in exactly two functions: `operational_dgraph_snapshot` (used by every
"give me the current graph" caller) and `graph_and_authorizations_snapshot`
(used by every slice/authorization caller); both consult
`effective_canonical_operational_graph` FIRST, falling back to the legacy
in-process `graph_store()` only when no canonical authority is active.

**Audited by repository-wide grep** (see the review package's
`legacy-endpoint-isolation-audit-v2.txt` for the exact command and output):
every consumer of the operational graph in this crate --
`api::alternative_paths`, `api::artifact_reconciliation`,
`api::governed_deployment_preflight`, `api::external_ai_inference`,
`api::operational_dgraph_acquisition`, `api::operational_analysis_context`,
`api::live_dashboard`, `api::secret_preflight`,
`api::secret_resolver_contract`, `api::graphql`, `api::sentinel_telemetry`
-- reaches the legacy store EXCLUSIVELY through `operational_dgraph_snapshot`
or `operational_dgraph_slice_snapshot` (itself built on
`graph_and_authorizations_snapshot`). There is no third code path that reads
`graph_store()` directly for a "current graph" decision, with exactly one
exception: `authorize_operational_reconciliation`'s own direct graph read,
which is likewise redirected to consult
`effective_canonical_operational_graph` first (resolved before the
function's `state_transaction()` lock is acquired, to avoid re-locking a
non-reentrant `std::sync::Mutex` on the same thread). No endpoint can see
canonical placement while another still serves the stale legacy graph.

**This GRAPH-level authority audit is necessary but not sufficient**
(independent review v2, correction 1): `api::governed_deployment_preflight`,
`api::secret_preflight`, and `api::secret_resolver_contract` all correctly
resolve their NODE SLICE through `operational_dgraph_slice_snapshot` (graph
authority correct), but each THEN independently resolved its ARTIFACT
context via the plain, authority-blind `build_artifact_reconciliation_context`
-- a SECOND, separate authority decision that v1 never audited. All three
now compute their own `PlacementAuthority` via `is_canonical_authority_active`
and call `build_artifact_reconciliation_context_with_authority`, exactly
like `get_operational_reconciliation_context`. See "Independent review v2
corrections" above and `legacy-endpoint-isolation-audit-v2.txt`.

## Legacy write paths fail closed once canonical authority is active

`put_operational_dgraph` (the legacy `PUT` for `OperationalDGraph`) now
checks `canonical_authority_active(project_id)` before validating or writing
anything; if a canonical D-Map-backed deployment is active for that project,
it refuses with 409 `canonical_dmap_authority_active`
(`dserver/src/api/operational_dgraph.rs::canonical_authority_conflict`)
rather than allowing a legacy write to silently override or race the
canonical projection. A project with no accepted canonical D-Map is
completely unaffected -- this guard only ever fires once canonical authority
exists for that specific project.

## Legacy `/api/v1/dmap` is unrelated and untouched

`dserver/src/api/dmap.rs` is an older, untyped Firebase JSON endpoint keyed
by project, predating both Stage 110 and Stage 111. It carries no schema,
has no relationship to `datum.dmap/1`, and its caller-supplied `"accepted"`
field (if present) is opaque application data -- it is never interpreted as
a D-Deploy acceptance, and this handler never parses its stored payload as a
`CanonicalDMapV1`. It is documented (a header comment, no functional change)
as legacy and is neither deleted nor reused by Stage 111.

## Authorization is not acceptance

`accept_proposal` performs schema/digest/CAS/eligibility validation and,
on success, updates in-memory `DDeployStateV1` fields. It calls no
authorization constructor, issues no `OperationalReconciliationAuthorization`,
invokes no dispatch, and touches no host -- `source_contains_no_host_mutation_calls`
(a source-scanning regression test, splitting the file at its own test-module
boundary to avoid detecting its own forbidden-string literals) asserts the
module's production code contains none of `std::process::Command`, a docker
invocation, native-process runtime mutation, `dispatch --execute`, governed
reconcile execution, or authorization issuance. Existing Stage 107A/109
authorization gates (execution-content-bound authorization, TOCTOU lease
checks) remain fully mandatory and completely unmodified by Stage 111.

**Old authorizations cannot authorize a new projection.**
`authorization_is_valid_for_context` (`dserver/src/core/operational_dgraph.rs:479`)
already requires exact `(project_id, dgraph_id, revision, node_id, binding_id)`
equality to match a graph. Since a canonical-derived operational graph's
`dgraph_id` (`ddeploy-operational:{project_id}`) is, by construction, never
equal to any legacy graph's own `dgraph_id`, any authorization issued against
the legacy graph automatically fails to match once canonical authority
activates -- with **zero** additional matching-logic changes. Proven by the
explicit regression test `old_legacy_authorization_cannot_authorize_canonical_derived_graph`
(`dserver/src/core/ddeploy.rs`), which constructs a legacy-style authorization,
confirms it DOES validate against the legacy-style graph it was issued for
(so the test is not vacuously true), builds a real canonical-derived graph at
the identical revision number via the actual acceptance path, and asserts
the SAME authorization fails against it.

## Multi-application: union of active deployments, explicit collision rejection

Per ADR-0009, `project_id` and `application_id` never collapse. The v0.1
model implemented (the brief's PREFERRED design): DServer maintains one
active accepted D-Map per `(project_id, application_id)`
(`DDeployStateV1.active_by_application`, keyed by `application_key`), and
derives the project's effective `OperationalDGraph` as the deterministic
UNION of every active application's precomputed desired-service map
(`union_project_operational_graph`). If two active applications in the same
project would place the same operational `service_id` -- something the
legacy per-project, non-application-scoped operational wire has no way to
disambiguate -- acceptance is rejected outright with
`operational_service_id_collision` (`detect_service_id_collisions`, checked
BEFORE any commit) rather than silently overwriting one application's
placement or inventing a hidden renamed id. `two_applications_both_active_in_one_project`
and `same_service_id_across_two_applications_is_rejected` cover both the
success and rejection paths.

## Proposal-time vs acceptance-time validation

`create_proposal` runs `validate_full` once, against the snapshots supplied
at proposal time. `accept_proposal` runs the IDENTICAL `validate_full`
function again, at acceptance time, against the immutably stored snapshot
and the CURRENT `ServiceArtifact` registry -- re-checking the proposal
digest (if supplied), the CAS revision, full D-Map feasibility (ABI,
individual/aggregate resource capacity, cardinality -- Stage 110, reused
verbatim), artifact eligibility, `target_nodes`, and `target_stages`.
`tampered_proposal_snapshot_cannot_be_accepted` and the eligibility-narrowed-
between-proposal-and-acceptance tests prove a proposal valid yesterday is
never automatically acceptable today.

## Post-acceptance artifact changes never move placement

Changing an artifact's CONTENT (image tag, native-process binary, execution
generation) after acceptance does not, and cannot, change an already-accepted
D-Map's placement -- `ActiveApplicationDeployment.operational_services`
is frozen at acceptance time (see "D-Map -> OperationalDGraph projection"
above). The existing artifact/reconciliation and runtime-parity machinery
(Stage 107A/109) continues to govern content changes exactly as before,
independent of Stage 111. A placement change of any kind requires a NEW
D-Deploy proposal and a new explicit acceptance.

## Persistence, restart, and versioning

`DDeployStateV1` (schema-versioned via `state_version`, currently `"0.1.0"`)
persists as one JSON blob under the NEW, separate `DOMAIN_DDEPLOY_STATE`
control-state domain (`dserver/src/storage/control_state.rs`) -- deliberately
never written into `DOMAIN_OPERATIONAL_DGRAPH` (the pre-existing legacy
operational graph domain) or the legacy Firebase DMap blob
(`api::dmap`). `load_state` bootstraps an empty v1 state when nothing has
been persisted yet, and fails closed (an explicit error, never a silent
reinterpretation) on any UNRECOGNIZED future `state_version` --
`unknown_future_state_version_fails_closed` and
`no_previous_state_bootstraps_empty_v1_state` cover both directions.
`state_survives_restart` proves a full lifecycle (create, accept, drop the
in-memory state, reload from the SAME on-disk path) reproduces the identical
active D-Map, digest, and derived operational provenance.

**Relational integrity, re-verified on every load (independent review v3,
extended in v4).** `validate_persisted_state_integrity`, called from
`load_state`, checks the WHOLE persisted blob's relationships, not merely
each record against itself, across seven categories: (A) every proposal's
map key, digest, D-Graph/D-Continuum snapshot cross-references, and (v4)
bidirectional Accepted-proposal-to-acceptance linkage; (B) every
acceptance's map key, referenced-proposal existence (a dangling reference
is corruption, not tolerated), digest, and historically-reconstructed
D-Map validity; (C) every active application's map key, bidirectional
proposal/acceptance linkage, an acceptance-cross-reference digest check
(`active_dmap` vs its own acceptance's `materialized_dmap_ref`), and (kept
as defense-in-depth, see "Independent review v4 corrections" above)
structural equality against the historical reconstruction; (D) the
`operational_services` snapshot's service-ID set matching
`active_dmap.placements`' service-ID set exactly, with correct map keys and
valid runtimes; (E) `project_operational_revision` bookkeeping, DERIVED
FROM ACCEPTED HISTORY (v4: never from `active_by_application`'s own keys,
which a corrupted blob can simply omit); (F, v4) every application with
accepted history has an active entry matching the LATEST accepted D-Map
revision for that application, ordered by the server-owned
`materialized_dmap_ref.revision`; (G, v4) the reconstructed project-wide
operational union for every project with accepted history passes
`validate_operational_dgraph`. Over thirty dedicated corruption tests
(`dserver/src/core/ddeploy.rs`) each tamper exactly one relationship,
assert `load_state` fails closed, and (v4) assert on the SPECIFIC error
text to confirm the INTENDED check fired rather than an unrelated earlier
one; `self_consistent_state_loads_cleanly` and
`positive_multi_application_multi_revision_state_reloads_cleanly` prove the
checks do not false-positive on real, uncorrupted lifecycles. These checks
establish consistency of RETAINED accepted history against the specific
corruption shapes tested -- they are not, and are not claimed to be, a
defense against an arbitrary wholesale rewrite of the underlying database.
This remains a bounded, fail-closed check, never a migration or repair
framework -- on any inconsistency, `load_state` returns `Err` and nothing
is auto-repaired.

## API surface

All five endpoints are project-scoped via `X-Datum-Project`
(`dserver/src/api/ddeploy.rs`):

| Method | Path | Behavior |
|---|---|---|
| `POST` | `/api/v1/ddeploy/proposals/validate` | Pure preview: validates a candidate proposal shape, persists nothing, activates nothing. 200 regardless of validity; `valid`/`findings` in the body. |
| `POST` | `/api/v1/ddeploy/proposals` | Creates and persists an immutable PENDING proposal after full validation. 201 on success, 422 with structured findings on rejection. |
| `GET` | `/api/v1/ddeploy/proposals/:proposal_id` | Fetches a proposal record and its lifecycle state. 200, or 404 (including for a cross-project proposal id -- see below). |
| `POST` | `/api/v1/ddeploy/proposals/:proposal_id/accept` | Revalidates, CAS-checks, materializes the D-Map, derives the projection, and atomically persists all of it. 200 (including idempotent replay), 404, 409 (stale / digest mismatch), 422 (invalid / collision), 500 (persistence failure). |
| `GET` | `/api/v1/ddeploy/active` | Returns every active application deployment for the project (optionally filtered by `?application_id=`). |

**Cross-tenant proposal isolation:** a proposal whose `project_id` does not
match the caller's `X-Datum-Project` header reports identically to a
genuinely nonexistent proposal id (404, `proposal_not_found`) -- never a
differentiated 422/409 -- so a caller cannot use the response shape to probe
whether a given proposal id exists in another project.

**No update/reject endpoint.** A proposal is immutable once created; a
changed placement is always a new proposal. A dedicated "reject" endpoint
was judged unnecessary for Stage 111 (pending + accepted states suffice);
this remains an open option for a later stage.

## Non-goals (explicit)

Not implemented in this stage, deliberately:

- **Stage 112**: any placement algorithm -- scoring, best-fit/first-fit,
  resource optimization, latency scoring, graph partitioning, GNN, RL,
  LLM-driven placement, cloud-first/fog-only policy. Every function in
  `core::ddeploy` either VALIDATES or MATERIALIZES a caller-supplied
  candidate placement; nothing selects, scores, or optimizes a node.
  Confirmed by a repository-wide regression grep for
  `score|choose_best|optimize|first_fit|best_fit|random|GNN|RL|LLM` limited
  to this stage's new files (see the review package's
  `no-stage112-placement-audit.txt`).
- **Stage 113**: D-Monitor / live telemetry. No CPU usage, memory-available,
  RSS, load average, live network state, RTT, or temperature ever
  participates in proposal validation or acceptance -- Stage 110's
  structural, declared capacity/capability fields are the only inputs.
- A canonical D-Code artifact registry -- `ServiceArtifact` remains the
  documented TRANSITIONAL operational bridge, not a redesign target.
- A new shared `datum-model` crate (ADR-0007, reaffirmed). `DDeployProposalV1`/
  `DDeployAcceptanceV1` are a DServer/D-Controller-owned concern and are not
  mirrored into the DATUM agent, since the agent does not consume them.
- Any change to Stage 110's `datum.dmap/1`/`datum.dgraph/1`/`datum.dcontinuum/1`
  wire semantics -- `CanonicalDMapV1::validate_against` is reused verbatim,
  never re-implemented or altered.
- Extending `datum.dcontinuum/1` with a D-Stage concept (see the
  `target_stages` audit above).
- A dedicated "reject proposal" endpoint.

## Validation

`dserver`'s full test suite (977 tests, including 58 Stage 111 `core::ddeploy`
tests) is green; `cargo build --locked` for both `DATUM` and `dserver` is
clean. Stage 107/110/111 Python checkers and their selftests, and
`mkdocs build --strict`, are recorded in the review package's
`test-results.txt` with exact totals for the specific package under review,
rather than restated here as a number that would go stale on the next fix.
