# Live 2-VM D-Graph Runtime Identity Validation

## Purpose

This record documents the successful live validation of node-scoped runtime identity in the materialized d-graph after `C37/110`.

## Problem

Local snapshots may use the same local runtime ID on different nodes. Before C37, materialization collapsed matching runtime IDs across nodes, producing `runtime_count=1` despite two nodes. In a distributed graph, local runtimes must be scoped by node.

## C37 Behavior

Materialized runtime IDs are now scoped by project and node. The original local ID is retained as `metadata.local_runtime_id`. Raw/latest snapshots remain unchanged; only materialized d-graph identity changes. Runtime relationships and runtime-health references use the scoped identity consistently.

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

`GET /api/v1/dgraph/materialized/summary` reported:

- `source_snapshot_count=2`
- `node_count=2`
- `runtime_count=2`
- `module_count=2`
- `relationship_count=4`
- `unhealthy_targets=2`
- `validation_errors=[]`
- `side_effects_performed=false`

`GET /api/v1/dgraph/planner/dry-run/summary` reported `status=planned`, `candidate_node_count=2`, and `unhealthy_node_count=0`.

## Unavailable-Node Validation

`fog-01` published the unavailable-node fixture:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture unavailable-node --publish
```

The materialized graph retained `runtime_count=2`; `relationship_count=5` and `unhealthy_targets=3`. The planner reported `candidate_node_count=1` and `unhealthy_node_count=1`. `side_effects_performed=false` remained preserved.

## Planner Semantics Observation

Earlier pre-C37/manual unavailable-node testing returned `blocked_count=2` and `warning_count=2`. The post-C37 live run returned `blocked_count=0` and `warning_count=0` while still reducing `candidate_node_count` and reporting `unhealthy_node_count`. This is a follow-up planner-semantics observation, not a runtime identity regression.

## Safety Confirmation

- No dispatch execution was performed.
- No deployment execution was performed.
- No installation execution was performed.
- No Docker/container runtime side effects were performed.
- No WASM artifact loading or execution was performed.
- No migration was performed.
- No flow reconfiguration was performed.
- No planner decision was executed.
- `side_effects_performed=false` was preserved.

## Follow-Ups

C38 adds metadata-only flow fixtures for the advisory, read-only logical flow `fog-01/chirpstack-primary -> fog-02/mqtt-adapter`. It does not publish real telemetry or execute reconfiguration.

After the runtime-identity validation, C38 confirmed flow materialization over the node-scoped runtimes.

1. **Planner semantics:** Clarify whether unavailable nodes should produce `blocked_count`/`warning_count` or only reduce `candidate_node_count`.
2. **Baseline health:** Baseline still has `unhealthy_targets=2` because module health remains unknown in metadata-only mode.
