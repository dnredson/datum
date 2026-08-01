# I32 — Live Three-Node D-Graph Smoke

`smoke-dgraph-3node-live.ps1` is the repeatable, fail-fast Windows operator
script for the validated metadata-only continuum:

```text
fog-01/chirpstack-primary -> fog-02/mqtt-adapter -> cloud-01/magistrala-core
```

It deliberately performs no recovery, dispatch, deployment, installation, Docker,
WASM, migration, planner-decision execution, or flow reconfiguration. The only
writes are explicit reporting-only D-Graph snapshot publications to `dserver`.

## Prerequisites

- Run from the `PD` repository root with PowerShell 7+ (`pwsh`). The script
  preflight rejects Windows PowerShell 5.1.
- `dserver` is running and reachable at `ServerUrl`, with its configured backend available.
- `cargo` is available locally, and `DATUM/Cargo.toml` is present.
- Create an untracked local `cloud-01` config from
  `scripts/templates/datum-config-cloud-01.example.toml`. This checkout currently
  has no operator-local `cloud-01` config, so use its real local path for
  `-CloudConfig`; do not refer to `192.168.10.106`.
- The three local configs for `fog-01`, `fog-02`, and `cloud-01` declare their
  respective node IDs. The local `fog-02` config must already point
  at a current persisted DATUM snapshot containing its service observation; the
  script never runs a collection cycle to create or refresh it.
- `DATUM/snapshot-fog-02.json` exists for the read-only observed-health step.
- All three configs use the same `project_id`, point to the same `dserver`, and
  retain disabled automatic recovery/assisted execution settings.

The command passes `-ServerUrl` and `-ProjectId` to every D-Graph publication as
the CLI `--url` and `--project-id` overrides. Those requested values are therefore
authoritative for baseline, degraded-flow, observed-health, and restoration
publication; a stale local `outputs.http` destination cannot redirect the smoke.

Do not put credentials, addresses, service-account paths, or generated snapshots in
the command or Git. `.gitignore` excludes the expected local config and smoke-output
patterns.

## Exact command

Set `-CloudConfig` to the real local `cloud-01` configuration path. `cloud-01` is a
third logical metadata-only node executed locally on Windows; it has no SSH target,
remote DATUM directory, or separate physical-machine requirement.

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke-dgraph-3node-live.ps1 `
  -ServerUrl http://192.168.10.101:8080 `
  -ProjectId smart-city-campinas `
  -Fog01Config .\DATUM\config-fog-01.local.toml `
  -Fog02Config .\DATUM\config-fog-02.local.toml `
  -CloudConfig .\DATUM\config-cloud-01.local.toml
```

## Expected results

The script publishes baseline fixtures for all three logical nodes, verifies each
latest snapshot, and asserts a materialized baseline of:

- 3 source snapshots, nodes, runtimes, and modules;
- 2 flows and 14 relationships;
- 3 unknown targets, no unavailable or unhealthy targets, and no validation errors;
- readiness `ready`, zero warnings/blockers/degraded flows;
- `safe_to_auto_execute=false`, `side_effects_performed=false`, and
  `metadata.execution_performed=false`.

It then publishes only the `fog-02` `degraded-flow` fixture and requires
`ready_with_warnings`, exactly one warning/degraded flow, zero blockers, and a
dry-run planner with no side effects. Next it reads and explicitly publishes
`--observed-health` from the persisted `fog-02` snapshot, asserting
`checks_executed=false` for every health entry and both safety flags false.

## Live validation status

I32 passed live validation on Windows `192.168.10.101`: all three logical nodes
were published from local configs, the advisory degraded-flow and read-only
observed-health steps passed, and mandatory baseline restoration returned the graph
to 3 snapshots/nodes/runtimes/modules, 2 flows, 14 relationships, 3 unknown targets,
zero unhealthy or unavailable targets, and readiness `ready`. No SSH or use of
`192.168.10.106` occurred. See
[the validation record](../docs/LIVE_I32_REPEATABLE_3NODE_SMOKE_VALIDATION.md).

## Safety and restoration

The script enters a `try/finally` after baseline publication starts. Its `finally`
block always republishes the `fog-02` baseline, including after a degraded-flow or
observed-health failure, then rechecks the complete restored baseline. A restoration
failure is reported prominently and makes the command fail. This is intentionally
fail-fast: any unmet local file, config, endpoint, schema, count, or safety-flag
precondition stops the run.

If the command is interrupted before PowerShell can run `finally` (for example a
machine power loss), manually restore from the repository root:

```powershell
Set-Location .\DATUM
cargo run --quiet -- dgraph-snapshot --config .\config-fog-02.local.toml --fixture baseline --publish
```

Then rerun the script from the beginning; it verifies the final restored state rather
than treating a publication as sufficient evidence.

## Offline contract test

This test parses the script and verifies the required parameters, safety contract,
baseline restoration branch, degraded-flow scenario, observed-health path, and count
assertions without contacting a server:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\tests\smoke-dgraph-3node-live-contract.tests.ps1
```
