# C44/130 - Read-Only Observed Health

## Status

C44 is complete. Formatting, tests, builds, local read-only inspection, explicit metadata
publication, materialized aggregation, and baseline restoration all passed. Project progress is
`114/130`.

See [the live C44 validation record](LIVE_C44_READ_ONLY_OBSERVED_HEALTH_VALIDATION.md).

## Objective

C44 allows `dgraph-snapshot` to derive module health from a previously persisted local DATUM
snapshot. It does not start a DATUM collection cycle and does not execute service checks.

The capability is explicit:

```text
datum dgraph-snapshot --observed-health [--publish]
```

Without `--observed-health`, the C43 behavior is unchanged and configured modules remain
`unknown` when no observation is supplied.

## Read-only boundary

The implementation may only:

1. read the JSON snapshot configured by `outputs.file_json`;
2. select a service entity whose `attrs.name` matches the configured module ID;
3. verify that the persisted snapshot belongs to the same `node_id`;
4. evaluate freshness from `observed_at_utc` and `ttl_s`;
5. translate a fresh explicit status into canonical D-Graph health metadata;
6. optionally publish that reporting-only metadata through the existing snapshot endpoint.

It does not:

- run `datum once` or a collection cycle;
- execute HTTP, TCP, MQTT, process, or container checks;
- invoke Docker, WASM, recovery, deployment, migration, or reconfiguration;
- execute planner decisions;
- write or mutate the persisted observation file.

## Normalization

| Persisted service status | D-Graph health |
| --- | --- |
| `available`, `healthy`, `ok`, `running` | `healthy` |
| `degraded`, `warning` | `degraded` |
| `unavailable`, `down`, `failed`, `critical`, `missing` | `unavailable` |
| absent or any other value | `unknown` |

An observation older than its `ttl_s`, or timestamped more than five seconds in the future, is
always `unknown`. A stale failure must therefore never be presented as a current operational
failure.

## Evidence carried in each report

Observed reports include:

- source `persisted_local_snapshot`;
- raw status and canonical status;
- observation and evaluation timestamps;
- `ttl_s`, computed age, and freshness;
- source cycle and service entity IDs;
- `checks_executed=false` and `reporting_only=true`.

The local D-Graph snapshot continues to guarantee:

```text
safe_to_auto_execute=false
side_effects_performed=false
```

## Completed validation

```text
dserver:
  cargo fmt --check: passed
  tests: 368/368 passed
  build: passed

DATUM:
  cargo fmt --check: passed
  tests: 360/360 passed
  build: passed
```

The live `fog-02` observation used source
`persisted_local_snapshot`, remained fresh at `age_s=10` with `ttl_s=60`, and preserved
`raw_status=unavailable` as canonical `unavailable`. The translation reported
`checks_executed=false`, `safe_to_auto_execute=false`, and
`side_effects_performed=false`.

After explicit publication, the three-node graph reported one unavailable target and two unknown
targets with no validation errors. The baseline fixture was then restored to 3 nodes, 3
node-scoped runtimes, 3 modules, 2 flows, 14 relationships, 3 unknown targets, zero unavailable
or unhealthy targets, and no side effects.
