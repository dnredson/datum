# Phase 117F — DCode Lifecycle & DDeploy Binding Audit

**Status:** Phase 117F AUDIT CLOSED (audit only). **Phase 117 remains OPEN.**
**NO PRODUCTION REGISTRY, DDEPLOY, AUTHORIZATION, OR DCOMPILE CODE WAS
CHANGED DURING PHASE 117F. NO VM OR RPI LIVE VALIDATION WAS PERFORMED.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117E closure, commit `0ad35e2f8b098e9b9cbf7ed77d23be128a2e0675`.

## 1. Executive conclusion

The silent-executable-switch threat is real and confirmed: nothing in
today's architecture prevents a mutable DCode registry from changing the
executable an already-accepted deployment resolves to, because
`DDeployProposalV1`'s own digest is structurally blind to artifact content
— `compute_proposal_digest` hashes only `dgraph_ref`/`dcontinuum_ref`/
`placements`, never any artifact reference. This is confirmed directly from
`ddeploy.rs:1251-1254`, not inferred.

The comparison with `ServiceArtifact` initially suggested by the task
turns out to have a different answer than "DDeploy already solved this for
containers." **DDeploy's `ServiceArtifact` eligibility check
(`validate_full`/`resolve_artifact_for_service`) never pins content
identity either — it only checks node/stage placement eligibility.**
`ServiceArtifact` avoids the H1→H2 conflict not through any DDeploy-level
content-pinning mechanism, but because its own registry is genuinely
**mutable in place** (`service_artifact_registry.rs`'s real `update()`
function), with currentness handled entirely downstream by Stage 107A's
own continuous, per-reconciliation-cycle re-validation
(`execution_projection_digest` recomputed fresh every cycle) — completely
independent of any DDeploy acceptance event. DCode/WASM's invocation model
is intentionally much stricter (per-invocation pinning, zero imports,
fresh-instance-per-call) and its registry is intentionally immutable — the
"asymmetry" is not a DCode design gap relative to ServiceArtifact; it is
two legitimately different security postures that happen to need two
different lifecycle solutions.

A second, independently-confirmed P0/P1 finding not anticipated by the
task's own framing: **ADR-0009 explicitly states `application_id` is
designed to be reused across multiple projects** ("a project references
one or more application D-Graph revisions... reusing one D-Graph across
installations... become expressible") — yet `dcode_registry`'s key
`(application_id, dserv_id)` carries no `project_id` component at all.
This is a genuine, evidence-confirmed under-scoping, not a hypothetical
edge case: it is the explicitly-designed use case that the current key
cannot safely support.

**Recommended direction (not implemented): an immutable, versioned DCode
registry keyed by `(project_id, application_id, dserv_id, descriptor_digest)`,
with exact descriptor-digest binding elevated into the PUBLIC
`DDeployProposalV1` schema** (not left as an internal-only snapshot) so
that a pure DCode-only redeploy is a real, digest-distinguishable proposal,
combined with acceptance-time revalidation of the exact bound descriptor
and a new binding field on `ActiveApplicationDeployment`.

## 2. Current authority model (frozen, unchanged by this audit)

DScript (source) → DCompile-request (compile mapping/resources/output
identity) → DCompile (pure candidate construction, 117C/117E) → DGraph
(application topology) → `CanonicalDServArtifactV1` (executable descriptor)
→ DCode registry (registered descriptor authority) → DMap (sole placement
authority) → DDeploy (proposal/acceptance/active-deployment authority) →
DCode authorization (invocation authority) → `DCodeModuleStore` (installed
bytes) → DMonitor (observed runtime reality). No boundary is moved by this
audit's recommendations — the exact artifact binding is proposed to live
in DDeploy's own proposal/acceptance/active-state machinery (new fields
there), never inside DGraph or DMap themselves (Section 15).

## 3. Silent executable-switch threat (confirmed)

Walking the task's own T0 scenario against real code: at T0, invocation
authorization (`authorize_dcode_invocation`) resolves the registered
artifact via `dcode_registry_state.get(application_id, service_id)`
(`dcode_authorization.rs:207`) — a live lookup by key, not a reference to
anything pinned at D-Deploy acceptance time. **If the registry ever allowed
`registry[A,S] = H2` to overwrite `H1` in place** (which it does not today
— `declare_dserv_artifact` rejects a differing descriptor at an existing
key with `Conflict`), the very next `authorize_dcode_invocation` call would
authorize H2, using the SAME active D-Map/D-Graph that was actually
accepted against H1's existence, with zero new governed decision. **Answer:
YES, this would be a genuine authority bypass** if the registry were ever
made naively mutable. The only reason this isn't exploitable *today* is
that the registry's own immutability accidentally serves as the safety net
— which is also exactly why relaxing that immutability (to solve H1→H2)
without adding an equivalent safety net elsewhere would reintroduce this
exact bypass. **This is the central security invariant Phase 117G's design
must preserve: relaxing per-key immutability must be paired with binding
the exact descriptor identity into governed deployment acceptance, not
merely done in isolation.**

