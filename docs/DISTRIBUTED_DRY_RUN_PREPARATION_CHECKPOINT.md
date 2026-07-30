# Distributed Dry-Run Preparation Checkpoint

## Purpose

This document summarizes what is now ready for distributed metadata-only and dry-run validation, and what still needs preparation before the first live `2-VM` and `3-VM` experiments.

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

## Remaining Gaps Before 3-VM Orchestration Dry-Run

- per-node config templates for `fog-01`, `fog-02`, and `cloud-01`
- richer flow metadata in `DATUM` config and snapshot generation
- explicit health and degraded-node reporting beyond fixture mode
- a multi-host orchestration script beyond the current `2-VM` draft
- expected module and flow names for reproducible planner output
- a decision on Firestore emulator versus real backend
- expected failure and degradation scenario order
- a baseline restoration procedure after degraded fixture publish

## Recommended Next Milestone Options

### Option `I26/100`: Live 2-VM Preflight and Config Template Pack

- create example config templates for `fog-01` and `fog-02`
- create a checklist for local paths and environment variables
- make no runtime behavior changes
- recommended before the first actual live distributed run

### Option `I27/100`: Execute the First Live 2-VM Metadata-Only Smoke

- run the distributed smoke script against a real environment
- collect output and operator notes
- document the result
- only after configs and backend reachability are ready

### Option `C35/100`: Richer DATUM Flow Metadata in D-Graph Snapshots

- add safe config-derived flow metadata where current snapshots are sparse
- improve planner visibility before `3-VM` tests

### Option `A9/100`: Macro Checkpoint at `100/100`

- close the current planning phase
- define the live distributed execution window
- record the required safety and environment prerequisites

Recommendation:

- prefer `I26/100` next if live VM configs are not ready
- prefer `I27/100` only if both VMs and the `dserver` backend are ready

## Practical Summary

The project is ready for a first live distributed dry-run once real `fog-01` and `fog-02` configs exist and both nodes can publish to the same `dserver` project. The missing work is now mostly environmental and reproducibility-oriented rather than architectural.

The `3-VM` path is planned but not yet operational. It still needs stronger flow metadata, repeatable degraded-node sequencing, and a clearer multi-host execution procedure before a live orchestration dry-run should be required.
