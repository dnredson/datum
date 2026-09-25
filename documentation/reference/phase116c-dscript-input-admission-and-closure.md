# Phase 116C — Canonical DScript Input Boundary + Phase 116 Closure

**Status:** Phase 116C CLOSED. **Phase 116 CLOSED.**
**Branch:** `agent/phase116-dscript`.
**Baseline:** Phase 116B closure, commit `6982d4887c0df8242af71db9118169d55e56a5c3`.

This document establishes the pure boundary by which untrusted serialized
`datum.dscript/1` JSON becomes a typed, structurally-valid, digest-known
`CanonicalDScriptV1` (or a deterministic, structured explanation of why
not), and performs the Phase 116 closure review the task explicitly
required rather than assumed.

## 0. Change from the original illustrative decomposition

Phase 116A proposed an illustrative `116B/116C/116D/116E/116F` split. That
was explicitly not a mandatory checklist. Evidence from 116A and 116B now
shows three of those illustrative slots are not currently justified: a
bespoke textual grammar/parser (JSON already IS the authoring syntax, matching
every other canonical contract), deeper semantic validation (`datum.dscript/1`
intentionally has no typed domain semantics beyond D-Functions and opaque
flat annotations), and a persisted "current DScript" proposal/acceptance
authority (no evidence requires one, and D-Deploy's existing lifecycle
concerns placement, not upstream source). This document is therefore 116C
performing the one thing still genuinely missing — the pure input boundary —
and then closing Phase 116 on that basis, rather than manufacturing 116D/E/F
to fill numbering slots.

## 1. Exact input boundary

`dserver/src/core/canonical_dscript.rs`:

```rust
pub enum DScriptInputError {
    Deserialization(String),
    Validation(Vec<CanonicalValidationFinding>),
    DigestComputation(String),
}

pub struct ValidatedDScriptInput {
    pub document: CanonicalDScriptV1,
    pub digest_sha256: String,
}

pub fn decode_and_validate_dscript_json(
    json: &str,
) -> Result<ValidatedDScriptInput, DScriptInputError>
```

Pipeline: JSON text → `serde_json::from_str::<CanonicalDScriptV1>` (structural
decode, `deny_unknown_fields` enforced) → `CanonicalDScriptV1::validate()`
(the ONE existing structural validator, not duplicated) →
`recompute_dscript_digest` (always freshly computed, never trusted from the
input) → `ValidatedDScriptInput`.

This is an INPUT BOUNDARY, not authority. A successful result means only:
"this exact serialized source decoded into the canonical v1 type and passed
its self-contained structural contract." It does not mean current, accepted,
approved, compiled, trusted, authorized, executable, or placed.

## 2. Naming: why not "admission"

The task's own naming caution was checked empirically, not assumed. This
repository already uses "admitted"/"admission" with an established,
distinct, authority-adjacent meaning:

- `dserver/src/core/dmonitor_interpretation.rs`: `admitted`/`admitted_evidence`
  — whether an observed DMonitor observation contributes to a drift/
  convergence decision.
- `dserver/src/api/operational_dgraph.rs`: "re-admission" — a decision tied
  to Resource Registry authority revalidation on load/restart.

Reusing "admission" for this boundary would invite exactly the confusion
Phase 116A/116B have been careful to avoid. The function is named
`decode_and_validate_dscript_json` instead — literal, unambiguous, and
matching one of the task's own suggested alternatives.

## 3. Error/result model

`DScriptInputError` distinguishes three failure classes, never conflating
them: `Deserialization` (malformed JSON, wrong types, a duplicate key for a
known field, or an unrecognized field — the document never reaches
`validate()`), `Validation` (the document decoded fine but failed its own
structural contract — carries the ACTUAL `Vec<CanonicalValidationFinding>`
`validate()` produced, never a re-derived or paraphrased version), and
`DigestComputation` (theoretically possible if JCS canonicalization ever
failed on an already-valid document; unreachable in practice given this
type's field set is `String`/`u64`/`BTreeMap<String, String>` only — no
field can ever hold a non-finite float — but modeled honestly as a distinct
variant rather than unwrapped or folded into `Deserialization`).

## 4. Unknown-field decision

**Decision: `#[serde(deny_unknown_fields)]` on both `CanonicalDScriptV1` and
`CanonicalDFunctionV1`, on both crates.**

