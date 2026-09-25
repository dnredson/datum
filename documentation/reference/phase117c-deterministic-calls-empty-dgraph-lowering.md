# Phase 117C — Deterministic Calls-Empty Candidate DGraph Lowering

**Status:** Phase 117C CLOSED. **Phase 117 remains OPEN.** This is the first
actual D-Compile lowering step. No D-Call derivation, no D-Code generation,
no DDeploy integration, no placement, no runtime execution.
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117B closure, commit `84a3d32b054f06b3b7f17c93ae632c8f2693c61e`.

## Exact lowering function

`dserver/src/core/dcompile.rs`:

```rust
pub fn lower_calls_empty_dgraph_candidate(
    dscript: &ValidatedDScriptInput,
    request: &ValidatedDCompileRequest,
) -> Result<DCompileDGraphCandidate, DCompileLoweringError>
```

Pure: no I/O, no global state, no timestamps, no random/generated IDs. Every
identity in the output (`dgraph_id`, `revision`, `service_id`s) is copied
verbatim from the explicit Phase 117B inputs. The one exception is
`dnode_abi`, assigned the sole legal v0.1 D-Node ABI explicitly.

## Input trust/revalidation decision (P0)

`ValidatedDScriptInput`/`ValidatedDCompileRequest` both have fully **public**
fields, confirmed by direct inspection — nothing prevents ordinary Rust code
from constructing either with a fabricated `digest_sha256`, entirely
bypassing `decode_and_validate_dscript_json`/`decode_and_validate_dcompile_request_json`.
**These wrapper types are therefore never trusted as self-certifying.**
Before constructing anything, `lower_calls_empty_dgraph_candidate` performs,
in order:

1. `dscript.document.validate()` — must be empty.
2. Recompute the D-Script digest and require it equal `dscript.digest_sha256`.
3. `request.request.validate()` — must be empty.
4. Recompute the request digest and require it equal `request.digest_sha256`.
5. `request.request.validate_against_dscript(dscript)` — must be empty.

Only after all five pass does construction begin. This mirrors this
project's established "recompute and compare, never trust cached/
self-asserted identity" discipline (Stage 107A lease/authorization
revalidation; `execution_projection`'s digest recomputation) rather than
inventing a new pattern.

**Changing `Validated*`'s field visibility to make fabrication impossible at
the type level was considered and rejected.** It would be an unrequested
redesign of a frozen prior-phase (116/117B) contract; this function's own
defensive revalidation is the smaller, sufficient, and already-precedented
fix, and does not touch either prior contract's public API.

## Error model

```rust
pub enum DCompileLoweringError {
    InvalidDScript(Vec<DScriptValidationFinding>),
    DScriptDigestMismatch { wrapper_claimed: String, recomputed: String },
    InvalidDCompileRequest(Vec<DScriptValidationFinding>),
    DCompileRequestDigestMismatch { wrapper_claimed: String, recomputed: String },
    RelationalMismatch(Vec<DScriptValidationFinding>),
    GeneratedDGraphInvalid(Vec<DGraphValidationFinding>),
    DigestComputationFailed(String),
}
```

Seven distinct classes, matching the task's own required minimum exactly.
Two nominally-distinct-but-structurally-identical `CanonicalValidationFinding`
types exist in this codebase (`canonical_dscript`'s own, which
`dcompile_request.rs` imports and reuses for its own `validate()`/
`validate_against_dscript()`; and `canonical_dgraph`'s own, returned by
`CanonicalDGraphV1::validate()`) — real findings are preserved verbatim in
each variant using the correct source type (aliased `DScriptValidationFinding`/
`DGraphValidationFinding` on import), never paraphrased into a string.

## Result type

```rust
pub struct DCompileDGraphCandidate {
    pub dgraph: CanonicalDGraphV1,
    pub dgraph_digest_sha256: String,
    pub dscript_digest_sha256: String,
    pub dcompile_request_digest_sha256: String,
}
```

