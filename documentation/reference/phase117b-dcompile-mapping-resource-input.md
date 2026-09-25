# Phase 117B — DCompile Mapping + Resource Input Contract

**Status:** Phase 117B CLOSED. **Phase 117 remains OPEN.** This is the first
production DCompile-specific contract — it is not yet a compiler, and no
DScript is lowered into a DGraph in this phase.
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117A closure, commit `648270a62b63f7c762c316cd0c5b5792d2527b9d`.

## Purpose

Phase 117A found `datum.dscript/1` alone cannot supply the D-Function→D-Serv
grouping, a candidate D-Graph's identity, or per-service resource
requirements — and that none of these may be invented (no naming-convention
grouping, no resource defaults, no reading requirements out of DScript's
deliberately opaque annotations). Phase 117B closes exactly those gaps with
one small, explicit, typed contract, so that a future Phase 117C lowering
step can consume `ValidatedDScriptInput + ValidatedDCompileRequest` and
produce a deterministic candidate `CanonicalDGraphV1` with no hidden
parameters other than the one already-justified auto-fill (`DNODE_ABI_V0`).

## Exact wire shape

```rust
pub const CANONICAL_DCOMPILE_REQUEST_SCHEMA: &str = "datum.dcompile-request/1";

pub struct DCompileRequestV1 {
    pub schema: String,
    pub application_id: String,
    pub dscript_ref: CanonicalContractReferenceV1,   // {id, revision, digest}
    pub output_dgraph_id: String,
    pub output_dgraph_revision: u64,
    pub services: Vec<DCompileServiceMappingV1>,     // #[serde(default)]
}

pub struct DCompileServiceMappingV1 {
    pub service_id: String,
    pub function_ids: Vec<String>,                   // #[serde(default)]
    pub resource_requirement: CanonicalDServResourceRequirementV1, // {cpu_cores, memory_bytes}
}
```

Both structs derive `Debug, Clone, Serialize, Deserialize, PartialEq, Eq` and
carry `#[serde(deny_unknown_fields)]`.

## Naming

`datum.dcompile-request/1` follows the exact `datum.<noun>/1` pattern every
other canonical schema in this repository uses (`datum.dscript/1`,
`datum.dgraph/1`, `datum.dmap/1`, `datum.ddeploy-proposal/1`). No alternative
was evidence-supported.

Field names were checked against sibling conventions before being chosen,
per the task's explicit instruction not to copy the illustrative shape
blindly:

- `dscript_ref` (not `dscript_reference`) — matches the terse `_ref` suffix
  already used for the identical purpose elsewhere (`dgraph_ref`,
  `dcontinuum_ref` on `DDeployProposalV1`/`PlannerProvenanceV1`).
- `output_dgraph_id`/`output_dgraph_revision` (not `dgraph_id`/`dgraph_revision`
  bare) — the `output_` prefix disambiguates this from a REFERENCE to an
  existing D-Graph (which this is not — it names the identity a future
  candidate D-Graph should carry, before that D-Graph exists).
- `DCompileServiceMappingV1` (not `DCompileMappingV1`) — one instance of this
  type describes one service's mapping; `services: Vec<DCompileServiceMappingV1>`
  reads correctly as "a list of per-service mappings," matching
  `CanonicalDGraphV1.services: Vec<CanonicalDServV1>`'s own naming shape.
- `function_ids` (not `functions`) — this list holds bare identifier strings,
  not `CanonicalDFunctionV1` values; naming it `function_ids` avoids implying
  it carries full D-Function objects.

## DScript reference binding

`dscript_ref: CanonicalContractReferenceV1` (`{id, revision, digest}`) binds
the request immutably to the exact D-Script it was prepared for. This type
already lives in `canonical_dmap.rs` but is genuinely project-wide — it is
already reused, for unrelated purposes, by `dcall.rs`, `dcode_authorization.rs`,
`ddeploy.rs`, and `operational_dgraph.rs`. Importing it here follows that
exact, already-established precedent; it does not create a new
DCompile→D-Map semantic dependency, because none of those other modules
became "D-Map-coupled" by importing it either — it is simply this project's
one shared `{id, revision, digest}` reference shape. A new local type was
considered and rejected as unnecessary duplication of an already-correct,
already-precedented shape.