This was an explicit decision, not a default. Sibling precedent was checked
directly: `canonical_dgraph.rs`/`canonical_dmap.rs` (Stage-110-era) do NOT
use `deny_unknown_fields`, but `canonical_dforward.rs`/`canonical_dmap_v2.rs`/
`canonical_dmonitor.rs` (Phase 112+) DO — and ADR-0024 states this directly:
"Digest and validation follow the newer, stricter Phase-112+ canonical
convention rather than the older Stage-110-era one: `deny_unknown_fields` on
every struct." `datum.dscript/1` is a brand-new v1 contract (Phase 116,
later than Phase 112) with zero deployed backward-compatibility burden, so
the newer, stricter convention applies with no migration cost. No new ADR
was needed for this specific decision — it is already established, cited,
project-wide policy; ADR-0025 remains scoped to the annotation-representation
decision alone.

This closes a real integrity gap the task named directly: without it,
`{"function_id": "decode", "node_id": "fog-01"}` would silently drop
`node_id`, and a human reader might wrongly believe placement was expressed.
With `deny_unknown_fields`, that input is a hard deserialization error
instead (test: `i14_unknown_field_is_rejected_not_silently_ignored`).

**Consequence for the existing fixture:** the Phase 116B fixture
(`scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json`)
previously carried sibling `x_note`/`expected_digest_sha256` metadata keys
alongside the canonical fields. Under `deny_unknown_fields` those keys would
themselves be rejected. The fixture was updated to be the PURE canonical
document only — matching `canonical_dforward`'s own fixture shape exactly —
and the expected digest moved into the test code as a literal (matching
`canonical_dforward::real_positive_fixture_digest_matches_dmap_v2_bundle_reference`'s
own established pattern). The digest value itself is unchanged
(`sha256:3f01cf0ee34e8e368d5f0241617588b04e894564f82aa553565e69c723ed38ae`):
removing non-canonical sibling JSON keys never affected it, since the digest
was always computed from the deserialized typed struct, which never had
those fields to begin with. This is fixture/test housekeeping made necessary
by the `deny_unknown_fields` decision, not a change to `CanonicalDScriptV1`'s
shape.

## 5. Duplicate JSON object key investigation

Investigated empirically, not assumed: a scratch test (`{"x":"A","x":"B"}`
against a minimal derived-`Deserialize` struct, with and without
`deny_unknown_fields`) showed `serde_json` returns
`Error("duplicate field \`x\`", ...)` in BOTH cases. **Finding: serde's
derived `Deserialize` implementation rejects a duplicate JSON object key for
a KNOWN field unconditionally — this is independent of
`deny_unknown_fields`, which only governs UNRECOGNIZED keys.** No bespoke
duplicate-key handling was written because none is needed; this is
documented behavior proven by
`duplicate_json_object_key_for_a_known_field_is_rejected` (both crates,
asserting `DScriptInputError::Deserialization` with a "duplicate field"
message on the dserver side, and a plain deserialization error on the
DATUM side).

## 6. Digest behavior

`decode_and_validate_dscript_json` never accepts a caller-provided digest —
there is no `dscript_digest` field on `CanonicalDScriptV1` to read one from
in the first place (the Phase 116B decision, unchanged). The digest returned
is always freshly computed by `recompute_dscript_digest` from the document
this same function just decoded. Proven three ways: (1)
`digest_returned_by_the_boundary_is_always_freshly_recomputed` — two
byte-different but semantically identical JSON documents (compact vs.
pretty-printed, re-serialized via `serde_json::Value` so no string content
is touched) decoded independently yield the same digest; (2)
`caller_provided_dscript_digest_field_is_rejected_never_trusted` — an
injected `"dscript_digest": "sha256:deadbeef"` field is rejected outright
by `deny_unknown_fields`, so it can never even reach a point where it could
be mistaken for authority; (3) `i3_function_array_permutation_yields_the_same_digest_through_the_boundary`
— reordering `functions` in the input JSON yields the same digest through
the full boundary, not merely through `recompute_dscript_digest` called
directly.

## 7. JSON authoring decision

**Canonical DScript v1 is authored/interchanged as JSON today.** This
follows the existing canonical DATUM contract convention (DGraph, DMap,
DForward, DMonitor are all JSON-authored with no custom syntax). This is
not unfinished work blocking Phase 116 closure — it is a deliberate v1
decision, reaffirmed by this phase after actively looking for evidence a
custom syntax is needed and finding none (Section 8).

## 8. Why no custom parser was implemented

No evidence, in Phase 116A's audit or anywhere found since, shows DScript
needs a bespoke textual grammar. JSON + serde already IS the source
serialization syntax, matching every other canonical DATUM contract's
established authoring convention. No parser/lexer/grammar dependency
(`pest`, `nom`, `lalrpop`, `logos`, `chumsky`, `tree-sitter`) was added to
any manifest. This is classified as **DEFERRED UNTIL EVIDENCE REQUIRES IT**,
not an incomplete blocker — a future human-friendly syntax may be
introduced only if a concrete requirement demonstrates JSON is insufficient,
at which point it would compile down to exactly this same typed
`CanonicalDScriptV1`, not replace it.

