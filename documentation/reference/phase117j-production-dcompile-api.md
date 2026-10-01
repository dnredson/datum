# Phase 117J — Production DCompile HTTP API

**Status:** Phase 117J CLOSED. **Phase 117 remains OPEN.**
**Branch:** `agent/phase117-dcompile`.
**Baseline:** Phase 117I audit closure, commit `31527877a2b1d14fb903ed744cad7664074f05f5`.
**NO VM OR RPI LIVE VALIDATION WAS REQUIRED FOR PHASE 117J** (local
Rust/unit/integration tests only, per this phase's own scope).

This phase implements, without deviation, the design
`documentation/reference/phase117i-production-dcompile-api-handoff-audit.md`
recommended: a single, stateless, deterministic
`POST /api/v1/dcompile/compile` endpoint that is the FIRST real production
caller of the Phase 117 compiler pipeline (117B-E). No server-side authority
transition (registration, proposal creation, acceptance, placement) was
added — those remain the caller's own, separate, explicit next steps
against the already-existing `/api/v1/dcode/artifacts` and
`/api/v1/ddeploy/proposals` endpoints (117G/117H).

## 1. Production gap closed

Before this phase, `dserver::core::dcompile`/`dcompile_dcode`'s lowering
functions (`lower_calls_empty_dgraph_candidate`,
`lower_dcode_artifact_candidates`) were real, pure, exhaustively tested, and
had **zero** production caller — no `api/dcompile.rs`, no
`/api/v1/dcompile/...` route existed anywhere. `DCompileDGraphCandidate`/
`DCompileCandidateBundle` were both explicitly documented as "NOT a wire
contract" for exactly this reason. This phase adds the first, and so far
only, production consumer: `core::dcompile_submission::compile_submission`,
exposed over HTTP as `POST /api/v1/dcompile/compile`.

## 2. Endpoint

```
POST /api/v1/dcompile/compile
```

`dserver/src/api/dcompile.rs`, registered once in `main.rs`. Pure,
deterministic, side-effect-free: the handler touches no control-state DB,
no D-Code registry, no D-Deploy state, no D-Node registry, no filesystem, no
network, and acquires none of `DDEPLOY_TRANSACTION`/`DCODE_TRANSACTION`/
`DNODE_TRANSACTION`. It only deserializes the request, calls
`compile_submission`, and maps the result onto HTTP.

## 3. Submission schema (`datum.dcompile-submission/1`)

```rust
pub struct DCompileSubmissionV1 {
    pub schema: String,
    pub dscript: CanonicalDScriptV1,
    pub output_dgraph_id: String,
    pub output_dgraph_revision: u64,
    pub services: Vec<DCompileServiceMappingV1>,       // reused verbatim from 117B
    pub dcode_services: Vec<DCompileDCodeServiceInputV1>, // reused verbatim from 117E
}
```

`#[serde(deny_unknown_fields)]`. Contains ONLY true external compile
inputs: no `dscript_ref`, no `dgraph_ref`, no digest of any kind, no
`application_id` at the outer level (the single source of application
identity is `dscript.application_id`, read exactly once), no `project_id`,
no placement/D-Continuum/node/D-Map/proposal field, no timestamp, no random
identifier. `J18`/`J25`/`J46` in `dcompile_submission.rs`'s own test module
prove each of these directly (an unknown field, or any of `dgraph_ref`/
`project_id`/`node_id`/etc., fails deserialization or is simply absent from
what the type can even represent).

### Example

```json
{
  "schema": "datum.dcompile-submission/1",
  "dscript": {
    "schema": "datum.dscript/1",
    "application_id": "app:sensor-pipeline",
    "dscript_id": "dscript:sensor-pipeline",
    "revision": 1,
    "functions": [{"function_id": "decode_uplink", "annotations": {}}]
  },
  "output_dgraph_id": "dgraph:sensor-pipeline",
  "output_dgraph_revision": 1,
  "services": [{
    "service_id": "decode",
    "function_ids": ["decode_uplink"],
    "resource_requirement": {"cpu_cores": 1, "memory_bytes": 134217728}
  }],
  "dcode_services": [{
    "service_id": "decode",
    "artifact_id": "dserv:sensor-pipeline:decode",
    "version": "0.1.0",
    "sha256": "<64-lowercase-hex, DECLARED by whoever built the module -- never verified by DCompile>",
    "size_bytes": 65536,
    "ports": {"inputs": [], "outputs": []},
    "limits": {"fuel_per_invocation": 5000000, "timeout_ms": 200, "max_memory_pages": 4, "max_message_bytes": 65536, "max_emits_per_invocation": 8}
  }]
}
```

## 4. Response schema (`datum.dcompile-candidate/1`)

```rust
pub struct DCompileCandidateResponseV1 {
    pub schema: String,
    pub application_id: String,
    pub dscript_ref: CanonicalContractReferenceV1,      // server-computed
    pub mapping: DCompileRequestV1,                      // full mapping, canonically ordered
    pub dcompile_request_digest: String,                 // server-computed
    pub dgraph: CanonicalDGraphV1,                        // directly POST-able as ProposalInputPayload.dgraph
    pub dgraph_ref: CanonicalContractReferenceV1,        // server-computed
    pub dcode_artifacts: Vec<DCompileArtifactResultV1>,  // one per service, directly POST-able individually
    pub dcode_artifact_authority: BTreeMap<String, DCodeArtifactAuthorityBindingV1>, // pure convenience, DDeploy-ready
}

pub struct DCompileArtifactResultV1 {
    pub service_id: String,
    pub artifact: CanonicalDServArtifactV1,
    pub descriptor_digest: String,
}
```

No field named `current`/`active`/`accepted`/`registered`/`deployed`/
`authorized`/`installed` anywhere — `j45_response_contains_no_active_or_accepted_field`
confirms this directly by scanning serialized response JSON keys.

## 5. Derived-reference construction (the central design problem)

`DCompileRequestV1.dscript_ref.digest` must equal the D-Script's own real
digest; `DCompileDCodeInputV1.dgraph_ref.digest` must equal the candidate
D-Graph's own real digest — a value that does not exist until lowering has
already run. A caller cannot honestly supply either before compilation
happens. `compile_submission` therefore constructs both itself:

1. `validate_and_digest_dscript(submission.dscript)` → `ValidatedDScriptInput`.
2. Construct `DCompileRequestV1` with `dscript_ref` built from step 1's own
   `document.dscript_id`/`revision`/`digest_sha256` — never caller-supplied
   — then `validate_and_digest_dcompile_request(request)` →
   `ValidatedDCompileRequest`.
3. **Intentional double D-Graph lowering** (load-bearing, not an
   oversight): call `lower_calls_empty_dgraph_candidate` here, solely to
   obtain the fresh D-Graph identity/digest `DCompileDCodeInputV1.dgraph_ref`
   needs. This preliminary candidate is then discarded.
   `lower_dcode_artifact_candidates` (step 5) calls the exact same function
   again internally, as part of its own existing, unmodified 117E trust
   boundary (never weakened, per this phase's explicit instruction).
   `preliminary_dgraph_lowering_equals_final_bundles_dgraph_candidate`
   proves both calls produce an identical result for the same validated
   inputs — the repetition is a small, cheap, provably-redundant pure
   computation, never a source of drift.
4. Construct `DCompileDCodeInputV1` with `dgraph_ref` built from step 3's
   result — never caller-supplied.
5. Call `lower_dcode_artifact_candidates` (117E, unmodified) with the
   validated D-Script, validated request, and constructed D-Code input.
6. Map the result into `DCompileCandidateResponseV1`.

Every step after (1) reuses an EXISTING, unmodified function — no
validation/lowering/digest rule was reimplemented anywhere in this module.

## 6. Typed validate+digest helpers (new, shared)

`canonical_dscript::validate_and_digest_dscript(document: CanonicalDScriptV1)`
and `dcompile_request::validate_and_digest_dcompile_request(request: DCompileRequestV1)`
are new, small, pure functions extracted from the existing
`decode_and_validate_*_json` functions' own bodies. Both historical
JSON-string decoders are now thin wrappers around them:

```rust
pub fn decode_and_validate_dscript_json(json: &str) -> Result<ValidatedDScriptInput, DScriptInputError> {
    let document: CanonicalDScriptV1 = serde_json::from_str(json)...?;
    validate_and_digest_dscript(document)
}
```

This is a pure refactor with zero logic change — `typed_and_json_boundaries_agree_on_a_valid_document`/
`typed_and_json_boundaries_agree_on_the_same_validation_findings` (and the
equivalent pair in `dcompile_request.rs`) prove the JSON boundary and the
new typed boundary are byte-for-byte/finding-for-finding equivalent. The
production submission endpoint uses the typed helpers directly — no JSON
round-trip of an already-typed value anywhere.

## 7. Intentional double lowering (Section 18 of the task)

See §5 step 3. Explicitly NOT optimized away: reaching into
`lower_dcode_artifact_candidates` to skip its own internal re-lowering
would weaken a trust boundary 117C/117E already established, and this
phase found no evidence justifying that. A pure function computing the
same small result twice was preferred instead.

## 8. Deterministic canonical response mapping (Section 20)

`DCompileRequestV1.services`/each service's `function_ids` are semantically
unordered sets; `recompute_dcompile_request_digest` already normalizes both
before hashing, but the WIRE `mapping` field would otherwise echo the
caller's own submitted vector order verbatim — making two
semantically-identical submissions (differing only in submitted ordering)
produce different serialized JSON. `canonicalize_mapping` sorts a clone of
the validated request (services by `service_id`, each service's
`function_ids`) before placing it in the response. This changes nothing
about `DCompileRequestV1`'s own frozen semantics or
`dcompile_request_digest`'s value (the digest function re-sorts internally
regardless) — only the deterministic PRESENTATION of `mapping`.
`j28`-`j32` (dscript function reorder, mapping service reorder, function_ids
reorder, dcode_services reorder, combined) all prove the FULL serialized
response is byte-identical across semantically-equivalent reorderings.

## 9. Provenance semantics (five identities, never collapsed)

| Identity | Computed by | Byte-proven? | Where in the response |
|---|---|---|---|
| D-Script digest | Server, from the submitted document | No — structural hash of declared source | `dscript_ref.digest` |
| D-Compile-request digest | Server, from the internally-constructed request | N/A | `dcompile_request_digest` (full `mapping` also included, not merely the digest — a digest alone can never let a caller reconstruct WHICH mapping produced a given D-Graph, per `dcompile.rs`'s own `c12_mapping_provenance_collision_same_dgraph_different_request_digest` proof) |
| D-Graph digest | Server, from the generated candidate | N/A | `dgraph_ref.digest` |
| Module SHA-256 | **Externally declared** by whoever built the module | **No — NEVER byte-verified by D-Compile** | `dcode_artifacts[i].artifact.dcode.sha256` |
| D-Serv artifact descriptor digest | Server, from the constructed candidate | N/A (descriptor metadata, not module bytes) | `dcode_artifacts[i].descriptor_digest` |

No compile-result identity/digest was added (117I Section 13 re-confirmed:
nothing under the stateless model is stored or looked up by one).

## 10. Module-digest trust boundary (unchanged, restated explicitly)

The API documentation and this module's own doc comments say **declared**
module SHA-256, never **verified**. D-Compile never receives module bytes
and cannot establish `sha256`/`size_bytes` describe real content — that
proof remains entirely D-Node-side (`DCodeModuleStore::install`/
`invoke_dcode`, 117D/117H). D-Compile also proves nothing about whether a
module actually implements the D-Functions its D-Compile-request mapped to
its service — binding is by declared identity only, never source-to-binary
provenance.

## 11. Zero-transformation registry/DDeploy handoff (proven, not asserted)

- `j36_j37_compile_artifact_directly_declares_and_digest_matches`: every
  `dcode_artifacts[i].artifact` value, UNMODIFIED, POSTs successfully to
  the real `declare_dserv_artifact_revision` (117G), and the returned
  digest equals `descriptor_digest` exactly.
- `j38_response_binding_validates_against_registered_revisions`: the
  response's own `dgraph`/`dcode_artifact_authority`, UNMODIFIED, pass the
  real `validate_dcode_artifact_authority_bindings` (117G) once the
  artifacts above are registered.
- `j39_j40_j41_local_compile_register_propose_accept_binds_the_exact_compiler_produced_digest`:
  the FULL local chain — compile → declare every artifact → `create_proposal`
  (response's `dgraph`/`dcode_artifact_authority` passed through completely
  unmodified, only `project_id`/`dcontinuum`/`placements`/`source` supplied
  as genuinely-downstream context) → `accept_proposal_with_dcode_binding_revalidation`
  (117G) → confirm the newly active binding's `descriptor_digest` is
  EXACTLY the compiler-produced one via `resolve_active_dcode_binding`
  (117G). This is the concrete, tested proof that the handoff schema
  genuinely fits existing DDeploy with no adaptation layer.

## 12. Authority boundaries (confirmed, task Section 4/42)

| Concern | DCompile may produce? | DCompile may activate? |
|---|---|---|
| Source identity | Yes (digests it) | No |
| Mapping intent | Yes (produces + digests it) | No |
| Topology (D-Graph) | Yes (candidate only) | No |
| D-Code descriptor | Yes (candidate only) | No |
| Registry entry | No | No |
| D-Code active revision | No (produces the CONVENIENCE map only) | **No** |
| Placement | No | No |
| Proposal / acceptance | No | No |
| Installed module bytes | No | No |
| Invocation authorization | No | No |

`j42`-`j44` prove this structurally: `compile_submission` takes no
registry/D-Deploy-state parameter at all, so a freshly constructed
registry/state remains empty regardless of how many times compilation ran.

## 13. Determinism, empty program, recompile

- **Determinism** (`j26`/`j27`): identical submission twice → exactly
  equal response, byte-identical JSON. No timestamp, random/UUID
  identifier, server hostname, or D-Node identity anywhere in either
  schema.
- **Empty program** (`j3`): `dscript.functions = []`, `services = []`,
  `dcode_services = []` compiles to a valid, empty candidate (empty
  D-Graph services/calls, zero artifacts, empty binding map). Compile
  validity and deployment validity remain two separate questions — this
  API layer never reinterprets an empty program as an error.
- **Recompile** (`j33`-`j35`): same D-Script/mapping, different module
  digest for one service → byte-identical `dgraph`/`dgraph_ref`/`mapping`/
  `dcompile_request_digest`, different `descriptor_digest` for exactly that
  service (`j33`). Same module bytes, different `limits` → same module
  digest, different descriptor digest (`j34`). Annotation-only D-Script
  change → D-Script digest changes, D-Graph/artifact unaffected (`j35`).
  This is the direct preparation for the future live H1→H2 proof (§16).

## 14. Error model

Every existing structured-finding error type is preserved and surfaced,
never flattened to a bare `400`. `DCompileSubmissionError` wraps the
EXISTING, unmodified `DCompileLoweringError` (117C)/`DCompileDCodeLoweringError`
(117E) verbatim for their respective stages rather than re-deriving
separate variants for what those types already model completely:

```rust
pub enum DCompileSubmissionError {
    InvalidSubmission(Vec<CanonicalValidationFinding>),       // new envelope's own facts only
    InvalidDScript(Vec<CanonicalValidationFinding>),
    DScriptDigestComputation(String),
    InvalidDCompileRequest(Vec<CanonicalValidationFinding>),
    DCompileRequestDigestComputation(String),
    DGraphLowering(DCompileLoweringError),                    // wraps 117C verbatim
    DCodeLowering(DCompileDCodeLoweringError),                // wraps 117E verbatim
}
```

HTTP mapping: every finding-carrying variant → `422` with
`{"error": "<stable_code>", "findings": [{"code", "message"}]}`, following
`api::dcode.rs`/`api::ddeploy.rs`'s own established convention.
`GeneratedArtifactInvalid` includes `service_id` structurally in the JSON
body — a caller never parses a message string to learn which service
failed (`post_compile_includes_service_id_for_a_generated_artifact_invalid_error`).
Digest-computation failures and the two theoretically-unreachable digest-
mismatch/generated-D-Graph-invalid variants (honestly modeled, never
reachable given this handler's own honest wrapper construction) map to
`500`. Malformed JSON / unknown top-level field uses Axum's standard JSON
rejection (`deny_unknown_fields` on the new envelope, per this project's
established convention).

## 15. Route registration and handler tests

Exactly one route, `POST /api/v1/dcompile/compile` →
`api::dcompile::post_compile`, registered once in `main.rs`.
`production_route_is_registered_exactly_once_as_post` proves this by
scanning `main.rs`'s own source (mirroring `api::ddeploy.rs`'s established
static-source-scan test convention — this crate has no precedent for
constructing a real `AppState`/router inside a unit test). The handler
itself is exercised directly (`post_compile_returns_success_for_a_valid_submission`,
`post_compile_returns_structured_422_for_an_invalid_submission`,
`post_compile_includes_service_id_for_a_generated_artifact_invalid_error`)
via `#[tokio::test]` — no new HTTP-testing framework/dependency was added;
`tokio`'s test macro is already available via this crate's existing
full-featured `tokio` dependency.

## 16. Test results

- `core::dcompile_submission` (new): 45 tests, covering the full J1-J47
  matrix (input validation, derived-identity proofs, determinism/reorder
  invariance, recompile behavior, zero-transformation handoff including
  the full compile→register→propose→accept integration test, and authority
  negative-space), plus the intentional-double-lowering proof and the
  response-canonicalization proof.
- `core::canonical_dscript` / `core::dcompile_request` (new, 2 each):
  typed/JSON boundary equivalence proofs for the refactored
  `validate_and_digest_*` helpers.
- `api::dcompile` (new): 4 tests — handler success/failure, structured
  `service_id`-bearing error, route registration.
- Full `dserver` regression: **1859 passed, 0 failed, 0 ignored** (up from
  the 1806/0/0 Phase 117H baseline; +53 new tests).
- `DATUM --lib --locked`: 1229 passed, 0 failed, 1 ignored (unchanged —
  zero DATUM files touched this phase).
- `DATUM --locked` (default suite): 447 + 6 + 10 + ... = 490 passed total
  across all binaries/integration tests, 0 failed (unchanged), including
  the two real spawned-DServer suites (`dcall_two_node_integration`,
  `dcall_mqtt_governed_e2e`).
- Both `--release` builds succeed.
- `git diff --check`: clean.

## 17. Explicit exclusions (unchanged from the 117I design)

No D-Code registration, no D-Deploy proposal/acceptance, no placement/
D-Continuum selection, no module-byte upload/download/installation, no
D-Call derivation (`calls` remains always empty), no `project_id` on any
compile-semantic type, no compile-result persistence, no compile-result
identity/digest, no staged (multi-endpoint) API added alongside the
one-shot endpoint.

## 18. Stale-comment corrections (narrow, per task Section 56)

Three doc comments that predated a real production consumer were updated
to reflect that one now exists, without otherwise changing their meaning:
`canonical_dscript.rs`'s module doc ("D-Compile (Phase 117, not yet
implemented) will map this..." → now names 117C/117E/117J and the exact
v1 partition-cardinality limitation), and the `DCompileDGraphCandidate`/
`DCompileCandidateBundle` doc comments (both previously said "no proven
persistence/API consumer exists yet" → now name `compile_submission` as
the real consumer while restating that the internal type still
deliberately carries no `Serialize`/`Deserialize`, per 117I's own Decision
B). No other documentation was touched — this was not a broad cleanup
pass.

