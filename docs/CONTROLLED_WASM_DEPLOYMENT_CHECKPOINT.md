# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `136/150`
- latest_checkpoint: `C64`
- predecessor_phase_final: `C58_DISTRIBUTED_DRY_RUN_PREPARATION_CHECKPOINT_FINAL.md`
- phase_entry_checkpoint: `C59/150`
- external_wasm_artifact_loader: validated
- artifact_sha256_integrity: validated
- generic_verified_wasm_runner: validated
- typed_deployment_manifest: validated
- node_scoped_deployment_assignment: validated
- datum_artifact_acquisition_preflight: implementation_ready_for_validation
- governed_allow_transition_available: false
- next_checkpoint: `C65/150`
- next_scope: validate DATUM node-scoped artifact acquisition, SHA-256 verification, and compile-only Wasmtime preflight

C59 defined the external D-Graph boundary, centralized/full and distributed/node_slice deployment semantics, and the controlled artifact-backed WASM sandbox corridor.

C60 validated explicit external local `.wasm` loading and isolated Wasmtime execution while rejecting guest host imports.

C61 added deterministic content identity as `sha256:<digest>` and an integrity-gated execution path that rejects malformed or mismatched expected digests before Wasmtime compilation/instantiation.

C62 generalized the verified artifact corridor with a dynamic scalar ABI supporting `i32`, `i64`, `f32`, and `f64`, including zero/multiple arguments and zero/multiple results. `v128` and reference types remain blocked. C62 Linux validation passed with DATUM `389 passed; 0 failed` and build PASS.

C63 added the dserver-owned typed Deployment Manifest contract. It binds project, module, function, artifact URI, SHA-256/content identity, target node, runtime limits, strategy, and graph-distribution semantics while remaining non-executable evidence. C63 Linux validation passed with dserver `452 passed; 0 failed` and build PASS.

C64 derives typed node-scoped deployment views from a validated C63 manifest. Distributed/node_slice views contain only assignments whose `target_node` matches the requested node; centralized/full returns the complete assignment set only to the central target; non-target nodes receive an explicit empty view. C64 independently rejects cross-node assignment leakage and preserves the fail-closed execution boundary.

C64 Linux validation evidence on `2026-08-15`:

- `cargo fmt --manifest-path dserver/Cargo.toml -- --check`: PASS;
- dserver tests: `464 passed; 0 failed`;
- `cargo build --manifest-path dserver/Cargo.toml`: PASS;
- no dependency or lockfile changes;
- no governed execution allow transition;
- no execution or deployment side effects introduced.

C64 is complete at `136/150`.

C65 implementation adds a DATUM-side consumer for the C64 node-scoped JSON shape and a bounded artifact preparation corridor. DATUM validates the local-node address and assignment boundary before artifact access, supports `file://` and HTTPS acquisition, keeps insecure HTTP opt-in for trusted lab/LAN validation only, applies size and request-time bounds, disables redirects, and keeps network-delivered artifacts in memory without persistent writes.

C65 computes SHA-256 over the exact acquired bytes and requires the assignment content identity to match. Only verified artifacts reach a compile-only Wasmtime preflight, which rejects guest host imports and missing/non-function exports.

C65 deliberately records `wasmtime_instantiated=false` and `guest_execution_performed=false`. Instantiation is deferred because a WebAssembly start function can run automatically during instantiation. The first explicit instantiation/invocation remains C66.

C65 adds 12 DATUM tests. With the validated C62 DATUM baseline at `389 passed; 0 failed`, the expected C65 total is `401 passed; 0 failed`.

The checkpoint remains at `136/150` until DATUM formatting, full tests, and build pass on the C65 branch.

The distributed demo target is also recorded in `docs/DISTRIBUTED_DEMO_PREDICTIVE_DGRAPH_TARGET.md`, including Raspberry Pi + local PC + VM, optional AWS bootstrap via Ansible, a live D-Graph dashboard, and a read-only predictive what-if layer with a GNN candidate and LLM natural-language interface.

Next after validation: close C65 at `137/150` and proceed to C66 explicit live single-node instantiation/invocation and runtime evidence.