## 9. Why no deeper semantic validation was implemented

`datum.dscript/1` intentionally carries no typed domain semantics beyond a
D-Function's identity and its opaque flat annotations (Phase 116B, ADR-0025).
There is nothing to validate more deeply than `CanonicalDScriptV1::validate()`
already does without inventing the exact domain semantics (CPU/memory/QoS/
placement/etc.) Phase 116A and 116B both explicitly declined to invent.
Annotation VALUES remain uninterpreted — Section 11 below states this
explicitly as a standing rule, not merely an omission.

## 10. Why no persistence/current-DScript authority was implemented

No evidence anywhere in this repository shows a persisted "current DScript"
proposal/acceptance lifecycle is required to begin Phase 117. D-Deploy's
existing proposal/acceptance machinery concerns candidate PLACEMENT
(`CanonicalDMapV1`), not upstream source — repurposing it for DScript merely
because it already has a lifecycle would conflate two different kinds of
"currency" (source revision vs. accepted placement) that this project's
architecture has consistently kept separate. A DScript revision can be an
explicit, immutable, caller-supplied `CanonicalDScriptV1` handed directly to
a future DCompile call — exactly like DGraph/DMap documents are already
authored and handed around as explicit JSON in tests today — without
becoming global desired-state authority. This is classified as **DEFERRED
UNTIL EVIDENCE REQUIRES IT**.

## 11. Annotation semantics remain opaque (reaffirmed)

No annotation key or value is interpreted, reserved, or given meaning
anywhere in this phase's additions. No key such as `cpu`/`memory`/`node`/
`qos`/`placement`/`timeout`/`retry` is recognized specially. An annotation
string may contain arbitrary text; that text carries no canonical
operational authority merely because it appears inside an annotation.
ADR-0025 remains fully authoritative and unmodified.

## 12. DFunction set semantics (reaffirmed)

`functions` remains conceptually a SET; input array order is never execution
order. The input boundary introduces no ordering semantics of its own — I3
(`i3_function_array_permutation_yields_the_same_digest_through_the_boundary`)
proves permutations that decode to the same semantic document produce the
same digest through the full boundary, not merely through the underlying
digest function called directly.

## 13. DDeploy boundary review

DScript was NOT added to `DDeployProposalV1`, `ProposalRecord`,
`DDeployAcceptanceV1`, or `DDeployState` in this phase. D-Deploy's semantics
(candidate placement → explicit acceptance → authoritative
`CanonicalDMapV1`) remain untouched and unreferenced by
`canonical_dscript.rs`. How an explicit DScript revision/digest flows through
D-Compile into D-Graph + D-Code and then into the existing governed
proposal/acceptance mechanisms is explicitly left to Phase 117 to determine
— not guessed at here.

## 14. Mirror discipline decision

Per ADR-0007, contract parity (the wire shape, INCLUDING the
`deny_unknown_fields` decision) is mandatory on both `dserver::core::canonical_dscript`
and `DATUM::agent::canonical_dscript`, and is proven identically on both
sides (unknown-field rejection test, duplicate-key rejection test,
caller-provided-digest rejection test, cross-crate fixture parity test).

**`decode_and_validate_dscript_json`/`DScriptInputError`/
`ValidatedDScriptInput` were added ONLY to `dserver`.** Phase 117 DCompile —
this boundary's first expected substantive consumer — is expected to be a
server-side responsibility (D-Compile's output, a D-Graph proposal, flows
into D-Deploy's existing server-side proposal/acceptance mechanism, Section
13). DATUM (the node agent) has no proven consumer for validating raw
DScript source; it only ever receives already-compiled operational
reconciliation contexts. Adding the same helper function to DATUM merely for
symmetry would be unnecessary production API surface with no caller — the
task's own instruction ("parity of every helper function is not
automatically mandatory... make the smallest defensible decision") is
applied here directly.

## 15. Tests

**dserver** (36 tests in `canonical_dscript`, 17 new this phase): I1-I5
positive (minimal valid JSON, multi-function fixture, permutation digest
equality, annotation preservation, shared-fixture pass-through), I6-I14
negative (malformed JSON, wrong schema, blank `application_id`/`dscript_id`,
revision 0, blank `function_id`, duplicate `function_id`, blank annotation
key, unknown field), the caller-provided-digest negative case, the
duplicate-JSON-key documentation test, and the always-fresh-digest proof.

**DATUM** (22 tests in `canonical_dscript`, 3 new this phase): unknown-field
rejection, duplicate-key rejection, and caller-provided-digest rejection —
proving wire-CONTRACT parity without duplicating the admission-boundary
helper (Section 14).

All finding-code assertions use `f.code == "..."`, never full-message string
matching.

## 16. Test/build results

