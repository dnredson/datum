# C66 — Explicit Live Single-Node WASM Execution

- checkpoint: `C66/150`
- target_progress_after_validation: `138/150`
- predecessor: `C65/150`
- phase: Controlled WASM Deployment
- status: implementation_ready_for_validation

## Objective

Perform the first explicit live DATUM execution from a real node-scoped deployment assignment and a C65-verified artifact.

C66 is the first checkpoint in this phase that deliberately crosses the Wasmtime instantiation boundary. It remains a separate controlled sandbox corridor and does not authorize canonical governed `wasm_execution`.

## Operational entrypoint

C66 adds:

```text
datum wasm-deploy-live
```

The command consumes a node-scoped deployment JSON file, selects one explicit assignment, acquires and verifies only that assignment's artifact through the C65 corridor, validates the requested scalar ABI, and then instantiates/invokes it only when `--execute` is present.

Representative invocation:

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
9. the supplied arguments match the scalar function signature;
10. a positive fuel budget is available;
11. the operator supplied the explicit `--execute` flag.

A signature mismatch or missing explicit execution confirmation is blocked before Wasmtime instantiation.

## Scalar ABI

Live invocation supports the C62 scalar ABI:

- `i32`;
- `i64`;
- `f32`;
- `f64`.

CLI arguments use `TYPE:VALUE`, for example:

```text
--arg i32:2
--arg i64:42
--arg f32:1.5
--arg f64:2.25
```

`v128` and reference values remain unsupported.

## Runtime evidence

`LiveWasmExecutionReport` records:

- execution UUID and UTC observation time;
- manifest/project/node/assignment/module/function identity;
- artifact content ID and URI;
- invocation arguments and results;
- acquisition evidence inherited from C65;
- integrity and compile evidence;
- Wasmtime instantiation attempted/completed;
- function invocation attempted;
- definite guest execution evidence for function calls;
- conservative `guest_execution_may_have_occurred` evidence once instantiation is attempted because a module may contain a start function;
- fuel configured/consumed/exhausted;
- timeout metadata plus `timeout_enforced=false`;
- `wasm_guest_side_effects=false`;
- `host_mutation_performed=false` for the controlled execution itself;
- `governed_execution_performed=false`;
- `safe_to_auto_execute=false`.

If instantiation fails, the report explicitly states that a module start function may have executed before the failure rather than claiming a stronger fact than the runtime can prove from this corridor.

## Resource-limit status

Fuel remains the enforced execution budget.

`timeout_ms` remains reported metadata only. C66 does not claim wall-clock timeout enforcement.

## Reproducible live fixture

`scripts/c66_prepare_live_fixture.py` creates a minimal real WebAssembly binary exporting:

```text
add(i32, i32) -> i32
```

The script:

- writes the `.wasm` artifact;
- calculates SHA-256 over the exact bytes;
- creates a node-scoped deployment JSON with matching `content_id`;
- prints the exact `cargo run` command for the live invocation.

Prepare it with:

```bash
python3 scripts/c66_prepare_live_fixture.py --node-id fog-01
```

The live success criterion is an execution report containing at least:

```text
status=executed
integrity_verified=true
wasmtime_instantiated=true
function_invocation_attempted=true
guest_execution_performed=true
governed_execution_performed=false
timeout_enforced=false
result i32 = 5
```

## Tests added

C66 adds 11 focused DATUM tests:

- 8 runtime tests for explicit execution, signature gating, generic i64 execution, traps/start behavior, assignment selection, and node identity;
- 3 CLI scalar parser tests.

With the validated C65 DATUM baseline at `401 passed; 0 failed`, the expected unit-test target is `412 passed; 0 failed`.

## Validation required before checkpoint advancement

First run regression validation:

```bash
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Then perform the real live invocation:

```bash
python3 scripts/c66_prepare_live_fixture.py --node-id fog-01

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

Do not advance to `138/150` until both the full regression and the live execution succeed.

## Next intended increment

C67 expands this path to a real multi-node distributed deployment, targeting the demo topology of Raspberry Pi + local PC + VM with distinct node-scoped assignments and artifacts. Optional AWS/Ansible bootstrap can be layered on top without making public-cloud availability a dependency of the core demonstration.
