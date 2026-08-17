# C67 — Live Distributed Three-Node Deployment Final

- checkpoint: `C67/150`
- progress_after_validation: `139/150`
- phase: Controlled WASM Deployment
- status: validated
- validation_date: `2026-08-15`

## Result

C67 successfully validated one distributed Deployment Manifest across three Sentinel/DATUM execution nodes with distinct node-scoped assignments and distinct remotely delivered WebAssembly artifacts.

The validated laboratory topology was:

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

Both remote VMs reached the local dserver and artifact repository over the laboratory network.

## Regression evidence

Before live execution, the C67 branch passed:

```text
dserver cargo fmt: PASS
dserver cargo test: 474 passed; 0 failed
dserver cargo build: PASS
Python syntax validation for all C67 scripts: PASS
```

The two fresh Xubuntu VMs were bootstrapped independently and successfully built the same Sentinel/DATUM execution code. VM 1 used Rust 1.95.0; VM 2 used Rust 1.97.1.

## Global manifest and node slicing

The dserver accepted one manifest with:

```text
manifest_id=c67-live-three-node
project_id=c67-live-project
strategy=distributed
graph_distribution=node_slice
source_assignment_count=3
```

C64-derived views were inspected before execution and each contained exactly one assignment:

```text
fog-01   -> fog-scale
local-01 -> local-aggregate
cloud-01 -> cloud-delta
```

No cross-node assignment was present in any validated node slice.

## Distinct artifacts

The controlled artifact repository served three independent WebAssembly binary modules:

```text
fog-scale.wasm
  size: 47 bytes
  function: fog_scale(i32, i32) -> i32
  content_id: sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c

local-aggregate.wasm
  size: 53 bytes
  function: local_aggregate(i32, i32) -> i32
  content_id: sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d

cloud-delta.wasm
  size: 49 bytes
  function: cloud_delta(i32, i32) -> i32
  content_id: sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
```

All three artifacts were fetched through host-side HTTP acquisition into memory. The live reports recorded network delivery and no persistent artifact write.

## Clean live campaign

After resetting C67 in-memory experiment state by republishing the manifest, exactly one execution was performed for each logical node.

Final validator output:

```text
status=PASS
manifest_id=c67-live-three-node
project_id=c67-live-project
report_count=3
```

Validated node evidence:

```text
fog-01
  assignment_id=fog-scale
  result_i32=24
  fuel_consumed=4
  artifact_content_id=sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c

local-01
  assignment_id=local-aggregate
  result_i32=30
  fuel_consumed=4
  artifact_content_id=sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d

cloud-01
  assignment_id=cloud-delta
  result_i32=20
  fuel_consumed=4
  artifact_content_id=sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
```

The final validator also required three distinct non-empty execution IDs and three distinct artifact content IDs.

For every accepted report, the dserver required and the clean campaign satisfied:

```text
status=executed
explicit_execute=true
integrity_verified=true
wasmtime_compiled=true
wasmtime_instantiated=true
function_invocation_attempted=true
guest_execution_performed=true
artifact_delivery_performed=true
host_network_fetch_performed=true
persistent_write_performed=false
host_mutation_performed=false
wasm_guest_side_effects=false
safe_to_auto_execute=false
governed_execution_performed=false
timeout_enforced=false
```

Each assignment configured a fuel budget of 1,000,000 Wasmtime fuel units and consumed 4 units in this minimal scalar workload. Fuel was enforced; the configured `timeout_ms=1000` remained informational and `timeout_enforced=false` was reported explicitly.

## What C67 proves

C67 proves the following controlled deployment path with real network-separated execution hosts:

```text
one distributed Deployment Manifest
  -> dserver validation
  -> C64 node_slice derivation
  -> node-specific Sentinel/DATUM execution
  -> remote artifact acquisition
  -> SHA-256/content identity verification
  -> Wasmtime compilation and instantiation
  -> explicit function invocation
  -> per-node LiveWasmExecutionReport
  -> centralized dserver validation and collection
```

C67 does not claim automatic WASM guest-to-guest dataflow. The values `24 -> 30 -> 20` were deliberately supplied as explicit validation inputs to make the conceptual fog -> local -> cloud sequence visible.

C67 also does not authorize canonical governed `wasm_execution`; that path remains fail-closed.

## Identity hardening observation

During exploratory execution, a `local-01` assignment was accidentally invoked from the VM physically hosting `cloud-01` by explicitly passing `--node-id local-01`. The report was logically consistent with the manifest and therefore accepted by the current controlled corridor.

This exposed an important hardening requirement for the later governed path: normal Sentinel identity must be bound to the installed agent/host identity and authentication material rather than accepted from an operator-provided node override. A deployment for another node must then fail independently of command-line claims.

This observation does not invalidate the final C67 campaign; experiment state was reset and the clean three-node campaign passed with exactly one intended execution per logical node.

## Next

Proceed to I34/140: close the end-to-end loop from external D-Graph through dserver deployment derivation, node-scoped Sentinel consumption, runtime reports, and materialized observed deployment state, reducing the temporary Python orchestration glue used by C67.