A caller merely asserting "this belongs to that D-Script" is never trusted:
`DCompileRequestV1::validate_against_dscript` recomputes the comparison
against a real `ValidatedDScriptInput` (the exact type
`decode_and_validate_dscript_json` produces), checking `application_id`,
`dscript_ref.id` against `dscript_id`, `dscript_ref.revision` against
`revision`, and `dscript_ref.digest` against the D-Script's own recomputed
`digest_sha256` — never a caller-asserted digest taken on faith.

## Output D-Graph identity decision

`output_dgraph_id`/`output_dgraph_revision` are explicit, flat fields on the
request, per Phase 117A Section 5's own recommendation: a future D-Graph's
identity must not become a hidden, undocumented parameter Phase 117C invents
on its own. This makes the eventual determinism property honest and
literal: *same validated D-Script + same validated compile request → same
candidate D-Graph*, with no extra out-of-band argument a caller could vary
without it showing up in this request's own digest. No repository evidence
argued for keeping D-Graph identity outside this request, so the preferred
design was adopted without qualification.

## Service-ID ownership decision

The mapping supplier chooses `service_id` in v1. DCompile does not invent,
hash, or derive it from `function_ids` — Phase 117A found no precedent
anywhere in this codebase for hashing a canonical identity out of content;
every existing identity (`service_id`, `call_id`, `artifact_id`, `dscript_id`)
is human/tool-chosen text. `function_id == service_id` by naming convention
is explicitly rejected, since the vocabulary permits non-1:1 mapping and no
evidence ties the two together even in the 1:1 case.

## Exact-partition mapping semantics (v1 limitation, not a vocabulary claim)

Phase 117A found concrete historical evidence (the disclaimed VS1 sketch)
only for a **disjoint partition**: one D-Script, several D-Servs, each
owning a distinct, non-overlapping subset of D-Functions. Whether a
D-Function may legitimately belong to multiple D-Servs simultaneously
(shared vs. duplicated implementation) is explicitly undefined by any
repository evidence. `datum.dcompile-request/1` v1 therefore requires every
D-Script function to be mapped to **exactly one** service:

- Every D-Script function appears in some service's `function_ids` (else
  `unmapped_function`).
- No function ID outside the D-Script's own set may appear in any service's
  `function_ids` (else `unknown_function_mapping`).
- No function ID may appear in more than one service's `function_ids` (else
  `overlapping_function_mapping`).

This supports one-function-to-one-service, many-functions-to-one-service
(M:1), and one-D-Script-to-many-services-via-disjoint-subsets (1:N in the
partition sense) — it does **not** claim to implement the vocabulary's
still-undefined overlapping 1:N/M:N cases (shared/duplicated implementation
across services). A future, evidence-driven schema revision may extend this
once those semantics are actually defined; this document records the
distinction explicitly so the limitation is never mistaken for a permanent
architectural position.

## Empty DScript behavior

An empty D-Script (`functions = {}`) together with an empty mapping
(`services = []`) validates cleanly — this falls out naturally from the
relational rules above (the empty union of mapped functions exactly equals
the empty D-Script function set, so no `unmapped_function`/
`unknown_function_mapping` finding is produced) rather than needing a
special-cased rule. Conversely, a non-empty D-Script with an empty mapping
fails (`unmapped_function` for every function), and an empty D-Script with a
non-empty mapping fails (`unknown_function_mapping` for every mapped
function) — both are proven directly by tests, not merely asserted.

## Resource requirement semantics

Every non-empty service mapping requires an **explicit** `resource_requirement:
CanonicalDServResourceRequirementV1` (`cpu_cores: u32, memory_bytes: u64`,
reused directly from `canonical_dgraph.rs` rather than mirrored — the two
types are semantically identical and DCompile's whole purpose is to
eventually produce a `CanonicalDGraphV1`, so this import is a natural,
non-awkward dependency, unlike the D-Map case). Validated at the request
boundary itself (`cpu_cores >= 1`, `memory_bytes >= 1`) since
`CanonicalDServResourceRequirementV1` as a bare type performs no validation
of its own — that check lives at the owning contract's `validate()`, exactly
as `CanonicalDGraphV1` does today, and this request is now that owning
contract for its own purposes. Never defaulted, never inferred from service
or function count, DContinuum, machine capacity, telemetry, or DMonitor —
and never read from any D-Script annotation, no matter how suggestive an
annotation key looks (proven directly by
`annotations_never_influence_validation_outcome`, which uses deliberately
misleading annotation content and confirms zero effect on the result).

