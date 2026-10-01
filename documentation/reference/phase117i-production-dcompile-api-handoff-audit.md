# Phase 117I — Production DCompile API & Handoff Audit

**Status:** Phase 117I AUDIT CLOSED (audit/design only). **Phase 117 remains
OPEN. NO PRODUCTION DCOMPILE API OR AUTHORITY TRANSITION WAS ADDED DURING
PHASE 117I.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117H closure, commit `a384989fb316456683573386a52e2152def716be`.
**NO VM OR RPI WAS USED FOR THIS PHASE.**

## 1. Executive conclusion

DCompile's core compiler pipeline (117B-E) is real, pure, deterministic, and
fully tested — but it has no production caller. It exists only as
`dserver::core` functions with `#[allow(dead_code)]`-adjacent "no consumer
yet" warnings, exactly as 117B/C/D/E's own closure reports honestly
disclosed at the time. This phase confirms that gap is now the correct
thing to close, and designs — without implementing — the smallest
production interface that closes it.

**The smallest correct interface is a single, one-shot, stateless HTTP
endpoint (`POST /api/v1/dcompile/compile`) that accepts a new, additive
wire envelope (`datum.dcompile-submission/1`) containing the caller's TRUE
external inputs only — a `CanonicalDScriptV1` document, a compile-intent
mapping (reusing `DCompileServiceMappingV1` verbatim, minus the
self-referential `dscript_ref`), and per-service D-Code content/policy
facts (reusing `DCompileDCodeServiceInputV1` verbatim, minus the
self-referential `dgraph_ref`) — and internally, deterministically,
constructs every derived reference (`dscript_ref`, `dgraph_ref`) itself
before running the existing, frozen `lower_dcode_artifact_candidates`
pipeline unchanged.** This is "Option C" in Section 6 below, and it is
recommended specifically because it is the only option that eliminates,
by construction, an entire class of caller error (a caller can no longer
submit a wrong `dscript_ref`/`dgraph_ref` digest, because it never supplies
one) without inventing any new compiler semantics or duplicating 117B/E's
frozen input contracts.

The response is a **new wire-only result type** (`datum.dcompile-candidate/1`,
Section 9), never a `Serialize`-ified version of the internal
`DCompileCandidateBundle` (Section 9's internal/wire separation holds).
It contains the compiled `CanonicalDGraphV1` verbatim (directly POST-able
into `ProposalInputPayload.dgraph` with zero transformation — a concrete,
evidence-confirmed finding, Section 14/Section 17), one
`CanonicalDServArtifactV1` per service (directly POST-able to
`/api/v1/dcode/artifacts`), and a pure convenience
`BTreeMap<service_id, DCodeArtifactAuthorityBindingV1>` (directly usable,
byte-for-byte, as `ProposalInputPayload.dcode_artifact_authority` —
Section 15/16) — plus the full provenance chain (D-Script digest, the
complete D-Compile-request mapping and its digest, D-Graph digest,
per-service module/descriptor digests). No field ever implies
"registered," "active," "accepted," or "deployed."

**Provenance persistence does NOT block the next live phase (Section 12,
45).** DDeploy's own existing validation (`validate_dcode_artifact_authority_bindings`,
`recompute_dgraph_digest`) never trusts a caller-asserted digest as
self-certifying — it always independently recomputes and compares, exactly
this project's universal discipline. This means DDeploy's CORRECTNESS never
depended on, and does not need, a server-persisted record proving "this
candidate really came from a real DCompile call" — recomputation already
proves the CONTENT is internally consistent regardless of who produced it,
exactly like `ProposalSource::Deterministic` is already an honest,
non-authoritative label today. **Recommended provenance model: P1 — no
compile persistence, pure stateless API, full provenance returned to the
caller.** A compile-result identity/digest is correspondingly **not
needed** (Section 13) under P1, for the same reason 117E already rejected
a bundle-level input digest: nothing is keyed/stored by it.

**An HTTP API is required for Phase 117 closure** (Section 46) — this
project's stated end goal is an operational system, and 117H's own live
governed-client work has no real compiler entry point to exercise without
one; a live Phase 117 proof that hand-writes `CanonicalDServArtifactV1`
JSON, as 117H's own closing report already flagged, would not actually
prove DCompile works. Deeper provenance PERSISTENCE (P2-P5) is a separate,
legitimately deferrable concern, exactly like 117H deferred DCompile
provenance persistence in its own remaining-gaps list.

## 2. Current production gap

Confirmed by direct inspection, not inference: `dserver/src/core/mod.rs`
declares `pub mod dcompile; pub mod dcompile_dcode; pub mod dcompile_request;`
with zero corresponding entries anywhere under `dserver/src/api/` or
`dserver/src/main.rs`'s route table — no `api/dcompile.rs` file exists, no
`/api/v1/dcompile/...` route of any kind is registered. Every one of the
four core modules' own closure reports (117B/C/D/E) states, verbatim, "no
production consumer exists yet, and none was manufactured artificially" —
this is a long-standing, explicitly and repeatedly disclosed gap, not a
newly discovered defect. `DCompileDGraphCandidate`/`DCompileCandidateBundle`
are explicitly documented as "NOT a wire contract" in both their own
module doc comments (117C, 117E) for exactly this reason: no proven
consumer existed to design a wire shape against.

## 3. Current core pipeline

Traced directly from source, confirmed byte-for-byte against 117A-E's own
closure reports (no drift found):

```
decode_and_validate_dscript_json(json: &str)
    -> Result<ValidatedDScriptInput { document: CanonicalDScriptV1, digest_sha256: String }, DScriptInputError>

decode_and_validate_dcompile_request_json(json: &str)
    -> Result<ValidatedDCompileRequest { request: DCompileRequestV1, digest_sha256: String }, DCompileRequestInputError>

decode_and_validate_dcode_input_json(json: &str)
    -> Result<DCompileDCodeInputV1, DCompileDCodeInputError>

lower_calls_empty_dgraph_candidate(dscript: &ValidatedDScriptInput, request: &ValidatedDCompileRequest)
    -> Result<DCompileDGraphCandidate { dgraph: CanonicalDGraphV1, dgraph_digest_sha256, dscript_digest_sha256, dcompile_request_digest_sha256 }, DCompileLoweringError>

lower_dcode_artifact_candidates(dscript: &ValidatedDScriptInput, request: &ValidatedDCompileRequest, dcode_input: &DCompileDCodeInputV1)
    -> Result<DCompileCandidateBundle { dgraph_candidate: DCompileDGraphCandidate, dcode_artifacts: Vec<DCompileDServArtifactCandidate { artifact: CanonicalDServArtifactV1, descriptor_digest_sha256: String }> }, DCompileDCodeLoweringError>
```

