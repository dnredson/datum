# D-Graph Materialization Checkpoint

## Purpose

This document closes the completed d-graph materialization phase and records the current architecture, safety boundaries, limitations, and the next macro phase.

Status:

- safe assisted control-plane phase: complete `60/60`
- operator-reviewed local remediation phase: complete `70/70`
- WASM Runtime v0 phase: complete `80/80`
- d-graph materialization phase: complete `90/90`

This checkpoint is documentation-only. It does not introduce planner endpoints, deployment behavior, or runtime flow reconfiguration.

## Completed Scope

The completed d-graph materialization phase delivered:

- pure `dserver` `DGraph` model foundations
- pure `dserver` d-graph validation helpers
- pure `dserver` d-graph relationship derivation helpers
- pure `dserver` d-graph summary helper
- pure `dserver` d-graph planner dry-run foundation
- `DATUM` local d-graph snapshot generation
- `dserver` local d-graph snapshot ingestion and read-back
- `DATUM` d-graph snapshot publishing and read-back
- repo-root smoke coverage for local d-graph snapshot publishing
- `dserver` project d-graph materialization from latest node snapshots
- `DATUM` project d-graph materialized viewer
- `DATUM` project d-graph summary viewer
- repo-root smoke coverage for the project materialized d-graph path

Delivered milestones in this phase:

- `G1/90`: pure `dserver` d-graph model foundation
- `G2/90`: pure `dserver` d-graph planner dry-run foundation
- `C30/90`: `DATUM` local d-graph snapshot reporting
- `S30/90`: `dserver` local d-graph snapshot ingestion/read-back
- `C31/90`: `DATUM` d-graph snapshot publishing/read-back
- `I21/90`: smoke coverage for local d-graph snapshot publishing
- `S31/90`: `dserver` read-only project d-graph materialization
- `C32/90`: `DATUM` project d-graph materialized/summary viewer
- `I22/90`: smoke coverage for project d-graph materialization

## Current Safety Guarantees

The completed phase preserves these guarantees:

- no planner endpoint yet
- no planner execution from smoke
- no deployment execution
- no installation execution
- no Docker `run`, `pull`, `rm`, or `inspect`
- no artifact transfer
- no WASM artifact loading
- no registered WASM module execution
- no migration execution
- no runtime flow rewiring
- no active config writes
- no local mapping mutation
- d-graph snapshot publishing is metadata-only
- project d-graph materialization is read-only
- `side_effects_performed` remains false

## Current Architecture State

Current d-graph data path:

`DATUM` config-derived local d-graph snapshot  
→ `DATUM` explicit publish  
→ `dserver` raw/latest local snapshot storage  
→ `dserver` project materialization from latest snapshots  
→ `DATUM` materialized/summary viewer  
→ repo-root smoke validation

Operationally, the system now supports:

- local config-derived node snapshot generation
- explicit project-scoped read-back of latest local snapshots
- project-level d-graph materialization from latest published node snapshots
- human-readable and JSON viewer paths for materialized graph metadata
- non-destructive smoke validation of the end-to-end metadata path

## Known Limitations

Current limitations remain:

- materialization depends on latest published snapshots
- graph may be partial if `DATUM` config is minimal
- `validation_errors` may appear for partial graph data
- no live multi-node materialization has been validated yet
- no `2-VM` or `3-VM` smoke exists yet
- planner dry-run foundation exists as pure code but is not exposed through API
- materialized d-graph is not yet used to produce operator-facing deployment or reconfiguration plans
- no real deployment action exists
- no real reconfiguration action exists

## Next Macro Phase

Next macro phase: `D-Graph Planner API and Distributed Test Preparation`

Proposed milestones:

- `S32/100`: `dserver` read-only d-graph planner dry-run endpoint
  - input: materialized project d-graph from latest snapshots
  - output: dry-run placement and reconfiguration plan
  - no execution

- `C33/100`: `DATUM` d-graph planner dry-run viewer
  - fetch and display planner dry-run output
  - no execution

- `I23/100`: smoke test for d-graph planner dry-run endpoint and viewer
  - validate `side_effects_performed=false`
  - validate no deployment or reconfiguration execution

- `A7/100`: `2-VM` distributed test plan
  - define topology, roles, configs, expected node IDs, and expected snapshots
  - no code unless needed

- `I24/100`: `2-VM` metadata-only distributed smoke
  - one `dserver`, two `DATUM` nodes
  - publish snapshots and capabilities
  - materialize project graph
  - run planner dry-run
  - no deployment execution

- `A8/100`: `3-VM` orchestration and reconfiguration test plan
  - define node failure, module relocation, and flow reconfiguration expectations

## Distributed Testing Roadmap

### Local

- one machine
- one `DATUM` config
- local d-graph snapshot
- materialized d-graph
- planner dry-run

### 2 VMs

- one `dserver`
- two `DATUM` clients
- two node IDs
- publish snapshots from both nodes
- materialize graph with two nodes
- validate placement alternatives

### 3 VMs

- one `dserver`
- three `DATUM` clients
- failure or degraded node scenario
- planner proposes relocation or reconfiguration
- still dry-run initially

### Later

- controlled local action pilot only after explicit safety checkpoint
- possible integration with pre-existing containers or WASM v0-safe diagnostics
- no automatic remediation loops without policy, approval, and reporting

## Closing Note

The d-graph materialization phase is complete. The system now has a stable metadata path from local `DATUM` snapshot generation through `dserver` project materialization and client-side read-back, while preserving the non-destructive safety posture established in earlier phases.

The next step is not execution expansion. The next step is exposing planner dry-run output safely and preparing multi-node metadata validation before any deployment or reconfiguration behavior is introduced.
