# C60 — External WASM Artifact Representation and Loader

- checkpoint: `C60/150`
- progress: `132/150`
- predecessor: `C59/150`
- phase: Controlled WASM Deployment
- status: complete
- validation_date: `2026-08-14`

## Objective

Replace the embedded-only WASM boundary with a first artifact-backed runtime primitive while preserving the C59 controlled sandbox contract.

C60 does not introduce deployment automation, installation, dispatch, D-Graph mutation, or a governed `wasm_execution` allow transition.

## Implementation

`DATUM/src/wasm_runtime/artifact.rs` introduces:

- `LoadedWasmArtifact` as the local artifact representation;
- `load_local_wasm_artifact(...)` for explicit `.wasm` filesystem loading;
- WebAssembly binary magic-header validation before runtime compilation;
- `run_local_wasm_artifact(...)` for isolated Wasmtime execution;
- `run_local_wasm_artifact_report(...)` for the existing runtime-report model;
- fuel accounting using the existing `WasmRuntimeLimits` contract;
- explicit rejection of every guest host import before instantiation.

The first external artifact ABI intentionally remains narrow: one exported function with signature `(i32, i32) -> i32`.

## Safety boundary

Host-side filesystem access is used only to load the selected `.wasm` artifact bytes. The WASM guest receives no filesystem, network, command, Docker, WASI, installation-task, or assisted-execution imports.

A successful external artifact execution still reports:

- `safe_to_auto_execute=false`;
- `side_effects_performed=false`;
- `filesystem_access=false`;
- `network_access=false`;
- `host_command_access=false`;
- `docker_access=false`.

The canonical C46–C57 governed execution boundary remains unchanged and fail-closed.

## Tests added

C60 includes unit coverage for:

1. valid external `.wasm` loading;
2. rejection of non-`.wasm` paths;
3. rejection of invalid WebAssembly binary magic;
4. successful external `add(2, 3) -> 5` execution;
5. fail-closed rejection of modules declaring a host import;
6. missing-export failure without side effects;
7. zero-fuel failure;
8. source isolation against process, network, Docker, installation-task, assisted-execution, and WASI coupling.

## Validation evidence

Executed locally on Windows against the C60 branch on `2026-08-14`:

```powershell
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Observed result:

- formatting check completed without a reported formatting failure;
- tests: `360 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`;
- build: `Finished dev profile` successfully;
- build emitted 7 non-blocking `dead_code` warnings and no errors.

The warnings concern existing unused configuration fields/functions and embedded WASM helper functions; they do not represent a C60 validation failure.

## Exit criteria

C60 is complete because:

- external `.wasm` bytes can be loaded from an explicitly selected local path;
- malformed or non-WASM artifacts fail closed;
- guest imports are rejected before instantiation;
- the artifact can execute inside the isolated Wasmtime runtime with bounded fuel;
- safety flags remain non-authorizing and side-effect-free for the guest;
- no deployment, installation, assisted-execution, dispatch, or governed execution path is opened;
- DATUM regression tests and build pass.

## Next intended increment

C61/150 begins artifact identity and integrity binding. The next practical demo surface may expose the artifact-backed corridor through an explicit local CLI command, but no automatic deployment or governed execution transition is authorized by C60.
