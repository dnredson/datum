# C42/130 — Live Three-Node Metadata-Only Validation

## Purpose

This record documents the successful live validation of the three-node logical continuum prepared in `C41/130`:

```text
fog-01/chirpstack-primary -> fog-02/mqtt-adapter -> cloud-01/magistrala-core
```

The validation remained metadata-only, fixture/reporting-only, advisory, and read-only. No deployment, dispatch, reconfiguration, Docker, or WASM execution was performed.

## Environment

- Repository base: `95a2de9`
- Project: `smart-city-campinas`
- Windows `192.168.10.101`: `dserver`, DATUM node `fog-01`, and DATUM node `fog-02`
- Ubuntu VM `192.168.10.106`: DATUM node `cloud-01`

The two Windows DATUM configs used distinct logical node identities and published independent latest snapshots. Local C42 configs were kept untracked.

## Local Fixture Preflight

Each logical node generated its baseline fixture successfully before publication:

- `fog-01`: module `chirpstack-primary` and flow `flow-chirpstack-to-mqtt-adapter`
- `fog-02`: module `mqtt-adapter` and flow `flow-mqtt-adapter-to-magistrala-core`
- `cloud-01`: module `magistrala-core` and no locally declared flow

Every local fixture reported:

- `safe_to_auto_execute=false`
- `side_effects_performed=false`

## Baseline Publication And Read-Back

The three baseline fixtures were explicitly published and read back through `dserver`.

Observed latest-snapshot state:

- `count=3`
- nodes: `cloud-01`, `fog-01`, and `fog-02`
- the cloud-bound flow was declared only by `fog-02`
- all persisted snapshots preserved `safe_to_auto_execute=false`
- all persisted snapshots preserved `side_effects_performed=false`

## Baseline Materialization

The live baseline materialized as:

- `source_snapshot_count=3`
- `node_count=3`
- `runtime_count=3`
- `module_count=3`
- `artifact_count=0`
- `flow_count=2`
- `relationship_count=14`
- `unhealthy_targets=3`
- `validation_error_count=0`
- `side_effects_performed=false`

The `unhealthy_targets=3` result represents the three metadata-only modules whose operational health remains `unknown`. It does not represent unhealthy nodes; the planner reported `unhealthy_node_count=0`.

## Baseline Planner And Readiness

The planner summary reported:

- `status=planned`
- `placement_count=3`
- `reconfiguration_count=0`
- `blocked_count=0`
- `warning_count=0`
- `candidate_node_count=3`
- `unhealthy_node_count=0`
- `degraded_flow_count=0`
- `side_effects_performed=false`

The consolidated readiness result reported:

- `readiness_status=ready`
- `source_snapshot_count=3`
- `node_count=3`
- `runtime_count=3`
- `module_count=3`
- `flow_count=2`
- `planner_status=planned`
- `warning_count=0`
- `blocked_count=0`
- `degraded_flow_count=0`
- `side_effects_performed=false`
- `execution_performed=false`

Readiness checks confirmed snapshots, multiple nodes, node-scoped runtimes, modules, flow metadata, planner output, no materialization errors, no warnings, no blockers, no degraded flows, and no side effects.

## Degraded-Flow Advisory Validation

Only `fog-02` was replaced with the `degraded-flow` fixture. The resulting live readiness state was:

- `readiness_status=ready_with_warnings`
- `source_snapshot_count=3`
- `node_count=3`
- `runtime_count=3`
- `module_count=3`
- `flow_count=2`
- `relationship_count=15`
- `planner_status=planned`
- `placement_count=3`
- `reconfiguration_count=0`
- `warning_count=1`
- `blocked_count=0`
- `degraded_flow_count=1`
- `candidate_node_count=3`
- `unhealthy_node_count=0`
- `side_effects_performed=false`
- `execution_performed=false`

The full planner dry-run returned the deterministic advisory:

- `warning_id=warning:flow:flow-mqtt-adapter-to-magistrala-core:degraded`
- `target_kind=flow`
- `target_id=flow-mqtt-adapter-to-magistrala-core`
- `kind=degraded_flow`
- `severity=warning`
- `side_effects_performed=false`
- `execution_performed=false`

Its reason states that the flow is degraded and that the metadata-only dry-run remains advisory without executing reconfiguration.

## Baseline Restoration

The `fog-02` baseline fixture was republished after the degraded-flow check.

The final persisted state returned to:

- `readiness_status=ready`
- `source_snapshot_count=3`
- `node_count=3`
- `runtime_count=3`
- `module_count=3`
- `flow_count=2`
- `relationship_count=14`
- `planner_status=planned`
- `placement_count=3`
- `warning_count=0`
- `blocked_count=0`
- `degraded_flow_count=0`
- `side_effects_performed=false`
- `execution_performed=false`

## What Was Proven

- Three logical DATUM nodes can publish independent metadata-only snapshots across two physical hosts.
- The multi-hop logical chain materializes with three node-scoped runtimes, three canonical modules, and two cross-node flows.
- The planner produces one advisory placement per candidate node without executing placement.
- Degrading the cloud-bound flow produces exactly one structured advisory warning.
- The degraded-flow scenario does not create a reconfiguration proposal or blocker.
- Restoring the baseline removes the warning and returns readiness to `ready`.
- Safety and execution flags remain false throughout the complete sequence.

## Safety Confirmation

- No real telemetry ingestion or sensor data publication was performed.
- No dispatch, deployment, installation, migration, or planner decision execution was performed.
- No Docker/container runtime side effects were performed.
- No WASM artifact loading or execution was performed.
- No automatic flow rewiring or reconfiguration was performed.
- No committed configuration or local mapping was mutated.
- `side_effects_performed=false` was preserved.
- `execution_performed=false` was preserved.

## Known Limitations

1. Module health remains `unknown` in the metadata-only baseline, so `unhealthy_targets=3` is expected.
2. Flow and module observations remain fixture-based rather than derived from live services or telemetry.
3. The three logical nodes were distributed over two physical hosts; this was not a three-physical-host run.
4. The planner remains advisory and intentionally does not execute placement or reconfiguration.
5. Seven pre-existing Rust dead-code warnings remain visible during DATUM `cargo run`.

## Recommended Follow-Ups

1. Add read-only observation-derived module health while distinguishing `unknown` from genuinely unhealthy state.
2. Add a repeatable multi-host three-node metadata-only smoke script.
3. Enrich planner advisory proposals without enabling execution.
4. Defer any executed placement, recovery, migration, or reconfiguration until a separate safety contract is designed and validated.