## 4. Current registry semantics

`DCodeArtifactRegistryStateV1 { state_version, artifacts: BTreeMap<String, CanonicalDServArtifactV1> }`,
keyed by `dserv_artifact_key(application_id, dserv_id)` — a length-prefixed
composition (`dcode_registry.rs:18-29`), explicitly modeled on
`ddeploy::application_key`'s own scheme, but **without** `project_id`
(Section 17 below). `declare_dserv_artifact` (`dcode_registry.rs:69-91`):
validates the descriptor, then either no-ops on byte-identical re-declaration
or fails closed with `Conflict` on any difference — confirmed exactly as
Phase 117D found, re-verified here directly. `validate_persisted_dcode_registry_integrity`
(`dcode_registry.rs:104`) re-validates every stored artifact and map-key
consistency on load, mirroring `dnode_registry`'s own precedent — no silent
repair of corruption (satisfies F7 of the task's failure-scenario list:
corrupted persisted state fails closed on reload, by existing design).

## 5. Current DDeploy artifact semantics

Both DDeploy v1 (`ddeploy.rs`) and v2 (`ddeploy_v2.rs`) are live production
paths today, not one legacy/compatibility-only — confirmed via
`dserver/src/main.rs`'s route table: `/api/v1/ddeploy/proposals` and
`/api/v1/ddeploy/v2/proposals` both exist, with their own
create/accept handlers. `post_create_proposal` and the deterministic-placement
endpoint `post_plan_proposal` both funnel into the same v1
`create_proposal` (`ddeploy.rs:1862`). **Any future DCode integration must
address both paths — hardening only one leaves the other unprotected.**

`ServiceArtifact` eligibility (`validate_full`/`resolve_artifact_for_service`,
`ddeploy.rs:1331-1625`) resolves a `ServiceArtifact` by
`artifact_id`/`service_id`/alias match against a **freshly-fetched, live**
`artifacts: &[ServiceArtifact]` slice (`fetch_project_artifacts`,
confirmed always a current-state fetch, never a frozen snapshot) and checks
only `check_node_eligibility` (does `control_plane.target_nodes` include
the candidate node) and `check_target_stages` — **placement eligibility,
never content identity**. `PlannerProvenanceV1.eligibility` persists a
`PlacementEligibilityView` (service_id → target-nodes set) as plan-time
audit provenance only, for `ProposalSource::Deterministic` proposals — not
a content-pinning mechanism.

`ActiveApplicationDeployment` (`ddeploy.rs:306-326`):
`{application_id, active_dmap, dmap_digest, proposal_id, acceptance_id,
accepted_at_utc, operational_services}`. `operational_services` is derived
at acceptance time (`derive_application_operational_services`,
`ddeploy.rs:1667-1697`) from the SAME live `artifacts` slice, but the
stored `OperationalDesiredService` only carries `{service_id, runtime,
desired_state, application_id, dmap_ref}` — `runtime` is just the artifact's
`runtime.kind` string (e.g. `"container"`); **no image/content digest of
any kind is captured into this persisted state.** Actual content resolution
for containers happens later, live, per reconciliation cycle
(Stage 107A/109's own established, independently-audited pattern from
Phase 115).

## 6. Current DCode authorization semantics (reconfirmed, exact)

`authorize_dcode_invocation` (`dcode_authorization.rs:109-277`, fully
re-traced): active D-Map → D-Graph snapshot → service membership + ABI →
D-Map placement on the requested node → D-Node ABI support → **live
registry lookup by `(application_id, service_id)`** → artifact self-validation
→ artifact ABI match → empty stage constraints. **It does currently
authorize "whatever artifact happens to be registered for
(application_id, service_id)"** — this is safe TODAY only because exactly
one artifact can ever occupy that key. If the registry becomes
multi-version, this lookup must change from "get whatever is there" to
"get the EXACT descriptor digest the active deployment is bound to" —
otherwise the same bypass as Section 3 reappears in a different guise (an
ambiguous multi-version registry with no exact-selector lookup would let
authorization silently pick an arbitrary/latest version).