Deliberately not a wire contract: no `Serialize`/`Deserialize`, no
`datum.dcompile-result/1` schema invented — there is no proven persistence/
API consumer yet (Phase 117F may define one once DDeploy integration is
real). No timestamp field — same inputs must always produce the same
result. Never described as "accepted"/"current"/"active" anywhere in its
own documentation.

## Generated CanonicalDGraphV1 mapping

| Output field | Source |
|---|---|
| `schema` | `CANONICAL_DGRAPH_SCHEMA` constant |
| `application_id` | `dscript.document.application_id` |
| `dgraph_id` | `request.request.output_dgraph_id` |
| `revision` | `request.request.output_dgraph_revision` |
| `services[].service_id` | `mapping.service_id` |
| `services[].execution_requirement.dnode_abi` | `DNODE_ABI_V0` (the sole legal v0.1 ABI, assigned explicitly — never called a "default"; there is no second value to default away from) |
| `services[].resource_requirement` | `mapping.resource_requirement`, copied exactly — never normalized, scaled, summed, clamped, or inferred |
| `calls` | `Vec::new()` unconditionally |

`request.request.services[].function_ids` is consulted ONLY during Step 5's
relational validation; the returned D-Graph cannot and does not encode
D-Function membership at all (see the provenance section below).

## Service ordering rule

Generated `services` are sorted by `service_id` before the candidate is
constructed — not merely before digesting (as `recompute_dgraph_digest`
already does), but as part of the returned `CanonicalDGraphV1` value itself.
Consequently two semantically-equivalent requests differing only in
`services` vector order produce not merely the same digest but an
**identical** `CanonicalDGraphV1` value (`c4_service_vector_reorder_produces_identical_dgraph_and_digest`).
Function-ID order within a service has zero effect for the same reason —
function IDs never appear in the output at all
(`c5_function_id_reorder_within_service_produces_identical_dgraph_and_digest`).

## ABI assignment

`DNODE_ABI_V0` ("datum-dnode/0") is assigned to every generated service
explicitly, in code, every time — never behind a `Default` impl, never
described in comments or documentation as a "default." Phase 117A
established it is the sole legal v0.1 ABI; Phase 117B deliberately omitted
an ABI selector from the request for exactly this reason. Proven by
`c9_every_service_gets_dnode_abi_v0`.

## calls-empty behavior

`calls: Vec::new()` unconditionally, with no synthetic sequential/dependency/
pipeline/self/function-order edges ever constructed. Zero calls means
DCompile currently knows no communication edges — it does not mean services
cannot communicate under some future, richer compile input. Proven by
`c10_calls_are_always_empty`.

## Empty DScript behavior

An empty D-Script (`functions = {}`) with an empty request (`services = []`)
lowers deterministically to `CanonicalDGraphV1 { services: [], calls: [] }`
carrying the request's explicit output identity, and this candidate
validates cleanly — no artificial error is raised merely because the
application is empty (`c7_empty_dscript_and_empty_request_lowers_to_empty_valid_graph`).

## Generated DGraph self-validation

After construction, `dgraph.validate()` is called and must return no
findings before the candidate is ever returned — defense in depth, using
the single existing canonical validator (`CanonicalDGraphV1::validate()`),
never a re-derived rule set. This should be structurally unreachable given
already-validated inputs (proven never to fire spuriously across every
successful-path test via `c24_generated_dgraph_self_validation_is_invoked_on_every_success_path`,
which independently re-runs `dgraph.validate()` on every produced
candidate and confirms it is genuinely valid, not merely assumed valid
because lowering returned `Ok`).

## Digest handling

`dgraph_digest_sha256` is always freshly computed via
`recompute_dgraph_digest`, never trusted from any input — proven directly
(`dgraph_digest_matches_independent_recomputation`). `dscript_digest_sha256`/
`dcompile_request_digest_sha256` on the result are the wrapper's own
already-independently-verified digests (verified in Steps 2/4 above), also
proven to match independent recomputation
(`request_and_dscript_digests_match_independent_recomputation`).

