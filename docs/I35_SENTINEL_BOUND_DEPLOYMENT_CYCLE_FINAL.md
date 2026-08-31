# I35 — Sentinel-Bound Deployment Cycle Final

- checkpoint: `I35/141`
- predecessor: `I34/140`
- phase: Controlled WASM Deployment
- validation_date: `2026-08-31`
- status: `validated`

## Closure

I35 removes the temporary Python orchestration glue from the successful controlled deployment path while preserving explicit operator authorization and fail-closed governed execution.

The validated path is now:

```text
external D-Graph
  -> dserver derives Deployment Manifest
  -> dserver derives node_slice
  -> Sentinel reads local agent.node_id
  -> Sentinel fetches its own slice
  -> Sentinel requires exactly one delivered assignment
  -> Sentinel acquires + verifies the WASM artifact
  -> explicit --execute remains mandatory
  -> Sentinel executes through the controlled Wasmtime corridor
  -> Sentinel publishes LiveWasmExecutionReport directly to dserver
  -> dserver materializes the observed D-Graph
```

The I35 success path does not use `scripts/c67_run_distributed_node.py`.

## Local regression

The current I35 branch passed the complete DATUM regression gate on the Windows desktop:

```text
cargo fmt --manifest-path DATUM/Cargo.toml -- --check: PASS
cargo test --manifest-path DATUM/Cargo.toml: 419 passed; 0 failed
cargo build --manifest-path DATUM/Cargo.toml: PASS
```

The earlier `408` count was only a planning estimate derived from a historical 401-test baseline plus seven focused I35 tests. The authoritative observed branch result is `419 passed; 0 failed` because additional DATUM tests had accumulated.

## Fail-closed validation

The following operator attempts were explicitly exercised and rejected with the expected errors:

```text
missing --execute                          -> rejected
--node-id override in bound mode           -> rejected
manual assignment instead of `auto`        -> rejected
plain HTTP without --allow-insecure-http   -> rejected
```

The focused unit suite also covers empty local identity, zero-assignment slices, and multiple-assignment slices.

The I35 control-plane path therefore establishes configuration-bound logical node identity from `agent.node_id`. This closes the C67 command-line identity override weakness for the new path, but it is not cryptographic physical-host attestation.

## Configuration compatibility fix

During live preparation, the versioned `DATUM/config.linux` was found to contain an outdated `implemented_by` map form. The current parser expects a sequence. I35 corrected the examples from:

```toml
implemented_by = { container_name = "mosquitto" }
```

to:

```toml
implemented_by = [{ container_name = "mosquitto" }]
```

and likewise for the DATUM process mapping.

## Final live topology

```text
192.168.10.101  Windows desktop
  dserver :8080
  artifact repository :8090
  Sentinel local-01

192.168.10.103  Xubuntu VM
  Sentinel fog-01

192.168.10.108  Xubuntu VM
  Sentinel cloud-01
```

The two VMs used clean I35 Git clones through the configured SSH GitHub transport. Per-host validation configs bound:

```text
.103 -> agent.node_id=fog-01
.101 -> agent.node_id=local-01
.108 -> agent.node_id=cloud-01
```

The dserver was restarted before the final campaign. The expected in-memory reset was confirmed by a 404 for the deployment slice before republishing the external D-Graph. The D-Graph was then republished and `GET /live-reports` returned `count=0` before any final execution.

## Final direct-Sentinel executions

Exactly one final execution was recorded per node:

```text
fog-01
  assignment=fog-scale
  result_i32=24
  fuel_consumed=4
  execution_id=9dd726b4-a8d2-4195-bc2b-0d11b8592ba5
  report_published=true

local-01
  assignment=local-aggregate
  result_i32=30
  fuel_consumed=4
  execution_id=cc3120c8-c063-465f-a41a-b0be09d43776
  report_published=true

cloud-01
  assignment=cloud-delta
  result_i32=20
  fuel_consumed=4
  execution_id=617e5509-0b01-4415-8af9-6e19286cf417
  report_published=true
```

The three artifacts retained the validated content identities:

```text
fog-scale.wasm
sha256:ef6f0bbe18ff074bb7d197172234217fe4a16892f4944ff83d043d054368b96c

local-aggregate.wasm
sha256:27a26fecd3441a4525f4d0d3a8c71b4f8d1c685115a2a622b238257ac6ebd66d

cloud-delta.wasm
sha256:919a114011eefd3e1b92d82cba2c6d15efc42c31206f4aff73658b7d72ce0a1a
```

`GET /api/v1/controlled-wasm/live-reports` returned exactly three reports. All three retained:

```text
status=executed
explicit_execute=true
integrity_verified=true
wasmtime_compiled=true
wasmtime_instantiated=true
guest_execution_performed=true
artifact_delivery_performed=true
persistent_write_performed=false
host_mutation_performed=false
wasm_guest_side_effects=false
safe_to_auto_execute=false
governed_execution_performed=false
timeout_enforced=false
```

## Observed D-Graph gate

The final observed-state validator returned:

```text
status=PASS
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
report_count=3
wasm_report_observes_relationships=6
```

Materialized module observations were:

```text
fog-scale-module       -> fog-01   -> 24 -> fuel=4
local-aggregate-module -> local-01 -> 30 -> fuel=4
cloud-delta-module     -> cloud-01 -> 20 -> fuel=4
```

The report IDs matched the three direct-Sentinel execution IDs above.

## What I35 proves beyond I34

I34 proved the complete external D-Graph -> deployment -> report -> observed D-Graph loop, but temporary Python glue fetched node slices, invoked the Sentinel, and published reports.

I35 moves those responsibilities into the Sentinel process itself. The operator now supplies the control-plane URL, scalar function inputs, and explicit execution authorization; the Sentinel binds its logical identity from configuration, fetches its own slice, selects the single delivered assignment, performs artifact verification/execution, and publishes its own report.

## Non-claims

I35 does not claim:

- cryptographic physical-host or agent attestation;
- continuous polling or autonomous activation;
- scheduling of multiple assignments;
- guest-to-guest WASM dataflow;
- production TLS/authentication setup;
- enforced wall-clock timeout interruption;
- canonical governed automatic execution.

The canonical governed execution path remains fail-closed.

I35 therefore advances the Controlled WASM Deployment phase to `141/150`.