## 7. ServiceArtifact comparison (the key reframe)

| Dimension | ServiceArtifact path | DCode current path | Recommended DCode path |
|---|---|---|---|
| Registry key | `(project_id, artifact_id)` (confirmed project-scoped — `service_artifact_registry.rs`) | `(application_id, dserv_id)` — **no `project_id`** | `(project_id, application_id, dserv_id, descriptor_digest)` |
| Mutability | **Mutable in place** (`update()` exists) | **Immutable per key** (no replacement path) | **Immutable per descriptor digest**; new digest = new, additional entry, never an overwrite |
| Eligibility check | Node/stage placement only, never content | N/A (no DDeploy integration exists) | Node/stage/ABI + exact descriptor-digest existence, at both proposal-creation and acceptance |
| Content-identity pinning | **None at DDeploy layer** — resolved live at reconciliation time, cycle by cycle | **Full pinning at invocation time**, but registry itself has no versioning to pin ACROSS deployments | Exact descriptor digest bound into the proposal itself (public schema), re-verified at acceptance |
| Currentness authority | Stage 107A continuous re-validation (independent of DDeploy) | Registry existence (today; ambiguous once multi-version) | Active-deployment binding (`ActiveApplicationDeployment`), never bare registry existence |
| Replacement/redeploy | Update the registry; takes effect next reconciliation cycle, no new DDeploy proposal required | Not possible at all today | Requires a NEW DDeploy proposal explicitly bound to the new descriptor digest — a governed transition, by design stricter than ServiceArtifact's model |

**This is not forced symmetry** — the task's own instruction to not force
exact symmetry where semantics differ is honored: DCode's stricter,
invocation-pinned execution model justifies a stricter, acceptance-gated
redeploy model, even though ServiceArtifact's own looser model works fine
for its own (much weaker per-invocation guarantee) execution semantics.

## 8. Registry evolution options (evaluated)

- **R1 (mutable one-slot):** rejected outright — this is literally the
  silent-switch bypass from Section 3, made real.
- **R2 (immutable multi-version, keyed by `(application_id, dserv_id, descriptor_digest)`):**
  strong candidate, but under-scoped per Section 17 — needs `project_id`
  added.
- **R3 (keyed by `(application_id, dserv_id, artifact_id, version)`):**
  rejected — Phase 117D already proved `artifact_id`/`version` carry zero
  uniqueness/authority guarantee; two different content revisions could
  legally share the same `artifact_id`/`version` label (nothing forbids
  it), making this key ambiguous in exactly the way R2's digest-based key
  is not.
- **R4 (globally digest-keyed descriptor objects + relational indices):**
  architecturally the most "correct" content-addressed-storage shape, but
  adds real complexity (a second index structure) with no evidence any
  current consumer needs cross-application/cross-project descriptor
  deduplication. Not ruled out for a later phase, but not the minimal
  correct next step.
- **R5 (`service_id` changes per revision):** explicitly rejected — Section
  13.
- **Recommended: R2, corrected for project scope** —
  `(project_id, application_id, dserv_id, descriptor_digest)`, immutable
  per full key, multiple revisions coexisting for the same
  `(project_id, application_id, dserv_id)` triple.

## 9. Exact artifact-binding options (evaluated)

Per Section 15's B1-B8 list: **B1 (DGraph)** and **B2 (DMap)** rejected —
neither is evidence-supported as an executable-identity home (DGraph is
topology; DMap is placement; Phase 117A/117D already established this
boundary and no new evidence here contradicts it). **B3 (public
`DDeployProposalV1` schema):** required, per Section 10's finding below —
without it, a pure DCode-only redeploy cannot be a semantically distinct
proposal. **B4 (internal `ProposalRecord` snapshot):** also needed, for the
FULL descriptor content (not just its digest) — mirroring `dgraph_snapshot`'s
own precedent exactly, so replay can reconstruct exact historical content,
not merely prove a digest matched. **B5 (`DDeployAcceptanceV1`):** the
natural place to re-verify the bound descriptor(s) still exist/validate at
acceptance time (Section 11). **B6 (`ActiveApplicationDeployment`):**
required, as the currentness home (Section 12) — mirrors
`operational_services`'s own existing per-service map shape. **B7 (separate
canonical `DCodeDeploymentBinding` contract):** not evidence-supported as a
SEPARATE top-level canonical contract; the binding is naturally a per-service
field addition to the existing B3/B4/B5/B6 homes, not a new standalone
schema. **Recommendation: B3 + B4 + B5 + B6 together** — a combination, per
the task's own B8 option, is genuinely the smallest correct model, because
each of the four serves a different, non-overlapping role (public intent,
internal provenance, acceptance-time gate, active-state currentness).

