# Live C44 Read-Only Observed Health Validation

## Purpose

This document records the live validation that closes `C44/130`. The increment derives
canonical D-Graph module health from a previously persisted local DATUM snapshot while keeping
the translation reporting-only and non-executing.

Project progress advances to `114/130`.

## Validation Boundary

The observed-health command was validated with:

- `[auto_recovery] enabled = false`
- `[auto_recovery] dry_run = true`
- explicit `--observed-health`
- no `dispatch --execute`
- no planner decision execution
- no deployment, migration, flow reconfiguration, Docker, or WASM execution

A separate `datum once` cycle was used only to create a current persisted observation before
the read-only translation. The observed-health command itself did not start checks or a collection
cycle.

## Rust Validation

The local Windows validation completed successfully:

- dserver `cargo fmt --check`: passed
- dserver tests: `368/368` passed
- dserver build: passed with one pre-existing dead-code warning
- DATUM `cargo fmt --check`: passed after applying rustfmt
- DATUM tests: `360/360` passed
- DATUM build: passed with seven pre-existing dead-code warnings

## Persisted Observation Preparation

The local observation was generated for `fog-02`:

```text
mode=once
cycle=40b9351f-111c-4cc6-aff6-8fae3bd90451
dur_ms=3621
entities=3
edges=0
events=5
actions=3
file_json=ok
```

The configured snapshot file was created successfully. When the server was not running, the
optional HTTP output reported an expected send error while file persistence still completed with
exit code zero.

## Local Read-Only Translation

The immediate local `dgraph-snapshot --observed-health --json` result was:

```text
node_id=fog-02
health_count=1
source=persisted_local_snapshot
snapshot_cycle_id=40b9351f-111c-4cc6-aff6-8fae3bd90451
checks_executed=false
safe_to_auto_execute=false
side_effects_performed=false
```

The persisted `mqtt-adapter` observation was translated as:

```text
status=unavailable
raw_status=unavailable
observation_fresh=true
age_s=10
ttl_s=60
reason=derived from a fresh persisted service observation without executing checks
```

This proves that a fresh explicit failure is preserved without executing a new check.

## Published Observed-Health Materialization

After explicit metadata publication, the materialized summary reported:

```text
source_snapshot_count=3
node_count=3
runtime_count=3
module_count=3
flow_count=1
relationship_count=10
healthy_targets=0
unknown_targets=2
degraded_targets=0
unavailable_targets=1
unhealthy_targets=1
validation_error_count=0
side_effects_performed=false
```

The temporary `flow_count=1` and `relationship_count=10` were expected because the real
config-derived `fog-02` snapshot replaced that node's baseline fixture, which carries an
additional logical flow.

## Baseline Restoration

The `fog-02` baseline fixture was republished after the observed-health evidence was captured.
The final restored materialized summary was:

```text
source_snapshot_count=3
node_count=3
runtime_count=3
module_count=3
flow_count=2
relationship_count=14
healthy_targets=0
unknown_targets=3
degraded_targets=0
unavailable_targets=0
unhealthy_targets=0
validation_error_count=0
side_effects_performed=false
```

This matches the restored C42/C43 three-node baseline.

## Result

C44 passed live validation:

- a current persisted observation can be read and normalized
- the source cycle, raw status, TTL, age, and freshness are preserved
- fresh explicit failure remains `unavailable`
- the translation reports `checks_executed=false`
- publication remains explicit and metadata-only
- `safe_to_auto_execute=false`
- `side_effects_performed=false`
- the three-node baseline was restored after the test
