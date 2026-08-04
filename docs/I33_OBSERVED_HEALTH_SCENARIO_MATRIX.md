# I33/130 — Observed-Health Scenario Matrix

## Status

The repeatable I33 live-validation script and its offline contract test are prepared.
Live execution against the three-node environment is still required before I33 can be
closed and the project checkpoint can advance beyond `116/130`.

The first live run published the initial baseline, then failed in the first
`Assert-Baseline` because the smoke incorrectly read materialized graph counts from the
top-level response. It did not enter the scenario matrix. Its emergency restoration
published the `fog-02` baseline. This run is not approval evidence for I33.

The second live run published the initial baseline and validated
`materialized/summary`; `readiness/summary` also returned valid JSON. The initial
baseline did not complete because its obsolete `safe_to_auto_execute` assertion was
not part of the readiness contract, so no matrix scenario was started. The emergency
restoration republished `fog-02`, and its materialized validation passed, but its full
validation did not finish after reaching the same readiness assertion. This run is also
not approval evidence for I33.

## Objective

I33 expands the live evidence for C44's read-only persisted-health translation. It
exercises six cases for `fog-02/mqtt-adapter`:

| Scenario | Persisted observation | Expected canonical health |
| --- | --- | --- |
| `healthy` | fresh `available` | `healthy` |
| `degraded` | fresh `warning` | `degraded` |
| `stale` | `unavailable`, older than `ttl_s` | `unknown` |
| `absent` | no matching service entity | `unknown` |
| `unrecognized` | fresh `mystery-state` | `unknown` |
| `future-skewed` | `available`, more than five seconds ahead | `unknown` |

The matrix distinguishes explicit current health from missing or unreliable evidence.
It never presents stale, absent, unrecognized, or future-skewed data as current failure
or availability.

## Source preservation

The script reads the operator's existing `fog-02` config and persisted snapshot, then
creates scenario-specific copies under the ignored `scripts/.smoke-output/` directory.
Only the copies are edited. SHA-256 hashes of the source config and source snapshot are
checked before and after the matrix. Temporary evidence is removed after success unless
`-KeepEvidence` is supplied.

Each temporary config keeps the operator's normal settings but redirects
`outputs.file_json.path` to its scenario snapshot copy. Every publication still receives
the command-line `--url` and `--project-id` overrides, so a stale local HTTP destination
cannot redirect metadata to a different server or project.

## Restoration contract

The script first establishes the three-node baseline. Every scenario then runs inside
its own `try/finally` block. The `finally` block republishes the `fog-02` baseline and
requires the complete restored state:

```text
source_snapshot_count=3                 (top-level)
summary.node_count=3
summary.runtime_count=3
summary.module_count=3
summary.flow_count=2
summary.relationship_count=14
summary.healthy_targets=0
summary.unknown_targets=3
summary.degraded_targets=0
summary.unavailable_targets=0
summary.unhealthy_targets=0
validation_errors.Count=0               (top-level collection)
readiness_status=ready
readiness.checks.side_effects_performed=false
readiness.side_effects_performed=false
readiness.metadata.execution_performed=false
```

`GET /api/v1/dgraph/materialized/summary` is validated only against this nested
contract. The smoke deliberately does not accept a flattened fallback.

`checks_executed=false` is validated in each observed-health entry's metadata.
`safe_to_auto_execute=false` belongs to the local snapshot and publication response
contracts, where the smoke continues to validate it. Readiness has no such field;
non-execution there is established only by `checks.side_effects_performed=false`,
`side_effects_performed=false`, and `metadata.execution_performed=false`.

An additional emergency restoration runs if a failure occurs before a scenario's normal
restoration is confirmed.

## Safety boundary

I33 performs only:

- read-only loading of the existing local config and persisted snapshot;
- creation of ignored scenario copies;
- local `dgraph-snapshot --observed-health --json` translation;
- explicit reporting-only snapshot publication;
- read-only materialized/readiness queries;
- baseline fixture publication for restoration.

It does not run a DATUM collection cycle or service check and does not invoke placement,
recovery, dispatch, deployment, installation, Docker, WASM, migration, reconfiguration,
or planner execution. Every scenario requires `checks_executed=false` in health
metadata, `safe_to_auto_execute=false` in local snapshot/publication contracts, and
the readiness non-execution triplet: `checks.side_effects_performed=false`,
`side_effects_performed=false`, and `metadata.execution_performed=false`.

## Offline validation

Run from the repository root:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\tests\smoke-dgraph-observed-health-matrix-contract.tests.ps1
```

The contract test parses the script and verifies all six scenarios, source-copy-only
writes, URL/project overrides, the nested materialized-summary paths, deterministic
`validation_errors` counting, rejection of legacy top-level count assertions,
per-scenario and emergency restoration, source hash checks, safety flags, and the
absence of operational execution paths.

## Live command

Run from the repository root after starting `dserver` and confirming the three untracked
local configs and the persisted `fog-02` snapshot:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File `
  .\scripts\smoke-dgraph-observed-health-matrix.ps1 `
  -ServerUrl http://192.168.10.101:8080 `
  -ProjectId smart-city-campinas `
  -Fog01Config .\DATUM\config-fog-01.local.toml `
  -Fog02Config .\DATUM\config-fog-02.local.toml `
  -CloudConfig .\DATUM\config-cloud-01.local.toml `
  -ObservedSnapshot .\DATUM\snapshot-fog-02.json
```

Do not commit the local configs, persisted snapshot, temporary scenario copies, or live
output. After a successful live run, record the exact evidence in a separate validation
document and only then advance the checkpoint from `116/130` to `117/130`.
