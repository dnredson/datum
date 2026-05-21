# 2-VM D-Graph Metadata Smoke Draft

## Purpose

`scripts/smoke-dgraph-2vm-metadata-only.ps1` is a parameterized draft smoke script for the first distributed `DATUM`/`dserver` d-graph test.

It is intended to validate a metadata-only path across two node identities and one server:

- publish d-graph snapshots from two `DATUM` configs
- read latest node snapshots back from `dserver`
- read the project latest snapshot list
- read the materialized project d-graph summary
- read the planner dry-run summary
- confirm `side_effects_performed=false`

It does not execute planner decisions, deploy modules, call Docker, or reconfigure flows.

Related files:

- example templates:
  - `scripts/templates/datum-config-fog-01.example.toml`
  - `scripts/templates/datum-config-fog-02.example.toml`
- live checklist:
  - `docs/DGRAPH_2VM_LIVE_PREFLIGHT_CHECKLIST.md`

## Topology Assumptions

The draft assumes:

- one reachable `dserver`
- one shared `project_id`
- two `DATUM` config files with different `node_id` values
- one config for `fog-01`
- one config for `fog-02`

Typical first distributed setup:

- `VM1`: `dserver`
- `VM2`: `DATUM fog-02`
- optional `DATUM fog-01` on `VM1` or another reachable machine

## Required Inputs

The script accepts:

- `ServerUrl`
- `ProjectId`
- `DatumDir`
- `Node1Id`
- `Node1Config`
- `Node2Id`
- `Node2Config`
- `FixtureScenario` (optional)

Default identities are:

- `Node1Id=fog-01`
- `Node2Id=fog-02`
- `ProjectId=smart-city-campinas`

The server must already be running and reachable at `ServerUrl` for live execution.

## Example

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\smoke-dgraph-2vm-metadata-only.ps1 `
  -ServerUrl "http://<dserver-ip>:8080" `
  -ProjectId "smart-city-campinas" `
  -DatumDir ".\DATUM" `
  -Node1Id "fog-01" `
  -Node1Config ".\DATUM\config-fog-01.toml" `
  -Node2Id "fog-02" `
  -Node2Config ".\DATUM\config-fog-02.toml"
```

## Template Copy Workflow

Use the example templates as a starting point only.

1. Copy `scripts/templates/datum-config-fog-01.example.toml` to an untracked local file such as `DATUM/config-fog-01.local.toml`.
2. Copy `scripts/templates/datum-config-fog-02.example.toml` to an untracked local file such as `DATUM/config-fog-02.local.toml`.
3. Edit `node_id`, `project_id`, server URLs, and any local container or service names.
4. Keep those real environment files out of Git.

The templates are sanitized examples only. They must not contain credentials, tokens, service-account paths, or other environment secrets.

## Draft Validation Mode

Use `-AllowMissingConfigs` when the distributed environment is not ready yet.

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\smoke-dgraph-2vm-metadata-only.ps1 -AllowMissingConfigs
```

If one or both config paths are missing, the script exits successfully after a documentation/preflight message:

- no `DATUM` commands are run
- no server checks are run
- the script does not claim that a live distributed smoke passed

This mode exists so the draft can be syntax-validated safely in the repo before live `2-VM` execution is available.

## Optional Switches

- `-SkipPublish`: skip `DATUM dgraph-snapshot --publish` and run server read-back only
- `-SkipDatumCommands`: skip all `DATUM` commands and run server-side checks only
- `-AllowMissingConfigs`: permit documentation/preflight exit when config files are not present
- `-FixtureScenario <scenario>`: append `--fixture <scenario>` to both publish commands for metadata-only degraded or unavailable simulations

## What The Script Does

- publishes metadata-only d-graph snapshots
- reads latest node snapshots
- reads project latest snapshots
- reads materialized graph summary
- reads planner dry-run summary
- runs `DATUM` materialized/planner summary viewers when not skipped
- checks `side_effects_performed=false`
- can optionally publish fixture scenarios such as `degraded-node` or `unavailable-node` when `-FixtureScenario` is provided

## What The Script Does Not Do

- execute planner decisions
- execute deployment
- execute installation
- call `dispatch --execute`
- call Docker
- perform Docker runtime probing
- load external WASM
- execute registered WASM modules
- run migrations
- transfer artifacts
- reconfigure flows
- write active config
- modify local mappings

## Notes

- The script is intentionally conservative. It treats planner output as advisory/read-only only.
- Snapshot and planner results may be sparse, empty, blocked, or partially blocked depending on config completeness.
- Fixture scenarios are useful for pre-live degraded or unavailable node simulations and remain metadata-only.
- The live `2-VM` environment still needs real `fog-01` and `fog-02` configs before the draft becomes an operational smoke run.
- The live preflight checklist in `docs/DGRAPH_2VM_LIVE_PREFLIGHT_CHECKLIST.md` should be completed before the first required real distributed run.
