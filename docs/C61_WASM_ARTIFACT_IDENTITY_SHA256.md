# C61 — WASM Artifact Identity and SHA-256 Integrity Verification

- checkpoint: `C61/150`
- target_progress_after_validation: `133/150`
- predecessor: `C60/150`
- phase: Controlled WASM Deployment
- status: implementation_ready_for_validation

## Objective

Bind every explicitly loaded external WASM artifact to a deterministic content identity and require SHA-256 integrity verification before it can enter the verified controlled runtime path.

C61 does not introduce deployment automation, installation, dispatch, D-Graph mutation, remote artifact acquisition, signature verification, provenance trust, or a governed `wasm_execution` allow transition.

## Implementation

`DATUM/src/wasm_runtime/integrity.rs` introduces:

- `WasmArtifactIdentity`;
- deterministic SHA-256 calculation over the exact loaded artifact bytes;
- content identity in the form `sha256:<64-hex-digest>`;
- strict expected-digest normalization and validation;
- `verify_loaded_wasm_artifact_sha256(...)`;
- `run_verified_local_wasm_artifact(...)`;
- `run_verified_local_wasm_artifact_report(...)`.

The verified execution order is:

1. artifact already loaded explicitly by the C60 local loader;
2. calculate content SHA-256;
3. validate the expected digest syntax;
4. compare expected and calculated digests;
5. only on a match, enter the existing C60 Wasmtime runtime path.

A malformed expected digest or a mismatch returns a fail-closed `blocked` runtime result before Wasmtime compilation/instantiation and before fuel is consumed.

## Identity semantics

For C61, artifact identity is content-addressed:

```text
sha256:<digest>
```

The identity is deterministic for identical bytes and includes the loaded artifact size as metadata. Path names are not part of the content identity.

C61 accepts the expected digest as either the raw 64 hexadecimal characters or the same value prefixed by `sha256:`. Hexadecimal case is normalized before comparison.

## Safety boundary

C61 preserves all C59–C60 sandbox invariants:

- `safe_to_auto_execute=false`;
- integrity mismatch is fail-closed;
- no guest filesystem access;
- no guest network access;
- no host-command access;
- no Docker access;
- no WASI integration;
- no installation-task or assisted-execution coupling;
- no automatic trigger from D-Graph, readiness, health, planner, or prerequisites.

SHA-256 verification is an integrity gate for the controlled artifact corridor. It is not operator authorization and does not satisfy the broader governed execution gates by itself.

## Tests added

C61 adds unit coverage for:

1. SHA-256 known vector (`abc`);
2. deterministic content identity;
3. verified external `add(2, 3) -> 5` execution;
4. prefixed/uppercase expected digest normalization;
5. digest mismatch blocking without execution/fuel consumption;
6. malformed digest blocking;
7. proof that integrity mismatch is evaluated before Wasmtime compilation.

With the post-C60 Linux baseline at `368/368`, the expected DATUM total after C61 is `375` tests.

## Dependency and lockfile

C61 adds the RustCrypto `sha2` crate as a direct DATUM dependency. The branch remains validation-pending until Cargo updates `DATUM/Cargo.lock` on the Linux validation checkout and that lockfile update is committed to the branch.

## Validation required before checkpoint advancement

Run from the repository root on the C61 branch:

```bash
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Expected test target: `375 passed; 0 failed`.

Do not advance the phase checkpoint to `133/150` and do not merge until the checks pass and the Cargo lockfile is synchronized.

## Next intended increment

After C61 is validated and integrated, C62 generalizes the artifact-backed function runner beyond the current narrow `(i32, i32) -> i32` demonstration ABI while keeping the verified integrity gate in front of execution.