## D-Node ABI: deferred, not included

`DNODE_ABI_V0` ("datum-dnode/0") is the sole legal D-Node ABI anywhere in
this architecture today — there is no second value a caller could
meaningfully select. Adding an ABI field to this request would be fake
configurability. Phase 117C will assign the sole valid constant explicitly
when it eventually lowers a D-Graph, exactly as every existing
hand-authored D-Graph fixture already does today (a real value, always
written literally, never silently defaulted at the type level).

## No D-Call input, no D-Code content

Neither exists in this contract. Phase 117A found DScript carries no
call/invocation relationship at all, and this repository has no
source-to-binary build path anywhere — D-Code content is always externally
built and already digested before this repository ever sees it. Both
remain entirely out of `datum.dcompile-request/1` v1's scope; a future,
separate contract revision or sibling type would need to add them once real
evidence exists for how to source that information.

## Validation layers

Two explicit, non-duplicating layers, mirroring the task's own recommended
split:

- **`DCompileRequestV1::validate(&self)`** — pure, self-contained structural
  validation. Never touches a D-Script. Includes the overlapping-membership
  check (a purely internal request invariant: whether the same function ID
  appears in two different services is fully determinable from the request
  alone, with no D-Script needed).
- **`DCompileRequestV1::validate_against_dscript(&self, dscript: &ValidatedDScriptInput)`**
  — relational validation against a real, already-validated D-Script:
  identity/digest match, unknown-function detection, unmapped-function
  detection. Never duplicates `CanonicalDScriptV1::validate()`'s own rules;
  callers are expected to call `validate()` first (both are exercised
  together in every positive test).

## Finding/error codes introduced

`schema_invalid`, `missing_field`, `revision_invalid`, `dscript_digest_malformed`,
`duplicate_service_id`, `empty_service_function_set`, `duplicate_function_id`,
`overlapping_function_mapping`, `invalid_cpu_requirement`,
`invalid_memory_requirement` (all from `validate()`); `application_id_mismatch`,
`dscript_id_mismatch`, `dscript_revision_mismatch`, `dscript_digest_mismatch`,
`unknown_function_mapping`, `unmapped_function` (from `validate_against_dscript`).
Distinct, specific codes were used for the mapping-relational errors
(`unmapped_function`, `unknown_function_mapping`, `overlapping_function_mapping`)
rather than collapsing them into `missing_field`, per the task's explicit
instruction not to hide a semantic defect behind a generic code.

## Digest normalization

`recompute_dcompile_request_digest` (a free function — no `request_digest`
field exists on the type itself, mirroring `canonical_dgraph`/`canonical_dscript`'s
own precedent) sorts a cloned copy's `services` by `service_id` and each
service's `function_ids` before JCS-hashing, so reordering either never
changes the digest. The wire JSON itself is never reordered. Proven:
service-order independence, function-ID-order independence within a service,
and digest sensitivity to every semantically meaningful field (`dscript_ref.digest`,
`output_dgraph_id`, `service_id`, function membership, `cpu_cores`,
`memory_bytes`) in one consolidated test.

## Unknown-field policy

`#[serde(deny_unknown_fields)]` on both types, following the same
newer-convention decision Phase 116C already made and cited (ADR-0024) for
`datum.dscript/1` — a brand-new v1 contract with zero deployed
compatibility burden. Proven to reject a placement-looking `"node_id"`
field, a `"calls"` field, a `"retry"` field, and a caller-supplied
`"request_digest"` field — none of these can ever be silently ignored.

## Server-side only, and why

`DCompileRequestV1` and its input boundary exist only in `dserver`. Phase
116C already established DCompile is expected to be a server-side/
control-plane responsibility; Phase 117A found no D-Node-agent consumer for
DCompile input of any kind. No concrete cross-crate requirement was found
during this phase either, so per ADR-0007's parity discipline ("parity of
every helper function is not automatically mandatory"), this contract is
not mirrored into `DATUM::agent` merely for symmetry.

## Tests

