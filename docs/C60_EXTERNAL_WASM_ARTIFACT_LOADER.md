# C60 — External WASM Artifact Representation and Loader

- checkpoint: `C60/150`
- target_progress_after_validation: `132/150`
- predecessor: `C59/150`
- phase: Controlled WASM Deployment
- status: implementation_ready_for_validation

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
8. a source guard against process, network, Docker, installation-task, assisted-execution, and WASI coupling.

## Validation required before checkpoint advancement

Run from the repository root:

```powershell
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Do not advance the phase checkpoint to `132/150` until these validations pass.

## Next intended increment

After C60 is validated and integrated, expose the artifact-backed corridor through an explicit local CLI/demo path and begin artifact identity/integrity binding without connecting it to deployment automation.