- Focused (`canonical_dscript`): dserver 36/36 passed; DATUM 22/22 passed.
- Full dserver suite: 1666 passed, 0 failed (1649 baseline + 17 new).
- Full DATUM lib suite: 1262 passed, 0 failed, 1 ignored (1259 baseline + 3
  new).
- `cargo build --locked --release`: both crates succeeded (a stale leftover
  `DServer.exe` process from a prior session's testing was stopped first,
  the same known issue as Phase 115H/116B).
- `cargo fmt --check`: both modified files reformatted directly with
  `rustfmt` (they were already brand-new files as of Phase 116B; this
  phase's additions were formatted the same way) and report zero
  deviations; all other pre-existing deviations elsewhere in both crates
  are unrelated and unchanged.
- `git diff --check`: clean.

## 17. Security / authority review

Confirmed unchanged: DMap remains sole placement authority; DForward still
owns DIoT/provider topology; DCode/ServiceArtifact still own executable
identity; DMonitor still owns observed runtime reality; D-Deploy remains
placement proposal/acceptance authority (Section 13); Stage 107A governed
reconciliation/lease authorization is untouched (no file in that path was
modified). `decode_and_validate_dscript_json` grants no runtime or
deployment authority of any kind — it is a pure, in-memory, side-effect-free
function (no filesystem access, no network access, no host mutation, no
database write) that returns a typed value or a structured error, nothing
more.

## 18. Files changed

- `dserver/src/core/canonical_dscript.rs` (extended: `deny_unknown_fields`,
  input boundary, new tests)
- `DATUM/src/agent/canonical_dscript.rs` (extended: `deny_unknown_fields`,
  parity tests)
- `scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json`
  (reduced to the pure canonical document)
- `documentation/reference/phase116c-dscript-input-admission-and-closure.md`
  (this document)

## 19. Phase 116 closure matrix

| # | Question | Answer |
|---|---|---|
| A | Does DScript now have a canonical typed contract? | **YES** — `CanonicalDScriptV1`/`CanonicalDFunctionV1`, `datum.dscript/1` (Phase 116B). |
| B | Can untrusted serialized DScript be deterministically decoded and structurally validated? | **YES** — `decode_and_validate_dscript_json` (Section 1). |
| C | Can its canonical digest be recomputed without trusting caller input? | **YES** — no `dscript_digest` field exists to trust; always freshly recomputed (Section 6). |
| D | Are D-Function set/order semantics defined? | **YES** — a set, keyed by `function_id`; array order carries no meaning; digest is order-independent through the full boundary (Section 12). |
| E | Are annotations structurally represented without invented semantics? | **YES** — opaque `BTreeMap<String, String>`, ADR-0025, reaffirmed unmodified (Section 11). |
| F | Does DScript remain unable to claim placement/runtime/executable authority? | **YES** — no such field exists on either type; authority-negative tests on both crates confirm this at the serialization level. |
| G | Is any bespoke textual parser actually required to begin DCompile? | **NO** — JSON authoring is sufficient (Sections 7-8). |
| H | Is any DScript persistence/current-state authority actually required to begin DCompile? | **NO** — an explicit, immutable, caller-supplied document suffices (Section 10). |
| I | Is any deeper semantic validation possible today without inventing semantics? | **NO** — there is no domain semantics to validate beyond what `validate()` already checks (Section 9). |

**A-F = YES, G-I = NO. Phase 116 closes.**

## 20. Phase 116 deferred items

Classified as **DEFERRED UNTIL EVIDENCE REQUIRES THEM** — not incomplete
blockers:

- Custom DScript textual DSL/parser.
- Rich annotation types (beyond opaque flat strings).
- Control flow, dependencies, or any sequencing construct.
- Semantic interpretation of annotation values.
- Persisted "current DScript" state.
- A DScript proposal/acceptance HTTP API.

A future `datum.dscript/2` schema may be appropriate if later evidence
requires incompatible richer semantics. `datum.dscript/1` is not overloaded
to anticipate that; it is intentionally minimal.

## 21. Next phase

**Phase 117 — DCompile** may begin. Per the roadmap, DCompile compiles a
DScript into a DGraph and DCode, mapping D-Function → D-Serv with
potentially 1:1, 1:N, M:1, or M:N cardinality. Recommended entry point:
**Phase 117A — DCompile Architecture / Lowering Audit**, determining (before
any implementation): what a generated D-Serv identity should be; how
D-Function groups are selected/grouped into a D-Serv; how D-Call edges are
derived, if at all; what D-Code generation actually means in today's
implementation; how a generated D-Graph/D-Code enters the existing D-Deploy/
ServiceArtifact paths (Section 13's open boundary); and how deterministic
compilation is proven. None of those questions are answered here.