## 10. Proposal identity implications (major finding, confirmed directly)

`compute_proposal_digest` (`ddeploy.rs:1251-1254`) computes
`jcs::canonical_digest_sha256_hex` over the serialized `DDeployProposalV1`
struct directly — whose fields are exactly `{schema, proposal_id,
project_id, application_id, source, created_at_utc,
expected_active_dmap_revision, dgraph_ref, dcontinuum_ref, placements}`.
**None of these reference artifact/DCode content in any way.** Confirmed:
two proposals identical in every one of these fields but intended to bind
to two DIFFERENT DCode descriptor revisions would produce the IDENTICAL
proposal digest today. **This directly answers Section 39/40: DDeploy's
proposal model, as it exists today, genuinely cannot represent "a pure
DCode-only redeploy" as a distinct governed decision** — if `dgraph_ref`
and `placements` are unchanged, two proposals differing only in intended
DCode binding are indistinguishable by every mechanism DDeploy currently
has (digest, `expected_active_dmap_revision` compare-and-swap, or shape
validation). **This forces a real public-schema decision**: exact DCode
descriptor binding (at minimum, one digest per service) must become part
of `DDeployProposalV1` itself, not merely `ProposalRecord`'s internal
snapshot — otherwise the proposal DIGEST (which is what
`accept_proposal`'s compare-and-swap and idempotent-replay logic actually
keys on) cannot see the change, and "propose a DCode-only update" has no
way to exist as a real, distinguishable proposal.

## 11. Acceptance-time revalidation (recommended model)

Mirroring `ServiceArtifact`'s own acceptance-time re-fetch pattern
(Section 5), but for content identity rather than placement eligibility: at
acceptance, for each service with a bound descriptor digest, re-fetch that
EXACT digest from the (now-multi-version) registry and confirm (a) it
still exists, (b) it still independently validates
(`CanonicalDServArtifactV1::validate()`), and (c) its digest still equals
what the proposal declared. **"Current artifact" is the wrong question
once the registry is multi-version** — Section 21 is right that the
correct question becomes "does the EXACT bound revision still exist and
validate," never "what does the registry currently consider latest" (there
is no "latest" concept, and none should be invented — see the P0 rationale
in Section 26).

## 12. Active-state binding (recommended model)

Add a per-service descriptor-digest map to `ActiveApplicationDeployment`,
structurally parallel to its existing `operational_services:
BTreeMap<String, OperationalDesiredService>` field (same key shape,
same "no node identity duplicated here" discipline the existing field's
own doc comment already establishes for placement) — e.g. conceptually
`dcode_bindings: BTreeMap<String, DCodeBindingRef>` (`service_id →
{descriptor_digest, application_id, dserv_id}`), populated at acceptance
from the proposal's own bound descriptor references (Section 10), never
independently computed from "whatever the registry currently holds."
**This is the ONLY safe place currentness may live** — never a bare
registry lookup (Section 6), never a separately-mutable pointer that could
drift independently of an actual DDeploy acceptance event (Section 24's
warning, directly validated).

## 13. Historical replay (real, pre-existing limitation — not new)

