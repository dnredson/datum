# Controlled WASM Deployment Readiness Checkpoint

- phase_status: active
- progress: `144/150`
- latest_checkpoint: `I38`
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
- sentinel_registration_lease: validated
- sentinel_deployment_inbox_discovery: validated
- governed_allow_transition_available: false
- next_checkpoint: `I39/145`
- next_scope: accept explicit multi-assignment execution only through a deterministic batch scheduling contract while preserving cross-node isolation and explicit authorization

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
I37  explicit Sentinel registration + heartbeat/lease presence state
I38  read-only Sentinel deployment inbox discovery
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

The I35 control-plane path rejects missing `--execute`, `--node-id` override, manual assignment selection, and plain HTTP without explicit lab opt-in. This establishes configuration-bound logical identity, not cryptographic physical-agent attestation.

Evidence:

- `docs/I35_REGRESSION_VALIDATION_20260831.md`
- `docs/I35_SENTINEL_BOUND_DEPLOYMENT_CYCLE.md`
- `docs/I35_SENTINEL_BOUND_DEPLOYMENT_CYCLE_FINAL.md`

## I36 — validated at 142/150

I36 persists controlled state in a versioned local JSON envelope containing `source_dgraph`, `active_manifest`, and `live_reports`. Candidate mutation ordering is `validate -> persist -> activate in memory`. Restoration revalidates the source D-Graph, freshly re-derives and compares the manifest, validates every report, and rejects duplicate execution IDs.

Regression:

```text
dserver fmt: PASS
dserver test: 488 passed; 0 failed
dserver build: PASS
```

Restart gate:

```text
pre-restart report_count=1
execution_id=d80622ca-f534-49c9-a10b-f0e33dc94998
restart without D-Graph republish
post-restart report_count=1
same execution_id=True
source D-Graph restored=True
```

Corruption gate:

```text
invalid non-JSON state -> not activated
controlled state -> empty
GET controlled D-Graph -> 404
later valid D-Graph POST -> accepted and persisted
```

Evidence:

- `docs/I36_PERSISTENT_CONTROLLED_STATE.md`
- `docs/I36_PERSISTENT_CONTROLLED_STATE_FINAL.md`

## I37 — validated at 143/150

I37 adds explicit logical Sentinel presence independent from deployment execution.

Validated endpoints:

```text
PUT  /api/v1/sentinels/:node_id/registration
POST /api/v1/sentinels/:node_id/heartbeat
GET  /api/v1/sentinels
GET  /api/v1/sentinels/:node_id
```

The dserver derives `active` / `stale` from server time and `lease_seconds`; the client cannot submit status. `node_id`, `instance_id`, stage, capabilities, registration timestamp, last-seen timestamp, and lease duration are retained.

Regression:

```text
dserver fmt: PASS
dserver test: 495 passed; 0 failed
dserver build: PASS
```

Live topology registration:

```text
local-01 instance=local-i37 stage=local lease=300s
cloud-01 instance=cloud-i37 stage=cloud lease=300s
fog-01   instance=fog-i37   stage=fog   lease=3s
```

The intentionally short fog lease demonstrated:

```text
registration -> active
wait beyond lease -> stale
heartbeat from same instance -> active
registered_at_utc unchanged=True
last_seen_at_utc changed=True
```

Fail-closed gates:

```text
heartbeat instance_id=IMPOSTOR -> HTTP 409
path fog-01 + payload cloud-01 -> HTTP 400
```

Aggregate registration returned `count=3`; because the fog lease is only three seconds, snapshots legitimately alternate between `active_count=3/stale_count=0` immediately after refresh and `active_count=2/stale_count=1` after fog expiry. Active/stale is therefore a live time-derived status rather than a sticky flag.

Materialized views explicitly state:

```text
deployment_side_effects_performed=false
authenticated_identity_attested=false
```

I37 lease state remains intentionally in-memory so restart cannot falsely preserve an active presence lease. Registration is logical presence tracking only, not cryptographic host attestation.

Evidence:

- `docs/I37_SENTINEL_REGISTRATION_LEASE_PLAN.md`
- `docs/I37_SENTINEL_REGISTRATION_LEASE_FINAL.md`

## I38 — validated at 144/150

I38 adds read-only inbox discovery to the Sentinel-bound control-plane path. The explicit selector `--assignment inbox` fetches only the node-scoped deployment for configured `agent.node_id`, validates the returned slice, and classifies the result as `no_work`, `one_assignment`, or `multiple_assignments` without transitioning into artifact or execution behavior.

Regression:

```text
DATUM fmt: PASS
DATUM test: 426 passed; 0 failed
DATUM build: PASS
```

Live discovery evidence:

```text
no active manifest:
  local-01 -> no_work -> assignment_count=0

active i34-live-project manifest:
  local-01 -> one_assignment -> local-aggregate
  fog-01   -> one_assignment -> fog-scale
  cloud-01 -> one_assignment -> cloud-delta
  probe-01 -> no_work -> empty slice
```

Every discovery result retained:

```text
control_plane_fetch_performed=true
artifact_acquisition_performed=false
wasmtime_compiled=false
guest_execution_performed=false
report_published=false
governed_execution_performed=false
deployment_side_effects_performed=false
```

The artifact repository at `192.168.10.101:8090` was deliberately unavailable and discovery still succeeded. Controlled live-report count remained `0 -> 0`, proving the inbox path did not create execution evidence.

Evidence:

- `docs/I38_SENTINEL_DEPLOYMENT_INBOX.md`
- `docs/I38_SENTINEL_DEPLOYMENT_INBOX_FINAL.md`

## Remaining path to 150/150

The fixed roadmap is recorded in `docs/CONTROLLED_WASM_DEPLOYMENT_142_TO_150_ROADMAP.md`.

```text
I39/145 explicit multi-assignment scheduling
I40/146 dependency-aware deployment stages
I41/147 deployment lifecycle and retry evidence
I42/148 resource-aware observed D-Graph join
I43/149 deployment readiness and governance evidence
I44/150 phase-final integration and reproducibility package
```

The critical path intentionally excludes Kubernetes, Ansible, AWS, and canonical automatic governed execution.

## Next

I39/145 removes I35's deliberately narrow exactly-one-assignment execution restriction through an explicit batch scheduling mode. Multiple assignments must be ordered deterministically, remain node-scoped, produce per-assignment reports/evidence, and require an explicit operator transition. No automatic governed execution is enabled.