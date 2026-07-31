# Distributed Dry-Run Preparation Checkpoint

## Purpose

This document summarizes readiness for distributed metadata-only and dry-run validation, records the completed live two-node and three-logical-node experiments, and tracks the remaining work before any executed orchestration.

Status notes:

- `A7/100` documented the first `2-VM` distributed metadata-only test plan.
- `I24/100` added a parameterized `2-VM` metadata-only smoke draft.
- `A8/100` documented the first `3-VM` orchestration and reconfiguration dry-run test plan.
- `C34/100` added local d-graph snapshot fixture mode for degraded and failure simulation.
- `I25/100` extended smoke coverage with fixture scenarios and planner visibility checks.
- `R4/100` records distributed dry-run readiness before the first required live `2-VM` execution.
- `I28/110` completed live two-node metadata-only validation; the system is ready for the next refinement phase.
- `C37/110` materializes runtime entities as node-scoped entities, preventing identical local runtime IDs from collapsing across nodes.
- C37 resolved the runtime identity collapse in live 2-VM validation; the next recommended refinement is richer flow metadata fixtures.
- `C38/110` adds metadata-only flow fixtures for the advisory/read-only logical flow `fog-01/chirpstack-primary -> fog-02/mqtt-adapter`; no telemetry is published and no reconfiguration is executed.
- `C38/110` completed live validation of the metadata-only cross-node flow fixtures; the next recommended refinement is flow-aware planner advisory semantics.
- `C39/110` makes degraded flow metadata visible as planner advisory warnings. Degraded flows do not trigger automatic reconfiguration; the planner remains read-only/dry-run, and `status=planned` may include advisory warnings.
- `C39/110` completed live validation of advisory warning semantics for degraded flows.
- `C40/110` adds a consolidated read-only D-Graph readiness summary combining latest snapshots, materialized graph and planner dry-run summaries, validation errors, and safety checks. It remains metadata-only and does not execute deployment, dispatch, or reconfiguration.
- The live metadata-only D-Graph validation phase reached `110/110`. Its final baseline readiness status is `ready`, its final degraded-flow status is `ready_with_warnings`, and the system is ready for the next macro phase. See [the final phase record](LIVE_DGRAPH_METADATA_ONLY_VALIDATION_PHASE_FINAL.md).
- `C41/130` adds the `cloud-01/magistrala-core` metadata-only fixture, the second logical flow `fog-02/mqtt-adapter -> cloud-01/magistrala-core`, a sanitized cloud config template, and a two-host/three-node validation plan. Live publication remains deferred to the next milestone. See [the C41 plan](C41_CLOUD_01_METADATA_ONLY_3NODE_CONTINUUM_PLAN.md).
- `C42/130` completed live three-logical-node metadata-only validation across two physical hosts. The baseline materialized 3 nodes, 3 node-scoped runtimes, 3 modules, 2 flows, and 14 relationships with `readiness_status=ready`; the degraded cloud-bound flow produced one advisory warning with `readiness_status=ready_with_warnings`; the restored final baseline returned to zero warnings and zero degraded flows. No execution or side effects occurred. See [the C42 live record](LIVE_3NODE_DGRAPH_METADATA_ONLY_VALIDATION.md).

## Completed Readiness Scope

The project now has:

- `DATUM` local d-graph snapshot generation
- `DATUM` explicit d-graph snapshot publishing
- `dserver` local d-graph snapshot ingestion and read-back
- `dserver` project d-graph materialization
- `DATUM` materialized d-graph viewer
- `dserver` read-only d-graph planner dry-run endpoint
- `DATUM` planner dry-run viewer
- fixture scenario generation for `baseline`, `degraded-node`, `unavailable-node`, `missing-module`, `degraded-flow`, and `no-candidate`
- smoke coverage for snapshot publishing
- smoke coverage for materialization
- smoke coverage for planner dry-run
- smoke coverage for fixture scenarios
- a `2-VM` metadata-only smoke script draft
- `2-VM` and `3-VM` distributed test plans
- sanitized config templates for `fog-01`, `fog-02`, and `cloud-01`
- successful live three-logical-node baseline publication and read-back
- live validation of the multi-hop metadata-only flow chain
- a validated degraded-flow sequence with baseline restoration

## Current Safety Guarantees

The current distributed dry-run preparation remains intentionally non-destructive:

- no `dispatch --execute`
- no planner decision execution
- no deployment execution
- no installation execution
- no Docker `run`/`pull`/`rm`/`inspect`
- no artifact transfer
- no WASM artifact loading
- no registered WASM module execution
- no migration execution
- no runtime flow rewiring
- no active config writes
- no local mapping mutation
- d-graph snapshots are metadata-only
- fixtures are metadata-only
- planner dry-run is advisory and read-only
- `side_effects_performed` remains `false` in the relevant snapshot, materialization, and planner contracts

## Live 2-VM Readiness Checklist

Required:

- `dserver` reachable from both `DATUM` nodes
- Firestore emulator or configured backend available for `dserver`
- unique `DATUM` config for `fog-01`
- unique `DATUM` config for `fog-02`
- same `project_id` in both configs
- correct `dserver` base URL in both configs
- `cargo run -- dgraph-snapshot --config <config> --json` works on both nodes
- `cargo run -- dgraph-snapshot --config <config> --publish` works on both nodes
- server latest snapshot read-back works for both node IDs
- materialized summary reports `source_snapshot_count >= 2`
- planner dry-run summary returns `side_effects_performed=false`

Optional:

- use `-FixtureScenario baseline`
- use `-FixtureScenario unavailable-node`
- use one real config-derived snapshot and one fixture snapshot
- run `DATUM` viewer commands from only one operator machine

## Remaining Gaps Before Executed Multi-Node Orchestration

- read-only health derived from service observations rather than fixture-only state
- explicit distinction between `unknown` metadata-only health and genuinely unhealthy state
- a reusable multi-host three-node smoke script
- richer advisory reconfiguration proposal semantics
- a separate safety contract before any placement, recovery, migration, or reconfiguration execution

## Recommended Next Milestone Options

### Option `C43/130`: Read-Only Observed Health Normalization

- derive module health from safe local service observations
- preserve `unknown` as distinct from degraded or unavailable
- publish only metadata
- keep planner, dispatch, recovery, Docker, and WASM execution disabled
- recommended as the next capability increment

### Option `I32/130`: Repeatable Live Three-Node Smoke Script

- encode the validated publication, read-back, degraded-flow, and restoration sequence
- keep real environment values in untracked local inputs
- assert the final restored baseline and safety flags

### Option `C44/130`: Enriched Planner Advisory Proposals

- represent candidate reconfiguration proposals as structured advisory output
- do not execute placement or reconfiguration
- retain deterministic warning and safety metadata

Recommendation:

- prefer `C43/130` next because the main remaining semantic gap is that metadata-only module health is still `unknown`
- follow with `I32/130` once the observed-health contract is stable

## Practical Summary

C42 proves that the current D-Graph pipeline can represent and validate a three-logical-node, two-flow continuum across two physical hosts while remaining metadata-only and non-executing. The final persisted baseline is clean and ready.

The next architecture boundary is no longer basic distributed publication or materialization. It is converting safe local observations into trustworthy health semantics while preserving the distinction between unknown and unhealthy state. Executed orchestration remains deliberately out of scope.
