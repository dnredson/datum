# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `138/150`
- latest_checkpoint: `C66`
- predecessor_phase_final: `C58_DISTRIBUTED_DRY_RUN_PREPARATION_CHECKPOINT_FINAL.md`
- phase_entry_checkpoint: `C59/150`
- external_wasm_artifact_loader: validated
- artifact_sha256_integrity: validated
- generic_verified_wasm_runner: validated
- typed_deployment_manifest: validated
- node_scoped_deployment_assignment: validated
- datum_artifact_acquisition_preflight: validated
- live_single_node_wasm_execution: validated
- governed_allow_transition_available: false
- next_checkpoint: `C67/150`
- next_scope: real multi-node distributed deployment with distinct node-scoped assignments/artifacts

C59 defined the external D-Graph boundary, centralized/full and distributed/node_slice deployment semantics, and the controlled artifact-backed WASM sandbox corridor.

C60 validated explicit external local `.wasm` loading and isolated Wasmtime execution while rejecting guest host imports.

C61 added deterministic content identity as `sha256:<digest>` and an integrity-gated execution path that rejects malformed or mismatched expected digests before Wasmtime compilation/instantiation.

C62 generalized the verified artifact corridor with a dynamic scalar ABI supporting `i32`, `i64`, `f32`, and `f64`, including zero/multiple arguments and zero/multiple results. `v128` and reference types remain blocked. C62 Linux validation passed with DATUM `389 passed; 0 failed` and build PASS.

C63 added the dserver-owned typed Deployment Manifest contract. It binds project, module, function, artifact URI, SHA-256/content identity, target node, runtime limits, strategy, and graph-distribution semantics while remaining non-executable evidence. C63 Linux validation passed with dserver `452 passed; 0 failed` and build PASS.

C64 derives typed node-scoped deployment views from a validated C63 manifest. Distributed/node_slice views contain only assignments whose `target_node` matches the requested node; centralized/full returns the complete assignment set only to the central target; non-target nodes receive an explicit empty view. C64 independently rejects cross-node assignment leakage and preserves the fail-closed execution boundary.

C64 Linux validation passed with dserver `464 passed; 0 failed` and build PASS. C64 is complete at `136/150`.

C65 adds a DATUM-side consumer for the C64 node-scoped JSON shape and a bounded artifact preparation corridor. DATUM validates the local-node address and assignment boundary before artifact access, supports `file://` and HTTPS acquisition, keeps insecure HTTP opt-in for trusted lab/LAN validation only, applies size and request-time bounds, disables redirects, and keeps network-delivered artifacts in memory without persistent writes.

C65 Linux validation passed with DATUM `401 passed; 0 failed` and build PASS. C65 is complete at `137/150`.

C66 introduces the explicit `wasm-deploy-live` DATUM command. It validates the complete local node-scoped view, selects one requested assignment before artifact acquisition, reuses the C65 acquisition/SHA-256/preflight corridor, validates the scalar function signature, requires `--execute`, applies the configured fuel budget, and crosses the Wasmtime instantiation/invocation boundary.

C66 Linux validation evidence on `2026-08-15`:

- DATUM formatting/test/build validation: reported PASS with no errors; the exact numeric test summary line was not pasted;
- real artifact: `.c66-live/c66-add.wasm`, 41 bytes;
- artifact SHA-256/content ID: `sha256:f61fd62f57c41269c3c23f360eeaf1090b1db9c38651106674d48bc65dba88ba`;
- node: `fog-01`;
- assignment: `c66-add-assignment`;
- function: `add(i32, i32) -> i32`;
- arguments: `2`, `3`;
- result: `5`;
- `status=executed`;
- `integrity_verified=true`;
- `wasmtime_compiled=true`;
- `wasmtime_instantiated=true`;
- `function_invocation_attempted=true`;
- `guest_execution_performed=true`;
- `wasm_guest_side_effects=false`;
- `governed_execution_performed=false`;
- `host_mutation_performed=false`;
- `safe_to_auto_execute=false`;
- fuel: `1000000` configured, `4` consumed, not exhausted;
- `timeout_ms=1000`, `timeout_enforced=false`.

C66 is complete at `138/150`.

The distributed demo target is recorded in `docs/DISTRIBUTED_DEMO_PREDICTIVE_DGRAPH_TARGET.md`, including Raspberry Pi + local PC + VM, optional AWS bootstrap via Ansible, a live D-Graph dashboard, and a read-only predictive what-if layer with a GNN candidate and LLM natural-language interface.

Next: C67 real multi-node distributed deployment with distinct node-scoped assignments/artifacts. Canonical governed `wasm_execution` remains fail-closed.
