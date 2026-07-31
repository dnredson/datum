# C44/130 - Read-Only Observed Health

## Status

Implementation prepared for Rust validation. The project remains at `113/130` until formatting,
tests, builds, and a live read-only validation pass.

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

## Required validation

Before C44 can close:

```powershell
cargo fmt --manifest-path dserver\Cargo.toml -- --check
cargo test --manifest-path dserver\Cargo.toml
cargo build --manifest-path dserver\Cargo.toml

cargo fmt --manifest-path DATUM\Cargo.toml -- --check
cargo test --manifest-path DATUM\Cargo.toml
cargo build --manifest-path DATUM\Cargo.toml
```

The live validation must first inspect a current `snapshot.json` and then exercise only the
read-only translation. Publishing observed health remains a separate, explicit step after local
output is confirmed.
