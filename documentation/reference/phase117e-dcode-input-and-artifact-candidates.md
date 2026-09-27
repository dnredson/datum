# Phase 117E — External DCode Content/Policy Input + Deterministic DServ Artifact Candidate Construction

**Status:** Phase 117E CLOSED. **Phase 117 remains OPEN.** This is a PURE
D-Compile step: no bytes built, no registration, no distribution, no DDeploy
call, no placement, no invocation authorization, no execution.
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117D closure, commit `2c5af716d6e1642764d67c0078af928426fee9b0`.

## Exact input schema/types

`dserver/src/core/dcompile_dcode.rs`:

```rust
pub const CANONICAL_DCOMPILE_DCODE_INPUT_SCHEMA: &str = "datum.dcompile-dcode-input/1";

pub struct DCompileDCodeInputV1 {
    pub schema: String,
    pub application_id: String,
    pub dgraph_ref: CanonicalContractReferenceV1,   // {id, revision, digest}
    pub services: Vec<DCompileDCodeServiceInputV1>, // #[serde(default)]
}

pub struct DCompileDCodeServiceInputV1 {
    pub service_id: String,
    pub artifact_id: String,
    pub version: String,
    pub sha256: String,       // exactly 64 lowercase hex, no prefix
    pub size_bytes: u64,
    pub ports: DServArtifactPortsV1,   // reused directly from dserv_artifact.rs
    pub limits: DServArtifactLimitsV1, // reused directly from dserv_artifact.rs
}
```

Both `#[serde(deny_unknown_fields)]`. `datum.dcompile-dcode-input/1` follows
the exact naming convention 117B established for
`datum.dcompile-request/1` — no evidence supported a different family name.

## DGraph reference/binding decision

`dgraph_ref: CanonicalContractReferenceV1` (`{id, revision, digest}`) binds
the input immutably to the exact candidate D-Graph it was prepared for,
reusing the same type 117B already reused for `dscript_ref` — not a new
coupling; `CanonicalContractReferenceV1` remains this project's one
genuinely shared reference shape. Relational validation
(`validate_against_dgraph`) checks `application_id`/`dgraph_ref.id`/
`dgraph_ref.revision`/`dgraph_ref.digest` against a **freshly-produced**
`DCompileDGraphCandidate` — never a caller-supplied one (see the trust
section below).

## Service-coverage semantics (not DGraph validity)

For a bundle to construct successfully, every generated D-Serv must have
exactly one D-Code input, and no unknown service may appear:
`unknown_service_dcode_input` and `missing_service_dcode_input` are
relational findings (need the real candidate D-Graph to detect);
`duplicate_service_input` (two entries in the SAME input list sharing a
`service_id`) is a purely internal structural finding, detected without any
D-Graph at all. **This is a D-Compile-candidate-bundle completeness rule
only.** `CanonicalDGraphV1::validate()` is never touched, extended, or made
to depend on registry/D-Code state — a D-Graph with zero services, or with
services that have no D-Code input at all, remains structurally valid on
its own terms, exactly as Phase 117C already proved for the empty case.

## External module-content identity representation