Exact current wire types (`Serialize + Deserialize + deny_unknown_fields`,
confirmed by direct source inspection):

```rust
// datum.dscript/1
pub struct CanonicalDScriptV1 { schema, application_id, dscript_id: String, revision: u64, functions: Vec<CanonicalDFunctionV1> }
pub struct CanonicalDFunctionV1 { function_id: String, annotations: BTreeMap<String, String> }

// datum.dcompile-request/1
pub struct DCompileRequestV1 { schema, application_id: String, dscript_ref: CanonicalContractReferenceV1, output_dgraph_id: String, output_dgraph_revision: u64, services: Vec<DCompileServiceMappingV1> }
pub struct DCompileServiceMappingV1 { service_id: String, function_ids: Vec<String>, resource_requirement: CanonicalDServResourceRequirementV1 }

// datum.dcompile-dcode-input/1
pub struct DCompileDCodeInputV1 { schema, application_id: String, dgraph_ref: CanonicalContractReferenceV1, services: Vec<DCompileDCodeServiceInputV1> }
pub struct DCompileDCodeServiceInputV1 { service_id, artifact_id, version, sha256: String, size_bytes: u64, ports: DServArtifactPortsV1, limits: DServArtifactLimitsV1 }
```

`DCompileDGraphCandidate`/`DCompileCandidateBundle`/`DCompileDServArtifactCandidate`
have **no** `Serialize`/`Deserialize` (confirmed by direct inspection — no
`#[derive]` includes either trait on any of the three). This audit does
**not** propose changing any of the six frozen input/mapping types above —
every field, validation rule, and finding code listed in 117B/D/E's own
reports is treated as load-bearing and unmodified by this design.

## 4. Current downstream handoff

Traced directly from `dserver/src/core/dcode_registry.rs`,
`dserver/src/api/dcode.rs`, `dserver/src/core/ddeploy.rs`,
`dserver/src/core/ddeploy_v2.rs`, `dserver/src/api/ddeploy.rs` (all
post-117G/117H current state — no drift since the previous two phases):

- **Declare a revision:** `POST /api/v1/dcode/artifacts` with a bare
  `CanonicalDServArtifactV1` JSON body → `declare_dserv_artifact_revision`
  → `DeclareDServArtifactResponse { artifact_id, descriptor_digest, inserted }`.
- **Construct a binding:** `DCodeArtifactAuthorityBindingV1 { descriptor_digest: String }` —
  one per bound `service_id`, `BTreeMap<String, DCodeArtifactAuthorityBindingV1>`.
- **Create a v1 proposal:** `POST /api/v1/ddeploy/proposals` (header:
  `project_id`), body `ProposalInputPayload { application_id, source, dgraph: CanonicalDGraphV1, dcontinuum: CanonicalDContinuumV1, placements: Vec<CanonicalDServPlacementV1>, dcode_artifact_authority: BTreeMap<String, DCodeArtifactAuthorityBindingV1> }` —
  **the full `CanonicalDGraphV1` value is accepted inline**, not a
  reference (confirmed, `api/ddeploy.rs:165-182`; already the same finding
  117A's own Section 16 made about `create_proposal`'s core signature).
  This handler already validates any supplied `dcode_artifact_authority`
  against CURRENT registry state via `validate_dcode_artifact_authority_bindings`
  before persisting (117G Section 19).
- **Create a v2 proposal:** `POST /api/v1/ddeploy/v2/proposals`, body
  `DeployV2CandidatePayload { application_id, source, dgraph: CanonicalDGraphV1, dcontinuum, dforward: CanonicalDForwardV1, placements: Vec<DServPlacementV2>, diot_placements, dcall_realizations, external_port_bindings, dcode_artifact_authority }` —
  same `dgraph`-inline pattern, plus a **mandatory `dforward: CanonicalDForwardV1`**
  DCompile has no role in producing (117A: "DForward — DCompile has no
  role... canonical definition is DScript→DGraph+DCode only" — unaffected
  by this audit).
- **Accept:** `POST /api/v1/ddeploy/proposals/:id/accept` /
  `.../v2/proposals/:id/accept` — unchanged by this audit, no DCompile
  role.

**Exact conceptual chain a caller must execute today, entirely by hand,**
to go from a `DCompileCandidateBundle` to an accepted deployment:

```
DCompileCandidateBundle (in-process value only, no API to obtain it from)
    -> for each dcode_artifacts[i]: POST candidate.artifact to /api/v1/dcode/artifacts
       -> record the returned descriptor_digest (or recompute it locally -- same value)
    -> hand-build BTreeMap<service_id, DCodeArtifactAuthorityBindingV1> from those digests
    -> supply candidate.dgraph_candidate.dgraph verbatim as ProposalInputPayload.dgraph
    -> supply a DContinuum (DCompile has none -- operator/DNode-registry-derived)
    -> supply placements (DCompile has none -- operator/planner-derived)
    -> POST the assembled ProposalInputPayload to /api/v1/ddeploy/proposals
    -> POST .../accept
```

Every step after "record descriptor_digest" is already exact, mechanical,
and requires no invention — the ONLY missing piece is a way to obtain the
`DCompileCandidateBundle` (and its handoff-ready binding map) from a real
external caller in the first place. This is precisely the gap this audit
closes.

## 5. Derived-reference dependency problem

