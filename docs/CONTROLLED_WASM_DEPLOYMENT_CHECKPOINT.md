# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `135/150`
- latest_checkpoint: `C63`
- predecessor_phase_final: `C58_DISTRIBUTED_DRY_RUN_PREPARATION_CHECKPOINT_FINAL.md`
- phase_entry_checkpoint: `C59/150`
- external_wasm_artifact_loader: validated
- artifact_sha256_integrity: validated
- generic_verified_wasm_runner: validated
- typed_deployment_manifest: validated
- node_scoped_deployment_assignment: implementation_ready_for_validation
- governed_allow_transition_available: false
- next_checkpoint: `C64/150`
- next_scope: validate node-scoped deployment assignment for centralized/full and distributed/node_slice

C59 defined the external D-Graph boundary, centralized/full and distributed/node_slice deployment semantics, and the controlled artifact-backed WASM sandbox corridor.

C60 validated explicit external local `.wasm` loading and isolated Wasmtime execution while rejecting guest host imports.

C61 added deterministic content identity as `sha256:<digest>` and an integrity-gated execution path that rejects malformed or mismatched expected digests before Wasmtime compilation/instantiation.

C62 generalized the verified artifact corridor with a dynamic scalar ABI supporting `i32`, `i64`, `f32`, and `f64`, including zero/multiple arguments and zero/multiple results. `v128` and reference types remain blocked. The public generic path verifies SHA-256 before runtime evaluation; the lower-level generic runner remains crate-visible only.

C62 Linux validation evidence on `2026-08-14`:

- `cargo fmt --manifest-path DATUM/Cargo.toml -- --check`: PASS;
- DATUM tests: `389 passed; 0 failed`;
- `cargo build --manifest-path DATUM/Cargo.toml`: PASS;
- no formatting fix required;
- no governed execution allow transition;
- no deployment automation introduced.

C62 is complete at `134/150`.

C63 adds the dserver-owned typed Deployment Manifest contract. It binds project, module, function, artifact URI, SHA-256/content identity, target node, runtime limits, strategy, and graph-distribution semantics while remaining non-executable evidence.

C63 accepts only `centralized/full` and `distributed/node_slice`. Centralized assignments must share one target node. Artifact `content_id` must match the normalized SHA-256 digest. The manifest execution boundary requires explicit invocation and forbids both automatic execution and any claim of governed authorization.

C63 Linux validation evidence on `2026-08-15`:

- `cargo fmt --manifest-path dserver/Cargo.toml -- --check`: PASS;
- dserver tests: `452 passed; 0 failed`;
- `cargo build --manifest-path dserver/Cargo.toml`: PASS;
- no governed execution allow transition;
- no execution or deployment side effects introduced.

C63 is complete at `135/150`.

C64 implementation derives typed node-scoped deployment views from a validated C63 manifest. Distributed/node_slice views contain only assignments whose `target_node` matches the requested node; centralized/full returns the complete assignment set only to the central target. Non-target nodes receive an explicit empty view. Source/selected assignment counts make the scoping visible and auditable.

C64 independently validates the derived view, rejects cross-node assignment leakage, preserves artifact identity and runtime limits, and keeps `explicit_invocation_required=true`, `automatic_execution_allowed=false`, and `governed_execution_authorized=false`.

The checkpoint remains at `135/150` until dserver formatting, full tests, and build pass on the C64 branch. Expected dserver target: `464 passed; 0 failed`.

Next after validation: close C64 at `136/150` and proceed to C65 DATUM artifact acquisition/instantiation corridor.
