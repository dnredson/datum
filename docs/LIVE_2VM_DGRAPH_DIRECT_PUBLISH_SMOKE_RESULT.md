# Live 2-VM D-Graph Direct Publish Smoke Result

## Purpose

This record captures the first successful live two-node, metadata-only distributed d-graph smoke test after `C36/110` corrected the direct publish contract.

## Environment

- Project: `smart-city-campinas`
- Windows host: `192.168.10.101` (`dserver` and DATUM node `fog-01`)
- Ubuntu VM: `192.168.10.106` (DATUM node `fog-02`)

## Commands Executed

Windows / `fog-01` baseline:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture baseline --publish
```

Ubuntu / `fog-02` baseline:

```text
cargo run -- dgraph-snapshot --config config-fog-02.local.toml --fixture baseline --publish
```

Windows / `fog-01` unavailable-node simulation:

```text
cargo run -- dgraph-snapshot --config config-fog-01.local.toml --fixture unavailable-node --publish
```

Read-only checks:

```text
GET /api/v1/dgraph-snapshots/latest
GET /api/v1/dgraph/materialized/summary
GET /api/v1/dgraph/planner/dry-run/summary
```

## Baseline Result

Direct publish succeeded from both nodes. The latest baseline snapshots identified `chirpstack-primary` on `fog-01` and `mqtt-adapter` on `fog-02`, with `fixture_scenario=baseline`, `safe_to_auto_execute=false`, and `side_effects_performed=false`.

- snapshot count: `2`
- node count: `2`
- module count: `2`
- relationship count: `4`
- placement count: `2`
- candidate node count: `2`
- planner status: `planned`
- side effects: `false`

## Failure Simulation Result

After publishing the `unavailable-node` fixture from `fog-01`:

- relationship count: `5`
- unhealthy targets: `3`
- planner status: `partially_blocked`
- blocked count: `2`
- warning count: `2`
- candidate node count: `1`
- unhealthy node count: `1`
- side effects: `false`

## Interpretation

The distributed metadata-only pipeline is working: `dserver` received snapshots from both Windows and Ubuntu, and the materialized d-graph used both latest snapshots. The planner observed the unavailable node and reduced its candidate set accordingly. The planner remains advisory and read-only.

## Safety Confirmation

- No dispatch, deployment, or installation execution was performed.
- No Docker/container runtime side effects were performed.
- No WASM artifact loading or execution was performed.
- No migration or flow reconfiguration was performed.
- No planner decision was executed.
- `side_effects_performed=false` was preserved throughout.

## Issues and Follow-Ups

1. **Runtime identity.** `runtime_count=1` despite two nodes because both fixtures use `runtime-container-local`. `C37/110` addresses this observation by node-scoping runtime identity during materialization.
2. **Module health.** Baseline reported `unhealthy_targets=2` because metadata-only modules have `observed_state=unknown`. This is expected for fixture/config-only snapshots and should remain documented.
3. **Flow metadata.** `flow_count=0` because current templates do not include richer flow metadata. Add richer flow metadata before 3-VM tests.

## C37 Validation Status

The C37 regression tests validate that matching local runtime IDs from `fog-01` and `fog-02` materialize as two canonical node-scoped runtime entities. A follow-up live check is deferred: the locally running `dserver` holds its executable lock, so it was not stopped or restarted for this documentation and materialization-only change.

## C37 Live Validation Update

C37 was subsequently live validated. The two-node materialized graph now reports `runtime_count=2`; runtime entities are node-scoped during materialization.

## C38 Flow Fixture Update

C38 introduces metadata-only flow fixtures. The two-node fixture can now represent the advisory, read-only logical flow `fog-01/chirpstack-primary -> fog-02/mqtt-adapter`; it neither publishes real telemetry nor executes reconfiguration.

## C38 Live Flow Validation Update

C38 was subsequently live validated. The 2-node baseline now materializes one metadata-only cross-node flow (`flow_count=1`).
