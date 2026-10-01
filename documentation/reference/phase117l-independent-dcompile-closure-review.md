# Phase 117L — Independent DCompile Closure Review

## 1. Verdict

**B — PASS WITH NON-BLOCKING KNOWN LIMITATIONS. PHASE 117 CAN CLOSE.**

No P0/P1 blocker was found. Every finding-carrying section below either
confirms an already-correct invariant directly against current source/
tests/live evidence, or classifies a real, honestly-disclosed v1 scope
limitation that does not compromise the authority boundaries Phase 117 is
responsible for. Section 25 lists every non-blocking limitation explicitly.

This review was performed entirely in the primary session — no subagent or
fork was used, per this task's own explicit instruction (following the
Phase 117I subagent scope violation). Every claim below was independently
re-derived from current source, current tests (re-run in this session), and
direct inspection of Phase 117K's recorded live evidence — not accepted
from prior phase reports on the strength of their own "PASS"/"CLOSED"
labels.

## 2. Scope and baseline

- Reviewed branch: `agent/phase117-dcompile`.
- Starting HEAD (confirmed clean): `876cb33b547dda2d108a42ef0091a03677b5a8c9`.
- Phase-116 closure baseline: `ff72b2b6401b37564a4fe949eb19a9c15c0b4326`.
- No live infrastructure was used or required for this review (per task
  Section 0/47) — this is a source/test/evidence-based review of work
  already live-proved in Phase 117K.

## 3. Full delta reviewed

```
git log --reverse --oneline ff72b2b6..HEAD   -> 16 commits (8 feat/fix, 8 docs)
git diff --stat ff72b2b6..HEAD               -> 43 files changed, 12944 insertions(+), 304 deletions(-)
git diff --check ff72b2b6..HEAD              -> clean
git diff --check (working tree)              -> clean
```

Production files changed, classified by concern:

| Concern | Files |
|---|---|
| DScript boundary | `canonical_dscript.rs` |
| DCompile mapping | `dcompile_request.rs` |
| DGraph lowering | `dcompile.rs` |
| DCode artifact lowering | `dcompile_dcode.rs` |
| Production DCompile API | `dcompile_submission.rs`, `api/dcompile.rs`, `api/mod.rs`, `main.rs` |
| DCode registry | `dcode_registry.rs` |
| DDeploy | `ddeploy.rs`, `ddeploy_v2.rs`, `api/ddeploy.rs` |
| DCode authorization | `dcode_authorization.rs`, `api/dcode.rs` |
| DCall compatibility adaptation | `core/dcall.rs`, `core/dcall/tests.rs`, `api/dcall.rs` |
| DATUM governed client | `DATUM/src/dcode/client.rs`, `smartsentinel-dcode-invoke.rs`, `smartsentinel-dcall-dispatch.rs`, `DATUM/src/dcall/delivery.rs` |
| Mechanical struct-literal adaptation (new required field) | `artifact_reconciliation_context.rs`, `dforward_dependency_readiness.rs`, `dforward_projection.rs`, `diot_runtime_binding.rs`, `dmonitor_interpretation.rs`, `dmonitor_readiness.rs` |
| ADR | `ADR-0021-canonical-dserv-artifact-and-dcode-wasm-v0.1.md` |
| Test fixture (new, additive) | `dcompile-request.sensor-pipeline.json` |
| Documentation (new, phase reports) | 11 `phase117*.md` files |

**No production change unrelated to Phase 117 was found.** Every one of
the "mechanical adaptation" files changed for exactly one reason: adding
the new `dcode_artifact_authority` field to a pre-existing struct literal
in an unrelated test module, because `CreateProposalInput`/
`DDeployProposalV1`/`DDeployProposalV2` gained a required field — confirmed
directly by re-inspecting the diffs (§22 below).

## 4. DCompile responsibility reconstructed

Read against ADR-0001, `CanonicalDScriptV1`'s own module doc comment, and
`dcompile.rs`/`dcompile_dcode.rs`'s own doc comments: DCompile's
implemented responsibility is **explicit, deterministic lowering** —
`(DScript, D-Function→D-Serv mapping+resources, D-Code content/policy
facts) → (candidate D-Graph, candidate D-Code descriptors)`. It is
NOT a parser, NOT a language compiler, and NOT a D-Call deriver. The
six-contract chain (DScript → DCompile-request → DGraph candidate →
DCode-input → DCode candidate → production API response) is fully
implemented and, as of 117J/117K, has a real production entry point and a
real live proof.

The layering this review confirmed is intact end to end: DGraph remains
pure application topology (no executable/placement field — confirmed
§11); DCode remains pure executable identity/descriptor (no placement
field — confirmed §13); DMap/DDeploy remain the sole placement and
active-authority owners (confirmed §18-20); DNode remains the sole
execution/installation boundary (confirmed §15, §17); DMonitor is
untouched by this phase entirely (grep-confirmed, §39).

## 5. DScript boundary

`canonical_dscript.rs` (Phase 116, touched in 117J only to add
`validate_and_digest_dscript` — a pure refactor extracting
`decode_and_validate_dscript_json`'s own body, proven byte-for-byte
equivalent by two new tests). Confirmed directly from source:

- `annotations: BTreeMap<String, String>` — opaque, never interpreted
  anywhere in `dcompile_request.rs`/`dcompile.rs` (confirmed by
  `annotations_never_influence_validation_outcome` and
  `c11_annotation_changes_do_not_affect_generated_dgraph`).
- No placement field, no executable-identity field (both confirmed by
  `no_forbidden_authority_field_exists_on_the_type`'s own forbidden-keyword
  scan, unchanged).
- `functions` order is non-semantic — `recompute_dscript_digest` sorts by
  `function_id` before hashing (confirmed,
  `digest_is_order_independent_for_functions`).
- `#[serde(deny_unknown_fields)]` on both types — confirmed
  (`i14_unknown_field_is_rejected_not_silently_ignored`).

**Result: intact. Phase 117 did not corrupt Phase 116 semantics.**

