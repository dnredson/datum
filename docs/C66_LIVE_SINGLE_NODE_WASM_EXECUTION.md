# C66 — Explicit Live Single-Node WASM Execution

- checkpoint: `C66/150`
- progress_after_validation: `138/150`
- predecessor: `C65/150`
- phase: Controlled WASM Deployment
- status: completed

## Objective

Perform the first explicit live DATUM execution from a real node-scoped deployment assignment and a C65-verified artifact.

C66 is the first checkpoint in this phase that deliberately crosses the Wasmtime instantiation boundary. It remains a separate controlled sandbox corridor and does not authorize canonical governed `wasm_execution`.

## Operational entrypoint

C66 adds:

```text
datum wasm-deploy-live
```

The command consumes a node-scoped deployment JSON file, selects one explicit assignment, acquires and verifies only that assignment's artifact through the C65 corridor, validates the requested scalar ABI, and instantiates/invokes it only when `--execute` is present.

Validated invocation:

```bash
cargo run --manifest-path DATUM/Cargo.toml -- \
  wasm-deploy-live \
  --node-id fog-01 \
  --deployment .c66-live/c66-node-deployment.json \
  --assignment c66-add-assignment \
  --arg i32:2 \
  --arg i32:3 \
  --execute \
  --invoked-by c66-live-validation \
  --json
```

`--invoked-by` is audit metadata only. It is not authenticated operator approval and must not be confused with the future governance gate.

## Explicit execution boundary

The live path requires all of the following before `Instance::new`:

1. the C64 node-scoped payload validates for the local DATUM node;
2. the requested assignment exists in that node view;
3. only the selected assignment is passed to C65 preparation;
4. artifact acquisition policy succeeds;
5. SHA-256/content identity verifies;
6. C65 Wasmtime compile-only preflight succeeds;
7. guest host imports remain absent;
8. the assigned export remains a function;
9. supplied arguments match the scalar function signature;
10. a positive fuel budget is available;
11. the operator supplied the explicit `--execute` flag.

A signature mismatch or missing explicit execution confirmation is blocked before Wasmtime instantiation.

## Scalar ABI

Live invocation supports the C62 scalar ABI: `i32`, `i64`, `f32`, and `f64`. `v128` and reference values remain unsupported.

## Runtime evidence

`LiveWasmExecutionReport` records execution UUID/time, manifest/project/node/assignment/module/function identity, artifact content identity/URI, invocation arguments/results, acquisition evidence, integrity/compile state, Wasmtime instantiation and function invocation state, fuel consumption, timeout metadata, and the controlled execution boundary.

If instantiation fails, the report explicitly records that a module start function may have executed before failure rather than claiming a stronger fact than the runtime can prove.

## Resource-limit status

Fuel is the enforced execution budget. `timeout_ms` remains reported metadata only; C66 does not claim wall-clock timeout enforcement.

## Tests added

C66 adds 11 focused DATUM tests:

- 8 runtime tests for explicit execution, signature gating, generic i64 execution, traps/start behavior, assignment selection, and node identity;
- 3 CLI scalar parser tests.

The validated C65 baseline was `401 passed; 0 failed`; the expected C66 total is `412 passed; 0 failed`.

## Validation evidence — 2026-08-15

The user reported the complete DATUM formatting/test/build validation finished without errors. The exact numeric `cargo test` summary line was not pasted, so this checkpoint records regression PASS without fabricating an observed test count.

The real live invocation then completed successfully on Linux with:

```text
execution_id=0be7008e-7232-4f48-85bc-9be1b37030da
manifest_id=c66-live-single-node
project_id=c66-live-project
node_id=fog-01
assignment_id=c66-add-assignment
module_id=c66-add-module
function_name=add
artifact_content_id=sha256:f61fd62f57c41269c3c23f360eeaf1090b1db9c38651106674d48bc65dba88ba
status=executed
result=i32:5
```

Observed runtime evidence:

```text
explicit_execute=true
integrity_verified=true
wasmtime_compiled=true
wasmtime_instantiation_attempted=true
wasmtime_instantiated=true
function_invocation_attempted=true
guest_execution_performed=true
guest_execution_may_have_occurred=true
wasm_guest_side_effects=false
governed_execution_performed=false
host_mutation_performed=false
safe_to_auto_execute=false
fuel_configured=1000000
fuel_consumed=4
fuel_exhausted=false
timeout_ms=1000
timeout_enforced=false
```

Artifact acquisition was a local filesystem read of a 41-byte verified `.wasm`; no network fetch, persistent write, or host mutation was performed.

This satisfies the C66 success criterion. C66 is complete at `138/150`.

## Next intended increment

C67 expands this path to a real multi-node distributed deployment, targeting the demo topology of Raspberry Pi + local PC + VM with distinct node-scoped assignments and artifacts. Optional AWS/Ansible bootstrap can be layered on top without making public-cloud availability a dependency of the core demonstration.
