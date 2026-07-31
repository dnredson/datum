# C41/130 — cloud-01 Metadata-Only Fixture and Three-Node Continuum Plan

## Purpose

This milestone extends the validated two-node D-Graph fixture into a three-node logical continuum:

```text
fog-01/chirpstack-primary -> fog-02/mqtt-adapter -> cloud-01/magistrala-core
```

C41 is implementation and preparation only. Live publication from the physical hosts is deferred to the next milestone.

## Physical And Logical Topology

The first validation uses two physical machines and three independent DATUM node identities:

| Physical host | Address | Logical role |
| --- | --- | --- |
| Windows local | `192.168.10.101` | `dserver`, `fog-01`, and `fog-02` |
| Ubuntu VM | `192.168.10.106` | `cloud-01` |

`fog-01` and `fog-02` are separate logical nodes even when both configs are executed on the Windows host. They must keep distinct `node_id` values and publish independent latest snapshots.

The previous physical VM identity `fog-02` is not reused in this validation. The VM is represented only as `cloud-01`.

## Metadata-Only Fixture Contract

Baseline fixtures model:

- `fog-01`: `chirpstack-primary` and `flow-chirpstack-to-mqtt-adapter`;
- `fog-02`: `mqtt-adapter` and `flow-mqtt-adapter-to-magistrala-core`;
- `cloud-01`: `magistrala-core`;
- protocol: `mqtt`;
- flow state: `configured`;
- fixture metadata: `fixture=true`, `reporting_only=true`, and `cross_node=true`.

Expected baseline materialization:

- `source_snapshot_count=3`;
- `node_count=3`;
- `runtime_count=3`;
- `module_count=3`;
- `flow_count=2`;
- both cross-node flow IDs are present;
- `side_effects_performed=false`.

Exact relationship counts should be recorded during live validation rather than treated as a hard precondition.

## Config Templates

Use these sanitized examples as starting points:

- `scripts/templates/datum-config-fog-01.example.toml`;
- `scripts/templates/datum-config-fog-02.example.toml`;
- `scripts/templates/datum-config-cloud-01.example.toml`.

Copy each example to an untracked local file before replacing placeholders. Do not commit real addresses in templates, credentials, tokens, service-account paths, or local container mappings.

For the first metadata-only run, keep dispatch, assisted execution, container execution, Docker collection, and automatic recovery disabled wherever the local config permits.

## Planned Live Sequence

1. Start `dserver` on Windows `192.168.10.101`.
2. Prepare two independent Windows configs for `fog-01` and `fog-02`.
3. Prepare the VM config for `cloud-01`.
4. Generate each baseline fixture locally with `--json` and confirm both safety flags are false.
5. Explicitly publish the three baseline fixtures.
6. Read back the latest snapshot for every node.
7. Query materialized summary, planner dry-run summary, and readiness summary.
8. Record the exact baseline counts and planner status.
9. If a degraded-flow check is performed, restore the baseline fixture afterward and confirm warnings return to zero.

Representative commands, executed from each DATUM checkout:

```powershell
cargo run -- dgraph-snapshot --config <config-path> --fixture baseline --json
cargo run -- dgraph-snapshot --config <config-path> --fixture baseline --publish
```

Read-only server checks:

```powershell
curl -H "X-Datum-Project: smart-city-campinas" http://192.168.10.101:8080/api/v1/dgraph/materialized/summary
curl -H "X-Datum-Project: smart-city-campinas" http://192.168.10.101:8080/api/v1/dgraph/planner/dry-run/summary
curl -H "X-Datum-Project: smart-city-campinas" http://192.168.10.101:8080/api/v1/dgraph/readiness/summary
```

## Success Criteria

The live milestone succeeds when:

- all three latest snapshots are readable;
- the materialized graph contains `fog-01`, `fog-02`, and `cloud-01`;
- runtimes remain node-scoped;
- the three expected modules and two expected flows are visible;
- readiness is `ready` for the restored baseline;
- the planner remains advisory and in dry-run mode;
- `side_effects_performed=false`;
- `metadata.execution_performed=false`;
- no materialization validation errors are introduced.

## Safety Boundaries

C41 and its follow-up live validation do not perform:

- real telemetry or sensor publication;
- dispatch execution;
- deployment or installation;
- Docker run, pull, remove, restart, or inspect actions;
- WASM artifact loading or execution;
- migration;
- automatic flow rewiring or reconfiguration;
- planner decision execution;
- config or local mapping mutation.

A planner proposal or warning is evidence only. It must not be executed.

## Deferred Work

The following remain outside C41:

- live publication to the two physical hosts;
- a multi-host orchestration script;
- config-derived real observations or telemetry;
- executed placement, migration, recovery, or reconfiguration;
- changing unknown metadata-only health into operational health semantics.

## C42 Live Validation Result

The planned follow-up was completed successfully in `C42/130`.

The live run validated three independent latest snapshots across two physical hosts, materialized three nodes, three node-scoped runtimes, three modules, two cross-node flows, and fourteen baseline relationships. Baseline readiness was `ready`; the isolated degraded-flow scenario was `ready_with_warnings` with one deterministic advisory warning and no reconfiguration or execution. The `fog-02` baseline was restored afterward, returning the final persisted state to zero warnings and zero degraded flows.

See [C42 Live Three-Node Metadata-Only Validation](LIVE_3NODE_DGRAPH_METADATA_ONLY_VALIDATION.md) for the complete evidence.
