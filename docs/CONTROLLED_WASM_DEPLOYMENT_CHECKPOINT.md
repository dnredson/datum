# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `139/150`
- latest_checkpoint: `C67`
- predecessor_phase_final: `C58_DISTRIBUTED_DRY_RUN_PREPARATION_CHECKPOINT_FINAL.md`
- phase_entry_checkpoint: `C59/150`
- external_wasm_artifact_loader: validated
- artifact_sha256_integrity: validated
- generic_verified_wasm_runner: validated
- typed_deployment_manifest: validated
- node_scoped_deployment_assignment: validated
- datum_artifact_acquisition_preflight: validated
- live_single_node_wasm_execution: validated
- live_distributed_three_node_deployment: validated
- end_to_end_external_dgraph_observed_state: live_validation_passed_regression_pending
- governed_allow_transition_available: false
- next_checkpoint: `I34/140`
- next_scope: reconfirm I34 local regression gate, then close external D-Graph -> derived Deployment Manifest -> node slices -> Sentinel/DATUM -> runtime reports -> materialized observed D-Graph

## Phase progression

C59 defined the external D-Graph boundary, centralized/full and distributed/node_slice deployment semantics, and the controlled artifact-backed WASM sandbox corridor.

C60 validated explicit external local `.wasm` loading and isolated Wasmtime execution while rejecting guest host imports.

C61 added deterministic `sha256:<digest>` content identity and integrity-gated execution before Wasmtime compilation/instantiation.

C62 generalized the verified artifact corridor to scalar `i32`, `i64`, `f32`, and `f64` arguments/results while keeping `v128` and reference types blocked. DATUM validation passed with `389 passed; 0 failed` and build PASS.

C63 added the dserver-owned typed Deployment Manifest binding project, module, function, artifact URI/content identity, target node, runtime limits, deployment strategy, and graph distribution. dserver validation passed with `452 passed; 0 failed` and build PASS.

C64 added validated node-scoped delivery. Distributed/node_slice views contain only assignments targeting the requested node; centralized/full returns the complete assignment set only to the central target; non-target nodes receive an explicit empty view. Cross-node leakage remains independently rejected. dserver validation passed with `464 passed; 0 failed` and build PASS.

C65 added the Sentinel/DATUM-side consumer and bounded artifact preparation corridor. It validates node addressing and assignment boundaries before artifact access, supports local file and HTTPS acquisition, allows insecure HTTP only by explicit lab opt-in, applies size and request-time bounds, disables redirects, verifies exact SHA-256/content identity, compiles in Wasmtime preflight, and retains network-delivered artifacts in memory without persistent writes. DATUM validation passed with `401 passed; 0 failed` and build PASS.

C66 added explicit live single-node execution through `wasm-deploy-live`. The command requires `--execute`, validates the scalar function signature, applies Wasmtime fuel, instantiates the verified artifact, invokes the assigned function, and emits detailed execution evidence. The validated live artifact returned `add(2,3)=5`, consumed 4 of 1,000,000 configured fuel units, and kept canonical governed execution fail-closed. C66 completed at `138/150`.

## C67 — validated live distributed deployment

C67 added a controlled in-memory dserver API for one active Deployment Manifest, C64-derived node slices, and centralized collection/validation of C66 `LiveWasmExecutionReport` evidence.

The validated laboratory topology on `2026-08-15` was:

```text
192.168.10.107  local machine
  dserver :8080
  artifact repository :8090
  Sentinel/DATUM local-01

192.168.10.103  VM 1
  Sentinel/DATUM fog-01

192.168.10.108  VM 2
  Sentinel/DATUM cloud-01
```

Local regression before live validation passed:

```text
dserver cargo fmt: PASS
dserver cargo test: 474 passed; 0 failed
dserver cargo build: PASS
C67 Python syntax validation: PASS
```

Both remote Xubuntu VMs were independently bootstrapped and successfully built the Sentinel/DATUM execution code.

One global manifest was accepted with:

```text
manifest_id=c67-live-three-node
project_id=c67-live-project
strategy=distributed
graph_distribution=node_slice
```

C64-generated views were inspected and each selected exactly one of three source assignments:

```text
fog-01   -> fog-scale
local-01 -> local-aggregate
cloud-01 -> cloud-delta
```

The three remotely served WebAssembly artifacts were distinct:

```text
fog-scale.wasm       47 bytes  sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c
local-aggregate.wasm 53 bytes  sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d
cloud-delta.wasm     49 bytes  sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
```

After resetting the C67 in-memory campaign by republishing the manifest, the clean final validator returned:

```text
status=PASS
report_count=3
fog-01   -> fog-scale       -> result_i32=24 -> fuel_consumed=4
local-01 -> local-aggregate -> result_i32=30 -> fuel_consumed=4
cloud-01 -> cloud-delta     -> result_i32=20 -> fuel_consumed=4
```

The PASS gate also required exactly three distinct execution IDs, three distinct artifact content IDs, successful SHA-256 verification, Wasmtime compilation/instantiation, guest function invocation, network artifact delivery, no persistent artifact write, no host mutation, no guest side effects, `safe_to_auto_execute=false`, `governed_execution_performed=false`, and `timeout_enforced=false`.

C67 therefore proves the controlled distributed path:

```text
one distributed Deployment Manifest
  -> dserver validation
  -> node_slice derivation
  -> node-specific Sentinel/DATUM consumption
  -> network artifact acquisition
  -> content integrity verification
  -> Wasmtime execution
  -> per-node live report
  -> centralized dserver validation/collection
```

Detailed evidence is recorded in `docs/C67_LIVE_DISTRIBUTED_THREE_NODE_DEPLOYMENT_FINAL.md`.

## Identity hardening observation

An exploratory accidental invocation demonstrated that the current C67 operator path can override logical node identity with `--node-id`. The final governed Sentinel path must instead bind node identity to the installed/authenticated agent and reject assignments for other nodes independently of command-line claims. The clean C67 campaign was reset and rerun correctly, so this observation is a follow-up hardening requirement rather than a validation failure.

## I34 — live validation PASS; regression reconfirmation pending

I34 closes the controlled provenance and observation loop around the validated C67 execution corridor.

A new dserver core bridge derives a typed C63 Deployment Manifest directly from an externally supplied canonical D-Graph by joining:

```text
DGraphWasmModuleRef
  -> DGraphModule
  -> DGraphArtifact
  +  DGraphPlacementDecision
  -> DeploymentAssignment
```

The external graph provides `metadata.dgraph_id`; the derived manifest carries that value as `source_dgraph_id`. Artifact URI and SHA-256 come from the D-Graph artifact, target node comes from the placement decision, and function/runtime limits come from the WASM descriptor. The resulting manifest remains `distributed/node_slice` and fail-closed for automatic/governed execution.

New controlled endpoints are:

```text
POST /api/v1/controlled-wasm/dgraph
GET  /api/v1/controlled-wasm/dgraph
GET  /api/v1/controlled-wasm/observed-dgraph
```

The existing C67 manifest/slice/live-report endpoints remain compatible.

On `2026-08-31`, the I34 live campaign was rerun with the desktop at `192.168.10.101`, fog Sentinel at `192.168.10.103`, and cloud Sentinel at `192.168.10.108`.

The externally prepared D-Graph activated successfully and dserver derived:

```text
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
strategy=distributed
graph_distribution=node_slice
assignment_count=3
```

Exactly one execution was performed per node:

```text
fog-01   -> fog-scale       -> result_i32=24 -> fuel_consumed=4 -> execution_id=ebb099e2-7ae2-43ff-a8ab-8eb15acd82ed
local-01 -> local-aggregate -> result_i32=30 -> fuel_consumed=4 -> execution_id=93df16ac-70b8-4442-991f-3c57d938e74f
cloud-01 -> cloud-delta     -> result_i32=20 -> fuel_consumed=4 -> execution_id=772555f9-5792-4afb-be5f-b2052d10a1af
```

`GET /api/v1/controlled-wasm/live-reports` returned exactly three reports. All three retained successful integrity verification, Wasmtime compilation/instantiation, network artifact delivery, no persistent write, no host mutation, no guest side effects, `safe_to_auto_execute=false`, `governed_execution_performed=false`, and `timeout_enforced=false`.

The final observed-state validator returned:

```text
status=PASS
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
report_count=3
wasm_report_observes_relationships=6
```

All three D-Graph modules materialized `observed_state=executed`, with results `24`, `30`, and `20` respectively. Detailed live evidence is recorded in `docs/I34_LIVE_VALIDATION_20260831.md`.

The live I34 corridor is therefore validated. The checkpoint remains `139/150` only until the local regression gate is reconfirmed on the current branch:

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml      # expected 482 passed; 0 failed
cargo build --manifest-path dserver/Cargo.toml
python3/py -3 -m py_compile I34/C67 validation scripts
```

## Next

Reconfirm the I34 local regression gate. If it passes, close I34 at `140/150`, mark PR #31 ready, and retain canonical governed `wasm_execution` fail-closed. The seven governance prerequisites still block automatic execution.
