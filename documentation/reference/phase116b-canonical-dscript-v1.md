# Phase 116B — Canonical DScript v1 Core Contract

**Status:** Phase 116B CLOSED. This is the first production implementation
of DScript in the repository; it is a narrow, conservative typed contract,
not the full phase. **Phase 116 itself has since CLOSED** — see
`documentation/reference/phase116c-dscript-input-admission-and-closure.md`
for the input boundary that completed it and the closure review. This
document's own content below is unmodified and remains an accurate record
of the 116B decisions; a small number of test/fixture details it describes
(the shared fixture's exact JSON shape, and whether unknown fields are
tolerated) were subsequently refined in 116C — see that document for the
current, authoritative state of those two points.
**Branch:** `agent/phase116-dscript`.
**Baseline:** Phase 116A closure, commit `6295d5157d5ea01af2d9d76365ef5497d2f7133b`.

## 1. Purpose

Phase 116A (`documentation/reference/phase116a-dscript-architecture-audit.md`)
found DScript existed only as a documentation-level vocabulary concept, with
zero Rust implementation anywhere in this repository's history. Phase 116B
establishes the smallest trustworthy typed contract a future Phase 117
DCompile can consume — nothing more. Its purpose is explicitly NOT to make
DScript feature-rich.

## 2. Scope

Implemented: `CanonicalDScriptV1`, `CanonicalDFunctionV1`, pure structural
validation, deterministic digesting (test/parity use, not a struct field),
DATUM/dserver parity, and focused tests. Not implemented (deferred per the
task's explicit exclusion list, and consistent with Phase 116A's own DScript
vs. DCompile vs. DEngine boundary, Section 13 of that audit): a parser or
textual DSL, a compiler, DGraph/DCode generation, D-Function→D-Serv
lowering, placement, runtime execution, persistence/current-state
authority, proposal/acceptance lifecycle, REST APIs, reconciliation, and
DMonitor integration.

## 3. Exact v1 type shape

```rust
pub const CANONICAL_DSCRIPT_SCHEMA: &str = "datum.dscript/1";

pub struct CanonicalDScriptV1 {
    pub schema: String,
    pub application_id: String,
    pub dscript_id: String,
    pub revision: u64,
    pub functions: Vec<CanonicalDFunctionV1>,   // #[serde(default)]
}

pub struct CanonicalDFunctionV1 {
    pub function_id: String,
    pub annotations: BTreeMap<String, String>,  // #[serde(default)]
}
```

Both types derive `Debug, Clone, Serialize, Deserialize, PartialEq, Eq`,
matching `CanonicalDGraphV1`/`CanonicalDServV1` exactly.

## 4. Why each field exists

- **`schema`** — matches sibling convention (`CanonicalDGraphV1::schema`,
  `CanonicalDMapV1::schema`) exactly; the task brief's suggested
  `schema_version` was rejected after checking sibling contracts, which
  uniformly use the shorter `schema` field name.
- **`application_id`** — a D-Script is defined (ADR-0001) as the source
  representation *of a D-Application*; every sibling canonical contract
  (`CanonicalDGraphV1`) scopes itself under `application_id`, distinct from
  `project_id` (ADR-0009: "a project references one or more application
  D-Graph/D-Script revisions, never the reverse"). No `project_id` field
  was added, for the same reason `CanonicalDGraphV1` has none.
- **`dscript_id`** — the document's own stable identity, mirroring
  `dgraph_id`/`dmap_id`'s naming pattern.
- **`revision`** — mirrors every sibling contract's revision counter.
  Phase 116A found no evidence of a DScript-specific identity scheme;
  Section 12 (Section 14 of the audit) explains why this is the minimum
  needed to distinguish DScript revisions without implying any
  acceptance/currency authority.
- **`functions`** — the canonical definition's "set of D-Functions." A
  Rust `Vec` is used purely as the wire-format container; see Section 8
  on why array order carries no meaning.
- **`function_id`** — chosen over `function_name` after checking this
  project's own convention: every sibling identity field is `<noun>_id`
  (`service_id`, `call_id`, `dgraph_id`), never `<noun>_name`. This also
  avoids conflating stable identity with a display/symbol name — a future
  D-Compile stage decides how a D-Function's identity maps to any actual
  generated symbol or D-Serv, not this type.
- **`annotations`** — see Section 5.

## 5. Annotation decision and its limitations

Phase 116A's own audit explicitly found no repository evidence defining
what "non-functional annotations" contain — not CPU/memory requirements,
latency targets, QoS, locality, placement hints, retries, timeouts,
readiness dependencies, or security requirements. The task brief
additionally forbade an uncontrolled catch-all (`serde_json::Value` or
`BTreeMap<String, Value>`) unless proven consistent with this project's
canonical-contract discipline — no such precedent was found.

**Decision: `annotations: BTreeMap<String, String>`.** This represents only
the minimum structural fact that a D-Function may carry zero or more named,
opaque values, without assigning any semantic authority to their contents:

- `BTreeMap`, not `HashMap`, guarantees deterministic serialization/
  iteration order — required by the task's own instruction (Section 18) and
  consistent with this project's broader digesting discipline (order must
  never silently affect canonical identity).