## 6. Mapping/resource input

`DCompileRequestV1`/`DCompileServiceMappingV1` (117B, untouched in
structure by 117J — only a new typed `validate_and_digest_dcompile_request`
sibling was added, same equivalence-proof pattern as §5). Confirmed:

- `output_dgraph_id`/`output_dgraph_revision` explicit, required.
- `resource_requirement` explicit, required, `cpu_cores`/`memory_bytes`
  both must be `>= 1` (`b28`/`b29`).
- **Exact partition enforced**: `overlapping_function_mapping` (`b21`),
  `unmapped_function`/`unknown_function_mapping` (`b22`/`b23`,
  `validate_against_dscript`) — no D-Function may belong to zero or more
  than one D-Serv.
- `service_id` is caller/tool-supplied text, never derived — confirmed by
  direct inspection (no hash/inference logic exists anywhere in
  `DCompileServiceMappingV1`'s construction path) and by the module's own
  doc comment citing "no precedent anywhere in this codebase for hashing a
  canonical identity out of content."
- No `function_id == service_id` inference anywhere (§40 confirms this by
  grep — the only occurrence of that phrase is the module doc comment
  stating it is NOT done).
- Request digest deterministic and order-independent (`b4`/`b5`).

**Result: intact, unmodified by 117J beyond the additive typed helper.**

## 7. DGraph lowering

`lower_calls_empty_dgraph_candidate` (117C, entirely unmodified by 117D-K
— confirmed: this function's own source body is untouched across the
whole reviewed diff range except for the doc-comment correction described
in §22). Directly re-confirmed from source:

- Defensive re-validation: re-runs `document.validate()`/`request.validate()`
  and independently recomputes BOTH digests, comparing against the
  wrapper's own claims, before constructing anything (`c13`-`c16`).
- Relational validation via `validate_against_dscript` (`c17`-`c22`).
- Output: `application_id` copied from D-Script; `dgraph_id`/`revision`
  copied from the request's explicit `output_dgraph_id`/`revision`; one
  `CanonicalDServV1` per mapping entry, sorted by `service_id`
  (`c4`); `dnode_abi` is the sole legal v0.1 ABI constant, never a
  "default" (module doc: "there is no second value to default away from");
  `calls: Vec::new()` unconditionally (`c10`).
- `CanonicalDGraphV1::validate()` is genuinely invoked before return
  (`c24`), and the digest is always freshly recomputed (never trusted,
  `dgraph_digest_matches_independent_recomputation`).
- No placement, no DDeploy call, no DNode call, no runtime mutation, no
  observed-state read — confirmed directly: this function's only imports
  are `canonical_dgraph`/`canonical_dscript`/`dcompile_request`, nothing
  from `ddeploy`/`dnode_registry`/`dmonitor_*`.

**Result: intact, load-bearing, and correctly reused unmodified by every
later phase through 117K.**

## 8. DCode lowering

`lower_dcode_artifact_candidates` (117E, entirely unmodified in its own
body by 117F-K). Confirmed:

- Step 1 delegates to `lower_calls_empty_dgraph_candidate` — never
  reimplemented (module doc: "Composition, not reimplementation").
- `DCompileDCodeServiceInputV1`: `sha256`/`size_bytes` externally
  declared; `ports`/`limits` explicit required policy; `artifact_id`/
  `version` explicit descriptive-only data (confirmed non-authoritative
  by the module doc's own 117D citations).
- Server-derived: `uri`/`content_id` (formatted from the declared
  `sha256`), `abi` (the sole legal ABI constants), `safety`/`state_model`/
  `instantiation`/`constraints` (fixed v0.1 values) — confirmed directly
  in the construction block (`dcompile_dcode.rs` lines ~486-517).
- `artifact.validate()` genuinely invoked per artifact (the only place a
  bad `limits`/`ports` value is ever caught — confirmed by the module's
  own doc comment and by `GeneratedArtifactInvalid`'s existence).
- `descriptor_digest_sha256` always freshly recomputed, never trusted.
- Coverage is EXACT (`missing_service_dcode_input`/
  `unknown_service_dcode_input`) — every generated D-Serv must have
  exactly one D-Code input, and vice versa; this bundle-completeness rule
  is explicitly scoped to THIS function's own output claim, never
  generalized into a D-Graph validity rule (module doc: "Coverage is a
  D-Compile-CANDIDATE completeness rule, not D-Graph validity").
- Result order: `dcode_artifacts` follows the candidate D-Graph's own
  deterministic service order, never the caller's input vector order.

**Result: intact.**

## 9. Provenance collision (re-confirmed)

`c12_mapping_provenance_collision_same_dgraph_different_request_digest`
(`dcompile.rs`, unmodified) directly proves: two different, individually
valid D-Function→D-Serv partitions of the SAME D-Script produce the
IDENTICAL D-Graph and D-Graph digest, while their own request digests
correctly differ. `DCompileDGraphCandidate` therefore correctly carries
all three digests (D-Script, D-Compile-request, D-Graph) as three
separate fields — none collapsed. The public 117J response type preserves
this: `dscript_ref`, `mapping`+`dcompile_request_digest`, `dgraph_ref` are
three independently-named response fields (`dcompile_submission.rs`
§9-11).

**Result: confirmed, unweakened, and correctly surfaced at the production
API boundary.**

## 10. Five-identity model

All five identities are distinct fields in `DCompileCandidateResponseV1`
(never aliased): `dscript_ref.digest`, `dcompile_request_digest` (+ the
full `mapping`), `dgraph_ref.digest`, `dcode_artifacts[i].artifact.dcode.sha256`,
`dcode_artifacts[i].descriptor_digest`. Live-reconfirmed against Phase
117K's actual recorded evidence (`phase117k-...md` §10/§24, cross-checked
against this review's own re-reading — not re-executed, per task Section
47):

```
H1 -> H2: dscript_ref / dcompile_request_digest / dgraph_ref  -- ALL EQUAL
H1 -> H2: module sha256 / descriptor_digest                    -- BOTH DIFFERENT
```

**Result: confirmed live, matches the Phase 117D/E model exactly.**

## 11. Module-digest trust boundary

`dcompile_dcode.rs`'s own field doc comment on `sha256` states explicitly:
"D-Compile never receives module bytes and therefore never verifies this
digest describes real content." Grep across the phase confirms zero
occurrences of "byte verified"/"byte-verified" describing the DCompile-side
digest anywhere in source or the 117-series docs; the consistent phrase
used everywhere is "declared." Actual byte verification remains entirely
DNode-side: `DCodeModuleStore::install` (verified live in 117K — both
`smartsentinel-dcode-install` calls succeeded specifically because the
declared SHA matched the real installed bytes) and `invoke_dcode`'s own
`R3`/module-digest-mismatch checks (unmodified `dcode/invoke.rs`, still
regression-tested — 27 tests, confirmed passing in this session's own
`dserver`/`DATUM` runs, §23).

**Result: intact, unweakened.**

## 12. Immutable versioned registry

`DCodeArtifactRegistryStateV2` (`dcode_registry.rs`, unmodified since
117G): confirmed by direct source inspection —
`artifacts: BTreeMap<logical_key, BTreeMap<descriptor_digest, CanonicalDServArtifactV1>>`,
`declare_dserv_artifact_revision` only ever inserts (never replaces/
removes), `get_exact` is the only governed-path lookup, exact
redeclaration of identical content is idempotent (same digest key,
no-op), `get_logical_if_unambiguous` fails closed
(`dcode_artifact_lookup_ambiguous`) the instant a second revision exists.
Grep confirms **zero** occurrence of `latest_artifact`/`current_artifact`/
`latest_version`/`active_revision` anywhere in this file (§ earlier grep,
this session). `migrate_v1_state_to_v2` is pure/deterministic;
`validate_persisted_dcode_registry_integrity_v2` fails closed on any
digest-key/content mismatch. Live-reconfirmed in 117K: D1/D2 coexisted,
exact GET succeeded for both, legacy logical GET returned `409` the moment
a second revision existed.

**Result: confirmed, both by unit test and live evidence.**

## 13. Project scope

Registry key is `(application_id, dserv_id, descriptor_digest)` — no
`project_id` (117G's own evidence-based conclusion from ADR-0009, re-read
and re-confirmed by this review, not merely accepted). Content-addressing
via `descriptor_digest` already prevents cross-project collision: two
projects with byte-identical content safely share one entry; two projects
with different content get two different, non-colliding entries. Project
scoping lives entirely in DDeploy's own already-project-scoped
proposal/acceptance layer (`DDeployProposalV1.project_id`,
`ProposalInputPayload`'s own `X-Datum-Project` header requirement,
confirmed unchanged). No path in `dcode_registry.rs`/`dcode_authorization.rs`
reads or filters on `project_id` at all.

**Result: confirmed no cross-project active-authority leakage is
structurally possible — the registry has no project concept to leak.**

## 14. DDeploy v1 exact binding

`DDeployProposalV1.dcode_artifact_authority: BTreeMap<String, DCodeArtifactAuthorityBindingV1>`
(`ddeploy.rs`) — `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`,
so it participates in `compute_proposal_digest` (a real binding changes
the proposal digest — confirmed by
`adding_a_dcode_binding_changes_the_proposal_digest`) while a pre-117G/
empty-binding proposal's digest is byte-identical to its pre-117G form
(confirmed by `legacy_proposal_digest_is_unchanged_when_dcode_binding_absent`).
`validate_dcode_artifact_authority_bindings` runs at creation
(`api::ddeploy::post_create_proposal`, before persist) and again at
acceptance via `accept_proposal_with_dcode_binding_revalidation`
(skipped only for an idempotent replay of an ALREADY-accepted proposal —
confirmed by `g26_idempotent_replay_never_revalidates_dcode_binding`, which
succeeds even against an EMPTY registry). Binding never mutates the
registry (confirmed: `validate_dcode_artifact_authority_bindings` takes
`&DCodeArtifactRegistryStateV2`, an immutable reference, and neither this
function nor its callers ever call `declare_dserv_artifact_revision`).

**Result: confirmed — registration != activation, pending != activation
(`g23`), acceptance = the only activation transition (`g25`/`g27`).**

## 15. DDeploy v2 exact binding

`DDeployProposalV2` gained the field-for-field-identical
`dcode_artifact_authority` (same shared `DCodeArtifactAuthorityBindingV1`
type — confirmed, not a second divergent type). `accept_v2_proposal_with_dcode_binding_revalidation`
mirrors the v1 wrapper exactly, delegating to the unmodified
`accept_v2_proposal`. `v2_parity_registering_a_new_revision_does_not_change_the_active_v2_binding`
directly proves `resolve_active_dcode_binding` behaves identically whether
active authority is DDeploy/1 or DDeploy/2 — the `active.v2.is_some()`
branch is the ONLY v1/v2-specific code anywhere in the authorization path
(re-confirmed by direct source read of `resolve_active_dcode_binding` in
this session).

**Result: confirmed, both paths hardened identically.**

## 16. Proposal-digest semantics (correction preserved)

Re-read directly from `DDeployProposalV1`'s own field doc comment
(`ddeploy.rs`): explicitly states the 117F "digest collision" framing was
imprecise — `compute_proposal_digest` hashes `proposal_id`/
`created_at_utc` too, so two freshly-created proposals already digest
differently regardless of D-Code binding; the REAL gap this phase closed
was the total absence of a field committing to WHICH D-Code revision a
proposal intends. This corrected wording is present verbatim in current
source, not merely in a historical doc. No current code or comment
revives the inaccurate "digest collision" framing (grep-confirmed clean).

**Result: confirmed, correction preserved in source, not just docs.**

## 17. Active-binding single source of truth

`resolve_active_dcode_binding` (`ddeploy.rs`, re-read directly this
session): derives the binding ENTIRELY from `state.proposals`/
`state.phase113_v2.proposals` via `active.proposal_id` — there is no
second, independently-persisted "active DCode binding" field anywhere on
`ActiveApplicationDeployment`/`ActiveDMapV2`. This is the same
"derive-on-demand" pattern `ActiveDMapV2` already uses for
`diot_artifact_authority` (confirmed by direct comparison, not merely
asserted).

**Result: confirmed — one non-drifting source of truth, no duplicated
mutable pointer exists to drift.**

## 18. Authorization / currentness

`authorize_dcode_invocation` (`dcode_authorization.rs`, re-read directly):
requires an active DDeploy authority (`active_placement_authority`), the
service to exist in the active D-Graph snapshot with the sole legal ABI,
an exact placement on the requested node, node ABI capability, and then
resolves the artifact via `resolve_active_dcode_binding` → `get_exact`
(falling back to `get_logical_if_unambiguous` ONLY when no explicit
binding exists, failing closed the instant that becomes ambiguous —
`F_DCODE_ACTIVE_BINDING_MISSING`). Returned `DCodeInvocationAuthorizationV1`
preserves `dserv_artifact_digest`, `dserv_artifact_descriptor_digest`,
`dmap_ref`, `dgraph_ref` — all four independently re-confirmed live in
117K's evidence.

`verify_dcode_authorization_still_current` re-derives the identical chain
and fails closed on any drift.
`g29_rebinding_switches_authorization_once_and_stales_the_old_one`
directly proves an authorization issued under D1 becomes stale once active
authority moves to D2 — while D1 remains perfectly validly registered
throughout (staleness is caused by "active binding changed," never by "D1
stopped existing").

**Result: confirmed both by unit test and by live 117K evidence (§19,
§21 below).**

## 19. Governed DATUM client

Re-confirmed (this session, by direct grep) that `fetch_dserv_artifact`
(the legacy logical fetch) has **zero** production call sites remaining —
the only source hit is a comment in `smartsentinel-dcode-invoke.rs`
explicitly stating it is NOT used. All three production callers
(`smartsentinel-dcode-invoke`'s governed `Invoke` mode,
`smartsentinel-dcall-dispatch`, `dcall::delivery::execute_destination_delivery`)
use `fetch_governed_dcode_execution_material`, which performs: fetch
authorization → verify it corresponds to the caller's own request → fetch
the EXACT `descriptor_digest` authorization names →
independently recompute the descriptor digest and verify it equals the
authorization's own → verify module digest relation → return. No
"latest"/"first"/"newest" selection path exists anywhere in this
function (re-read directly this session, `DATUM/src/dcode/client.rs`).

**Result: confirmed no governed fallback exists — matches this session's
own 117H implementation, unmodified since.**

## 20. Legacy logical GET

`get_dserv_artifact` (`api/dcode.rs`): zero revisions → `404`; exactly one
→ `200` with that revision; more than one → `409`
`dserv_artifact_lookup_ambiguous` (never a guess). Doc comment on the
handler explicitly labels it "NOT suitable for governed invocation."
Never called by any governed production client (confirmed §19). Live
re-confirmed in 117K (§16/§23 of that report).

**Result: confirmed.**

## 21. Production DCompile API

`POST /api/v1/dcompile/compile` (`api/dcompile.rs`, `dcompile_submission.rs`).
`DCompileSubmissionV1` contains: `schema`, `dscript`, `output_dgraph_id`,
`output_dgraph_revision`, `services`, `dcode_services` — confirmed by
direct struct read, `#[serde(deny_unknown_fields)]`. No `dscript_ref`, no
`dgraph_ref`, no digest field of any kind, no `application_id` at the
outer level (read once from `dscript.application_id`), no `project_id`,
no placement/D-Continuum/node/D-Map field. `j18`/`j25`/`j46` (re-read this
session) directly prove an unknown field, a supplied `dgraph_ref`, and any
project/placement field are each either a hard deserialization failure or
structurally impossible to express.

**Result: confirmed — caller cannot forge a derived reference because the
type itself has no field to carry one.**

## 22. Server-derived reference construction

`compile_submission` (re-read directly, `dcompile_submission.rs`): Step 1
`validate_and_digest_dscript` (typed, no JSON round-trip). Step 2
constructs `DCompileRequestV1` with `dscript_ref` built from Step 1's own
`document.dscript_id`/`revision`/`digest_sha256` — never caller data —
then `validate_and_digest_dcompile_request` (typed). Step 3: a
PRELIMINARY call to `lower_calls_empty_dgraph_candidate`, solely to obtain
the fresh D-Graph identity needed for `dgraph_ref`; this candidate is
discarded. Step 4 constructs `DCompileDCodeInputV1` with that server-derived
`dgraph_ref`. Step 5 calls `lower_dcode_artifact_candidates` (117E,
unmodified) — which internally re-runs `lower_calls_empty_dgraph_candidate`
a SECOND time as part of its own pre-existing, unmodified 117E trust
boundary. `preliminary_dgraph_lowering_equals_final_bundles_dgraph_candidate`
(re-read) proves both calls, given the same validated inputs, produce an
identical result — the repetition is provably a no-op duplication of a
pure computation, never a source of drift, and 117E's defense is never
weakened.

No JSON serialize/decode round-trip of already-typed data exists anywhere
in this path (confirmed: `validate_and_digest_dscript`/
`validate_and_digest_dcompile_request` both take the TYPED struct
directly, not a JSON string).

**Result: confirmed, exactly as designed in 117I and implemented in 117J.**

## 23. Wire/internal separation

Confirmed by direct grep this session:
`DCompileDGraphCandidate`/`DCompileCandidateBundle`/
`DCompileDServArtifactCandidate` derive `Debug, Clone, PartialEq, Eq` only
— no `Serialize`/`Deserialize` anywhere in their `#[derive(...)]`
attributes (`dcompile.rs`, `dcompile_dcode.rs`). `DCompileSubmissionV1`/
`DCompileCandidateResponseV1`/`DCompileArtifactResultV1` are new,
independent, purpose-built wire types (`dcompile_submission.rs`) that
`build_response` constructs BY MAPPING FROM the internal `DCompileCandidateBundle`
— never by attaching serde to it. This separation remains clean after
117J: the compiler's internal representation can still evolve without
silently becoming a protocol-breaking change.

**Result: confirmed clean.**

## 24. Response determinism

`canonicalize_mapping` (re-read): sorts `services` by `service_id` and
each service's `function_ids`, applied to the response's `mapping` field
before serialization. `dgraph`/`dcode_artifacts` already inherit
deterministic ordering from 117C/117E's own construction (sorted by
`service_id`). `dcode_artifact_authority` is a `BTreeMap` (deterministic
iteration/serialization order by construction — never a `HashMap`).
`j26`-`j32` (re-read, all passing per §23 of this review's own test run)
directly prove: identical submission twice → byte-identical JSON;
D-Script-function reorder, mapping-service reorder, function_ids reorder,
DCode-service-input reorder, and a COMBINED reorder of all four → each
produces a byte-identical serialized response to the canonical baseline.

**Result: confirmed — no meaningless caller ordering can leak into the
wire response.**

## 25. Zero-transformation handoff

Confirmed directly from `dcompile_submission.rs`'s own live-passing tests
(re-run this session, §23):
`j36_j37_compile_artifact_directly_declares_and_digest_matches` — every
`dcode_artifacts[i].artifact` POSTs successfully, UNMODIFIED, to the real
`declare_dserv_artifact_revision`, and the returned digest matches
`descriptor_digest` exactly.
`j38_response_binding_validates_against_registered_revisions` — the
response's own `dgraph`/`dcode_artifact_authority`, UNMODIFIED, pass the
real `validate_dcode_artifact_authority_bindings`.
`j39_j40_j41_local_compile_register_propose_accept_binds_the_exact_compiler_produced_digest` —
the FULL local chain (compile → declare → `create_proposal` with the
response's `dgraph`/`dcode_artifact_authority` passed through completely
unmodified → `accept_proposal_with_dcode_binding_revalidation` → confirm
the newly active binding is EXACTLY the compiler-produced digest via
`resolve_active_dcode_binding`) succeeds with zero adaptation layer. This
was then INDEPENDENTLY re-proved live against a real DServer/RPi in Phase
117K (§17 below).

**Result: confirmed twice — once deterministically (this test), once
live (117K).**

## 26. ServiceArtifact bridge — closure-critical question, answered precisely

Traced directly from `ddeploy.rs` source this session (`resolve_artifact_for_service`,
`check_node_eligibility`, `derive_application_operational_services`,
`validate_full`):

**What authority does `ServiceArtifact` contribute?** Exactly two things,
both PRE-DATING Phase 117 entirely (ADR-0002/ADR-0005, Stage 107A/111
architecture): (1) proposal/acceptance-time PLACEMENT ELIGIBILITY — does
`control_plane.target_nodes` include the placement's node? — checked once,
at proposal-validation time, never again after acceptance; (2) the
`runtime.kind` string feeding `derive_application_operational_services`'s
operational desired-state projection (DMonitor/operational-reconciliation-
facing metadata: what kind of runtime the operational graph reports).

**Does it affect DCode executable identity?** No. Direct source
inspection of `ServiceArtifact`'s own type (via `service_artifact_registry.rs`,
unchanged by Phase 117) confirms it has no `dcode`/`sha256`/
`descriptor_digest` field anywhere, and neither
`resolve_artifact_for_service` nor `check_node_eligibility` nor
`derive_application_operational_services` reads or writes anything related
to `DCodeArtifactAuthorityBindingV1`/`dcode_registry`/
`dcode_authorization`. The entire D-Code activation chain
(`dcode_artifact_authority` → `resolve_active_dcode_binding` →
`authorize_dcode_invocation` → `get_exact`) is structurally independent of
`ServiceArtifact` — confirmed by the fact that neither function appears
anywhere in either call chain.

**Could it silently choose H1/H2?** No — it has no visibility into D-Code
descriptor identity at all; it cannot express, reference, or influence
which descriptor digest a service is bound to.

**Is this required because DDeploy has two parallel artifact lineages,
or because DCompile output is incomplete?** The former, confirmed
directly: `ServiceArtifact`'s eligibility gate is a pre-existing,
structurally-separate DDeploy proposal-validation requirement that exists
independent of and unaffected by D-Code — Phase 117F's own audit (re-read
this session) already traced this exact asymmetry and found
`ServiceArtifact` avoids the H1→H2 problem through a completely different
mechanism (a genuinely mutable registry with live per-cycle
re-validation), not because DCompile's own output is missing anything.
DCompile's own output (D-Graph + D-Code descriptors + binding map) is
fully sufficient, on its own, to drive D-Code registration and D-Code
activation — 117K's live proof used ONLY compiler output for every D-Code-
related step; `ServiceArtifact` was declared purely to satisfy DDeploy
v1's own, orthogonal proposal-creation gate, and Phase 117K's own
documentation states this explicitly.

**Is removing this bridge a Phase 117 requirement?** No — converging
`ServiceArtifact`'s mutable-registry eligibility lineage with the
immutable D-Code registry lineage (if ever desired) is a DDeploy/runtime-
convergence concern for a future, separate phase; Phase 117's own
architecture never assigned that convergence to DCompile, and closing
Phase 117 does not require it.

**Answer: A — a known, legitimate, pre-existing downstream DDeploy
compatibility bridge, outside DCompile's responsibility. Non-blocking.**

## 27. Live H1/H2 evidence review (117K, independently re-checked)

Re-read `phase117k-live-production-dcompile-h1-h2-proof.md` in full this
session and cross-checked its recorded digests for internal consistency
(not re-executed, per task Section 47):

- H1 SHA `ee9fab27...a1ff51` != H2 SHA `4a0fef0f...9dc41` — consistent
  throughout every recorded step (compile responses, registration
  responses, authorizations, evidence files).
- D1 descriptor `sha256:4d13055e...81c1` != D2 descriptor
  `sha256:8fbd19c5...4dd7f8` — consistent throughout.
- `dscript_ref`/`dcompile_request_digest`/`dgraph_ref` are IDENTICAL
  between the recorded H1 and H2 compile responses — directly supports
  §9/§10 above with real, not merely unit-tested, evidence.
- DMap transition recorded as revision `1` (digest `33d583de...becceb`) →
  revision `2` (digest `3cc80a33...7003d8`) on the SAME `dmap_id` —
  consistent with an acceptance-only transition, not a new deployment.
- Authorization transition: `authorization_id` `...:1` (both the initial
  H1 authorization AND the pre-P2 authorization share this SAME id) →
  `...:2` only after P2's acceptance — directly supports the P0 proof
  that registering+installing D2 changed nothing until acceptance.
- `output_reason_sha256` values recorded for the H1 and H2 evidence match
  the independently-computed `SHA256("phase117k-v1")`/
  `SHA256("phase117k-v2")` exactly, and differ from each other — this is
  the strongest evidence in the whole phase: real WASM execution on a
  real RPi produced the exact, independently predicted output for each
  distinct module revision.

The recorded sequence genuinely supports: D1 active → D2 registered → H2
bytes installed → D1 STILL active (two independent checks: authorization
snapshot + a second real invocation) → P2 accepted → D2 active → H2
executed (evidence hash confirms real bytes ran).

**Result: confirmed consistent, no internal contradiction found.**

## 28. Security invariants S1-S14

| # | Invariant | Result |
|---|---|---|
| S1 | Caller cannot fake derived DScript/DGraph refs in production API | **HOLD** — no such field exists on `DCompileSubmissionV1` (§21) |
| S2 | DCompile does not activate authority | **HOLD** — `compile_submission` takes no registry/DDeploy-state parameter (`j42`-`j44`) |
| S3 | Artifact registration does not activate executable | **HOLD** — `declare_dserv_artifact_revision` only inserts (§12); live-confirmed 117K §17 |
| S4 | Byte installation does not activate executable | **HOLD** — live-confirmed 117K §17 (H2 bytes installed, D1 still active) |
| S5 | Pending proposal does not activate executable | **HOLD** — `g23`/117G test suite; live-inferred 117K (disclosed honestly, §29 below) |
| S6 | Only accepted DDeploy transition changes active DCode revision | **HOLD** — `g27`; live-confirmed 117K (P2 acceptance is the only state change that moved authorization from D1 to D2) |
| S7 | Authorization uses exact active descriptor | **HOLD** — §18; live-confirmed |
| S8 | Governed client fetches exact authorized descriptor | **HOLD** — §19; live-confirmed (117K's invoke CLI runs) |
| S9 | Module store verifies actual bytes against module SHA | **HOLD** — unmodified `DCodeModuleStore::install`, live-confirmed 117K (both installs succeeded because declared SHA matched real bytes) |
| S10 | Descriptor digest and module digest remain distinct | **HOLD** — §10/§14; live-confirmed (117K's H1/H2 module digests differ from their own descriptor digests, and 117D's original module-vs-descriptor distinction is unmodified) |
| S11 | Old descriptor remains historical after replacement | **HOLD** — live-confirmed 117K §22 (D1 still 200 after H2 active) |
| S12 | Old authorization becomes stale after governed redeploy | **HOLD**, with an honest qualification — see §32 |
| S13 | No "latest wins" path exists | **HOLD** — §12 grep-confirmed zero occurrences |
| S14 | No project-crossing active authority leak exists | **HOLD** — §13, registry has no project concept |

**All 14 hold. Zero violations found.**

## 29. Authority ownership matrix

| Concern | Canonical owner | Phase 117 effect | Activation authority? |
|---|---|---|---|
| Source identity | D-Script | Produces/digests it | No |
| Compile mapping | D-Compile-request | Produces/digests it | No |
| Topology | D-Graph | Produces candidate only | No |
| Executable descriptor | `CanonicalDServArtifactV1` | Produces candidate only | No |
| Registered descriptor revision | D-Code registry | Never registers itself | No |
| Active executable revision | D-Deploy (accepted binding) | Never activates | **No — exclusively D-Deploy's** |
| Placement | D-Map / planner | Never touches | No |
| D-Node structural capability | D-Node registry | Never touches | No |
| Installed bytes | D-Node `DCodeModuleStore` | Never touches | No |
| Invocation authorization | `dcode_authorization` | Never touches | No |
| Observed runtime evidence | D-Monitor | Never touches | No |

**Every "activation authority?" answer is No except one, and that one is
explicitly D-Deploy's, not DCompile's — this is Phase 117's exact,
correctly-drawn boundary.**

## 30. V1 limitations matrix

| Limitation | Classification |
|---|---|
| Exact-partition-only D-Function↔D-Serv mapping (no true 1:N/M:N overlap) | **CURRENT V1 CONTRACT** — §7 below |
| `calls = []` always | **CURRENT V1 CONTRACT** — §8 below |
| External module build (no source→WASM codegen) | **CURRENT V1 CONTRACT** — §6 below |
| No source-to-WASM correctness proof | **CURRENT V1 CONTRACT** (structural, honestly disclosed) |
| No automatic byte distribution | **DOWNSTREAM CONCERN** (DNode/deployment readiness, not DCompile) |
| No persistent compile history | **KNOWN DEFERRED FEATURE** — §34 below |
| No compile-result digest | **CURRENT V1 CONTRACT** (117I re-evaluated and rejected inventing one; conditioned on the stateless model, §13 of 117I) |
| Transitional `ServiceArtifact` bridge | **DOWNSTREAM CONCERN** — §26 above |
| No stage authority for non-empty `allowed_stages` | **DOWNSTREAM CONCERN** (pre-existing D-Node-structural-authority gap, unrelated to this phase, unchanged) |
| Stateless/fresh-instance-only D-Code | **CURRENT V1 CONTRACT** (ADR-0021, unmodified) |
| Sole ABI `datum-dnode/0` | **CURRENT V1 CONTRACT** (no second ABI exists to choose between) |

**None of these are CLOSURE BLOCKERS.** Every one is either an honestly-
scoped v1 contract limitation with no evidence of a broader existing
requirement, or a concern properly owned by a different, later phase.

## 31. Backward compatibility

- Historical (pre-117G) proposal digests remain byte-identical when the
  D-Code binding field is empty (`skip_serializing_if`), confirmed by
  `legacy_proposal_digest_is_unchanged_when_dcode_binding_absent`
  (re-read this session).
- v1 registry state migrates deterministically to v2
  (`migrate_v1_state_to_v2`, pure, content-preserving, confirmed §12).
- DDeploy v1/v2 both hardened identically (§14/§15).
- DIoT authority semantics (`diot_artifact_authority`,
  `resolve_diot_artifact_authority`) are untouched by Phase 117 — grep-
  confirmed: no Phase-117 commit modifies DIoT's own resolution logic; the
  only DIoT-adjacent files touched are mechanical struct-literal additions
  in unrelated test fixtures (§22 of this review, §3 table).

**Result: confirmed intact.**

## 32. Incidental file changes

Re-confirmed by direct diff inspection this session (not merely restated
from prior phase reports): `artifact_reconciliation_context.rs`,
`dforward_dependency_readiness.rs`, `dforward_projection.rs`,
`diot_runtime_binding.rs`, `dmonitor_interpretation.rs`,
`dmonitor_readiness.rs` each changed for EXACTLY one mechanical reason —
adding `dcode_artifact_authority: std::collections::BTreeMap::new(),` to a
pre-existing test-fixture struct literal that could no longer compile once
`CreateProposalInput`/`DDeployProposalV1` gained the new required field.
No logic in any of these six files changed. `ddeploy_v2.rs`'s only real
change is the semantic `dcode_artifact_authority` field addition to
`DDeployProposalV2` itself — directly in-scope for 117G's own "both v1 and
v2" requirement. `core/dcall.rs`/`core/dcall/tests.rs`/`api/dcall.rs`
changed to replace a flat, now-removed `.get()` registry call with
`.get_exact()` using a digest the SAME call's own `authorize_dcode_invocation`
result already proved — directly in-scope for 117G's registry
versioning propagating consistently to every caller of the old flat API,
not a DCall feature change.

**Result: confirmed — all incidental changes are compile/exhaustiveness
adaptations or directly in-scope registry-versioning propagation; none
introduces unrelated feature scope.**

## 33. Stale documentation/terminology findings

- `DCHECK` (the forbidden typo): **zero occurrences** anywhere in
  Phase-117 source or docs (grep-confirmed this session).
- "no production consumer"/"not yet implemented": **zero occurrences**
  remaining in current `dcompile*.rs`/`canonical_dscript.rs` (117J already
  corrected the three that existed — re-confirmed clean this session, not
  merely trusted from 117J's own report).
- No other stale current-state claim was found in the reviewed files.

**Result: clean.**

## 34. Tests/builds (all re-run in this session, not carried forward)

```
cargo test --manifest-path dserver/Cargo.toml --locked
  -> 1859 passed; 0 failed; 0 ignored   (+ 1 passed; 0 failed in control-state-inspect)

cargo test --manifest-path DATUM/Cargo.toml --lib --locked
  -> 1229 passed; 0 failed; 1 ignored

cargo test --manifest-path DATUM/Cargo.toml --locked
  -> 447 + 6 + 10 + ... = 490 passed total across all binaries/integration
     tests; 0 failed

cargo build --manifest-path dserver/Cargo.toml --locked --release
  -> succeeds (cached, no source change since 117K)

cargo build --manifest-path DATUM/Cargo.toml --locked --release
  -> succeeds (cached, no source change since 117K)

git diff --check ff72b2b6..HEAD   -> clean
git diff --check (working tree)   -> clean
```

**All results match the expected baselines exactly. No discrepancy, no
manipulation of expectations to fit prior reports.**

## 35. Closure blockers search

Explicitly checked against every example blocker listed in the task's own
Section 49:

- Production API forgeable derived references — **not found** (§21).
- Compiler output requires semantic reconstruction before hand-off —
  **not found** (§25, proved twice: unit test + live).
- Registering/installing a new descriptor silently alters active
  execution — **not found**; the opposite was live-proved (§27, S3/S4).
- Active authorization can resolve something other than the exact
  accepted binding — **not found** (§18, S7).
- Registry multi-version state can drift or choose "latest" — **not
  found** (§12, S13).
- Live proof did not actually execute compiler-produced descriptors —
  **not found**; 117K's compile responses were POSTed unmodified at every
  step (§27).
- DCompile owns placement/runtime state it should not own — **not
  found** (§39/§29 matrix).
- Persisted-state migration corrupts existing authority — **not found**
  (§12/§31).
- Project reuse leaks active artifact authority across projects — **not
  found** (§13/§14 of security invariants).

**Zero closure blockers found.**

## 36. Deferred / non-blocking items (complete list)

1. No source-to-WASM compiler/codegen (DCompile binds D-Functions/D-Servs
   to externally-produced executable content identity; it does not prove
   that content was generated FROM those D-Functions — §6 below).
2. Exact-partition-only D-Function↔D-Serv mapping — the vocabulary's full
   1:N/M:N overlap case remains explicitly undefined pending real
   evidence (§7 below).
3. `calls = []` always — no D-Call derivation exists or was added; D-Call
   remains Phase 112's own, separate concern (§8 below).
4. No automatic module-byte distribution (manual `scp` +
   `smartsentinel-dcode-install` only) — a later DNode-readiness/
   deployment concern, not DCompile's.
5. No server-side persisted compile-result history — compile provenance
   is returned in full to the caller but not stored; DDeploy never
   consults it as authority (it always independently recomputes and
   compares) — §34 below.
6. No compile-result identity/digest — 117I's own re-evaluated,
   evidence-based rejection stands; nothing under the current stateless
   model is keyed by one.
7. Transitional `ServiceArtifact` eligibility bridge in DDeploy v1 — a
   pre-existing, orthogonal DDeploy lineage, not a DCompile gap (§26/§29).
8. No canonical stage authority for non-empty `allowed_stages` — a
   pre-existing D-Node-structural-authority gap, unrelated to and
   unchanged by this phase.
9. Stateless, fresh-instance-per-invocation D-Code only; sole legal ABI
   `datum-dnode/0` — both are the actual v0.1 architecture, not
   incomplete implementations of a richer spec.

**None of the above blocks closure.**

## 37. Phase 117 final closure decision

**Does current Phase 117 provide a coherent, production-usable
implementation of explicit DScript mapping → deterministic DGraph
candidate + deterministic canonical DCode descriptor candidates →
production API → zero-transformation registry/DDeploy handoff, while
preserving all downstream authority boundaries? YES** — confirmed across
§4-§26 above, independently re-derived from current source and tests, not
accepted on the strength of prior phase claims.

**Did Phase 117 live-prove the critical DCode revision lifecycle on a
real DNode? YES** — confirmed by independent re-reading and internal-
consistency cross-check of Phase 117K's recorded evidence (§27), with two
explicit, honest qualifications preserved rather than overstated (§38-39
below).

**Both are YES. Phase 117 — DCompile — is CLOSED.**

## 38. Pending-P2 evidence qualification (preserved honestly)

Phase 117K explicitly disclosed it did not perform a separate live
invocation strictly between P2's creation and its acceptance (P2 was
created and accepted in immediate succession). This review does **not**
claim that exact row was live-probed. The "Pending P2" state is
sufficiently established by combining: (a) the §17 pre-P2-CREATION live
proof (fresh authorization + real invocation, both still D1/H1, taken
before P2 existed at all) with (b) Phase 117G's own already-established,
independently unit-tested guarantee that `create_proposal` never touches
`active_by_application` (e.g. `g23_pending_proposal_with_binding_activates_nothing`,
re-confirmed passing in this session's own test run). **This combination
is sufficient for closure** — stated explicitly, not silently assumed.

## 39. Old-authorization staleness qualification (preserved honestly)

Phase 117K did not call a public "verify-old-authorization" HTTP endpoint
because none exists, and none was added. Closure relies on: two real live
authorization snapshots (before/after P2's acceptance, showing a changed
`authorization_id`/`dmap_ref`/descriptor digest) plus the existing,
focused, deterministic `verify_dcode_authorization_still_current` test
suite (re-confirmed passing, unmodified since prior phases) proving that
exact re-derivation-and-comparison function returns "stale" under this
exact condition. **This is not rewritten as a live API proof of staleness
— it is stated precisely as what it is.**

## 40. Module distribution — explicit closure decision

Manual `scp` + the existing production `smartsentinel-dcode-install`
binary is the only provisioning path exercised or implied. **This is
correctly a later DNode-readiness/deployment-automation concern, not a
Phase 117 (DCompile) responsibility** — DCompile never claims to install
or distribute bytes; ADR-0021 and the 117D/E docs (both re-read this
session) consistently scope module distribution outside DCompile from
the very beginning of this phase line. **Non-blocking.**

## 41. Provenance persistence — explicit closure decision

Compile provenance (all five identities) is returned in full to the
caller but not persisted server-side (confirmed: `compile_submission`
takes no storage parameter, §25/§36). DDeploy's own authority never
consults compile provenance as a trust input — `validate_dcode_artifact_authority_bindings`
independently recomputes the descriptor digest from the ACTUAL registered
artifact and compares; `create_proposal`/`create_v2_proposal`
independently recompute `recompute_dgraph_digest` from the ACTUAL supplied
D-Graph. Provenance is explanatory/audit evidence only, never activation
authority. **Classified as a deferred enhancement, non-blocking** — not
claimed as implemented.

## 42. Phase 118 boundary

Phase 117 (DCompile) closes having delivered: DScript → explicit mapping
→ deterministic D-Graph/D-Code candidate lowering → a real, deterministic,
side-effect-free production HTTP entry point → a proven zero-transformation
hand-off into the pre-existing, independently-hardened D-Code registry and
D-Deploy governed-authority chain → a real, live, RPi-executed proof of
the full H1→H2 governed redeploy lifecycle.

**Phase 118 (D-Engine) begins from this closed boundary.** Per the macro
roadmap (117 = DCompile, 118 = D-Engine), Phase 118 may now address
D-Engine-level concerns (e.g. real D-Call derivation/topology evidence,
DCompile-to-D-Engine orchestration, or whatever D-Engine's own scope
turns out to require) WITHOUT retroactively treating any of §36's deferred
items as if they were unmet Phase-117 requirements — this review
explicitly declines to let D-Engine-shaped concerns (D-Call derivation,
automatic distribution, persisted provenance) become retroactive
Phase-117 blockers, per the task's own Section 51 instruction.

## 43. Evidence index

- Current source (re-read directly, this session):
  `dserver/src/core/canonical_dscript.rs`, `dcompile_request.rs`,
  `dcompile.rs`, `dcompile_dcode.rs`, `dcompile_submission.rs`,
  `dcode_registry.rs`, `ddeploy.rs`, `ddeploy_v2.rs`,
  `dcode_authorization.rs`, `dserver/src/api/dcompile.rs`, `dcode.rs`,
  `ddeploy.rs`, `dcall.rs`; `DATUM/src/dcode/client.rs`.
- Tests re-run: full `dserver`/`DATUM` regression suites (§34).
- Live evidence re-read and cross-checked:
  `documentation/reference/phase117k-live-production-dcompile-h1-h2-proof.md`.
- Prior phase reports read as claims+rationale, not as proof:
  `phase117a`...`phase117j` (all 10 prior reference docs).
- Git evidence: `git log`/`git diff --stat`/`git diff --check` over the
  full `ff72b2b6..HEAD` range and the working tree.