Exactly one real external fact is required per service: `sha256` (bare, 64
lowercase hex, no `sha256:` prefix — deliberately narrower than
`CanonicalDServArtifactV1.dcode.sha256`'s own tolerant parser, per Phase
117D's explicit recommendation to accept only one canonical spelling) plus
`size_bytes`. Both are treated as **declared, not proven**: D-Compile never
receives module bytes and cannot establish either describes real content —
that proof remains entirely DATUM-side (`DCodeModuleStore::install`/
`invoke_dcode`'s own verification against actually-loaded bytes, Phase
117D Section 8). This module only formats/binds the declared digest into
ADR-0021's three required representations.

## sha256 validation rule

`is_bare_sha256_hex`: exactly 64 characters, all ASCII hex digits, no
uppercase — rejecting a `sha256:`-prefixed spelling outright
(`module_sha256_malformed`), even though the underlying artifact field
itself would accept either. One canonical input spelling only.

## size_bytes treatment

Copied verbatim from the input — never computed (D-Compile has no bytes to
measure), never derived from limits/resources/digest. No `> 0` rule was
added: no evidence in `CanonicalDServArtifactV1::validate()` or ADR-0021
requires `size_bytes` to be non-zero (unlike `limits.*`, which the artifact
validator does require `> 0`) — inventing a stricter rule here would be
exactly the "diverging second definition" the task explicitly warned
against.

## URI / content_id derivation

Given the declared digest `H`: `dcode.uri = "datum-blob://sha256/{H}"`,
`dcode.content_id = "sha256:{H}"`. Both are pure syntactic transformations
of the one supplied fact, never independently accepted from the caller (an
attempt to supply either directly is rejected as an unknown JSON field —
tests `e33`/`e34`). Constructing this URI does **not** upload, publish, or
make bytes retrievable anywhere — Phase 117D already established
`datum-blob://` has no network resolver anywhere in this codebase.

## artifact_id / version semantics

Both are explicit, required, non-empty external input, copied verbatim —
never generated, never derived from `service_id`/module digest/D-Graph
digest (Phase 117D Section 9/12: the registry keys on
`(application_id, dserv_id)`, never on either). `version` in particular
carries **zero replacement/upgrade/currentness authority**: changing it can
never replace an already-registered artifact for the same
`(application_id, dserv_id)` — Phase 117D's own P0 finding, reaffirmed here
and deliberately not touched (see Explicit exclusions).

## limits / ports representation

Both reuse the canonical `dserv_artifact.rs` types directly
(`DServArtifactLimitsV1`, `DServArtifactPortsV1`) rather than being mirrored
or narrowed — required fields, never defaulted, never derived from
`CanonicalDGraphV1.resource_requirement` (a different concept: deployment
capacity vs. per-invocation sandbox bound, Phase 117D Section 17), D-Script
annotations, or function count. `ports` stays a **required** field (not
`#[serde(default)]`) so that "this module declares zero ports" is always an
explicit statement rather than an accidental omission — even though an
empty `DServArtifactPortsV1` is itself the zero-value produced either way.

**Deliberately not duplicated here:** `limits.* > 0` and any port-shape
constraint. `CanonicalDServArtifactV1::validate()` is the sole enforcer;
`e30_zero_limit_value_caught_only_by_generated_artifact_validation` proves
directly that `DCompileDCodeInputV1::validate()` lets a zero limit value
through structurally, and the FINAL artifact-validation step (Section
below) is what actually catches it — proving that gate is real, not
decorative, without inventing a second, potentially-divergent definition of
artifact validity. (No equivalent "invalid port metadata" structural rule
exists anywhere in the canonical artifact validator today, confirmed by
direct inspection — this module does not invent one either.)

## ABI / safety / state / instantiation / allowed_stages fixed wiring

All assigned as literal constants, exactly matching Phase 117D's Section 6
findings — never exposed as input fields, never called "defaults" since no
second legal v0.1 value exists for any of them:

```rust
abi: DServArtifactAbiV1 {
    name: "datum-dnode".to_string(),
    version: "0".to_string(),
    required_exports: DNODE_ABI_V0_REQUIRED_EXPORTS.iter().map(|s| s.to_string()).collect(),
    allowed_imports: Vec::new(),
},
safety: DServArtifactSafetyV1::default(),  // confirmed: every flag false
state_model: "stateless".to_string(),
instantiation: "fresh_instance_per_invocation".to_string(),
constraints: DServArtifactConstraintsV1::default(),  // allowed_stages = []
```

`allowed_stages` is unconditionally `[]` and is NOT exposed in the input
contract — Phase 117D established a non-empty value is structurally legal
but permanently unauthorizable under v0.1 (`F_DCODE_ARTIFACT_STAGE_CONSTRAINT_UNVERIFIABLE`);
D-Compile must never manufacture an artifact guaranteed to fail
authorization.

## qos / provenance / probes / signature_ref behavior

All three `Value` fields are set to `serde_json::Value::Null` — Phase 117D
found zero production interpretation of any of them, so D-Compile v1
intentionally emits no semantics here rather than inventing a
caller-passthrough or a catch-all use. **`provenance` in particular is
never used to smuggle `dscript_digest`/`dcompile_request_digest`/
`dgraph_digest`** — those remain typed, separate compiler-result fields on
`DCompileCandidateBundle`/`DCompileDGraphCandidate`, never folded into this
uninterpreted JSON field. `signature_ref` is set to `None` — no signing
system exists in this architecture; inventing a reference would be a fake
completeness claim.

## Candidate-construction API

```rust
pub fn lower_dcode_artifact_candidates(
    dscript: &ValidatedDScriptInput,
    request: &ValidatedDCompileRequest,
    dcode_input: &DCompileDCodeInputV1,
) -> Result<DCompileCandidateBundle, DCompileDCodeLoweringError>
```

Composition, not reimplementation: Step 1 calls
`dcompile::lower_calls_empty_dgraph_candidate` exactly once and never
re-derives D-Script/D-Compile-request validation, relational validation,
D-Graph construction, or digest verification — any future tightening of
117C automatically remains effective here. Steps 2-3 validate the D-Code
input structurally and relationally against the FRESH candidate Step 1 just
produced. Step 4 constructs one `CanonicalDServArtifactV1` per generated
service, in the candidate D-Graph's own already-deterministic `service_id`
order (never the caller's input-vector order). Step 5 runs the canonical
`CanonicalDServArtifactV1::validate()` on every constructed artifact,
failing the WHOLE bundle closed if any one artifact is invalid — never a
partial success. Step 6 recomputes each artifact's descriptor digest
independently.

## Trust rule (unchanged from 117C, not weakened)

`lower_dcode_artifact_candidates` never accepts a caller-supplied
`DCompileDGraphCandidate` as sufficient proof of anything — it always
derives one itself, fresh, from `dscript`/`request`, inheriting 117C's own
P0 defensive revalidation (structural re-validation + independent digest
recomputation of both wrapper types) automatically. `e37` proves a forged
`ValidatedDScriptInput` fails at Step 1, before this module's own logic
ever runs, with the exact underlying `DCompileLoweringError` surfaced
unchanged.

## Error model

```rust
pub enum DCompileDCodeLoweringError {
    DGraphLowering(DCompileLoweringError),
    InvalidDCodeInput(Vec<DScriptValidationFinding>),
    DCodeInputRelationalMismatch(Vec<DScriptValidationFinding>),
    GeneratedArtifactInvalid { service_id: String, findings: Vec<DGraphValidationFinding> },
    DigestComputationFailed(String),
}
```

`GeneratedArtifactInvalid` uses `canonical_dgraph`'s own
`CanonicalValidationFinding` type (aliased `DGraphValidationFinding`), since
`CanonicalDServArtifactV1::validate()` itself imports and returns that
exact type — matching `dcompile.rs`'s own established per-source-type
aliasing convention exactly, never conflating two nominally-distinct
finding types.

A separate, JSON-decode-only boundary exists for the input type alone
(`decode_and_validate_dcode_input_json`, `DCompileDCodeInputError` with two
variants: `Deserialization`/`Validation`) — mirroring
`canonical_dscript`/`dcompile_request`'s own boundary shape, minus a digest
(see below).

## Result type

```rust
pub struct DCompileDServArtifactCandidate {
    pub artifact: CanonicalDServArtifactV1,
    pub descriptor_digest_sha256: String,
}

pub struct DCompileCandidateBundle {
    pub dgraph_candidate: DCompileDGraphCandidate,
    pub dcode_artifacts: Vec<DCompileDServArtifactCandidate>,
}
```

No `Serialize`/`Deserialize`, no `datum.dcompile-result/1` schema invented —
no proven persistence/API consumer exists yet. `CanonicalDServArtifactV1`
itself was not modified in any way; no D-Compile-specific field
(`dscript_digest`, `dcompile_request_digest`, `dgraph_digest`,
`compiler_version`) was added to it — that provenance lives entirely
outside the canonical executable descriptor, on `DCompileDGraphCandidate`
(unchanged from 117C) and `DCompileDServArtifactCandidate`.

## Why no bundle-level input digest was added

Considered and explicitly rejected (Section 42 of the task): each
artifact's own `descriptor_digest_sha256` already transitively captures
everything semantically meaningful about that service's D-Code input,
since the entire descriptor is constructed directly from it. A
bundle-level "input digest" would be a new identity with no distinct
purpose beyond what the D-Graph digest plus the set of per-artifact
descriptor digests already, together, fully identify. Adding one would
also force a real, non-obvious decision this task explicitly flagged as
unproven by precedent — whether `ports` order is semantically unordered for
digesting purposes (the existing artifact digest normalizes
`abi.required_exports`/`allowed_imports` order but says nothing about
`ports`) — which this phase has no evidence to decide either way. Omitting
the digest avoids inventing that answer.

## Generated artifact self-validation

Every constructed `CanonicalDServArtifactV1` is run through its own
existing `validate()` before being returned — the canonical validator
remains the sole source of truth, never re-derived. `e30`/`e38` prove this
gate is real: a zero `limits` value passes this module's own structural
`validate()` cleanly and is caught only here.

## Descriptor digest handling

`recompute_dserv_artifact_digest` is called once per successfully-validated
artifact, always freshly, never trusted from any input (`e12`).

## Five-identity provenance model (extended from 117C/117D)

| Identity | Meaning |
|---|---|
| D-Script digest | logical source contract |
| D-Compile-request digest | function mapping/resource/topology intent |
| D-Graph digest | generated application topology |
| D-Code module digest (`artifact.dcode.sha256`) | exact declared executable bytes (never verified against real bytes here) |
| D-Serv artifact descriptor digest | exact executable metadata/policy/service binding — the whole descriptor |

Independence proven directly: `e13` (same module bytes, different limits →
same module digest, different descriptor digest) and `e14` (same bytes,
different service binding → same module digest, different descriptor
digest, because `application_id`/`dserv_id` are themselves descriptor
content) — both exactly matching Phase 117D's predicted independence cases.
`e16` reconfirms annotations change D-Script identity but never artifact
policy.

## Function-to-binary proof limitation

Stated explicitly, not glossed over: binding a `service_id` to a declared
module digest proves nothing about whether that module actually implements
the D-Functions the D-Compile request mapped to it. The only enforceable
code-level proof anywhere in this architecture remains: loaded bytes match
the declared digest, and the module satisfies the `datum-dnode/0` ABI/
runtime constraints. No static source-analysis provenance is invented.

## Registry replacement limitation (unsolved, by design)

Phase 117D's P0 (one immutable artifact per `(application_id, dserv_id)`,
no `H1 → H2` replacement mechanism) is untouched. This module can happily
construct an `H2` candidate for an already-`H1`-registered service — that
candidate's mere constructibility does **not** imply the registry would
accept it; `declare_dserv_artifact` is never called by this phase at all.

## DDeploy eligibility limitation (unsolved, by design)

Phase 117D's finding that `ddeploy.rs` has zero D-Code-registry integration
(unlike its real `ServiceArtifact` eligibility path) is untouched. This
phase makes no claim that DDeploy now enforces D-Code completeness for any
candidate it accepts.

## Byte-distribution limitation (unsolved, by design)

`datum-blob://` construction here does not upload, publish, fetch, or
otherwise distribute bytes. Installation onto a D-Node remains the
existing, entirely manual `smartsentinel-dcode-install` CLI path,
unchanged and untouched.

## DCall status

Remains deferred. Ports were exercised with representative matching
`payload_schema` values (`e9`) purely as local interface metadata; no
`CanonicalDCallV1` edge was created, derived, or implied, and
`derive_dcall_route` was never invoked — there is no D-Call edge for it to
route (`e15` reconfirms the D-Graph stays exactly the 117C calls-empty
graph).

## Tests

37 tests in `dserver::core::dcompile_dcode`, covering E1–E30, E32–E38 of the
task's required matrix directly (E31, "invalid port metadata," was
evaluated and found to have no real failure mode to test: `CanonicalDServArtifactV1::validate()`
itself contains no structural port-shape check today — confirmed by direct
inspection — so this task does not invent one either, consistent with
"do not create a diverging second definition of valid artifact ports";
`e9` instead proves non-empty, syntactically valid ports round-trip
correctly).