## 19. Remaining Phase 117 gaps

- **DCompile provenance persistence** — still not implemented (117I's P1
  stateless recommendation stands; DDeploy never needed to trust
  round-tripped provenance as authority in the first place, since it
  always independently recomputes and compares, so this remains a
  legitimately deferrable, non-blocking concern).
- **The live H1→H2 proof** (Phase 117K, see §20) — not run in this phase.
- **No independent Phase 117 closure review** has yet been performed
  across 117A-117J as a whole.

## 20. Future live Phase 117K plan

The next live phase must drive the entire chain through this phase's real
`POST /api/v1/dcompile/compile` entry point — a live test that hand-writes
`CanonicalDServArtifactV1` JSON directly, bypassing the compiler, would not
be a valid Phase 117 closure proof. Concretely:

```
DScript (H1 module facts) -> POST /api/v1/dcompile/compile -> candidate (D1)
    -> POST each artifact to /api/v1/dcode/artifacts (register D1)
    -> POST + accept a proposal using the response's own dgraph/dcode_artifact_authority verbatim
    -> governed execution of D1 (117H's exact-revision client)
    -> POST /api/v1/dcompile/compile AGAIN, same dscript/mapping, new dcode_services[i].sha256/size_bytes
       -> candidate (D2): same dgraph/dgraph_ref, different descriptor_digest for that service
    -> register D2 (D1 remains registered and active, unaffected -- 117G)
    -> prove D1 still active (a fresh authorization still names D1's digest)
    -> create + accept a NEW proposal binding to D2
    -> prove D2 becomes active, D1 becomes historical, the old D1 authorization goes stale
    -> governed execution of D2 (117H's exact-revision client), now genuinely
       exercising a compiler-produced D1 -> D2 transition end to end
```

Two tiny modules whose `datum_handle` output is trivially, observably
DIFFERENT (H1 tags its output "v1", H2 tags "v2") are preferred over
same-bytes-different-limits, because only an observable output difference
proves EXECUTION actually switched bytes, not merely that metadata
switched. Module preparation itself belongs to that future phase, not this
one.

**Phase 117J itself required NO VM/RPi.** Phase 117K is the correct point
to start live infrastructure — this audit found no remaining architectural
question blocking that.
