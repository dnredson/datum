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
- `C41/130` adds the `cloud-01/magistrala-core` metadata-only fixture, the second logical flow `fog-02/mqtt-adapter -> cloud-01/magistrala-core`, a sanitized cloud config template, and a two-host/three-node validation plan. Its deferred repeatable smoke was completed by I32. See [the C41 plan](C41_CLOUD_01_METADATA_ONLY_3NODE_CONTINUUM_PLAN.md).
- `C42/130` completed live three-logical-node metadata-only validation across two physical hosts. The baseline materialized 3 nodes, 3 node-scoped runtimes, 3 modules, 2 flows, and 14 relationships with `readiness_status=ready`; the degraded cloud-bound flow produced one advisory warning with `readiness_status=ready_with_warnings`; the restored final baseline returned to zero warnings and zero degraded flows. No execution or side effects occurred. See [the C42 live record](LIVE_3NODE_DGRAPH_METADATA_ONLY_VALIDATION.md).
- `C43/130` completed read-only health normalization and live validation. The same restored C42 baseline now reports `unknown_targets=3` and `unhealthy_targets=0`, with `readiness_status=ready`, zero warnings or blockers, and no execution or side effects. See [the C43 implementation record](C43_READ_ONLY_HEALTH_NORMALIZATION.md) and [live validation record](LIVE_C43_READ_ONLY_HEALTH_NORMALIZATION_VALIDATION.md).
- `C44/130` completed read-only observed-health derivation and live validation. A fresh persisted `fog-02/mqtt-adapter` observation was preserved as `unavailable` with source, raw status, age, TTL, and freshness metadata while `checks_executed=false`, `safe_to_auto_execute=false`, and `side_effects_performed=false`. Explicit publication materialized one unavailable and two unknown targets; the final restored baseline returned to 3 nodes, 3 node-scoped runtimes, 3 modules, 2 flows, 14 relationships, 3 unknown targets, zero unavailable or unhealthy targets, zero validation errors, and no side effects. Progress is `114/130`. See [the C44 implementation record](C44_READ_ONLY_OBSERVED_HEALTH.md) and [live validation record](LIVE_C44_READ_ONLY_OBSERVED_HEALTH_VALIDATION.md).
- `I32/130` completed the repeatable live three-node smoke locally on Windows `192.168.10.101`. `fog-01`, `fog-02`, and logical metadata-only `cloud-01` were all published with local DATUM configurations; no SSH or use of `192.168.10.106` occurred. Baseline, degraded-flow advisory, persisted observed-health publication, mandatory restoration, and final baseline checks passed with `checks_executed=false`, `safe_to_auto_execute=false`, and `side_effects_performed=false`. Progress is `115/130`. See [the I32 live validation record](LIVE_I32_REPEATABLE_3NODE_SMOKE_VALIDATION.md).
- `C45/130` adds deterministic structured advisory proposals to the full planner dry-run response. Degraded flows can now carry graph-backed evidence, related IDs, candidate observations, and explicit limitations while remaining non-executable. The summary contract is unchanged, no graph mutation occurs, and no placement, recovery, migration, or reconfiguration is executed. Progress is `116/130`. See [the C45 contract](C45_ENRICHED_PLANNER_ADVISORY_PROPOSALS.md).
- The first I33 live attempt published the initial baseline but failed in its first `Assert-Baseline` before any scenario because the smoke read materialized graph counts from obsolete top-level paths. The final emergency restoration published the `fog-02` baseline. It is not I33 approval evidence; progress remains `116/130` pending a corrected live matrix run.
- The second I33 live attempt published and materialized the initial baseline, then received a valid readiness response but failed its obsolete readiness `safe_to_auto_execute` assertion before any scenario. Emergency restoration republished `fog-02` and passed materialized validation, but full restoration validation encountered the same readiness assertion. It is not I33 approval evidence; progress remains `116/130`.
- `I33/130` completed live observed-health scenario validation on the third run. The initial three-node baseline and all six scenarios (`healthy`, `degraded`, `stale`, `absent`, `unrecognized`, and `future-skewed`) passed local, publication, materialized, readiness, and per-scenario baseline-restoration checks. Source config and persisted snapshot hashes were preserved, and no operational side effects occurred. The first two runs remain non-qualifying troubleshooting attempts. Progress is `117/130`. See [the I33 validation record](I33_OBSERVED_HEALTH_SCENARIO_MATRIX.md).
- `C46/130` completed the separate, fail-closed execution safety contract and its validation. Its read-only endpoint denies placement, recovery, migration, reconfiguration, deployment, installation, dispatch execution, and WASM execution even when D-Graph readiness is `ready`. On the final branch head, `cargo fmt --check`, all 7 focused tests, all 381 dserver tests, `git diff --check`, and the clean-worktree check passed. Live validation against the restored three-node baseline returned `decision=denied`, seven unsatisfied gates, eight unauthorized operations, and no execution or side effects. Progress is `118/130`. See [the C46 contract and validation record](C46_EXECUTION_SAFETY_CONTRACT.md).

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
- normalized health counts that distinguish `unknown`, `healthy`, `degraded`, and `unavailable`
- live confirmation that metadata-only `unknown` state is not reported as unhealthy
- opt-in health derivation from fresh persisted local service observations
- live confirmation that observed-health translation executes no checks and preserves explicit failure classes
- explicit reporting-only publication of observed health with final three-node baseline restoration
- a repeatable live three-node smoke executed entirely from local Windows DATUM configurations
- structured, deterministic, graph-backed planner advisory proposals with no execution path
- live observed-health coverage for healthy, degraded, stale, absent, unrecognized, and future-skewed states with validated restoration after every scenario
- a validated fail-closed execution safety contract that keeps readiness and planner evidence non-authoritative

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
- observed-health translation reads persisted observations without executing checks
- planner dry-run is advisory and read-only
- `side_effects_performed` remains `false` in the relevant snapshot, materialization, and planner contracts
- the validated C46 endpoint is read-only and always denies execution authority; it does not create an execution path

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

- implement every explicit execution gate in a later authorized phase before considering any allow decision

## Recommended Next Milestone Options

### Completed `I33/130`: Observed-Health Scenario Matrix

- exercise healthy, degraded, stale, absent, unrecognized, and future-skewed observations live
- preserve the read-only and reporting-only boundary
- restore the three-node baseline after every scenario

Completed at `117/130`; no follow-on increment is started by this record.

### Completed `C46/130`: Execution Safety Contract

- expose a separate read-only contract for the execution authorization boundary
- treat readiness and planner proposals as observational evidence only
- deny every in-scope operation fail-closed
- report missing policy, approval, integrity, allowlist, capability, resource-limit, rollback, and audit gates
- perform no writes and invoke no executor

Completed and validated at `118/130`. The focused and full Rust suites, formatting and repository
checks, and the restored three-node live endpoint assertions passed. No later increment is started
by this record.

## Practical Summary

C42 proves that the current D-Graph pipeline can represent and validate a three-logical-node,
two-flow continuum while remaining metadata-only and non-executing. C43 proves that the restored
baseline distinguishes unobserved metadata from explicit unhealthy state. C44 proves that a fresh
persisted service observation can be translated and explicitly published as canonical health
without running checks or performing operational side effects. I32 makes that sequence repeatable
and validates it locally for all three logical nodes, including restoration.

I33 completes the six-state observed-health scenario matrix with validated restoration after every
publication. C46 establishes and validates a separate fail-closed authorization boundary: even a
ready graph and concrete planner proposals do not authorize execution. Progress is `118/130`.
Executed orchestration remains deliberately out of scope, and no later increment is started by
this record.