## Files changed

- `dserver/src/core/dcompile_dcode.rs` (new)
- `dserver/src/core/mod.rs` (added `pub mod dcompile_dcode;`)
- `documentation/reference/phase117e-dcode-input-and-artifact-candidates.md` (this document)

`dcompile.rs`, `dcompile_request.rs`, `dserv_artifact.rs`,
`dcode_registry.rs`, `dcode_authorization.rs`, `dcall.rs` were **not**
modified — this phase composes with them exclusively.

## Test/build results

- Focused (`dcompile_dcode::`): 37/37 passed.
- Full dserver suite: 1770 passed, 0 failed (1733 baseline + 37 new).
- Full DATUM lib suite: 1262 passed, 0 failed, 1 ignored — unchanged, as
  expected (this phase is dserver-only).
- `cargo build --locked --release`: both crates succeeded.
- `cargo fmt --check`: the new file was formatted directly with `rustfmt`
  (brand-new file, per this project's established practice) and reports
  zero deviations.
- `git diff --check`: clean.
- New warnings: `dcompile_dcode.rs`'s types/functions are flagged as unused
  in a plain, non-test build — expected, matching the identical,
  already-approved situation for `dcompile.rs`/`dcompile_request.rs` in
  Phases 117B/117C: no production consumer exists yet, and none was
  manufactured artificially.

## Authority review

Explicitly confirmed unchanged: the external builder owns actual
executable-bytes/content truth (D-Compile never establishes it, only
formats a declared claim). D-Compile creates candidate descriptor metadata
only. `CanonicalDServArtifactV1` remains the sole executable-descriptor
contract, unmodified. `DCodeArtifactRegistryStateV1` remains explicit
registry authority — `declare_dserv_artifact` is never called. D-Graph
remains topology (unmodified, reused verbatim from 117C). D-Map remains
placement authority. D-Deploy remains proposal/acceptance authority
(untouched). D-Code authorization (`authorize_dcode_invocation`) remains
invocation authority (untouched, uncalled). `DCodeModuleStore` remains
installed-byte storage (untouched, uncalled). D-Monitor remains runtime
observation authority. Stage 107A remains host-mutation authority
(untouched).

## ADR decision

**No new ADR was created.** The candidate strongly resembling a durable
decision — "D-Compile v1 consumes narrow external D-Code content/policy
input and constructs canonical descriptor candidates deterministically" —
is fully and precisely recorded in this document and follows directly from
architectural facts ADR-0021 already establishes (the fixed-constant
fields) and Phase 117D already audited (the content-identity trust
boundary). Creating an ADR now would either duplicate ADR-0021's own
already-durable contract definition or restate this document's own
narrower implementation decision without adding independent durability —
neither earns a separate permanent record. Revisit if a future phase makes
this narrow-input shape a persisted, externally-facing API contract (at
which point the API surface itself, not this internal function signature,
would be the thing worth an ADR).

## Next phase

Per Phase 117D's own framing, not automatically assumed: of the five known
remaining real blockers (A: registry replacement, B: DDeploy eligibility,
C: manual byte distribution, D: DCall topology evidence, E: unpersisted
compiler provenance), this phase resolved none of them by design and
created no new evidence for D. The recommended next step is **Phase 117F —
DCompile/DCode lifecycle and DDeploy handoff audit/hardening**, focused on
A and B specifically (both confirmed real, both blocking an actual
end-to-end deploy of a D-Compile-produced application, neither solvable by
more candidate-construction work) — C is properly a DNode/deployment
distribution concern, not D-Compile's, and should not be pulled into Phase
117 scope; D remains correctly deferred pending real evidence. Not
implemented here.
