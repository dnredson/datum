# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `142/150`
- latest_checkpoint: `I36`
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
- controlled_wasm_state_persistence: validated
- governed_allow_transition_available: false
- next_checkpoint: `I37/143`
- next_scope: add Sentinel registration and lease/heartbeat state so dserver can distinguish active and stale logical agents without treating deployment requests as presence evidence

## Phase progression

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
I36  persisted + revalidated controlled state across dserver restart
```

Detailed evidence for every earlier checkpoint remains in the corresponding final/checkpoint documents.

## I34 — validated at 140/150

I34 closed the controlled provenance/observation loop:

```text
External D-Graph
  -> dserver-derived Deployment Manifest
  -> node slices
  -> controlled Sentinel execution
  -> runtime reports
  -> observed D-Graph
```

Regression and live evidence:

```text
dserver fmt: PASS
dserver test: 482 passed; 0 failed
dserver build: PASS
report_count=3
fog-01   -> 24
local-01 -> 30
cloud-01 -> 20
wasm_report_observes_relationships=6
```

Evidence:

- `docs/I34_LIVE_VALIDATION_20260831.md`
- `docs/I34_END_TO_END_DGRAPH_OBSERVED_STATE_FINAL.md`

## I35 — validated at 141/150

I35 removed the temporary Python orchestration glue from the successful deployment path. The Sentinel now reads bound `agent.node_id`, fetches its own node slice, accepts only the delivered single assignment, acquires/verifies/executes the WASM under explicit `--execute`, and publishes its own `LiveWasmExecutionReport` directly to dserver.

The I35 control-plane path rejects:

```text
missing --execute
--node-id override
manual assignment instead of auto
HTTP control-plane access without explicit lab opt-in
```

Regression and live evidence:

```text
DATUM fmt: PASS
DATUM test: 419 passed; 0 failed
DATUM build: PASS
fog-01   -> 24 -> 9dd726b4-a8d2-4195-bc2b-0d11b8592ba5
local-01 -> 30 -> cc3120c8-c063-465f-a41a-b0be09d43776
cloud-01 -> 20 -> 617e5509-0b01-4415-8af9-6e19286cf417
observed D-Graph: PASS
report_count=3
wasm_report_observes_relationships=6
```

This establishes configuration-bound logical identity, not cryptographic physical-agent attestation.

Evidence:

- `docs/I35_REGRESSION_VALIDATION_20260831.md`
- `docs/I35_SENTINEL_BOUND_DEPLOYMENT_CYCLE.md`
- `docs/I35_SENTINEL_BOUND_DEPLOYMENT_CYCLE_FINAL.md`

## I36 — validated at 142/150

I36 removes the dserver restart-state loss observed during I35. Controlled state is persisted in a versioned local JSON envelope containing:

```text
source_dgraph
active_manifest
live_reports
```

Runtime storage is local-file based and defaults to `.datum-controlled-wasm-state.json`, with `DATUM_CONTROLLED_WASM_STATE_PATH` available for operator/lab isolation. WASM binaries are not embedded in the state file; their artifact URI/content identity remains referenced from the D-Graph/manifest/report evidence.

Every candidate mutation follows:

```text
validate -> persist -> activate in memory
```

Restoration does not blindly trust the file. It revalidates the source D-Graph, freshly re-derives and compares the active manifest, validates every live report against that manifest, and rejects duplicate execution IDs before activation.

### I36 regression

```text
dserver fmt: PASS
dserver test: 488 passed; 0 failed
dserver build: PASS
```

Only the two existing non-blocking dead-code warnings remained:

```text
EXECUTION_PREREQUISITE_GATE_NAMES
default_embedded_add_module_descriptor
```

### I36 restart gate — PASS

A fresh D-Graph/manifest state was persisted, followed by one direct I35 Sentinel execution:

```text
node_id=fog-01
assignment_id=fog-scale
result_i32=24
fuel_consumed=4
execution_id=d80622ca-f534-49c9-a10b-f0e33dc94998
```

Before restart `live-reports count=1`. The dserver process was stopped and restarted without deleting the state file and without republishing the D-Graph. After restart the same state was restored:

```text
report_count=1
execution_id=d80622ca-f534-49c9-a10b-f0e33dc94998
node_id=fog-01
result_i32=24
```

PowerShell equality checks for report count and execution ID both returned `True`. The full external D-Graph was also available by GET after restart without a new POST.

### I36 corruption gate — PASS

A separate disposable persisted-state file was replaced with deliberately invalid non-JSON content. On controlled-state initialization the dserver logged that the persisted state could not be parsed and explicitly started controlled state empty.

`GET /api/v1/controlled-wasm/dgraph` returned:

```text
HTTP/1.1 404 Not Found
{"error":"no external controlled D-Graph has been published"}
```

The invalid state was therefore not activated. The dserver remained operational, and a later valid D-Graph POST successfully established a new valid controlled state and replaced the corrupt runtime file.

Validated persistence semantics are now:

```text
valid state -> persist -> restart -> revalidate -> restore
corrupt state -> reject -> controlled state empty
empty state -> accept valid D-Graph -> persist valid state
```

Evidence:

- `docs/I36_PERSISTENT_CONTROLLED_STATE.md`
- `docs/I36_PERSISTENT_CONTROLLED_STATE_FINAL.md`

## Remaining path to 150/150

The fixed roadmap is recorded in `docs/CONTROLLED_WASM_DEPLOYMENT_142_TO_150_ROADMAP.md`.

```text
I37/143 Sentinel registration and lease state
I38/144 Sentinel deployment inbox discovery
I39/145 explicit multi-assignment scheduling
I40/146 dependency-aware deployment stages
I41/147 deployment lifecycle and retry evidence
I42/148 resource-aware observed D-Graph join
I43/149 deployment readiness and governance evidence
I44/150 phase-final integration and reproducibility package
```

The critical path intentionally excludes Kubernetes, Ansible, AWS, and canonical automatic governed execution.

## Next

I37/143 should let each Sentinel establish explicit presence independently of deployment execution. dserver should retain a logical registration record with `node_id`, an instance identifier, capabilities/stage summary, last-seen timestamp, lease duration, and derived active/stale status. Node/path identity mismatch must fail closed. This is presence/lease tracking only and must not be described as cryptographic host attestation.