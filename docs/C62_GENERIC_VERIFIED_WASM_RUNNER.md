# C62 — Generic Verified Artifact-Backed WASM Runner

- checkpoint: `C62/150`
- progress_after_validation: `134/150`
- predecessor: `C61/150`
- phase: Controlled WASM Deployment
- status: validated_complete

## Objective

Generalize the artifact-backed execution corridor beyond the C60/C61 demonstration ABI `(i32, i32) -> i32` while preserving SHA-256 verification, no-host-import isolation, bounded fuel, and the fail-closed governance boundary.

C62 does not introduce deployment automation, remote artifact acquisition, installation, dispatch, D-Graph mutation, signatures, provenance trust, WASI, or a governed `wasm_execution` allow transition.

## Generic scalar ABI

`DATUM/src/wasm_runtime/generic_runner.rs` introduces a dynamic scalar runtime model:

- `WasmScalarValue::I32(i32)`;
- `WasmScalarValue::I64(i64)`;
- `WasmScalarValue::F32(f32)`;
- `WasmScalarValue::F64(f64)`;
- `GenericWasmRuntimeInput`;
- `GenericWasmRuntimeResult`.

The runner supports:

- zero or more scalar parameters;
- zero or more scalar results;
- mixed scalar parameter types;
- multiple return values;
- dynamic exported-function lookup;
- dynamic signature inspection before invocation.

For C62, `v128` and reference types remain explicitly unsupported and fail closed.

## Verified execution order

The public C62 path is `run_verified_generic_local_wasm_artifact(...)` in `wasm_runtime::integrity`.

Its order is:

1. receive an artifact already loaded by the explicit C60 local loader;
2. normalize and verify the expected SHA-256 using C61;
3. reject malformed or mismatched integrity metadata before Wasmtime runtime evaluation;
4. compile the artifact with Wasmtime;
5. reject every guest host import;
6. resolve the requested exported function dynamically;
7. inspect its parameter and result types;
8. reject unsupported types or argument/type mismatches;
9. invoke through Wasmtime's dynamic `Func::call` path;
10. return scalar results and fuel outcome.

The underlying unverified generic runner is crate-visible only (`pub(crate)`). The external C62 API is the integrity-gated path.

## Result semantics

`GenericWasmRuntimeResult` separates artifact integrity from runtime policy outcome:

- `integrity_verified` indicates whether SHA-256 verification succeeded;
- `artifact_content_id` carries the verified `sha256:<digest>` identity when available;
- `status` still reports `executed`, `blocked`, or `failed`;
- `safe_to_auto_execute` remains `false`;
- `side_effects_performed` remains `false`;
- filesystem/network/host-command/Docker access remain `false`;
- configured fuel and observed fuel outcome remain visible.

This allows an artifact to be integrity-valid but still blocked by the scalar ABI policy, e.g. when the requested function returns `v128`.

## Safety boundary

C62 preserves the controlled sandbox invariants:

- no guest host imports;
- no WASI;
- no filesystem access granted to the guest;
- no network access granted to the guest;
- no host command execution;
- no Docker access;
- no installation-task or assisted-execution coupling;
- no automatic D-Graph/planner/readiness trigger;
- SHA-256 integrity is required on the public generic execution path;
- general governed `wasm_execution` remains denied/fail-closed.

## Tests added

C62 adds coverage for:

1. generic `i32` execution;
2. generic `i64` execution;
3. generic `f32` execution;
4. generic `f64` execution;
5. mixed-parameter / multi-value results;
6. zero-argument / zero-result functions;
7. argument-type mismatch blocking;
8. unsupported `v128` signature blocking;
9. guest host-import blocking;
10. zero-fuel failure;
11. source isolation guard;
12. SHA-256 verified generic `i64` execution;
13. generic integrity mismatch before runtime compilation;
14. proof that an integrity-valid artifact can still be blocked by signature policy.

## Linux validation — 2026-08-14

Executed from the repository root on the C62 branch:

```bash
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Observed:

- formatting check: PASS;
- DATUM tests: `389 passed; 0 failed`;
- DATUM build: PASS;
- no formatting fix was required;
- no new dependency or lockfile change was introduced by C62.

C62 is therefore complete at `134/150`.

## Next intended increment

C63 introduces the typed Deployment Manifest that binds project/module/function/artifact identity to deployment strategy and graph-distribution semantics (`centralized/full` or `distributed/node_slice`) without yet auto-executing governed deployment actions.