40 tests in `dserver::core::dcompile_request`, covering the full B1–B31
matrix the task specified (positive grouping shapes including M:1 and 1:N
partition, digest order-independence, empty-DScript/empty-mapping validity,
cross-validation identity checks, every structural and relational negative
case, the overlap test with its explicit v1-limitation-not-vocabulary-claim
documentation, resource-requirement negatives, unknown-field/caller-digest
rejection through the JSON boundary, an authority-negative serialization
scan, and a fixture-based test proving the real `dscript.sensor-pipeline.json`
fixture and a new, non-VS1-derived `dcompile-request.sensor-pipeline.json`
fixture validate against each other end to end).

## Authority boundaries (reviewed explicitly)

- DScript remains source authority only — untouched.
- This request describes explicit compilation intent/input only; it
  produces no DGraph, no DCode, no proposal.
- DMap remains sole placement authority — no placement field exists on
  either type (authority-negative test confirms no `node_id`/`host`/
  `placement`/`zone`/`target_node` key ever appears in the serialized JSON).
- DForward/DIoT/DMonitor are untouched — no readiness, health, or telemetry
  concept appears anywhere in this contract.
- DDeploy (`DDeployProposalV1`, `ProposalRecord`, `create_proposal`,
  `PlannerProvenanceV1`, acceptance) was not modified in any way.
- Stage 107A governed reconciliation/authorization is entirely untouched —
  no file in that path was modified.
- No host mutation exists anywhere in this module — `validate()`,
  `validate_against_dscript()`, and `recompute_dcompile_request_digest()`
  are pure, in-memory, side-effect-free functions.

## ADR decision

No new ADR was created. The exact-partition-mapping decision is a
narrowly-scoped v1 slice limitation, fully documented above and in the
module's own doc comment, explicitly framed as provisional pending future
evidence about overlapping-membership semantics — not a durable,
project-wide architectural commitment in the sense ADR-0025 (annotation
representation, a decision with permanent wire-format consequences for
every future DScript document) represents. If a future phase does define
overlapping-membership semantics and extends this schema, that decision —
not this one — would be the one warranting an ADR.

## Files changed

- `dserver/src/core/dcompile_request.rs` (new)
- `dserver/src/core/mod.rs` (added `pub mod dcompile_request;`)
- `scripts/fixtures/datum-contracts/canonical-v1/dcompile-request.sensor-pipeline.json` (new)
- `documentation/reference/phase117b-dcompile-mapping-resource-input.md` (this document)

## Test/build results

- Focused (`dcompile_request`): 40/40 passed.
- Full dserver suite: 1706 passed, 0 failed (1666 baseline + 40 new).
- Full DATUM lib suite: 1262 passed, 0 failed, 1 ignored — unchanged, as
  expected (this phase is dserver-only).
- `cargo build --locked --release`: both crates succeeded.
- `cargo fmt --check`: the new file was formatted directly with `rustfmt`
  (brand-new file, per this project's established practice) and reports
  zero deviations.
- `git diff --check`: clean.
- New warnings: `dcompile_request.rs`'s types/functions are flagged as
  unused in a plain, non-test build — expected and pre-approved (Phase
  116B/C precedent): this contract has no production consumer yet, and
  none was manufactured artificially. Phase 117C is expected to become its
  first substantive consumer.

## Explicit 117B exclusions

No `compile()`/`lower()`/`generate_dgraph()`/`compile_dscript()`/
`dscript_to_dgraph()`/`generate_dcode()` function. No DGraph lowering, no
DCall derivation, no DCode generation, no placement selection, no DMap
mutation, no DForward/DIoT generation, no DMonitor read or mutation, no
automatic DDeploy proposal creation or acceptance, no automatic artifact
authorization, no AI-based mapping/grouping/resource inference, and no
interpretation of any `CanonicalDFunctionV1.annotations` key or value for
any purpose.

## Next phase

**Phase 117C — Deterministic Calls-Empty Candidate DGraph Lowering** may
begin: given a `ValidatedDScriptInput` and a `ValidatedDCompileRequest`
already cross-validated against it, deterministically produce a candidate
`CanonicalDGraphV1` with `schema = datum.dgraph/1`, `application_id` from
the D-Script, `dgraph_id`/`revision` from `output_dgraph_id`/
`output_dgraph_revision`, `services` from the mapping (`dnode_abi = DNODE_ABI_V0`
assigned explicitly), resources from the mapping's `resource_requirement`
values, and `calls = []`. 117C must validate its own generated D-Graph
before returning it. None of this is implemented here.