## DScript / request / DGraph provenance distinction (central finding)

`CanonicalDGraphV1` does not, and structurally cannot, record which
D-Function belongs to which D-Serv — Phase 116/117A both established this.
Consequently:

- **DScript digest** = exact source-contract identity (including opaque
  annotations).
- **DCompile-request digest** = exact compile mapping/resource/output-identity
  intent — the **only** one of the three that captures D-Function → D-Serv
  membership.
- **DGraph digest** = exact generated application topology, and *only* that.

These answer three different questions and must never be collapsed into
one. `DCompileDGraphCandidate` carries all three precisely so a caller never
loses the distinction by discarding the request after lowering.

## Mapping-provenance collision test result (the central proof)

`c12_mapping_provenance_collision_same_dgraph_different_request_digest`:
one D-Script with functions `{A, B}`; request R1 maps `svc-a←A, svc-b←B`;
request R2 maps `svc-a←B, svc-b←A` (identical application/output identity/
resources otherwise). Both are individually valid exact partitions. Result,
exactly as predicted:

- `R1.digest_sha256 != R2.digest_sha256` (different mappings, different
  request digests — confirmed).
- `lower(R1).dgraph == lower(R2).dgraph` (the generated D-Graph cannot
  distinguish them — confirmed).
- `lower(R1).dgraph_digest_sha256 == lower(R2).dgraph_digest_sha256`
  (confirmed).
- `lower(R1).dcompile_request_digest_sha256 != lower(R2).dcompile_request_digest_sha256`
  (the distinction survives in the lowering *result*, even though it is
  lost in the D-Graph itself — confirmed).

**Conclusion, stated plainly: a D-Graph digest alone can never serve as
proof of which D-Compile mapping produced it.** Any future persistence or
audit mechanism (Phase 117F+) that cares about mapping provenance must
retain the D-Compile-request digest (or the request itself) alongside the
D-Graph, never substitute the D-Graph digest for it.

## Annotation/topology-independence result

`c11_annotation_changes_do_not_affect_generated_dgraph`: two D-Scripts with
identical function IDs but different opaque annotations produce different
D-Script digests (confirmed — annotations DO change D-Script identity) and,
via requests correctly referencing each D-Script's own real digest, identical
candidate D-Graphs and D-Graph digests (confirmed). Annotations are never
read anywhere in this module — they have zero D-Graph-lowering semantics in
DCompile v1, not because they are meaningless overall, but because ADR-0025
forbids interpreting them and no evidence assigns them any lowering role.

## Defensive forged-wrapper tests (C13–C23)

All implemented and passing: wrong D-Script digest (`C13`), structurally
invalid D-Script document smuggled via a forged wrapper (`C14`), wrong
request digest (`C15`), structurally invalid request smuggled via a forged
wrapper (`C16`), application/D-Script-ID/revision/digest relational
mismatches (`C17`–`C20`), unknown mapped function (`C21`), unmapped D-Script
function (`C22`), and an overlapping mapping smuggled through a forged
`ValidatedDCompileRequest` (`C23`) — caught by `DCompileRequestV1::validate()`'s
own existing structural overlap check (Step 3), before any D-Graph is ever
constructed, exactly as Section 23 of the task requires.

## Tests

27 total in `dserver::core::dcompile` (all new — brand-new module), covering
C1–C24 plus digest-recomputation proofs and an explicit no-input-mutation
test — see above for the specific results of C1–C24.

## Files changed

- `dserver/src/core/dcompile.rs` (new)
- `dserver/src/core/mod.rs` (added `pub mod dcompile;`)
- `documentation/reference/phase117c-deterministic-calls-empty-dgraph-lowering.md` (this document)

`dcompile_request.rs` (the 117B input-contract module) was not modified —
Phase 117B's contract and validation remain exactly as closed; 117C only
consumes them.

## Focused tests / full regression / build results

