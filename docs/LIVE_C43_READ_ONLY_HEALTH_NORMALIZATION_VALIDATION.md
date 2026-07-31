# C43/130 — Live Read-Only Health Normalization Validation

## Purpose

This record closes the live validation of the C43 read-only health normalization contract against the restored C42 three-node baseline.

The validation confirms that metadata-only health with status `unknown` is reported separately from explicit degraded or unavailable state. It does not add service probes or execute orchestration.

## Validated Revision

- repository: `dnredson/datum`
- branch under test: `main`
- revision: `caa0e46`
- project: `smart-city-campinas`
- operator environment: Windows PowerShell
- logical nodes: `fog-01`, `fog-02`, and `cloud-01`
- physical hosts: two
- local operator config: untracked and not committed

The previously persisted C42 baseline snapshots were reused. Restarting `dserver` on the C43 revision was sufficient to recalculate the materialized summary with the normalized contract.

## Prior Rust Verification

Before merge, the C43 implementation passed:

- `dserver`: 368/368 tests, `cargo fmt --check`, and build;
- `DATUM`: 353/353 tests, `cargo fmt --check`, and build;
- only the pre-existing DATUM `dead_code` warnings remained.

## Materialized Summary

The live materialized summary returned:

```text
project_id             : smart-city-campinas
source_snapshot_count  : 3
node_count             : 3
runtime_count          : 3
module_count           : 3
flow_count             : 2
relationship_count     : 14
healthy_targets        : 0
unknown_targets        : 3
degraded_targets       : 0
unavailable_targets    : 0
unhealthy_targets      : 0
validation_error_count : 0
side_effects_performed : False
```

This is the central C43 regression result: the three metadata-only module reports remain operationally unobserved, but they no longer produce a false unhealthy count.

## Planner And Readiness Summary

The combined live readiness read returned:

```text
readiness_status       : ready
source_snapshot_count  : 3
node_count             : 3
runtime_count          : 3
module_count           : 3
flow_count             : 2
relationship_count     : 14
healthy_targets        : 0
unknown_targets        : 3
degraded_targets       : 0
unavailable_targets    : 0
unhealthy_targets      : 0
planner_status         : planned
placement_count        : 3
warning_count          : 0
blocked_count          : 0
degraded_flow_count    : 0
unhealthy_node_count   : 0
side_effects_performed : False
execution_performed    : False
```

The readiness checks returned:

```text
has_snapshots              : True
has_multiple_nodes         : True
has_node_scoped_runtimes   : True
has_modules                : True
has_flow_metadata          : True
has_planner_output         : True
has_materialization_errors : False
has_warnings               : False
has_blockers               : False
has_degraded_flows         : False
side_effects_performed     : False
```

## Interpretation

The live result confirms all intended C43 semantics:

- every health report is counted in exactly one normalized category;
- `unknown` is not interpreted as a failure;
- `unhealthy_targets == degraded_targets + unavailable_targets`;
- the clean C42 baseline remains `ready`;
- planner output remains advisory;
- no execution or side effects occurred.

The historical C42 value `unhealthy_targets=3` remains a valid record of the pre-C43 contract. For the same persisted baseline under C43, the authoritative result is now `unknown_targets=3` and `unhealthy_targets=0`.

## Safety Boundary

The validation did not:

- execute service, container, Docker, WASM, placement, recovery, migration, or reconfiguration actions;
- publish telemetry;
- mutate active configuration or local mappings;
- replace the metadata-only health reports with live service observations.

## Closure

C43/130 is implementation-, Rust-, and live-validated. Project progress advances from `112/130` to `113/130`.

The next capability boundary is read-only health derived from fresh, safe local service observations while preserving the normalized categories and all existing non-execution guarantees.
