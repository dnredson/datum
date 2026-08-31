# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `141/150`
- latest_checkpoint: `I35`
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
- end_to_end_external_dgraph_observed_state: validated
- sentinel_bound_deployment_cycle: validated
- governed_allow_transition_available: false
- next_checkpoint: `I36/142`
- next_scope: persist controlled dserver deployment state across restart while validating restored state fail-closed

## Phase progression

The Controlled WASM Deployment phase entered at C59 and has incrementally established:

```text
C59  external D-Graph boundary and deployment semantics
C60  isolated external WASM loading/execution
C61  SHA-256 content identity
C62  generic scalar ABI
C63  typed dserver Deployment Manifest
C64  node-scoped deployment delivery
C65  Sentinel bounded artifact acquisition/preflight
C66  explicit live single-node execution
C67  live distributed three-node deployment
I34  external D-Graph -> manifest -> reports -> observed D-Graph
I35  Sentinel-bound slice discovery/execution/report publication
```

Detailed evidence for earlier checkpoints remains in their corresponding final/checkpoint documents.

## I34 — validated at 140/150

I34 closed the controlled provenance and observation loop:

```text
External D-Graph
  -> dserver-derived typed Deployment Manifest
  -> node slices
  -> Sentinel execution corridor
  -> runtime reports
  -> observed D-Graph
```

Final I34 regression:

```text
dserver fmt: PASS
dserver test: 482 passed; 0 failed
dserver build: PASS
Python syntax validation: PASS
```

Final I34 live topology:

```text
192.168.10.101  desktop: dserver + artifact repository + local-01
192.168.10.103  Xubuntu VM: fog-01
192.168.10.108  Xubuntu VM: cloud-01
```

Final I34 observed-state validation:

```text
status=PASS
report_count=3
fog-01   -> 24
local-01 -> 30
cloud-01 -> 20
wasm_report_observes_relationships=6
```

Detailed evidence:

- `docs/I34_LIVE_VALIDATION_20260831.md`
- `docs/I34_END_TO_END_DGRAPH_OBSERVED_STATE_FINAL.md`

## I35 — validated at 141/150

I35 removed the temporary Python orchestration glue from the successful controlled deployment path.

The validated Sentinel-bound path is:

```text
external D-Graph
  -> dserver derives Deployment Manifest
  -> dserver derives node_slice
  -> Sentinel reads agent.node_id
  -> Sentinel fetches its own slice
  -> Sentinel selects the single delivered assignment
  -> Sentinel acquires + verifies artifact
  -> explicit --execute
  -> Wasmtime execution
  -> Sentinel publishes LiveWasmExecutionReport directly to dserver
  -> dserver materializes observed D-Graph
```

The operator no longer supplies a logical node identity in the I35 path. `--node-id` is rejected and logical identity comes from `agent.node_id`. Manual assignment selection is also rejected; `--assignment auto` accepts only the single assignment delivered in the node slice.

This is configuration-bound logical identity, not cryptographic physical-agent attestation.

### I35 regression

```text
DATUM fmt: PASS
DATUM test: 419 passed; 0 failed
DATUM build: PASS
```

### I35 fail-closed gates

The following were exercised and rejected with the expected errors:

```text
missing --execute
--node-id override
manual assignment instead of auto
HTTP control-plane access without explicit lab opt-in
```

Zero/multiple-assignment and empty-bound-identity cases are covered by focused unit tests.

### I35 final live campaign

The dserver was restarted before the final campaign, confirming the current in-memory-state limitation. After republishing the external D-Graph, `live-reports` returned `count=0`.

Exactly one direct Sentinel execution was then performed per node:

```text
fog-01
  assignment=fog-scale
  result_i32=24
  fuel_consumed=4
  execution_id=9dd726b4-a8d2-4195-bc2b-0d11b8592ba5

local-01
  assignment=local-aggregate
  result_i32=30
  fuel_consumed=4
  execution_id=cc3120c8-c063-465f-a41a-b0be09d43776

cloud-01
  assignment=cloud-delta
  result_i32=20
  fuel_consumed=4
  execution_id=617e5509-0b01-4415-8af9-6e19286cf417
```

All three reports retained:

```text
status=executed
integrity_verified=true
wasmtime_compiled=true
wasmtime_instantiated=true
guest_execution_performed=true
persistent_write_performed=false
host_mutation_performed=false
wasm_guest_side_effects=false
safe_to_auto_execute=false
governed_execution_performed=false
timeout_enforced=false
```

Final observed-state validation:

```text
status=PASS
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
report_count=3
wasm_report_observes_relationships=6
```

Detailed evidence:

- `docs/I35_REGRESSION_VALIDATION_20260831.md`
- `docs/I35_SENTINEL_BOUND_DEPLOYMENT_CYCLE.md`
- `docs/I35_SENTINEL_BOUND_DEPLOYMENT_CYCLE_FINAL.md`

During I35 preparation, the stale `DATUM/config.linux` `implemented_by` map syntax was also corrected to the sequence form expected by the current parser.

## Remaining path to 150/150

The remaining nine checkpoints are fixed in:

`docs/CONTROLLED_WASM_DEPLOYMENT_142_TO_150_ROADMAP.md`

Summary:

```text
I36/142 persistent controlled dserver deployment state
I37/143 Sentinel registration and lease state
I38/144 Sentinel deployment inbox discovery
I39/145 explicit multi-assignment scheduling
I40/146 dependency-aware deployment stages
I41/147 deployment lifecycle and retry evidence
I42/148 resource-aware observed D-Graph join
I43/149 deployment readiness and governance evidence
I44/150 phase-final integration and reproducibility package
```

The critical path intentionally excludes Kubernetes, Ansible, AWS, and canonical automatic governed execution. Those can consume the stable deployment interfaces after phase closure.

## Next

I36/142 removes the dserver restart-state loss exposed directly during the I35 campaign. Source D-Graph, derived manifest, and accepted live reports should persist atomically and be revalidated before restoration. Corrupt persisted state must fail closed rather than activate an invalid deployment.