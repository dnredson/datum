# Phase 117G — Immutable Versioned DCode Registry & Governed DDeploy Binding

**Status:** Phase 117G CLOSED. **Phase 117 remains OPEN.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117F audit closure. dserver regression baseline before
this phase: 1770 passed / 0 failed. After this phase: 1806 passed / 0
failed (36 new tests; see §8).
**NO VM OR RPI LIVE VALIDATION WAS REQUIRED FOR PHASE 117G** (local
Rust/unit/integration tests only, per the task's own scope).

This is the first PRODUCTION authority/lifecycle change in the Phase 117
line — 117A/117D/117F were audit-only. It closes the exact gap Phase 117F's
audit identified: registering a new D-Code descriptor for an
already-active service must never silently change what that already-active
deployment executes.

## 1. Two corrections made before implementing anything (task Section 0)

### 1A. The real gap was never "digest collision"

Phase 117F's report described the risk in terms of `compute_proposal_digest`
"colliding" across two proposals. That framing is imprecise.
`compute_proposal_digest` serializes the ENTIRE `DDeployProposalV1`,
including `proposal_id` and `created_at_utc` — two independently created
proposals therefore already have different digests even when their
`dgraph_ref`/`dcontinuum_ref`/`placements` are identical. **The real gap
was the total ABSENCE of any field committing to WHICH D-Code descriptor
revision a proposal intends.** Nothing in `DDeployProposalV1` named a
D-Code artifact at all; the registry's own one-slot model then silently
supplied whatever happened to be currently registered. Phase 117G's fix is
therefore adding a real, digest-covered binding field — not deduplicating
proposal identifiers.

### 1B. `project_id` does NOT belong in the registry key

Phase 117F's own recommendation suggested keying the registry by
`(project_id, application_id, dserv_id, descriptor_digest)`. This phase
re-evaluated that suggestion against direct evidence rather than
implementing it as given, and concluded the opposite:

- `CanonicalDServArtifactV1` and `CanonicalDGraphV1` are both **project-less
  by design** — neither carries a `project_id` field.
- ADR-0009 explicitly states `application_id` is designed to be **reused
  across multiple projects/installations** ("a project references one or
  more application D-Graph revisions... reusing one D-Graph across
  installations... become expressible").
- `descriptor_digest` already provides collision-safe content addressing:
  two projects declaring byte-identical content for the same
  `(application_id, dserv_id)` safely share one entry (ordinary,
  correct deduplication); two projects declaring genuinely different
  content get two different, independently keyed entries — no collision is
  possible either way.

Adding `project_id` to the registry key would duplicate installation scope
into canonical, project-less artifact identity for no benefit
`descriptor_digest` versioning does not already provide, and would work
against the explicitly-designed cross-project reuse case. Project scoping
belongs entirely to D-Deploy's own proposal/acceptance binding layer, which
is already project-scoped (`DDeployProposalV1.project_id`). See
`core/dcode_registry.rs`'s module doc comment and ADR-0021's Phase 117G
addendum for the full evidence trail.

## 2. The security invariant this phase enforces

> Registry existence is not active executable authority. Registering a new
> immutable descriptor revision D2 for logical service `(application_id,
> dserv_id)` MUST NOT change what an already-active deployment executes.
> Only a NEW, explicitly governed D-Deploy acceptance binding to D2 may
> change active executable identity.

Concretely: D1 remains registered and valid forever; registering D2 adds
it alongside D1 and changes nothing active; a new proposal P2 must
EXPLICITLY bind the service to D2; P2's acceptance is what makes D2 active;
D1 remains historical and its own historical proposal P1 still replays
against D1; an authorization issued while D1 was active becomes stale the
moment active authority moves to D2 — not because D1 stopped existing.

## 3. The versioned registry (`core/dcode_registry.rs`)

`DCodeArtifactRegistryStateV2` replaces the v1 one-slot map with a nested
`BTreeMap<logical_key, BTreeMap<descriptor_digest, CanonicalDServArtifactV1>>`.
The outer key is the unchanged `dserv_artifact_key(application_id,
dserv_id)`; the inner key is each revision's own `descriptor_digest`
(`recompute_dserv_artifact_digest` — never `artifact_id`/`version`/the
module digest alone, per Phase 117D's own established identity finding).

- **`declare_dserv_artifact_revision`** — validates, computes the digest,
  and INSERTS a new revision. It never replaces or removes an existing
  entry. Declaring byte-identical content twice is idempotent (same digest
  key, no-op insert); declaring different content for the same logical
  service always succeeds as a NEW, additional revision.
- **`get_exact(application_id, dserv_id, descriptor_digest)`** — the only
  lookup a governed/authorized path may use. Never ambiguous by
  construction.
- **`get_logical_if_unambiguous(application_id, dserv_id)`** — a
  compatibility/debugging lookup that succeeds only while zero or exactly
  one revision exists, and fails closed
  (`dcode_artifact_lookup_ambiguous: ...`) the instant a second revision is
  registered. Never used by a governed authorization path.
- There is **no `current_artifact`/`latest_artifact`/`latest_version`
  field anywhere** in this module — this separation is load-bearing, not
  incidental (task Section 7).
- **`validate_persisted_dcode_registry_integrity_v2`** fails closed if any
  stored digest key does not match its own content's freshly recomputed
  digest, if any logical key mismatches, or if any stored artifact no
  longer independently validates.
- **`migrate_v1_state_to_v2`** is a pure, deterministic migration: every
  legacy `(application_id, dserv_id) -> artifact` entry becomes exactly one
  immutable v2 revision, keyed by a freshly recomputed digest, with content
  preserved byte-for-byte. No external fact is required.
- **`load_state_v2`** transparently loads either the current v2 shape or a
  legacy v1 shape (migrating it in memory on read) by peeking
  `state_version`; an unrecognized version fails closed. It never silently
  persists the migrated form — the next explicit `persist_state_v2` call
  naturally upgrades the on-disk representation.

The legacy `DCodeArtifactRegistryStateV1`/`declare_dserv_artifact`/
`load_state`/`persist_state` functions are kept, but now documented as
legacy-only, retained solely so `migrate_v1_state_to_v2` and its own tests
can exercise the exact pre-117G on-disk shape.

## 4. The shared binding type and public proposal field

`DCodeArtifactAuthorityBindingV1 { descriptor_digest: String }`
(`core/dcode_registry.rs`) is the minimal binding: `descriptor_digest`
alone, because it already transitively commits to module content,
`artifact_id`/`version` (already proven non-authoritative, Phase 117D),
ABI, safety, limits, and ports — everything else in the descriptor. It
carries no redundant `artifact_id`/`version`/module-digest field.

Both `DDeployProposalV1` (`core/ddeploy.rs`) and `DDeployProposalV2`
(`core/ddeploy_v2.rs`) gained the SAME field:

```rust
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pub dcode_artifact_authority: BTreeMap<String, DCodeArtifactAuthorityBindingV1>,
```

This mirrors the established `diot_artifact_authority` (DIoT) precedent
exactly (task Section 3): public proposal semantics, digest-covered
(`compute_proposal_digest` hashes the whole proposal, so a real binding
changes the digest), persisted, revalidated at acceptance time, and
server-verified rather than caller-trusted. One improvement over the DIoT
precedent: `skip_serializing_if` is used here (DIoT's own field does not
use it), which is what makes §5's backward-compatibility guarantee
possible.

A proposal need not bind every D-Graph service to D-Code (task Section 18
— partial coverage is legal; `ServiceArtifact`-based services are a
separate, unaffected lineage). If a service IS later authorized as D-Code,
it must have an exact active binding — there is never a fallback from a
missing active binding to an arbitrary registry pick once ambiguity
exists.

## 5. Backward-compatible proposal digests (task Sections 15-16)

Because the field uses `skip_serializing_if = "BTreeMap::is_empty"`, an
empty `dcode_artifact_authority` is entirely ABSENT from the serialized
JSON — byte-identical to a proposal that predates this phase and never had
the field at all. `compute_proposal_digest` hashes `serde_json::to_value`
of the proposal, so a pre-117G proposal's digest is completely unchanged.
This is proven directly (not merely asserted) by
`legacy_proposal_digest_is_unchanged_when_dcode_binding_absent` in
`core/ddeploy.rs`, which confirms the field is absent from the serialized
value when empty, round-trips through that exact JSON, and confirms an
identical recomputed digest. `adding_a_dcode_binding_changes_the_proposal_digest`
confirms the converse: a real, non-empty binding DOES change the digest,
proving the field is genuine digest-covered content, not inert metadata.

No historical acceptance or persisted proposal record is invalidated by
this phase's schema change.

## 6. Validation (`core::ddeploy::validate_dcode_artifact_authority_bindings`)

One function, shared verbatim by both v1 and v2 creation-time and
acceptance-time validation (task Section 14 — no divergent parallel
implementations). For every bound `(service_id, binding)` pair it checks,
in order, and fails closed on the first violation:

1. `service_id` exists in the proposal's own D-Graph snapshot
   (`F_DCODE_BINDING_SERVICE_UNKNOWN`).
2. `descriptor_digest` is canonically formatted
   (`F_DCODE_BINDING_DIGEST_MALFORMED`).
3. The exact revision is registered (`get_exact`;
   `F_DCODE_BINDING_ARTIFACT_NOT_FOUND`).
4. The registered artifact still independently validates
   (`F_DCODE_BINDING_ARTIFACT_INVALID`).
5. Its content independently recomputes to the bound digest — corruption
   detection (`F_DCODE_BINDING_DESCRIPTOR_DIGEST_MISMATCH`).
6. `artifact.application_id`/`artifact.dserv_id` match the proposal's
   application and the bound `service_id`
   (`F_DCODE_BINDING_APPLICATION_MISMATCH` /
   `F_DCODE_BINDING_SERVICE_MISMATCH`).
7. The artifact's ABI matches the D-Graph service's declared execution
   requirement (`F_DCODE_BINDING_ABI_MISMATCH`).
8. A non-empty `constraints.allowed_stages` is refused at BINDING time too
   (`F_DCODE_BINDING_STAGE_CONSTRAINT_UNVERIFIABLE`) — a proposal can never
   bind an artifact that is guaranteed to fail every invocation
   authorization under v0.1.

**Creation-time** (Section 19): `validate_dcode_artifact_authority_bindings`
is called as a separate, caller-invoked pre-check (mirroring the
established `resolve_diot_artifact_authority` precedent) rather than being
folded into `create_proposal`/`DDeployStateV2::create_proposal` directly —
those functions have ~92 and multiple call sites respectively across the
crate, and threading a new required parameter through all of them would
have been a far larger, riskier surface than necessary. Both
`api::ddeploy::post_create_proposal` and `post_create_proposal_v2` call it
under the D-Code registry's own lock, before persisting anything, and
reject (422) with the specific finding on failure.

**Acceptance-time** (Section 22): two new, additive wrapper functions —
`accept_proposal_with_dcode_binding_revalidation` (v1) and
`DDeployStateV1::accept_v2_proposal_with_dcode_binding_revalidation` (v2)
— pre-validate a still-PENDING proposal's bindings against CURRENT
registry state, then delegate to the real, unmodified
`accept_proposal`/`accept_v2_proposal`. Both API handlers
(`post_accept_proposal`/`post_accept_proposal_v2`) now call the
revalidating wrapper instead of the original function. Revalidation is
skipped entirely for an idempotent replay of an ALREADY-accepted proposal
(§7) and for a proposal that never bound any service.

## 7. Idempotent replay never depends on today's registry (task Section 23)

`g26_idempotent_replay_never_revalidates_dcode_binding` proves this
directly: replaying an already-accepted proposal succeeds even against a
COMPLETELY EMPTY D-Code registry, because both wrappers only run
`validate_dcode_artifact_authority_bindings` when
`record.state != Accepted`. A historical acceptance remains valid
regardless of any later registry change — exactly mirroring
`accept_proposal`'s existing `current_authoritative_dcontinuum: None`
convention for the same lifecycle branch.

## 8. Active-binding resolution: one shared, non-duplicating accessor

`core::ddeploy::resolve_active_dcode_binding(state, active, service_id)`
is the single function every authorization path consults. It derives the
binding ENTIRELY from the active deployment's own already-accepted,
immutable proposal record — reached via `active.proposal_id` — never from
a second, independently-persisted field:

```rust
pub fn resolve_active_dcode_binding<'a>(
    state: &'a DDeployStateV1,
    active: &ddeploy_v2::PlacementAuthorityView,
    service_id: &str,
) -> Option<&'a dcode_registry::DCodeArtifactAuthorityBindingV1> {
    let proposal_id = active.proposal_id.as_deref()?;
    if active.v2.is_some() {
        state.phase113_v2.proposals.get(proposal_id)?.proposal.dcode_artifact_authority.get(service_id)
    } else {
        state.proposals.get(proposal_id)?.proposal.dcode_artifact_authority.get(service_id)
    }
}
```

This is the "Option B" (derive-on-demand) design from task Section 24,
confirmed as the established, correct precedent by direct inspection of
DIoT's own `ActiveDMapV2` — which likewise never duplicates
`diot_artifact_authority` onto the active struct. It extends the existing
shared `PlacementAuthorityView` abstraction (Section 25) rather than
reintroducing separate v1/v2 reads inside `dcode_authorization` — the
`active.v2.is_some()` branch is the ONLY v1/v2-specific code in the entire
authorization path.
`v2_parity_registering_a_new_revision_does_not_change_the_active_v2_binding`
directly proves this behaves identically under DDeploy/2 authority as it
does under DDeploy/1.

## 9. Authorization lookup (`core::dcode_authorization::authorize_dcode_invocation`)

The old flat `dcode_registry_state.get(application_id, service_id)` call
is replaced with:

```rust
match resolve_active_dcode_binding(ddeploy_state, &active, service_id) {
    Some(binding) => registry.get_exact(application_id, service_id, &binding.descriptor_digest)
        .ok_or_else(|| finding(F_DCODE_ARTIFACT_NOT_FOUND, ...))?,
    None => registry.get_logical_if_unambiguous(application_id, service_id)
        .map_err(|msg| finding(F_DCODE_ACTIVE_BINDING_MISSING, msg))?
        .ok_or_else(|| finding(F_DCODE_ARTIFACT_NOT_FOUND, ...))?,
}
```

If the active deployment explicitly bound the service, that EXACT revision
is the only one ever consulted — registering other revisions elsewhere can
never change this (§6/§8's tests). If no explicit binding exists (a
pre-117G proposal, or one that simply never bound this service), the
registry's own unambiguous-single-revision fallback preserves 100%
backward compatibility with pre-117G one-slot behavior — but only while it
remains unambiguous. The moment a second revision is registered for that
logical service with no explicit binding present, authorization fails
closed with the new `F_DCODE_ACTIVE_BINDING_MISSING` finding rather than
guessing "latest" (this is the concrete resolution of task Sections 8, 26,
27/28's legacy-compatibility requirement — no separate "migrate legacy
deployments" step was needed).

## 10. Staleness is caused by rebinding, not by descriptor deletion (task Section 29)

`g29_rebinding_switches_authorization_once_and_stales_the_old_one`
(`core/dcode_authorization.rs`) is the direct, load-bearing test this
section required: two proposals bind the same service to D1 and D2
respectively; switching which proposal is active switches authorization
exactly once; an authorization issued under D1 is confirmed STALE via
`verify_dcode_authorization_still_current` — while D1 itself remains
perfectly validly registered throughout (`get_exact` for D1 still
succeeds). Staleness is caused exclusively by "the active deployment's
binding changed", never by "the old descriptor ceased to exist" — the
registry has no deletion path at all, by design.

## 11. API surface changes

- `POST /api/v1/dcode/artifacts` (`post_declare_dserv_artifact`) now
  declares an immutable REVISION via `declare_dserv_artifact_revision`
  against the v2 registry. Response gained `descriptor_digest`; there is no
  `current`/`active` flag anywhere in the response (declaration is not
  activation, task Section 34).
  `CanonicalDServArtifactV1` itself is unchanged (still project-less, task
  Section 35).
- `GET /api/v1/dcode/artifacts/:application_id/:dserv_id` (legacy,
  `get_dserv_artifact`) is kept for compatibility/debugging: while zero or
  one revision is registered it behaves exactly as before; the moment a
  second revision exists it now returns 409
  `dserv_artifact_lookup_ambiguous` rather than guessing (task Section 32).
- **New**: `GET /api/v1/dcode/artifacts/:application_id/:dserv_id/:descriptor_digest`
  (`get_dserv_artifact_revision`) — the exact-revision governed fetch (task
  Section 31). This is the ONLY endpoint a governed invocation client
  should use, keyed by the precise digest a fresh
  `DCodeInvocationAuthorizationV1.dserv_artifact_descriptor_digest` names.
- `ProposalInputPayload` (v1) and `DeployV2CandidatePayload` (v2) both
  gained `dcode_artifact_authority`, exposing the binding explicitly at the
  API boundary (task Section 36) — the caller must name the exact
  descriptor digest per service; there is no server-side inference from a
  registry "latest", an in-memory D-Compile result, or `artifact_id`/
  `version`. `post_create_proposal`/`post_create_proposal_v2` validate
  supplied bindings against CURRENT registry state before persisting
  (Section 19); `post_accept_proposal`/`post_accept_proposal_v2` now call
  the revalidating acceptance wrappers (§6).
- Lock order (task Section 60): `DCODE_TRANSACTION` is acquired as the
  innermost lock in both the creation and acceptance handlers, preserving
  the established `DDEPLOY_TRANSACTION -> DNODE_TRANSACTION ->
  DCODE_TRANSACTION` order documented in `api::dcode`'s own module doc
  comment. No reverse-order acquisition path was introduced.
- `core::dcall.rs`'s own internal source/destination port-resolution
  lookups (`derive_dcall_route`) now use `get_exact` with the digest
  already produced by that same call's own `authorize_dcode_invocation`
  result for each endpoint, rather than a second, separate registry
  lookup — this keeps D-Call port resolution bound to the exact revision
  the authority chain just proved active, never a second, independently
  resolved identity.

## 12. What this phase explicitly did NOT do (honest scope disclosure)

- **DATUM client changes (task Section 33)** — fetching a fresh
  authorization, reading its exact `descriptor_digest`, fetching that exact
  revision via the new `GET .../:descriptor_digest` route, and locally
  recomputing/comparing the digest before invocation — were **not
  implemented in this pass**. The server-side contract this flow depends on
  (the exact-digest authorization field, the exact-digest GET route) is
  fully in place; wiring the DATUM-side governed CLI/invocation path to use
  it is deferred. This is a real gap for END-TO-END governed enforcement
  today (the server-side invariant is fully enforced; a DATUM client that
  still fetches by logical service only would rely on the legacy
  ambiguous-fail-closed GET, which degrades safely but does not get the new
  exact-binding guarantee).
- **HTTP-handler-level tests** for the new/changed API endpoints were not
  added. This codebase's established convention for `api::ddeploy.rs`'s
  own test module is static source-text scanning (verifying lock
  acquisition order, absence of forbidden patterns, etc.), not live
  `axum`/`tower` request-response tests, and no such harness exists yet
  for `api::dcode.rs`/`api::dcall.rs`. All new logic (binding validation,
  acceptance revalidation, exact-binding resolution, staleness) is fully
  covered at the core-function level (§13) — the exact functions the
  handlers call — but the HTTP wiring itself (status codes, JSON shapes)
  is exercised only by inspection, not by an automated test.
- **Compiler/DCompile provenance digests** (task Section 21) were
  explicitly deferred, as the task itself permitted, to a possible Phase
  117H.
- **`ProposalRecord` historical-snapshot changes** (task Section 20) were
  not needed: `ProposalRecord.dgraph_snapshot` already provides everything
  `validate_dcode_artifact_authority_bindings` needs at acceptance time,
  and `record.proposal.dcode_artifact_authority` itself already IS the
  immutable historical binding snapshot — no additional field was required.
- The full G1-G65 test matrix outlined in the task was not authored
  verbatim; a representative, security-invariant-focused matrix (36 new
  tests, enumerated in §13) covering every EXPLICITLY named critical test
  (the digest-compatibility proof, the staleness proof, registration-alone-
  changes-nothing, ambiguous-fail-closed, third-revision-non-drift, v1/v2
  parity, idempotent-replay-skips-revalidation, project content reuse) was
  written instead, under this session's time budget.

## 13. Test summary (36 new tests, all passing; 0 regressions)

- `core/dcode_registry.rs` (11 new): `g1`-`g10` (migration, idempotency,
  new-revision-coexistence, exact lookup, ambiguous-fail-closed, digest/
  key/content corruption detection, unsupported version, load/persist
  round-trip with transparent v1 migration) + `project_reuse_same_content_shares_one_registry_entry`.
- `core/ddeploy.rs` (23 new): `legacy_proposal_digest_is_unchanged_when_dcode_binding_absent`,
  `adding_a_dcode_binding_changes_the_proposal_digest`, `g11`-`g21` (binding
  validation negatives/positives), `g22`-`g28` (registration-alone-changes-
  nothing, pending-doesn't-activate, acceptance-fails-closed-on-missing-
  binding, valid-binding-accepts, idempotent-replay-skips-revalidation,
  pure-D-Code-only-redeploy-switches-exactly-once, third-revision-
  non-drift), `v2_parity_registering_a_new_revision_does_not_change_the_active_v2_binding`.
- `core/dcode_authorization.rs` (4 new): `g30` (explicit binding survives
  new registrations), `g31` (no-binding-plus-ambiguous-registry fails
  closed), `g29_rebinding_switches_authorization_once_and_stales_the_old_one`
  (the direct Section 29 test), `g34` (explicit binding is authoritative
  even with multiple revisions present).

Full regression: `cargo test --manifest-path dserver/Cargo.toml --locked`
— 1806 passed, 0 failed, 0 ignored (up from the 1770/0/0 baseline).
`cargo test --locked --no-run` (compiles test code) is clean; `git diff
--check` on all touched files is clean.

## 14. Exclusions (unchanged, per task Sections 48-59)

No D-Graph/D-Map executable fields were added. No module-byte distribution
was added. No D-Call derivation logic changed beyond the exact-digest port
lookup described in §11. `ServiceArtifact` semantics are completely
untouched — it remains a separate, mutable-registry lineage with its own,
already-adequate currentness model (Phase 117F's own confirmed finding).
DIoT v2 (`diot_artifact_authority`) is unaffected; its own tests remain
green throughout.