- Focused (`dcompile::`): 27/27 passed.
- Full dserver suite: 1733 passed, 0 failed (1706 baseline + 27 new).
- Full DATUM lib suite: 1262 passed, 0 failed, 1 ignored — unchanged, as
  expected (this phase is dserver-only).
- `cargo build --locked --release`: both crates succeeded.
- `cargo fmt --check`: the new file was formatted directly with `rustfmt`
  (brand-new file, per this project's established practice) and reports
  zero deviations.
- `git diff --check`: clean.
- New warnings: `dcompile.rs`'s types/functions are flagged as unused in a
  plain, non-test build — expected, matching the identical, already-approved
  situation for `dcompile_request.rs` in Phase 117B: no production consumer
  exists yet, and none was manufactured artificially.

## Authority/security review

Explicitly confirmed unchanged: D-Script remains source contract only.
D-Compile lowers explicit inputs to a *candidate* topology, never authority.
D-Graph owns application-topology semantics — the generated value is
structurally placement-free (no `node_id`/`host`/`stage`/`placement` field
exists on `CanonicalDGraphV1`, unaltered) and carries no executable identity
(no `image`/`wasm`/`sha256`/`content_id` field on `CanonicalDServV1`,
unaltered). D-Map remains sole placement authority — nothing in this module
selects nodes. D-Forward/D-IoT/D-Monitor are untouched — no readiness,
health, or telemetry concept appears anywhere. D-Deploy (`create_proposal`,
`DDeployProposalV1`, `ProposalRecord`, `PlannerProvenanceV1`, acceptance)
was not modified or called. Stage 107A governed reconciliation/authorization
is entirely untouched. No host mutation exists anywhere in this module —
`lower_calls_empty_dgraph_candidate` is a pure, in-memory, side-effect-free
function.

## ADR decision

**No new ADR was created.** The provenance distinction (a D-Graph digest
cannot identify D-Compile mapping provenance) is a durable architectural
fact, but it follows directly and necessarily from decisions already made
and documented in Phase 116 (D-Script has no D-Serv reference) and Phase
117A (D-Graph has no D-Function-membership field) — it is not a new
decision this phase makes, only a consequence this phase is the first to
observe and prove by test. Creating an ADR now, before Phase 117F decides
how (or whether) mapping provenance is actually persisted through DDeploy
integration, would risk freezing a persistence-shape decision this phase
does not make. This document records the fact and its evidence
prominently; an ADR remains appropriate once 117F makes a concrete,
permanent persistence decision that depends on it.

## Explicit exclusions

No `compile()`/`generate_dcode()`/`create_proposal()` call. No D-Call
derivation. No D-Code descriptor generation. No `DDeployProposalV1`/
`ProposalRecord`/`PlannerProvenanceV1` construction. No placement selection.
No artifact registration. No `ServiceArtifact`/`CanonicalDServArtifactV1`/
`dcode_registry`/`module_store`/`invoke` interaction. No new persisted wire
schema.

## Next phase

Per the task's own explicit instruction, the next substep is **not**
assumed to automatically be "117D — call-graph input + DCall derivation."
Phase 117A already found no evidence for call topology, and this phase did
not change that finding — deriving DCalls still has no evidence-based
source. The more concrete, evidence-grounded remaining blocker, confirmed
again by this phase's own central finding (Section "Mapping-provenance
collision"), is: **a candidate D-Graph alone cannot carry D-Compile mapping
provenance, and D-Code descriptor/content-identity wiring (Phase 117A's
Gap #1) still has no evidence-based input contract at all.** The
recommended next step is therefore closer to Phase 117A's own originally
identified **117E — D-Code descriptor wiring**, which may itself warrant a
short sub-audit (mirroring 117A's own method) before implementation, given
Phase 117A's finding that no build/content-identity input contract exists
anywhere in this repository today. A call-graph input contract (117D)
should be taken up only if and when a concrete integration need actually
requires D-Call edges — not before.