Confirmed directly (Section 3's exact field list): `DCompileRequestV1.dscript_ref.digest`
must equal the REAL D-Script's own `recompute_dscript_digest` output
(enforced by `validate_against_dscript`), and `DCompileDCodeInputV1.dgraph_ref.digest`
must equal the REAL candidate D-Graph's own `dgraph_digest_sha256`
(enforced by `validate_against_dgraph`) — a value that **does not exist
until Phase 117C's lowering has already run**. A caller cannot honestly
pre-populate `dgraph_ref` before compilation happens; the digest it would
need to supply is the OUTPUT of the very operation it is trying to invoke.
**Any wire design that asks an external caller to submit all three
existing types (D-Script, D-Compile-request, D-Code input) as pre-filled,
self-consistent JSON in one shot is asking the caller to already know a
value the server alone can produce — this is not merely awkward, it is
logically circular for `dgraph_ref` specifically.** This is the single
most important constraint on the API design (Section 6).

## 6. API option comparison

**Option A — Staged HTTP API** (`POST .../dscript/validate` →
`POST .../dgraph` → `POST .../dcode`). Solves Section 5's circularity by
construction (each stage returns the digest the next stage needs) but at a
real cost this audit confirms is NOT compensated by any persistence
benefit: **nothing is persisted between stages** (no DScript registry, no
intermediate-candidate store exists or is proposed) — the caller must
carry forward the RAW original content across calls anyway, so staging
adds real round trips without saving the caller from re-submitting
anything. Purely additive chattiness with no offsetting benefit under this
architecture's current (stateless-compiler) shape.

**Option B — One-shot submission with a brand-new envelope duplicating
compiler intent.** Ergonomic and circularity-free, but the task's own
concern is real: a naive design (e.g. inlining raw `services`/`resource`/
`dcode` fields directly, reinventing field names) would create a SECOND,
parallel definition of exactly what `DCompileServiceMappingV1`/
`DCompileDCodeServiceInputV1` already define, risking silent drift between
the wire envelope and the frozen internal contracts as either evolves
independently.

**Option C — Thin envelope that internally constructs the existing
canonical types, using derived (server-computed) references.** Solves
Section 5 (the server, not the caller, fills `dscript_ref`/`dgraph_ref`,
computed from what it just validated/produced) while explicitly REUSING
`DCompileServiceMappingV1`/`DCompileDCodeServiceInputV1` (both already
`Serialize + Deserialize`, confirmed Section 3) as NESTED sub-values inside
the new envelope — the only new wire surface is the outer envelope
struct and the removal of each inner type's own self-referential wrapper
fields (`schema`/`dscript_ref`/`dgraph_ref`), which the server supplies
internally instead. This is Option B corrected against its own stated
risk, not a fundamentally different design.

**Option D — Control-plane CLI instead of HTTP.** Evaluated and rejected
as a *sufficient* production interface (though a CLI wrapper over the
future HTTP endpoint remains a reasonable convenience — see Section 7's
"real production interface" test below): a CLI still needs to call
something to become a caller of the core pipeline, and this project's
established convention for every other control-plane transition
(declaration, proposal, acceptance) is an HTTP API a CLI itself is free to
wrap (`smartsentinel-dcode-invoke`, `smartsentinel-dcall-dispatch` are
exactly this shape — thin CLI wrappers over real HTTP endpoints, never the
authority themselves). A CLI that calls core functions directly and writes
JSON files moves the "no production entry point" problem sideways rather
than closing it, and gives no remote/multi-caller/service-to-service path
at all — a real regression relative to every other DServer capability.

**Option E — other.** None found with stronger evidence support than C.

### Options table

| | Option A (staged) | Option B (one-shot, new envelope) | Option C (thin derived-ref envelope) | Option D (CLI-only) |
|---|---|---|---|---|
| Semantic duplication | None (reuses existing types as-is) | High (new field names risk drifting from 117B/E types) | None (nests existing types verbatim, minus self-refs) | None (calls core functions directly) |
| Caller ergonomics | Poor (3 round trips, no persistence benefit) | Best | Best (same call shape as B) | Poor (local-only, no remote caller) |
| Trust boundary | Same as C | Same as C | Never trusts a caller-supplied derived digest | Same as C (in-process) |
| Derived-reference safety | Safe (each stage returns what's needed) | **Unsafe if it inlines `dscript_ref`/`dgraph_ref`** (circular, Section 5) | Safe by construction (server fills every derived ref) | Safe (direct function calls) |
| Deterministic behavior | Yes | Yes | Yes | Yes |
| Backward compatibility | N/A (new) | N/A (new) | N/A (new) | N/A (new) |
| Implementation complexity | Medium (3 handlers, no new types) | Low (1 handler, but a risky new schema) | Low-medium (1 handler, 1 small new envelope type) | Low (no HTTP layer) but doesn't close the gap |
| Live-test usability | Usable but multi-step | Usable | Usable, single call | Not usable as a real caller (Option D itself is the gap) |
| Provenance support | Full (same underlying types) | Full | Full | Full (informational only) |
| Recommended | No | No (as originally framed) | **Yes** | No |

## 7. Recommended API contract

One endpoint, one request type, one response type — no staged endpoints
added preemptively (mirrors this project's own "no API surface without a
proven consumer" discipline, applied consistently through 117B-E).

```
POST /api/v1/dcompile/compile
```

Pure, deterministic, side-effect-free — identical semantic input always
produces identical semantic output (Section 17 preserved exactly). No
registration, no proposal creation, no acceptance, no placement, no
authorization call happens inside this handler. Module location:
`dserver/src/api/dcompile.rs` (not created in this phase).

A future CLI (`smartsentinel-dcompile`, or similar) MAY wrap this endpoint
exactly as `smartsentinel-dcode-invoke`/`smartsentinel-dcall-dispatch`
already wrap their own HTTP endpoints — that would make Option D's
convenience available WITHOUT it being the sole production interface.

## 8. Recommended request shape

```rust
pub const DCOMPILE_SUBMISSION_SCHEMA: &str = "datum.dcompile-submission/1";

pub struct DCompileSubmissionV1 {
    pub schema: String,
    pub dscript: CanonicalDScriptV1,             // reused verbatim, unchanged
    pub output_dgraph_id: String,                // -> becomes DCompileRequestV1.output_dgraph_id
    pub output_dgraph_revision: u64,              // -> becomes DCompileRequestV1.output_dgraph_revision
    pub services: Vec<DCompileServiceMappingV1>,  // reused verbatim, unchanged
    pub dcode_services: Vec<DCompileDCodeServiceInputV1>, // reused verbatim, unchanged
}
```

The caller never supplies `dscript_ref`, `dgraph_ref`, or any digest of
any kind (Section 5). `application_id` is not a separate field — it is
read from `dscript.application_id` exactly once and used consistently
everywhere `DCompileRequestV1.application_id`/`DCompileDCodeInputV1.application_id`
are needed internally (both already require exact equality with the
D-Script's own `application_id` per 117B/E's existing relational rules,
so this is not a new relationship, only where the one true value comes
from). `#[serde(deny_unknown_fields)]` on the new envelope, following
every prior Phase-117 wire type's own convention.

Internal construction (illustrative, not implemented):

```rust
let dscript_input = /* replicate decode_and_validate_dscript_json's logic on submission.dscript, OR add a construct-and-validate-in-memory sibling -- design choice for 117J, not fixed here */;
let request = DCompileRequestV1 {
    schema: CANONICAL_DCOMPILE_REQUEST_SCHEMA.to_string(),
    application_id: submission.dscript.application_id.clone(),
    dscript_ref: CanonicalContractReferenceV1 { id: submission.dscript.dscript_id.clone(), revision: submission.dscript.revision, digest: dscript_input.digest_sha256.clone() },
    output_dgraph_id: submission.output_dgraph_id,
    output_dgraph_revision: submission.output_dgraph_revision,
    services: submission.services,
};
// validate + digest exactly as decode_and_validate_dcompile_request_json does, in memory (no re-serialization needed)
```

and analogously for `DCompileDCodeInputV1`, using the FRESH
`DCompileDGraphCandidate.dgraph_digest_sha256`
`lower_calls_empty_dgraph_candidate` just produced. **117J must decide**
whether this is done by literally serializing/re-decoding through the
existing `decode_and_validate_*_json` string boundary (simplest, reuses
the exact tested code path, small serialization overhead) or by adding a
new in-memory `validate_and_digest`-shaped sibling that skips the
JSON round-trip (marginally more efficient, requires a new, small, exactly
parallel function). This is an implementation-detail decision, not an
architectural one — this audit does not resolve it.

## 9. Recommended result shape

**Wire-only. Never a `Serialize`-derived version of `DCompileCandidateBundle`
itself** (Section 10's decision, below).

```rust
pub const DCOMPILE_CANDIDATE_SCHEMA: &str = "datum.dcompile-candidate/1";

pub struct DCompileCandidateResponseV1 {
    pub schema: String,
    pub application_id: String,

    // Provenance chain (Section 10/24) -- five identities preserved, never collapsed.
    pub dscript_ref: CanonicalContractReferenceV1,           // server-computed
    pub mapping: DCompileRequestV1,                           // the FULL mapping, not just its digest -- see rationale below
    pub dcompile_request_digest: String,                      // server-computed
    pub dgraph: CanonicalDGraphV1,                            // directly POST-able as ProposalInputPayload.dgraph / DeployV2CandidatePayload.dgraph
    pub dgraph_ref: CanonicalContractReferenceV1,             // server-computed

    pub dcode_artifacts: Vec<DCompileArtifactResultV1>,       // one per service, directly POST-able individually
    pub dcode_artifact_authority: std::collections::BTreeMap<String, DCodeArtifactAuthorityBindingV1>, // pure convenience, byte-identical shape DDeploy expects
}

pub struct DCompileArtifactResultV1 {
    pub service_id: String,
    pub artifact: CanonicalDServArtifactV1,   // directly POST-able to /api/v1/dcode/artifacts
    pub descriptor_digest: String,
}
```

**Why `mapping: DCompileRequestV1` is included in full, not merely its
digest:** 117C's own central proof
(`c12_mapping_provenance_collision_same_dgraph_different_request_digest`)
established that a D-Graph digest alone can never identify which mapping
produced it — only the request digest (or the request itself) can. A
digest alone lets a caller PROVE two things match; it does not let anyone
later RECONSTRUCT what the mapping actually was for audit/replay. This
mirrors `PlannerProvenanceV1`'s own existing precedent exactly (it stores
`decisions`/`committed_by_node`/`eligibility` in full, not merely a
digest, for the analogous algorithmic-placement-provenance purpose).
Embedding `DCompileRequestV1` costs nothing new here — it already derives
`Serialize`/`Deserialize` (Section 3), so no wire-type change to the
frozen type is required to expose it.

`dscript` itself (the full document, not just its ref) is deliberately
**not** echoed back in the response — the caller already has it verbatim
(it supplied it in the request), and 117A/116B already established
D-Script identity is `{id, revision, digest}` for reference purposes;
echoing the whole document a second time would be pure duplication with
no distinct informational value the ref doesn't already carry alongside
the caller's own copy.

No field named `current`/`active`/`accepted`/`registered`/`deployed`
anywhere (Section 34, confirmed by construction — none of the fields
above name any such concept).

## 10. Internal vs wire types

**Decision: B — keep internal types non-wire; map them into explicit,
separately-named API response types.** `DCompileDGraphCandidate`/
`DCompileCandidateBundle`/`DCompileDServArtifactCandidate` gain **no**
`Serialize`/`Deserialize` derive. `DCompileCandidateResponseV1`/
`DCompileArtifactResultV1` (Section 9) are new, purpose-built,
independently-versioned wire types that a handler constructs BY MAPPING
FROM the internal result — never by attaching serde to the compiler's own
internal representation. This preserves the freedom 117C/E's own
"deliberately not a wire contract" design already claimed: the compiler's
internal shape can still evolve (new internal fields, restructured
grouping) without that evolution silently becoming a protocol-breaking
change, and the wire schema's own evolution (Section 33) is governed
independently by explicit versioning discipline, not accidentally frozen
by whatever the compiler-internals struct happened to look like the day
an HTTP handler was bolted onto it.

## 11. Provenance model

The five identities established by 117D/117E, and this document's own
addition (per-service module digest is really `artifact.dcode.sha256`,
already inside `dcode_artifacts[i].artifact`, so it needs no separate
top-level field): D-Script digest, D-Compile-request digest, D-Graph
digest, module digest, D-Serv artifact descriptor digest. **All five are
present in the recommended response** (Section 9): `dscript_ref.digest`,
`dcompile_request_digest` (plus the full `mapping`), `dgraph_ref.digest`,
`dcode_artifacts[i].artifact.dcode.sha256`,
`dcode_artifacts[i].descriptor_digest`. `dscript_id`/`dscript_revision`
(inside `dscript_ref`), output `dgraph_id`/`revision` (inside `dgraph_ref`
AND inside `dgraph` itself), and `service_id -> descriptor digest` (=
`dcode_artifact_authority`) are all present without inventing any
additional field — Section 10 of the task's own checklist is satisfied by
the design already described, not by adding more surface.

## 12. Provenance persistence decision

**P1 — no compile persistence; DCompile remains a pure stateless compiler
API; the caller receives full provenance in the response.** Reasoning,
stated as the direct answer to the task's central question (Section 12 of
the task): **can DDeploy trust caller-supplied provenance round-tripped
back to it?** Yes, for CORRECTNESS, because DDeploy never actually relies
on trusting it as a claim — `validate_dcode_artifact_authority_bindings`
(117G) independently re-derives the descriptor digest from the ACTUAL
registered artifact and compares; `create_proposal`/`create_v2_proposal`
independently recompute `recompute_dgraph_digest` from the ACTUAL supplied
D-Graph value. Neither ever reads or trusts a caller-supplied
`dscript_ref`/`dcompile_request_digest` field for any authorization
decision (those fields do not even exist on `ProposalInputPayload` —
Section 4) — provenance would only ever be CARRIED, never CONSULTED, by
the existing downstream authority chain. This is the same trust posture
`ProposalSource::Deterministic`/`ProposalSource::Ai` already have today: an
honest, non-authoritative label, never itself a security boundary.

This does mean P1 offers no protection against a determined caller who
bypasses `/api/v1/dcompile/compile` entirely and hand-crafts a matching
`CanonicalDGraphV1`/`CanonicalDServArtifactV1` pair with a coincidentally
plausible-looking (but never-actually-compiled) provenance-shaped
response — but no persisted record would prevent that either unless
DDeploy were changed to REQUIRE and cross-check a server-side compile
record at proposal-creation time, which is a real, distinct, much larger
authority change this audit does not recommend (Section 46 explicitly
rules out DCompile-time authority transitions absent strong evidence, and
none exists here). **P1 is therefore not a compromise weaker than what a
persisted design would achieve for authority purposes — it is exactly as
strong, at far less complexity, because authority was never going to route
through provenance either way.**

P2 (a standalone persisted compile-result store) is rejected for now:
nothing consumes it (no caller has asked to look up "what did compile
number N produce"), matching this project's own repeated, explicit
"no speculative API surface without a proven consumer" discipline
(117B/C/D/E all state this verbatim about their own core functions before
this phase gave them one real consumer). P3/P4 (persist only once a
proposal references it / attach optional provenance to `ProposalRecord`)
remain legitimate FUTURE options once a real caller demonstrates a need
for historical audit beyond what the caller's own copy of the response
already provides — deferred, not rejected outright, exactly like 117H
deferred DCompile provenance persistence in its own closing report.

## 13. Compile-result identity decision

**Not needed**, for the same reason 117E already gave for rejecting a
bundle-level INPUT digest, re-evaluated against the new consumer and found
still to hold: nothing under the recommended P1 model is stored or looked
up BY a compile-result identity — a caller holds the full response value
in hand and acts on its individual, already-identified parts (`dgraph_ref`,
`dcompile_request_digest`, each artifact's own `descriptor_digest`)
directly. Inventing a single wrapping digest would (as 117E already found)
force an unproven, non-obvious ordering/normalization decision (e.g.
whether `dcode_artifacts` order is semantically unordered for digesting
purposes) with no consumer to justify making that call. **If a future
phase adopts P2/P3 persistence with a real keyed store, a compile-result
identity becomes necessary at that point** (as the store's primary key) —
this conclusion is conditioned on the persistence model, not permanent.

## 14. DCode registry handoff

`dcode_artifacts: Vec<DCompileArtifactResultV1>` (Section 9) makes
registration trivial and exact: each entry's `artifact` field is POST-able
directly, byte-for-byte, to `/api/v1/dcode/artifacts` with zero
recomputation of `uri`/`content_id`/ABI/safety/descriptor metadata by the
downstream caller — all of that is already DCompile output (117E). The
response additionally carries each artifact's own `descriptor_digest`
independently, so a caller does not even need to inspect
`declare_dserv_artifact`'s own response to learn it (though it will match
byte-for-byte if declared unmodified — both are `recompute_dserv_artifact_digest`
over the identical value).

## 15. DDeploy binding handoff

`dcode_artifact_authority: BTreeMap<String, DCodeArtifactAuthorityBindingV1>`
(Section 9) is constructed purely from `dcode_artifacts` — every entry's
`(service_id, descriptor_digest)` pair becomes one map entry — and is
**exactly** the type `ProposalInputPayload.dcode_artifact_authority`/
`DeployV2CandidatePayload.dcode_artifact_authority` already expect
(Section 4). A caller can pass this field through completely unmodified.
This grants no authority by existing in the response (Section 34) — it
only becomes a real binding once actually submitted inside a real
`create_proposal`/`create_v2_proposal` call, which independently
revalidates it against current registry state regardless of where it came
from (117G, unchanged, untouched by this audit).

## 16. Authority boundaries (Section 42 of the task, in full)

| Concern | Owner | DCompile may produce? | DCompile may activate? |
|---|---|---|---|
| Source identity | D-Script | Yes (echoes/digests it) | No |
| Mapping intent | D-Compile-request | Yes (produces + digests it) | No |
| Topology | D-Graph | Yes (candidate only) | No |
| D-Code descriptor | `CanonicalDServArtifactV1` | Yes (candidate only) | No |
| Registry entry | D-Code registry | No (never calls `declare_dserv_artifact_revision`) | No |
| D-Code active revision | D-Deploy binding | No (produces the CONVENIENCE map only, never binds it) | **No** |
| Placement | D-Map/planner | No | No |
| Proposal | D-Deploy | No (never calls `create_proposal`) | No |
| Acceptance | D-Deploy | No | No |
| Installed module bytes | D-Node `DCodeModuleStore` | No (never touches bytes) | No |
| Invocation authorization | `dcode_authorization` | No | No |

**Every "may activate?" answer is NO**, confirmed by design — this table
is the direct evidence base for Section 34's requirement and Section 7's
"pure, deterministic, side-effect-free" constraint.

## 17. Determinism

Confirmed preserved by construction: nothing in `DCompileSubmissionV1`
(Section 8) or `DCompileCandidateResponseV1` (Section 9) carries a
timestamp, a random/UUID compile identifier, a server hostname, or a
D-Node identity — the response is a pure function of the request's own
semantic content, exactly like `DCompileDGraphCandidate`/
`DCompileCandidateBundle` already are internally (117C/E, unchanged). Two
identical submissions at different times, from different callers, produce
byte-identical responses.

## 18. Empty program

Preserved. An empty `CanonicalDScriptV1` (`functions: []`) with an empty
`services`/`dcode_services` list produces a structurally valid, empty
`CanonicalDGraphV1`/empty `dcode_artifacts` list — exactly 117C/E's own
already-proven behavior (`c7_empty_dscript_and_empty_request_lowers_to_empty_valid_graph`).
This audit finds no reason for the API layer to reject this: compile
validity and deployment validity remain two separate questions (the task's
own Section 24) — whatever DDeploy/operational-activation later decides
about an empty deployment is unrelated to whether compiling an empty
program is itself a legitimate, successful operation.

## 19. Recompile behavior

The design directly supports both first-deploy and recompile without any
special-casing: calling `/api/v1/dcompile/compile` twice with the SAME
`dscript`/mapping but a DIFFERENT `dcode_services[i].sha256` for one
service produces a response whose `dgraph`/`dgraph_ref` are BYTE-IDENTICAL
to the first call (D-Graph carries no artifact reference at all, 117D
Section 11 — unaffected by a module-digest-only change) while
`dcode_artifacts[i].descriptor_digest` differs for exactly that service.
This is precisely 117D/117G's own established "D1/D2 coexistence" shape —
this audit adds nothing new here, it confirms the production response
format represents it honestly (the caller sees an unchanged D-Graph and a
new artifact candidate, exactly matching reality, never implying the
D-Graph itself needed to change for a pure D-Code-only redeploy).

## 20. Error model

Every existing structured-finding error type is preserved and surfaced,
never flattened. Recommended mapping, following `api::ddeploy.rs`/
`api::dcode.rs`'s own established `unprocessable(&findings)` / stable
`{"error": code, "findings": [...]}` convention (never a bare `400
"compile failed"`):

| Failure class | Owning error variant | HTTP status | Notes |
|---|---|---|---|
| Malformed D-Script JSON shape (would only arise if `dscript` fails its own embedded structural checks once decoded from the envelope) | `CanonicalDScriptV1::validate()` findings | 422 | never reaches Step 1 revalidation below |
| D-Script/request relational mismatch | eliminated by design for `dscript_ref`-class errors (Section 6); `overlapping_function_mapping`/`unmapped_function`/`unknown_function_mapping` still possible (real mapping errors, not reference errors) | 422 | `DCompileLoweringError::RelationalMismatch` |
| Generated D-Graph invalid | `DCompileLoweringError::GeneratedDGraphInvalid` | 422 | should be structurally unreachable given valid inputs (117C's own `c24` proof), modeled honestly anyway |
| Malformed D-Code input | `DCompileDCodeInputV1::validate()` findings | 422 | `DCompileDCodeLoweringError::InvalidDCodeInput` |
| D-Code input references wrong graph digest | eliminated by design (Section 6) | N/A | this whole class of caller error cannot occur |
| Missing/unknown service coverage | `DCompileDCodeLoweringError::DCodeInputRelationalMismatch` | 422 | |
| Generated artifact invalid | `DCompileDCodeLoweringError::GeneratedArtifactInvalid { service_id, findings }` | 422 | include `service_id` in the JSON body, never only in a message string |
| Digest computation failure | `DigestComputationFailed` | 500 | internal error, not a caller mistake |

## 21. Current-flow diagram

```
datum.dscript/1 JSON            datum.dcompile-request/1 JSON        datum.dcompile-dcode-input/1 JSON
        |                                |                                     |
        v                                v                                     v
decode_and_validate_dscript_json  decode_and_validate_dcompile_request_json    |
        |                                |                                     |
        +--------------> lower_calls_empty_dgraph_candidate                    |
        |                                |                                     |
        +----------------------------------------> lower_dcode_artifact_candidates <---+
                                          |
                                          v
                              DCompileCandidateBundle
                          (in-process value; NO Serialize;
                           NO production caller of any kind)
                                          X
                          dead end -- nothing consumes this today
```

## 22. Target-flow diagram

```
external caller (human, CI, future orchestrator)
        |
        v  DCompileSubmissionV1 { dscript, output_dgraph_id/revision, services, dcode_services }
POST /api/v1/dcompile/compile   (new, this audit's recommendation -- NOT implemented in 117I)
        |
        v  (internally: constructs DCompileRequestV1/DCompileDCodeInputV1 with SERVER-DERIVED refs,
        |   then calls the EXISTING, UNCHANGED lower_dcode_artifact_candidates)
        v
DCompileCandidateResponseV1 {
    dscript_ref, mapping, dcompile_request_digest,
    dgraph, dgraph_ref,
    dcode_artifacts[], dcode_artifact_authority
}
        |
        +--> for each dcode_artifacts[i].artifact: POST /api/v1/dcode/artifacts  (existing, 117G)
        |
        +--> caller supplies DContinuum + placements (operator / deterministic planner / future AI --
        |    entirely outside DCompile's scope, Section 18)
        |
        v
POST /api/v1/ddeploy/proposals { application_id, source, dgraph: <verbatim from response>,
                                  dcontinuum, placements, dcode_artifact_authority: <verbatim from response> }
        (existing, 117G)
        |
        v
POST /api/v1/ddeploy/proposals/:id/accept   (existing, unchanged)
        |
        v
governed execution via the existing 117G/117H exact-revision chain
```

## 23. Failure scenarios (Section 43 of the task, ownership assigned)

- **F1 malformed D-Script:** owned by the new endpoint's own decode step
  (422, `CanonicalDScriptV1::validate()` findings).
- **F2 request references wrong D-Script digest:** **eliminated by design**
  — the caller never supplies `dscript_ref` (Section 6/8).
- **F3 overlapping/unmapped functions:** owned by
  `DCompileRequestV1::validate()`/`validate_against_dscript` (real mapping
  errors survive; only the reference-digest class is eliminated).
- **F4 malformed D-Code input:** owned by `DCompileDCodeInputV1::validate()`.
- **F5 D-Code input references wrong graph digest:** **eliminated by
  design** — the caller never supplies `dgraph_ref` (Section 6/8).
- **F6 artifact invalid:** owned by `GeneratedArtifactInvalid`.
- **F7 caller declares compile-produced artifact but later binds wrong
  digest:** owned entirely downstream, by 117G's
  `validate_dcode_artifact_authority_bindings` at proposal creation/acceptance
  time — outside DCompile's own API, and unaffected by this audit; the
  compile response itself makes this class of caller error unlikely (it
  hands the caller the exact correct binding map) but cannot prevent a
  caller from ignoring it and substituting a different value by hand.
- **F8 caller never registers artifacts:** owned by 117G's
  `F_DCODE_BINDING_ARTIFACT_NOT_FOUND` at proposal-creation/acceptance
  time — DCompile has no visibility into whether registration happened.
- **F9 compile succeeds but placement fails:** owned entirely by
  D-Deploy/placement (capacity, D-Continuum, planner) — completely outside
  DCompile's scope (Section 18).
- **F10 compile succeeds but module bytes are absent on the D-Node:**
  owned by DATUM's `DCodeModuleStore`/`invoke_dcode` (117D/117H) — a
  D-Node-local operational concern DCompile never touches (Section 20 of
  the earlier 117D audit; still true, unaffected by this design).

## 24. Provenance matrix (Section 41 of the task)

| Identity | Computed by | Input/output | Byte-proven? | Semantic role | Persist? (recommended) |
|---|---|---|---|---|---|
| D-Script digest | Server, from caller-supplied D-Script document | Output (of decode) | No — no bytes involved, pure structural hash of declared source | Exact source-content identity | No (P1) |
| D-Compile-request digest | Server, from the internally-constructed request | Output | N/A | Exact mapping/resource/output-identity intent — the only identity capturing D-Function→D-Serv membership | No (P1) |
| D-Graph digest | Server, from the generated candidate | Output | N/A | Exact generated application topology only | No (P1) |
| Module SHA-256 | Externally declared by whoever built the module; formatted, never computed, by D-Compile | Input (declared fact) | **No — NEVER byte-verified by D-Compile**; only verified later, D-Node-side, against actually-loaded bytes | Exact executable bytes (claimed) | No (P1; the registry/module-store already persist the verified reality downstream) |
| D-Serv artifact descriptor digest | Server, from the constructed candidate | Output | N/A (descriptor metadata, not module bytes) | Exact executable metadata/policy/service-binding identity — the most granular of the five | No (P1) |
| Compile-result identity | N/A | N/A | N/A | **Not needed** under P1 (Section 13) | N/A |

## 25. Authority matrix (Section 42 of the task)

See Section 16 above (produced verbatim in the task's requested column
order); duplicated here for the report's own required section numbering
only — no additional content beyond Section 16.

## 26. Historical evidence

Full `git log --all -i --grep` search across ten candidate phrases
("DCompile endpoint", "compile API", "DScript compile", "compile result",
"compiler provenance", "source provenance", "compile bundle", "generated
DGraph", "DCompile handoff", "DCode candidate") returned **zero** hits
except one: `0ad35e2 docs: record Phase 117E DCode candidate construction`
— the already-known, CURRENT Phase 117E closure commit itself. **No
historical, superseded, or prototype API design of any kind was ever
attempted on any branch.** This confirms 117A/D's own earlier full-history
findings (VS1 sketch aside, itself unrelated to API design and already
disclaimed by ADR-0017) extend cleanly through the present: there is
nothing to resurrect, and nothing stale to avoid reintroducing. The
recommendation in this document is being made from first evidence, not
reconciled against any prior attempt.

## 27. Minimum 117J implementation slice

Recommended smallest correct implementation, derived directly from
Sections 6-20 above:

**Include:**
- `dserver/src/core/dcompile_submission.rs` (or similar) — a small,
  new, additive module: `DCompileSubmissionV1`/`DCompileCandidateResponseV1`/
  `DCompileArtifactResultV1` wire types (Section 8-9), plus one pure
  function (e.g. `compile_submission(submission: DCompileSubmissionV1) -> Result<DCompileCandidateResponseV1, DCompileSubmissionError>`)
  that internally constructs `DCompileRequestV1`/`DCompileDCodeInputV1`
  with server-derived refs and delegates entirely to the EXISTING,
  UNCHANGED `lower_dcode_artifact_candidates` — never reimplementing any
  117B-E validation/lowering rule.
- `dserver/src/api/dcompile.rs` — one handler,
  `POST /api/v1/dcompile/compile`, following `api::dcode.rs`'s own
  structured-error-response convention (Section 20).
- The pure convenience helper
  `dcode_authority_bindings_from_compile_candidate` the task's own Section
  16 names — confirmed appropriate to add: it reads only
  `DCompileCandidateBundle`/`DCompileCandidateResponseV1` fields, never
  touches registry/DDeploy state, and is directly reusable by the new
  HTTP handler and by future tests (task's own explicit constraint:
  "MUST NOT inspect registry, write registry, create proposal, accept
  deployment" — satisfied by construction, since it only reads its own
  input parameter).
- Comprehensive local tests: the exact same style/rigor as 117B-E's own
  30-40-test matrices — submission decode/validate positive+negative
  cases, derived-reference correctness (the response's `dscript_ref`/
  `dgraph_ref` actually match independent recomputation), determinism
  (identical submission twice → byte-identical response), empty-program
  behavior, recompile behavior (same D-Graph, new descriptor digest for
  one service), and an end-to-end test that takes a real compile response
  and successfully feeds it, unmodified, through the REAL
  `declare_dserv_artifact_revision`/`validate_dcode_artifact_authority_bindings`/
  `create_proposal`/`accept_proposal` functions already exercised by
  117G's own test suite — proving the handoff is genuinely frictionless,
  not merely asserted to be.

**Exclude (explicitly, per Section 7/34 of this audit and the task's own
Section 44):**
- No `declare_dserv_artifact_revision` call from inside the compile
  handler.
- No `create_proposal`/`accept_proposal` call from inside the compile
  handler.
- No placement/D-Continuum acquisition of any kind.
- No module-byte installation or distribution.
- No provenance persistence (P1, Section 12) — this can follow later,
  evidence-driven.
- No staged (`Option A`) endpoints added alongside the one-shot endpoint.

## 28. Future live-proof design

**The next live phase (117K, per Section 29's numbering below) must drive
the ENTIRE chain through the real `/api/v1/dcompile/compile` entry point
this document designs** — a live test that hand-writes
`CanonicalDServArtifactV1` JSON directly, bypassing the compiler, would
not be a valid Phase 117 closure proof (the task's own Section 28/36
requirement, reaffirmed here with the concrete API now named). Concretely,
the required live chain becomes:

```
DScript (H1 module facts) -> POST /api/v1/dcompile/compile -> candidate (D1)
    -> POST each artifact to /api/v1/dcode/artifacts (register D1)
    -> POST + accept a v1/v2 proposal using the response's own dgraph/dcode_artifact_authority verbatim
    -> governed execution of D1 (117H's exact-revision client)
    -> POST /api/v1/dcompile/compile AGAIN with the SAME dscript/mapping but a NEW dcode_services[i].sha256/size_bytes
       -> candidate (D2), same dgraph/dgraph_ref, different descriptor_digest for that service (Section 19)
    -> register D2 (D1 remains registered, unaffected -- 117G)
    -> prove D1 still active (a fresh authorization still names D1's digest)
    -> create + accept a NEW proposal binding to D2
    -> prove D2 becomes active, D1 becomes historical
    -> governed execution of D2 (117H's exact-revision client, now genuinely exercising a
       compiler-produced D1->D2 transition end to end)
```

**Recompile module preparation recommendation** (task Section 39, decision
deferred to that future phase, not built here): prefer two tiny modules
whose `datum_handle` output is trivially, observably DIFFERENT (e.g. H1
echoes/tags its output "v1", H2 tags "v2") over same-bytes-different-limits,
because only an observable output difference proves EXECUTION actually
switched bytes, not merely that metadata switched — a stronger, more
convincing live proof of the exact invariant Phase 117G/117H exist to
guarantee.

## 29. Phase 117 closure criteria

**Can Phase 117 legitimately close with only core DCompile functions and
internal tests, no production API?** **No — not credibly**, given this
project's own stated operational-system goal and given 117H's own closing
report already flagged that a live proof without a real compiler entry
point would be insufficient. An HTTP API is not optional decoration here;
it is the one remaining piece connecting a genuinely complete, tested
compiler (117B-E) to a genuinely complete, tested governed lifecycle
(117F-H) — without it, "Phase 117" describes two disconnected islands of
otherwise-excellent work. **Phase 117 closure requires, at minimum: this
audit's recommended API implemented (117J) and exercised by a real live
proof (117K/117L, see Section 30).** Provenance persistence (Section 12)
is explicitly NOT a closure requirement — P1 is sufficient indefinitely
unless future evidence says otherwise.

## 30. VM/RPi decision

**117J (API implementation + local tests): NO VM/RPi required.** This is
ordinary `dserver`-side Rust implementation and local test work, exactly
like 117B-H's own implementation phases.

**117K (the live proof described in Section 28): YES — this is the
correct point to start live infrastructure** (fog-01/cloud-01 or
equivalent, a real DServer, real D-Node agents, real module installation).
This directly reaffirms, rather than merely repeats, the expectation
stated in this phase's own task framing (Section 47) — the evidence
gathered in this audit (a fully-specified, evidence-grounded API design
with no remaining open architectural question blocking implementation)
supports proceeding exactly as expected, with no discovered reason to
delay live infrastructure further or to pull it earlier into 117J.

## Files changed

- `documentation/reference/phase117i-production-dcompile-api-handoff-audit.md` (this document, new)

No `dserver/`, `DATUM/`, ADR, or any other file was modified. No
production code, route, schema, or helper of any kind was added.

## A note on an existing documentation inconsistency (observed, not fixed)

While tracing `CreateProposalInput` (Section 4), this audit observed that
`dserver/src/core/ddeploy.rs`'s own doc comment on the
`dcode_artifact_authority` FIELD (added in Phase 117G) states bindings are
"Validated against CURRENT D-Code registry state inside `create_proposal`,"
while the doc comment on `create_proposal` itself, a few lines below,
correctly states the opposite: creation does NOT itself validate bindings,
a caller must call `validate_dcode_artifact_authority_bindings` separately
(confirmed correct by the actual code and by `api::ddeploy.rs::post_create_proposal`,
which does call it separately, before `create_proposal`, exactly as 117G's
own closure document describes). This is a small, stale doc-comment
artifact from 117G's own drafting, not a behavioral defect — Phase 117I
is audit-only and does not modify `dserver/` source, so it is recorded
here rather than corrected; a future phase that touches `ddeploy.rs` for
any other reason should fix the one-sentence field-level comment to match
the function-level comment it currently contradicts.

## Authority review

Explicitly confirmed, one more time, consistent with every prior Phase 117
phase: D-Script remains source identity only. D-Compile-request remains
compile-intent-only, never authority. D-Graph/D-Serv-artifact candidates
remain candidates, never active, registered, or accepted merely by being
returned from the recommended endpoint. D-Code registry remains explicit,
separate registration authority (117G, untouched). D-Deploy remains sole
proposal/acceptance/active-binding authority (117G/117H, untouched). D-Map
remains sole placement authority — the recommended request/response
carries no node/placement field anywhere. D-Node module-byte installation
remains untouched, manual, D-Node-local. Stage 107A governed reconciliation
is entirely untouched. No file under `dserver/src/core/`, `dserver/src/api/`,
or `DATUM/` was modified by this phase.
