# Live 2-VM D-Graph Flow Fixture Validation

## Purpose

This record documents the successful live validation of the metadata-only flow fixtures added in `C38/110`.

## Flow Modeled

The fixtures model one cross-node logical flow:

```text
fog-01/chirpstack-primary -> fog-02/mqtt-adapter
```

The flow protocol/kind is MQTT. It is fixture-only and reporting-only: no real telemetry is published or ingested.

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

Observed baseline materialization/planner metrics:

- `node_count=2`
- `runtime_count=2`
- `module_count=2`
- `flow_count=1`
- `relationship_count=8`
- `unhealthy_targets=2`
- `degraded_flow_count=0`
- `candidate_node_count=2`
- `side_effects_performed=false`

## Degraded-Flow Validation

`fog-01` published the degraded-flow fixture:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture degraded-flow --publish
```

The local output reported `modules=2`, `flows=1`, and `health entries=2`. The materialized `flow_count` remained `1`; `relationship_count` became `9` and `unhealthy_targets` became `3`. The planner reported `degraded_flow_count=1`. `side_effects_performed=false` remained preserved.

## Normalization

`fog-01` was restored to the baseline fixture:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture baseline --publish
```

After normalization, `degraded_flow_count` returned to `0`.

## Planner Semantics Observation

The planner correctly counts `degraded_flow_count=1`, but does not currently emit `warning_count` or `blocked_count` for degraded flows. This is a follow-up semantics item, not a C38 failure. Candidate follow-up: `C39/110` Flow-aware planner advisory semantics.

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

1. **C39/110 candidate — flow-aware planner advisory semantics.** Clarify whether `degraded_flow_count` should produce warnings, advisory reconfiguration candidates, and whether planner status should remain `planned` or become `degraded`/`partially_blocked`.
2. **3-VM preparation.** Richer flow fixtures can now support 3-node scenarios.
3. **Baseline unknown module health.** `unhealthy_targets=2` remains expected in metadata-only mode because module health is unknown.