- A `String` value (never `Value`) is structurally incapable of encoding a
  nested object, an array, a number, or any recursive structure. No matter
  what key or value text a caller supplies, an annotation cannot smuggle in
  placement, executable identity, or control flow through this field — this
  is a structural guarantee, not a policy one.
- Nothing in `canonical_dscript.rs` interprets any annotation key or value.
  `validate()` only requires a non-empty key (see Section 6); it does not
  recognize, reserve, or give meaning to any specific key name.

**Limitation, stated honestly:** this representation cannot express
anything beyond a flat set of string-to-string facts. It cannot represent
a number, a nested structure, or a typed enum. This is intentional — Phase
116A found no evidence to justify committing to a richer shape, and doing
so now would mean inventing domain semantics the repository does not
support. If a future phase (117 or later) proves a specific annotation
needs richer structure, that is a new, evidence-driven decision, not one
this contract should pre-empt.

## 6. Validation invariants

`CanonicalDScriptV1::validate(&self) -> Vec<CanonicalValidationFinding>` is
pure, self-contained (never touches another canonical contract — DScript
has no DServ/DGraph/DMap references in v1, so unlike `CanonicalDMapV1` it
needs no `validate_against`/cross-reference variant), performs no I/O, no
host mutation, and no global state mutation. Rules enforced:

| Rule | Finding code |
|---|---|
| `schema` must equal `datum.dscript/1` | `schema_invalid` |
| `application_id` non-empty (trimmed) | `missing_field` |
| `dscript_id` non-empty (trimmed) | `missing_field` |
| `revision >= 1` | `revision_invalid` |
| each `function_id` non-empty (trimmed) | `missing_field` |
| no two functions share a `function_id` | `duplicate_function_id` |
| no annotation key is empty (trimmed) | `missing_field` |

An empty `functions` collection validates cleanly (mirrors
`CanonicalDGraphV1`'s own `empty_collections_validate_cleanly` precedent:
whether a zero-function D-Script is *useful* is a D-Compile-level question,
not a v1 structural-validity one). An empty `annotations` map is likewise
always valid. No semantic validation beyond these structural rules is
performed — annotation *values* are never inspected, and no reference
resolution exists because v1 has no external references (Section 9).

## 7. Authority boundaries

Unchanged by this phase, verified explicitly:

- DMap remains sole placement authority — `CanonicalDScriptV1`/
  `CanonicalDFunctionV1` have no field capable of naming a node, host,
  zone, or placement target.
- DForward remains DIoT/provider-topology authority — untouched.
- DCode/ServiceArtifact remain executable-identity authority — no image,
  digest-of-executable, wasm module, binary, or container field exists on
  either type.
- DMonitor remains observed-runtime authority — no health/readiness/
  convergence/freshness/PID/restart-count field exists.
- DGraph/DCall topology is not duplicated — no `services`/`calls`/edge
  field exists on either type.
- No control-flow (sequence/parallel/branch/loop/event/trigger) or
  failure/recovery (retry/timeout/rollback/fallback) construct exists.
- Stage 107A governed reconciliation authorization and lease-cycle
  behavior are entirely untouched — no file in that path was modified.
- No host mutation, network access, or automatic proposal acceptance was
  added — `validate()` and `recompute_dscript_digest()` are pure functions
  over in-memory data.

A dedicated authority-negative test
(`no_forbidden_authority_field_exists_on_the_type`, both crates) serializes
a representative `CanonicalDScriptV1` and asserts none of a forbidden
keyword list (placement, executable-identity, D-Graph-topology, observed-
runtime-state, and control-flow/recovery terms) appears anywhere in the
resulting JSON.

## 8. Function ordering

