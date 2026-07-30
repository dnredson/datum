# Live 2-VM D-Graph Flow Advisory Validation

## Purpose

This record documents the successful live validation of `C39/110` flow-aware planner advisories.

## C39 Behavior

- Degraded flows are counted by `degraded_flow_count`.
- Degraded flows also emit deterministic structured advisory warnings.
- `warning_count` reflects degraded flows.
- `status=planned` remains valid when a dry-run plan is available.
- No execution or flow reconfiguration is performed.

## Live Environment

- Windows `192.168.10.101`: `dserver` and DATUM node `fog-01`
- Ubuntu VM `192.168.10.106`: DATUM node `fog-02`
- Project: `smart-city-campinas`

## Baseline Validation

Windows / `fog-01`:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture baseline --publish
```

Ubuntu / `fog-02`:

```text
cargo run -- dgraph-snapshot --config config-fog-02.local.toml --fixture baseline --publish
```

Observed planner/materialization metrics included:

- `flow_count=1`
- `warning_count=0`
- `degraded_flow_count=0`
- `candidate_node_count=2`
- `status=planned`
- `side_effects_performed=false`

## Degraded-Flow Validation

`fog-01` published the degraded-flow fixture:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture degraded-flow --publish
```

The planner reported `warning_count=1` and `degraded_flow_count=1`. It remained `status=planned`, and `side_effects_performed=false` was preserved.

## Structured Warning Validation

The planner dry-run returned the deterministic advisory warning:

- `warning_id=warning:flow:flow-chirpstack-to-mqtt-adapter:degraded`
- `metadata.kind=degraded_flow`
- `metadata.severity=warning`
- `metadata.target_kind=flow`
- `metadata.target_id=flow-chirpstack-to-mqtt-adapter`
- `metadata.execution_performed=false`
- `metadata.side_effects_performed=false`

Its reason stated that the flow is degraded and that the metadata-only dry-run remains advisory without executing reconfiguration.

## Normalization

`fog-01` was restored to the baseline fixture:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture baseline --publish
```

After normalization, `warning_count=0` and `degraded_flow_count=0`.

## Safety Confirmation

- No real telemetry ingestion was performed.
- No sensor data publication was performed.
- No dispatch execution was performed.
- No deployment execution was performed.
- No installation execution was performed.
- No Docker/container runtime side effects were performed.
- No WASM artifact loading or execution was performed.
- No migration was performed.
- No flow reconfiguration execution was performed.
- No planner decision was executed.
- `side_effects_performed=false` was preserved.

## Follow-Ups

1. `C40/110` candidate: prepare a final live validation checkpoint for the metadata-only D-Graph pipeline.
2. Future planner refinement: optionally convert degraded-flow warnings into advisory reconfiguration proposals, still without execution.
3. Future 3VM scenario: extend fixtures to represent a multi-hop flow across mist/fog/cloud.

## C40 Readiness Summary

`C40/110` adds `GET /api/v1/dgraph/readiness/summary`, a consolidated read-only view of latest snapshots, materialized graph state, planner dry-run state, validation errors, and safety/readiness checks. It remains metadata-only: it does not execute deployment, dispatch, or flow reconfiguration.
