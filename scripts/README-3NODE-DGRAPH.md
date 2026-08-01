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

- Run from the `PD` repository root on Windows PowerShell 7+.
- `dserver` is running and reachable at `ServerUrl`, with its configured backend available.
- `cargo` is available locally and on `cloud-01`.
- OpenSSH client is available locally and non-interactive SSH to `cloud-01` works
  (key-based authentication; no password prompt).
- Create untracked local configs from the templates in `scripts/templates/` for
  `fog-01`, `fog-02`, and `cloud-01`. The local `fog-02` config must already point
  at a current persisted DATUM snapshot containing its service observation; the
  script never runs a collection cycle to create or refresh it.
- All three configs use the same `project_id`, point to the same `dserver`, and
  retain disabled automatic recovery/assisted execution settings.

Do not put credentials, addresses, service-account paths, or generated snapshots in
the command or Git. `.gitignore` excludes the expected local config and smoke-output
patterns.

## Exact command

Replace only the three cloud connection/path values with your local values. The cloud
paths must be absolute POSIX paths on the remote host.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke-dgraph-3node-live.ps1 `
  -ServerUrl http://192.168.10.101:8080 `
  -ProjectId smart-city-campinas `
  -Fog01Config .\DATUM\config-fog-01.local.toml `
  -Fog02Config .\DATUM\config-fog-02.local.toml `
  -CloudSshTarget operator@192.168.10.106 `
  -CloudDatumDir /opt/DATUM `
  -CloudConfig /opt/DATUM/config-cloud-01.local.toml
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

## Safety and restoration

The script enters a `try/finally` after baseline publication starts. Its `finally`
block always republishes the `fog-02` baseline, including after a degraded-flow or
observed-health failure, then rechecks the complete restored baseline. A restoration
failure is reported prominently and makes the command fail. This is intentionally
fail-fast: any unmet endpoint, schema, count, safety flag, or remote-command
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
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\tests\smoke-dgraph-3node-live-contract.tests.ps1
```