`ProposalRecord` already snapshots `dgraph_snapshot`/`dcontinuum_snapshot`
immutably, and replay integrity is checked before replaying (confirmed via
the existing `accepted_proposal_replay_also_verifies_digest_integrity_before_replaying`
test). **But since neither ServiceArtifact NOR DCode content is snapshotted
into `ProposalRecord` today, replaying an old accepted proposal already
cannot reconstruct "what artifact content was true then" for containers
either** — this is a genuine, pre-existing architectural gap that predates
DCode entirely, not something DCode integration introduces. Recommendation:
`ProposalRecord` should gain the SAME kind of full-content snapshot for
bound DCode descriptors that `dgraph_snapshot` already provides for
D-Graph — this closes the gap for DCode specifically without being
required to retroactively fix ServiceArtifact's own pre-existing
limitation (out of this phase's scope, and out of Phase 117's scope
entirely — a container/native-process concern, not DCompile's).

## 14. Authorization currentness

Confirmed and reasoned through directly (Section 6): once versioned,
`registry.get(application_id, service_id)` (today's lookup) must become
`registry.get_exact(project_id, application_id, service_id, descriptor_digest)`,
with the exact digest sourced from `ActiveApplicationDeployment`'s new
binding field (Section 12), never from the registry's own notion of
"latest." `verify_dcode_authorization_still_current`'s re-derivation must
likewise re-check the ACTIVE BINDING, not mere registry existence — Section
26 proves this is required, not optional.

## 15. Where exact artifact binding lives (summary)

Public proposal intent → `DDeployProposalV1` (new field, Section 10).
Internal immutable provenance/replay → `ProposalRecord` (new field,
mirroring `dgraph_snapshot`, Section 13). Acceptance-time gate → new
revalidation logic in `accept_proposal`/`accept_v2_proposal` (Section 11).
Active currentness → `ActiveApplicationDeployment` (new field, Section
12). No change to `CanonicalDGraphV1` or `CanonicalDMapV1` themselves.

## 16. H1 → H2 scenario, walked end-to-end against the recommended model

- **P1 accepted with H1 (descriptor digest D1):** registry holds D1 at
  `(project_id, A, S, D1)`; proposal P1's own digest includes D1's
  reference; `ActiveApplicationDeployment` binds `S → D1`.
- **Recompile to H2 (descriptor digest D2):** D2 is declared into the
  registry as a NEW, additional immutable entry at
  `(project_id, A, S, D2)` — D1 is untouched, still present, still valid.
  **D2 existing in the registry grants it no authority whatsoever** — it
  is inert data until referenced by an accepted proposal (Section 16's own
  F1 failure scenario, confirmed satisfied by this design: registering H2
  alone changes nothing about what's active).
- **P2, explicitly bound to D2, is proposed:** its own digest differs from
  P1's (Section 10's fix makes this possible) even if `dgraph_ref`/
  placements are unchanged (Section 16's F8/Section 39 case).
- **P2 acceptance:** revalidates D2 exactly (Section 11), then updates
  `ActiveApplicationDeployment.dcode_bindings[S] = D2` atomically with the
  rest of acceptance's existing compare-and-swap.
- **Post-acceptance:** `authorize_dcode_invocation` resolves `S → D2` via
  the active binding (Section 14); any authorization still holding D1 is
  now stale the instant it re-derives (Section 26) — because the active
  BINDING changed, not because D1 itself became invalid (D1 remains a
  perfectly valid, structurally-correct historical descriptor).
- **Replaying P1:** reconstructs D1 exactly, because `ProposalRecord`'s
  new snapshot field (Section 13) preserved it, independent of D2's later
  existence.

**Every clause of the task's Section 37 scenario is satisfiable by this
recommended model** — none require weakening any existing invariant, and
none require inventing behavior unsupported by the evidence gathered.

## 17. Project/application identity scope (P0/P1 finding)

**ADR-0009** (`documentation/adr/ADR-0009-application-identity.md`) states
directly: `project_id` is "installation/tenant scope, authorization scope";
"a project references one or more application D-Graph revisions"; and
lists as an explicit intended consequence "reusing one D-Graph across
installations... become expressible." **This means the SAME `application_id`
being deployed under two different `project_id`s is not a hypothetical
edge case — it is the architecture's own explicitly designed use case.**
Corroborating evidence: `canonical_dmap_id(project_id, application_id)`
(D-Map's own identity composition) includes BOTH components, confirming
D-Map's own key is deliberately project-scoped for exactly this reason.
`dcode_registry`'s key `(application_id, dserv_id)` has no `project_id`
component — confirmed by direct code inspection, not inferred. **Classification:
P0-adjacent** (not merely theoretical): any future multi-version registry
design MUST include `project_id` in its key, or two unrelated projects
legitimately using the same `application_id` (ADR-0009's own stated goal)
will collide over the same DCode registry slot(s). This is folded directly
into the Section 8/9 recommendation (R2 corrected).

## 18. Compiler provenance placement

Kept explicitly separate from execution authority, per the task's own
instruction: D-Script digest, D-Compile-request digest, and D-Graph digest
(117C) plus module/descriptor digest (117E) answer "what produced this,"
never "what is active now." Recommendation: these belong on
`ProposalRecord`'s own provenance fields (mirroring `PlannerProvenanceV1`'s
existing precedent for algorithmic-placement provenance exactly) — never
inside `CanonicalDServArtifactV1.provenance` (already established as an
uninterpreted catch-all field in 117E, deliberately not used for this) and
never inside the invocation-authorization chain itself (`dcode_authorization.rs`),
which should keep authorizing based only on active-binding identity
(Section 14), not compiler history.

## 19. Current data flow (as it exists today)

```
external Wasm H1
    -> DCompile candidate descriptor (117E, out-of-band)
    -> explicit registration: declare_dserv_artifact
       (application_id, dserv_id) -- NO project_id -- one immutable slot
    -> [DDeploy proposal/acceptance proceeds completely independently --
        no reference to this artifact at all, confirmed zero DCode
        references in ddeploy.rs/ddeploy_v2.rs]
    -> authorize_dcode_invocation: live registry.get(application_id, service_id)
    -> invocation

H1 -> H2 fails at: declare_dserv_artifact returns Conflict (Section 4).
DDeploy eligibility is absent at: the entire proposal/acceptance path
(Section 5) -- there is no step where DDeploy even looks at the DCode
registry.
```

## 20. Target flow (recommended, not implemented)

```
external Wasm H2
    -> DCompile descriptor candidate D2 (117E, unchanged)
    -> register D2 as a NEW immutable revision:
       (project_id, application_id, dserv_id, descriptor_digest=D2)
       -- D1 remains, untouched, inert until referenced
    -> create DDeploy proposal P2, its public schema now carrying an
       explicit per-service {service_id -> descriptor_digest} binding
       (new DDeployProposalV1 field) -- so P2's own digest differs from
       P1's even if dgraph_ref/placements are identical
    -> acceptance re-verifies D2 exactly (exists, validates, digest matches)
       alongside existing DGraph/DContinuum/ServiceArtifact revalidation
    -> ActiveApplicationDeployment.dcode_bindings[service_id] = D2,
       updated atomically with the rest of acceptance
    -> authorize_dcode_invocation resolves the ACTIVE BINDING, not bare
       registry existence, then verifies the exact descriptor
    -> invocation
```

**Alternative flow compared (H2, registration-and-acceptance combined):**
a single "deploy" API call that registers D2 and creates+accepts P2
atomically in one transaction was considered (task's H2/H4 options) but
rejected as the RECOMMENDED default: it removes the useful, currently-real
property (Section 29) that a candidate/registered revision may safely
exist before ever being referenced by any proposal, with zero authority
consequence — collapsing registration and acceptance into one step would
make failure/rollback semantics harder to reason about for no evidenced
benefit, given nothing in the current architecture requires that
atomicity. The two-step flow (register, then separately propose/accept)
is the smaller, safer, currently-precedented (matches how D-Graph creation
and proposal creation are already two logically separate, though often
temporally adjacent, steps) design.

## 21. Identity matrix

| Identity | Owner | Mutability | Authority role | Historical role |
|---|---|---|---|---|
| Logical service identity (`service_id`) | D-Compile-request / D-Graph | Stable across recompiles (Section 13's own finding, unchanged) | Names WHAT is being executed, never WHICH revision | Constant across a service's whole lifetime |
| D-Graph identity/revision/digest | D-Graph / D-Compile (candidate) | Immutable per revision | Application topology | `ProposalRecord.dgraph_snapshot` already preserves it |
| D-Map identity/revision/digest | D-Deploy acceptance | Replaced wholesale on new acceptance | Placement | `ActiveApplicationDeployment.active_dmap` already preserves it |
| `artifact_id` | External/DCompile input | Free-form, descriptive only | None (117D/117E confirmed) | Provenance-only |
| `version` (artifact label) | External/DCompile input | Free-form, descriptive only | None (117D/117E confirmed) | Provenance-only |
| Module content digest (`dcode.sha256`) | External builder | Immutable once declared | Proves WHICH bytes, never authorizes by itself | Permanent once any descriptor references it |
| Descriptor digest | Recomputed (`recompute_dserv_artifact_digest`) | Immutable per descriptor | The real exact-revision identity (recommended authority key component) | Should be preserved in `ProposalRecord`'s new snapshot (Section 13) |
| Proposal identity/digest | `compute_proposal_digest` | Immutable once created | Names one governed deployment INTENT | Already fully historical via `ProposalRecord` |
| Acceptance identity | D-Deploy acceptance | Immutable once created | Names one governed transition EVENT | Already fully historical |
| Active-deployment currentness | `ActiveApplicationDeployment` (recommended: + `dcode_bindings`) | Replaced wholesale on new acceptance | The ONLY safe source of "what's active now" (Section 12/26) | Superseded on next acceptance, not deleted from history |

## 22. Lifecycle state machine (conceptual, not implemented)

```
[constructed candidate] (117E, ephemeral, no authority)
    -> [registered immutable revision] (exists in registry, still no authority alone -- F1)
    -> [referenced by a pending proposal] (still no authority -- a proposal that never
         gets accepted grants nothing, F3)
    -> [active via an accepted deployment] (the ONLY authority-bearing state)
    -> [historical/superseded] (once a later acceptance rebinds the service;
         remains valid, retrievable, replayable content -- never deleted by this phase, Section on GC)
```

**"Active artifact" is explicitly NOT a registry state** — confirmed by
this whole audit's reasoning: the registry's job is to answer "does this
exact descriptor exist and validate," never "is this the one currently in
use." That question belongs entirely to `ActiveApplicationDeployment`
(Section 12), as a DDeploy relation, exactly as the task's own Section 36
hypothesis suspected.

## 23. Failure scenarios (walked against the recommended model)

- **F1** (H2 registered, no proposal): H1 remains active — confirmed
  (registration alone grants nothing, Section 16/19).
- **F2** (P2 references H2, H2 disappears/corrupts before acceptance):
  acceptance's exact-digest revalidation (Section 11) fails closed.
- **F3** (P2 pending, H3 registered): P2 still refers to H2 explicitly (its
  bound digest is part of its own immutable public schema, Section 10) —
  "latest" is never consulted.
- **F4** (P2 accepted with H2, H3 later registered): H2 remains active —
  registering H3 has zero effect on `ActiveApplicationDeployment` (Section
  12's "never a separately-mutable pointer" discipline).
- **F5** (old H1 authorization used after P2/H2 acceptance): stale —
  confirmed by Section 14/26's binding-based re-derivation.
- **F6** (descriptor metadata changes, module bytes stay H2): exact
  descriptor-digest binding detects the difference (this is precisely what
  descriptor digest — as opposed to module digest alone — already exists
  to catch, per the existing `dcode_authorization.rs` R2A correction,
  Section 7 of Phase 117D).
- **F7** (registry persisted state corrupted): already fails closed today
  (`validate_persisted_dcode_registry_integrity`, Section 4) — no change
  needed.
- **F8** (identical D-Graph, only DCode revision changes): **requires a
  new DDeploy proposal** under the recommended model, and Section 10's
  schema change makes that proposal genuinely distinguishable from the
  prior one — this is the direct, confirmed answer to Section 39.

## 24. Compiler-provenance / execution-authority separation (reaffirmed)

Execution authority answers "what exact topology/placement/artifact is
active now" — entirely satisfied by D-Graph + D-Map + the new
`dcode_bindings` (Section 12), never by any of the five compiler-provenance
digests. Audit/reproducibility answers "what D-Script/D-Compile inputs
produced this" — belongs on `ProposalRecord` (Section 18), never inside
the authorization chain.

## 25. Authority matrix

| Concern | Candidate/metadata owner | Canonical authority | Currentness owner |
|---|---|---|---|
| Source | D-Script | D-Script's own digest | N/A (immutable per revision) |
| Compile mapping | D-Compile-request | D-Compile-request's own digest | N/A |
| Topology | D-Compile (candidate) | D-Graph | Active D-Map's `dgraph_ref` |
| Executable descriptor | D-Compile (candidate, 117E) | `CanonicalDServArtifactV1` | N/A — a descriptor is either registered or not, never "current" by itself |
| Executable revision registration | External build + D-Compile | Recommended: versioned registry (Section 8) | N/A — registry existence grants no authority |
| Deployment artifact binding | D-Deploy proposal (recommended new field) | D-Deploy acceptance | `ActiveApplicationDeployment.dcode_bindings` (recommended) |
| Placement | D-Map | D-Map | Active D-Map |
| Invocation | D-Code authorization chain | D-Code authorization chain | Re-derived every call, per `verify_dcode_authorization_still_current` |
| Installed bytes | `smartsentinel-dcode-install` (manual, unchanged) | `DCodeModuleStore` | Verified at every load, unchanged |
| Runtime observation | D-Monitor | D-Monitor | unchanged |

## 26. Migration / backward compatibility

No existing `CanonicalDServArtifactV1` fixture/document needs to change
shape — the recommended registry-key change (adding `project_id` and
`descriptor_digest` to the KEY, not the descriptor TYPE itself) is a
storage/lookup-layer change, not a wire-contract change to
`CanonicalDServArtifactV1`. Existing single-slot registrations would need a
one-time migration to the new key shape (out of this audit's scope to
design). `DDeployProposalV1`'s schema addition (Section 10) is additive
(a new optional-until-populated field) and would need a schema-version
bump per this project's own established discipline (matching how
`datum.dscript/1`/`datum.dcompile-request/1` each got their own schema
strings) — not evaluated further here since no implementation occurs in
this phase.

## 27. Recommended Phase 117G

Comparing the task's own 117G-A through 117G-E: **117G-C (registry +
DDeploy proposal binding + acceptance revalidation together)** is
recommended over splitting into 117G-A/117G-B separately, because Section
3's central security invariant (relaxing registry immutability is unsafe
without simultaneously binding exact identity into governed acceptance)
means the two halves are not independently safe to ship — a
versioned-registry-only slice (117G-A alone) would, by itself, do nothing
useful (nothing yet CONSUMES the new versions safely), while a
binding-contract-only slice (117G-B alone) has nothing to bind to yet.
117G-D (compiler provenance persistence only) is a reasonable, smaller,
INDEPENDENT slice that could genuinely ship first or in parallel, since it
touches only `ProposalRecord`'s own additive provenance fields and creates
no new authority surface. **Recommendation: 117G — DCode registry
versioning + proposal binding + acceptance revalidation (a combined slice,
matching 117G-C), optionally preceded or accompanied by a small, separable
117G-0 for compiler-provenance persistence alone (117G-D) if sequencing
value is found once implementation begins.** Both v1 and v2 DDeploy paths
must be addressed (Section 5) — do not harden only one.

## 28. Explicit exclusions (for whatever slice comes next)

No production code was changed in 117F itself. When 117G is eventually
scoped: no DGraph/DMap schema change (Section 9/15's own conclusion); no
retroactive fix to ServiceArtifact's own pre-existing replay limitation
(Section 13, out of Phase 117's scope); no garbage collection of old
registry revisions (Section 30 of the task — explicitly a later lifecycle
concern, not required for correctness); no byte-distribution automation
(Section 31 — remains manual, a DNode/deployment concern); no DCall
derivation (Section 32 — still no evidence).

## 29. Open questions

- Exact schema-version string and migration mechanics for
  `DDeployProposalV1`'s new binding field — not designed here.
- Whether `dcode_bindings` on `ActiveApplicationDeployment` should be keyed
  by `service_id` alone or `(service_id, descriptor_digest)` for extra
  defense-in-depth against a corrupted/tampered persisted state — a real
  but small design question for 117G, not resolved here.
- Whether DDeploy v1 and v2's eventual DCode integrations should share one
  implementation or two independently-hardened ones, given they already
  have separate `create_proposal`/`accept_proposal` implementations today.
- Retention policy for superseded registry revisions (Section 30) — this
  audit states retention is required for replay integrity but does not
  design a retention/GC policy.

## 30. Evidence index

- `dserver/src/core/dcode_registry.rs:1-91` (key composition, `declare_dserv_artifact`, immutability comment, integrity validation)
- `dserver/src/core/dcode_authorization.rs:109-277` (full 8-step authorization chain, live registry lookup)
- `dserver/src/core/dserv_artifact.rs` (unchanged since Phase 117D)
- `dserver/src/core/ddeploy.rs:306-326` (`ActiveApplicationDeployment`), `:1251-1254` (`compute_proposal_digest`), `:1331-1351` (`resolve_artifact_for_service`), `:1625` (`validate_full`), `:1667-1697` (`derive_application_operational_services`), `:1862` (`create_proposal`), `:242` (`PlannerProvenanceV1.eligibility`)
- `dserver/src/core/service_artifact_registry.rs:784` (mutable `update()`, confirming ServiceArtifact's genuinely different lifecycle model)
- `dserver/src/api/ddeploy.rs` (route-to-function mapping, `fetch_project_artifacts`)
- `dserver/src/main.rs` (route table confirming both DDeploy v1 and v2 are live)
- `documentation/adr/ADR-0009-application-identity.md` (full text — the `application_id`/`project_id` scoping finding, Section 17)
- `documentation/adr/ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md`
- `documentation/reference/phase117a/b/c/d/e-*.md` (all prior Phase 117 findings, frozen and reused, not reopened)
- Git history: `git log --all --grep`/`-S` searches for artifact replacement, version transition, descriptor digest, ServiceArtifact eligibility, proposal digest, application_id uniqueness — no evidence beyond this repository's own current, already-cited implementation commits and this session's own prior Phase 117D/117E documentation; no historical design for artifact replacement/versioning was ever found, confirming this is a genuinely open architectural question, not a regression.
