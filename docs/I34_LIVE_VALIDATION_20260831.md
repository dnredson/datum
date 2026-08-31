# I34 Live Validation — 2026-08-31

- checkpoint: `I34/140`
- predecessor checkpoint: `C67/139`
- validation_status: `live_pass`
- checkpoint_progress: `139/150` pending local regression gate
- topology_kind: controlled distributed three-node laboratory
- governed_execution_performed: false
- safe_to_auto_execute: false

## Validated topology

```text
192.168.10.101  desktop
  dserver :8080
  artifact repository :8090
  Sentinel/DATUM local-01

192.168.10.103  VM
  Sentinel/DATUM fog-01

192.168.10.108  VM
  Sentinel/DATUM cloud-01
```

The artifact repository served the same three C67 WebAssembly binaries, regenerated into the I34 external D-Graph using the active desktop address `192.168.10.101`.

```text
fog-scale.wasm       47 bytes  sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c
local-aggregate.wasm 53 bytes  sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d
cloud-delta.wasm     49 bytes  sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
```

## External D-Graph activation

The externally prepared graph was published to:

```text
POST /api/v1/controlled-wasm/dgraph
```

The dserver derived the Deployment Manifest rather than receiving a manually authored manifest.

Validated identity:

```text
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
strategy=distributed
graph_distribution=node_slice
assignment_count=3
```

Assignments were derived as:

```text
fog-scale       -> fog-scale-module       -> fog-01
local-aggregate -> local-aggregate-module -> local-01
cloud-delta     -> cloud-delta-module     -> cloud-01
```

The execution boundary remained fail-closed:

```text
explicit_invocation_required=true
automatic_execution_allowed=false
governed_execution_authorized=false
```

## Live executions

Exactly one intended execution was performed per node.

### fog-01 — 192.168.10.103

```text
assignment_id=fog-scale
function=fog_scale
args=i32:12,i32:2
result_i32=24
fuel_consumed=4
artifact_content_id=sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c
execution_id=ebb099e2-7ae2-43ff-a8ab-8eb15acd82ed
report_published=true
```

### local-01 — 192.168.10.101

```text
assignment_id=local-aggregate
function=local_aggregate
args=i32:24,i32:6
result_i32=30
fuel_consumed=4
artifact_content_id=sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d
execution_id=93df16ac-70b8-4442-991f-3c57d938e74f
report_published=true
```

### cloud-01 — 192.168.10.108

```text
assignment_id=cloud-delta
function=cloud_delta
args=i32:30,i32:10
result_i32=20
fuel_consumed=4
artifact_content_id=sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
execution_id=772555f9-5792-4afb-be5f-b2052d10a1af
report_published=true
```

## Collected report evidence

`GET /api/v1/controlled-wasm/live-reports` returned exactly three reports.

For all three executions, the collected evidence included:

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
fuel_configured=1000000
fuel_consumed=4
```

## Materialized observed D-Graph validation

The final validator was executed against:

```text
GET /api/v1/controlled-wasm/observed-dgraph
```

and returned:

```text
status=PASS
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
report_count=3
wasm_report_observes_relationships=6
```

Materialized module evidence:

```text
fog-scale-module
  node_id=fog-01
  result_i32=24
  fuel_consumed=4
  observed_state=executed

local-aggregate-module
  node_id=local-01
  result_i32=30
  fuel_consumed=4
  observed_state=executed

cloud-delta-module
  node_id=cloud-01
  result_i32=20
  fuel_consumed=4
  observed_state=executed
```

The observed D-Graph contained exactly six canonical report relationships:

```text
WasmRuntimeReport --OBSERVES--> Node
WasmRuntimeReport --OBSERVES--> WasmModuleDescriptor
```

for each of the three execution reports.

## Result

The I34 live campaign validates the intended controlled end-to-end path:

```text
External D-Graph
  -> dserver validation
  -> derived typed Deployment Manifest
  -> node-scoped deployment slices
  -> Sentinel/DATUM execution on three physical hosts
  -> network artifact acquisition
  -> SHA-256/content identity verification
  -> explicit Wasmtime execution
  -> three runtime reports
  -> dserver collection
  -> materialized observed D-Graph
  -> desired/observed execution evidence
```

This live campaign is a PASS. The checkpoint remains at `139/150` only until the I34 local regression gate recorded in `I34_END_TO_END_DGRAPH_OBSERVED_STATE.md` is reconfirmed: dserver formatting, 482-test regression, dserver build, and Python syntax validation.
