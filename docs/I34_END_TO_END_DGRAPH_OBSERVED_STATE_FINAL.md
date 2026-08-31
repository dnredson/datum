# I34 — End-to-End External D-Graph to Observed Deployment State — Final

- checkpoint: `I34/140`
- predecessor: `C67/139`
- phase: Controlled WASM Deployment
- status: `validated_complete`
- closed_at_utc: `2026-08-31`

## Result

I34 is validated complete.

The checkpoint closes the controlled end-to-end loop:

```text
External D-Graph
  -> dserver validation
  -> derived typed Deployment Manifest
  -> C64 node slices
  -> Sentinel/DATUM agents
  -> bounded artifact acquisition
  -> SHA-256/content identity verification
  -> explicit Wasmtime execution
  -> live runtime reports
  -> dserver
  -> materialized observed D-Graph
```

The external component remains the D-Graph producer. dserver consumes, validates, interprets, derives deployment state, and materializes observed evidence; it does not generate the source D-Graph.

Canonical governed automatic `wasm_execution` remains fail-closed.

## Regression validation

The current I34 branch passed the local regression gate on the Windows desktop:

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check
PASS

cargo test --manifest-path dserver/Cargo.toml
482 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo build --manifest-path dserver/Cargo.toml
PASS
```

The build emitted only the existing dead-code warnings:

```text
EXECUTION_PREREQUISITE_GATE_NAMES is never used
default_embedded_add_module_descriptor is never used
```

Python syntax validation also passed with no output:

```text
py -3 -m py_compile \
  scripts/i34_prepare_external_dgraph.py \
  scripts/i34_validate_observed_dgraph.py \
  scripts/c67_run_distributed_node.py
```

## Live topology

Validated on `2026-08-31`:

```text
192.168.10.101  Windows desktop
  dserver :8080
  artifact repository :8090
  Sentinel/DATUM local-01

192.168.10.103  Xubuntu VM
  Sentinel/DATUM fog-01

192.168.10.108  Xubuntu VM
  Sentinel/DATUM cloud-01
```

The artifact repository served the independently existing WASM binaries:

```text
fog-scale.wasm       47 bytes
local-aggregate.wasm 53 bytes
cloud-delta.wasm     49 bytes
```

## External D-Graph activation

`scripts/i34_prepare_external_dgraph.py` regenerated the source D-Graph directly from the WASM artifact bytes, including SHA-256 content identity and the current artifact host address.

The external D-Graph was published to:

```text
POST /api/v1/controlled-wasm/dgraph
```

The dserver accepted the graph and derived:

```text
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
strategy=distributed
graph_distribution=node_slice
assignment_count=3
side_effects_performed=false
```

The resulting assignments were:

```text
fog-scale       -> fog-scale-module       -> fog-01
local-aggregate -> local-aggregate-module -> local-01
cloud-delta     -> cloud-delta-module     -> cloud-01
```

Each node-scoped deployment view selected exactly one assignment from the three-assignment source manifest.

## Live execution results

Exactly one intended execution was performed per node.

### fog-01

```text
assignment_id=fog-scale
function=fog_scale
args=12,2
result_i32=24
fuel_consumed=4
artifact_content_id=sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c
execution_id=ebb099e2-7ae2-43ff-a8ab-8eb15acd82ed
report_published=true
```

### local-01

```text
assignment_id=local-aggregate
function=local_aggregate
args=24,6
result_i32=30
fuel_consumed=4
artifact_content_id=sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d
execution_id=93df16ac-70b8-4442-991f-3c57d938e74f
report_published=true
```

### cloud-01

```text
assignment_id=cloud-delta
function=cloud_delta
args=30,10
result_i32=20
fuel_consumed=4
artifact_content_id=sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
execution_id=772555f9-5792-4afb-be5f-b2052d10a1af
report_published=true
```

## Runtime evidence

`GET /api/v1/controlled-wasm/live-reports` returned exactly three reports.

For all three executions the evidence retained:

```text
status=executed
explicit_execute=true
integrity_verified=true
artifact_delivery_performed=true
host_network_fetch_performed=true
persistent_write_performed=false
wasmtime_compiled=true
wasmtime_instantiated=true
function_invocation_attempted=true
guest_execution_performed=true
wasm_guest_side_effects=false
host_mutation_performed=false
safe_to_auto_execute=false
governed_execution_performed=false
timeout_enforced=false
```

## Observed D-Graph validation

The final gate:

```text
py -3 scripts/i34_validate_observed_dgraph.py \
  --dserver http://192.168.10.101:8080
```

returned:

```text
status=PASS
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
report_count=3
wasm_report_observes_relationships=6
```

The three modules materialized:

```text
fog-scale-module       observed_state=executed result_i32=24
local-aggregate-module observed_state=executed result_i32=30
cloud-delta-module     observed_state=executed result_i32=20
```

Each `DGraphWasmRuntimeReportRef` was linked through canonical `OBSERVES` relationships to both its node and its WASM module descriptor, yielding exactly six report relationships.

## What I34 proves

I34 proves that an externally supplied canonical D-Graph can drive the controlled deployment corridor without a manually authored Deployment Manifest, and that real runtime evidence from the distributed Sentinel/DATUM executions can be projected back into a validated observed D-Graph.

The checkpoint therefore closes the distinction between deployment intent and observed deployment state for this controlled WASM corridor.

## Non-claims and open hardening requirements

I34 does not claim:

- autonomous Sentinel polling/execution without explicit operator activation;
- guest-to-guest WASM dataflow;
- authenticated physical Sentinel identity;
- wall-clock timeout enforcement;
- canonical governed automatic execution;
- production HTTPS artifact hosting.

The C67 identity observation remains open: the normal Sentinel path must bind `node_id` to installed/authenticated local identity rather than a freely supplied command-line value.

The temporary Python runner still performs control-plane orchestration around the Sentinel binary. A subsequent increment should move more of deployment activation, node identity, and report publication into the Sentinel agent itself while retaining the governed fail-closed boundary.

## Closure

I34 advances the Controlled WASM Deployment phase from `139/150` to `140/150`.
