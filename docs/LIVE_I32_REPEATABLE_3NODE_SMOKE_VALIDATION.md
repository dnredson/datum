# Live I32 Repeatable Three-Node Smoke Validation

## Status

`I32/130` is complete. The repeatable live three-node D-Graph smoke passed on
Windows host `192.168.10.101`. Project progress is `115/130`.

## Execution topology

The validation used three logical DATUM nodes, all executed locally on the Windows
operator host:

```text
fog-01/chirpstack-primary -> fog-02/mqtt-adapter -> cloud-01/magistrala-core
```

`cloud-01` is metadata-only logical topology, not a third physical machine. The
validation did not use Ubuntu VM `192.168.10.106`, SSH, a remote DATUM checkout, or
remote Cargo.

## Completed sequence

1. Local preflight completed with PowerShell 7+, Cargo/Cargo.toml, three local
   configurations, dserver availability, and the persisted `fog-02` snapshot.
2. Baseline fixtures were published locally for `fog-01`, `fog-02`, and `cloud-01`.
3. Latest-snapshot read-back and the materialized baseline were validated.
4. The local `fog-02` `degraded-flow` fixture produced the expected advisory warning;
   the planner was queried only through dry-run summary endpoints.
5. Observed health was read from the persisted local `fog-02` snapshot and explicitly
   published as reporting-only metadata.
6. The mandatory `finally` restoration republished the `fog-02` baseline and the final
   baseline validation passed.

## Final restored state

```text
source_snapshot_count=3
node_count=3
runtime_count=3
module_count=3
flow_count=2
relationship_count=14
unknown_targets=3
unhealthy_targets=0
unavailable_targets=0
readiness_status=ready
warning_count=0
degraded_flow_count=0
checks_executed=false
safe_to_auto_execute=false
side_effects_performed=false
```

## Safety boundary

The smoke invoked no recovery, dispatch, deployment, installation, Docker, WASM,
migration, reconfiguration, or planner-decision execution. Fixture and observed-health
publication were explicit metadata-only reporting operations; planner access remained
advisory and read-only/dry-run.

## Validation provenance

This record captures the completed live execution reported for I32. The offline
PowerShell contract test is re-run independently when this documentation changes; it
does not repeat the live smoke.