`functions: Vec<CanonicalDFunctionV1>` is a wire-format container for what
the canonical definition calls a *set* of D-Functions. Array position never
carries execution-order meaning — there is no sequencing semantics in v1.
`recompute_dscript_digest` proves this structurally: it sorts a cloned copy
by `function_id` before hashing, so two semantically identical D-Scripts
that merely list their functions in a different order produce the
IDENTICAL digest (test: `digest_is_order_independent_for_functions`,
mirroring `canonical_dgraph`'s own `digest_is_order_independent_for_services_and_calls`).
The wire JSON itself is never reordered — only the bytes fed into the
digest are normalized.

## 9. No DServ references

The task brief explicitly required re-deriving this rather than assuming
it: the canonical definition says D-Compile maps D-Function → D-Serv,
which is an OUTPUT-side concern of compilation, not an input-side
reference DScript itself must carry. No `service_id`, `target_service_id`,
or `dserv_id` field was added to `CanonicalDFunctionV1`. Consequently, v1's
validation is purely structural, with no reference-resolution layer — there
is nothing yet to resolve a reference against.

## 10. Serialization / parity strategy

Following this project's established no-shared-crate discipline (ADR-0007):
`canonical_dscript.rs` is independently written in both
`dserver/src/core/` and `DATUM/src/agent/`, not shared via a common crate.
Parity is proven the same way `canonical_dgraph` proves it: one shared JSON
fixture (`scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json`,
carrying a recorded `expected_digest_sha256`) is deserialized independently
by each crate's own type; each crate's own `recompute_dscript_digest`
recomputes the digest and both must match the SAME recorded value. This is
a real cross-crate proof, not a re-derivation from a shared type. The
obsolete VS1 fixture sketch (`scripts/fixtures/datum-contracts/vs1/`) was
left completely unmodified and was not used as a basis for this new
fixture.

No `dscript_digest` field was added to the type itself — per the task's
explicit instruction, digest identity remains a later (116E) concern;
`recompute_dscript_digest` is a free function, exactly mirroring
`canonical_dgraph::recompute_dgraph_digest`'s own precedent of never
storing a digest-of-itself on the canonical type.

## 11. Explicit exclusions

Not implemented in 116B, per the task's explicit list and Phase 116A's own
DScript/DCompile/DEngine boundary: a source syntax/parser, a compiler, D-Graph
generation, D-Code generation, D-Function→D-Serv lowering, placement, runtime
execution, persistence/current-state ("accepted"/"current revision")
authority, proposal/acceptance lifecycle, REST APIs, reconciliation
integration, and DMonitor integration. **DScript v1 is a compile-time
source contract. It does not compile itself. It does not generate DGraph
or DCode. It does not place services. It does not represent observed
runtime state.**

## 12. Tests

19 focused tests per crate (38 total), covering:

- Valid minimal document, valid multi-function document, empty `functions`
  collection, empty `annotations` map.
- Negative: wrong schema, blank `application_id`, blank `dscript_id`,
  revision 0, blank `function_id`, duplicate `function_id` (both adjacent
  and non-adjacent), blank annotation key.
- Round-trip serialization preserving identity and annotations.
- Digest determinism, digest sensitivity to function/annotation and
  revision changes, digest order-independence.
- Authority-negative serialization scan.
- Cross-crate fixture parity (`dserver_cross_crate_fixture_produces_the_expected_digest`,
  identical name and pattern on both sides).

All finding-code assertions use `f.code == "..."`, never fragile
full-message string matching.

## 13. Changed files

- `dserver/src/core/canonical_dscript.rs` (new)
- `DATUM/src/agent/canonical_dscript.rs` (new)
- `dserver/src/core/mod.rs` (added `pub mod canonical_dscript;`)
- `DATUM/src/agent/mod.rs` (added `pub mod canonical_dscript;`)
- `scripts/fixtures/datum-contracts/canonical-v1/dscript.sensor-pipeline.json` (new)
- `documentation/reference/phase116b-canonical-dscript-v1.md` (this document)

## 14. Test/build results

- Focused (`canonical_dscript`): dserver 19/19 passed; DATUM 19/19 passed.
- Full dserver suite: 1649 passed, 0 failed (1630 baseline + 19 new).
- Full DATUM lib suite: 1259 passed, 0 failed, 1 ignored (1240 baseline +
  19 new).
- `cargo build --locked --release`: both crates succeeded. DATUM: 86
  pre-existing warnings (unchanged from Phase 115H baseline). dserver: 57
  pre-existing warnings, none new.
- `cargo fmt --check`: the two new files were formatted with `rustfmt`
  directly (brand-new files, per this project's established practice of
  never reformatting pre-existing files with pre-existing deviations) and
  now report zero deviations; all other pre-existing deviations elsewhere
  in both crates are unrelated and unchanged.
- `git diff --check`: clean.

## 15. Future work

Deferred to later Phase 116 sub-steps or to Phase 117, not decided here:
whether a textual source syntax is ever needed (116C, only if JSON
authoring proves insufficient); deeper semantic validation once concrete
DCompile-direction evidence exists (116D); identity/versioning/proposal-
acceptance wiring against the existing D-Deploy flow (116E); and, for
Phase 117 itself, the actual D-Function→D-Serv lowering algorithm and
D-Graph/D-Code generation this contract exists to feed.
