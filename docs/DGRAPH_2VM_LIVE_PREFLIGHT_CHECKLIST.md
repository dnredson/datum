# D-Graph 2-VM Live Preflight Checklist

## Purpose

This checklist prepares the first live `2-VM` metadata-only distributed d-graph run without requiring planner decision execution, deployment, or flow reconfiguration.

Use it together with:

- [D-Graph 2-VM Distributed Test Plan](./DGRAPH_2VM_DISTRIBUTED_TEST_PLAN.md)
- [Distributed Dry-Run Preparation Checkpoint](./DISTRIBUTED_DRY_RUN_PREPARATION_CHECKPOINT.md)
- [2-VM D-Graph Metadata Smoke Draft](../scripts/README-2VM-DGRAPH.md)

> `I28/110` records a successful live 2-VM metadata-only direct publish smoke test. Direct DATUM `--publish` works after `C36/110`; manual JSON patching is no longer needed.

## A. Environment Assumptions

- one reachable `dserver`
- Firestore emulator or configured backend available for `dserver`
- two `DATUM` nodes, or two `DATUM` configs representing `fog-01` and `fog-02`
- same `project_id` on both nodes
- unique `node_id` on each node
- `dserver` URL reachable from both nodes

## B. Required Files

- `fog-01` config path
- `fog-02` config path
- optional fixture scenario such as `baseline` or `unavailable-node`
- smoke script path: `scripts/smoke-dgraph-2vm-metadata-only.ps1`
- example templates:
  - `scripts/templates/datum-config-fog-01.example.toml`
  - `scripts/templates/datum-config-fog-02.example.toml`

## C. Manual Preflight Commands On Each DATUM Node

Run on `fog-01`:

```powershell
cargo run -- dgraph-snapshot --config <fog-01-config> --json
cargo run -- dgraph-snapshot --config <fog-01-config> --fixture baseline --json
cargo run -- dgraph-snapshot --config <fog-01-config> --publish
```

Run on `fog-02`:

```powershell
cargo run -- dgraph-snapshot --config <fog-02-config> --json
cargo run -- dgraph-snapshot --config <fog-02-config> --fixture baseline --json
cargo run -- dgraph-snapshot --config <fog-02-config> --publish
```

Expected:

- clean JSON for local snapshot checks
- `safe_to_auto_execute=false`
- `side_effects_performed=false`
- explicit publish succeeds without executing anything

## D. Server Read-Back Checks

Check the latest per-node snapshots:

```powershell
curl -H "X-Datum-Project: <project-id>" http://<dserver-ip>:8080/api/v1/nodes/fog-01/dgraph-snapshot/latest
curl -H "X-Datum-Project: <project-id>" http://<dserver-ip>:8080/api/v1/nodes/fog-02/dgraph-snapshot/latest
```

Check the project snapshot list and read-only graph views:

```powershell
curl -H "X-Datum-Project: <project-id>" http://<dserver-ip>:8080/api/v1/dgraph-snapshots/latest
curl -H "X-Datum-Project: <project-id>" http://<dserver-ip>:8080/api/v1/dgraph/materialized/summary
curl -H "X-Datum-Project: <project-id>" http://<dserver-ip>:8080/api/v1/dgraph/planner/dry-run/summary
```

## E. Script Command

Basic live run:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\smoke-dgraph-2vm-metadata-only.ps1 `
  -ServerUrl "http://<dserver-ip>:8080" `
  -ProjectId "smart-city-campinas" `
  -DatumDir ".\DATUM" `
  -Node1Id "fog-01" `
  -Node1Config ".\scripts\templates\datum-config-fog-01.example.toml" `
  -Node2Id "fog-02" `
  -Node2Config ".\scripts\templates\datum-config-fog-02.example.toml"
```

Fixture-assisted live run:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\smoke-dgraph-2vm-metadata-only.ps1 `
  -ServerUrl "http://<dserver-ip>:8080" `
  -ProjectId "smart-city-campinas" `
  -DatumDir ".\DATUM" `
  -Node1Id "fog-01" `
  -Node1Config ".\scripts\templates\datum-config-fog-01.example.toml" `
  -Node2Id "fog-02" `
  -Node2Config ".\scripts\templates\datum-config-fog-02.example.toml" `
  -FixtureScenario "unavailable-node"
```

Before using those commands in a real environment, copy the example templates to untracked local config files and edit:

- `node_id`
- `project_id`
- `outputs.http.url`
- `dispatch.url`
- any local container names or module-like service entries

Do not commit real environment configs.

## F. Expected Success Criteria

- both node latest snapshots are readable
- project latest snapshot count is `>= 2`
- materialized graph `source_snapshot_count >= 2`
- planner summary returns clean JSON
- `side_effects_performed=false` everywhere
- no deployment or reconfiguration execution occurs

## G. Fatal Failure Criteria

- `side_effects_performed=true`
- `dispatch --execute` used
- Docker side effect observed
- config modified unexpectedly
- artifact downloaded
- planner decision executed
- flow reconfiguration performed
