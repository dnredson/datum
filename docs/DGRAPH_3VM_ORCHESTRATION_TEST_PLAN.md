# D-Graph 3-VM Orchestration Dry-Run Test Plan

## Purpose

This document defines the first `3-VM` orchestration and reconfiguration dry-run test for `DATUM` and `dserver`.

Status notes:

- `A8/100` documented the first `3-VM` orchestration and reconfiguration dry-run test plan.
- `C34/100` adds local fixture mode to simulate degraded, unavailable, missing-module, and related scenarios safely.
- `I25/100` validates fixture scenarios in smoke without touching Docker state, VM state, or planner execution.
- `R4/100` adds a distributed dry-run preparation checkpoint that summarizes live `2-VM` readiness and remaining `3-VM` preparation gaps.
- Fixture snapshots remain metadata-only and can later support `3-VM` dry-run validation.

The goal is to validate that:

- three independent `DATUM` node identities can publish metadata to one `dserver`
- `dserver` can materialize a multi-node project d-graph from the latest snapshots
- the read-only planner dry-run can propose placement or reconfiguration alternatives when one node, module, or flow becomes degraded or unavailable
- all results remain advisory only with no deployment, installation, planner decision execution, Docker action, or flow rewiring

This milestone is documentation-only. It does not change runtime behavior.

## Preferred Topology

Preferred first `3-VM` topology:

- one `dserver`
- three `DATUM` node identities
- metadata-only publishing
- dry-run-only planning
- no deployment execution

### VM1

- role: `dserver`
- services:
  - `dserver`
  - Firestore emulator or configured Firestore backend
- endpoint:
  - `http://<server-ip>:8080`

### VM2

- role: `DATUM` node `fog-01`
- represents:
  - primary fog or edge node
- example hosted metadata:
  - `chirpstack-primary`

### VM3

- role: `DATUM` node `fog-02` or `cloud-01`
- represents:
  - alternative placement or recovery node
- example hosted metadata:
  - `mqtt-adapter`
  - `data-processor`
  - fallback runtime metadata

## Optional Later Topology

If the first `3-VM` setup is easier to stage with mixed roles, an acceptable later variant is:

- `VM1`: `dserver` plus optional `cloud-01` `DATUM`
- `VM2`: `fog-01` `DATUM`
- `VM3`: `fog-02` `DATUM`

The first `3-VM` test remains metadata-only and dry-run only in every topology.

## Node Identities

Default node IDs:

- `fog-01`
- `fog-02`
- `cloud-01`

Default project ID:

- `smart-city-campinas`

Each per-node `DATUM` config should provide:

- a unique `node_id`
- the same `project_id`
- the same `dserver` base URL
- local d-graph snapshot metadata
- optional runtime, module, health, capability, and flow metadata

## Desired Graph Scenario

The target scenario for the first `3-VM` dry-run should include:

- three nodes
- at least two runtimes
- at least three modules
- at least one flow
- at least one plausible fallback placement candidate

### Example Modules

- `chirpstack-primary` on `fog-01`
- `mqtt-adapter` on `fog-02`
- `irrigation-decision-agent` or `data-processor` on `cloud-01`

### Example Flow

- `lorawan-ingestion-flow`
- `chirpstack-primary -> mqtt-adapter -> data-processor`

### Expected Planner Dry-Run Behavior

- if `fog-01` is healthy, planner status may be `empty` or `planned` depending on graph completeness
- if `fog-01` is degraded or unavailable, planner should avoid it for new placements when another candidate is available
- if a module is missing from its expected node, planner should propose placement when a compatible candidate exists
- if no candidate exists, planner should block safely
- if flow metadata is degraded, planner may emit a reconfiguration proposal or a warning without performing rewiring

## Degradation And Failure Scenarios

### Scenario A: Baseline Healthy Graph

- all three nodes publish snapshots
- materialized graph has `node_count >= 3`
- planner status may be `empty` or `planned`
- no execution occurs

### Scenario B: fog-01 Unavailable

- `fog-01` snapshot or health metadata marks the node unavailable or degraded
- planner dry-run should avoid `fog-01` for new placements
- planner may suggest `fog-02` or `cloud-01` as alternative candidates

### Scenario C: Module Missing

- one expected module on `fog-01` is marked missing
- another node exposes compatible runtime or capability metadata
- planner proposes placement or blocks when metadata is insufficient

### Scenario D: Flow Edge Degraded

- one flow edge or flow state becomes degraded or blocked
- planner emits reconfiguration proposals or warnings
- no actual flow rewiring occurs

### Scenario E: No Valid Candidate

- all candidate nodes are unavailable or lack required runtime support
- planner returns `blocked` or `partially_blocked`
- no fallback side effects occur

## Test Sequence

### Preparation

1. Start `dserver` on `VM1`.
2. Ensure Firestore emulator or configured backend is available.
3. Configure `DATUM` for `fog-01`, `fog-02`, and `cloud-01`.
4. Verify all nodes can reach the `dserver` HTTP endpoint.

### Step 1: Publish Baseline Snapshots

Run `DATUM dgraph-snapshot --publish` from each node or config variant.

Expected result:

- server latest snapshots count is `>= 3`
- each node has an independent latest snapshot
- all publishes remain metadata-only

### Step 2: Materialize Baseline Graph

Use:

- server `GET /api/v1/dgraph/materialized`
- server `GET /api/v1/dgraph/materialized/summary`

Expected result:

- `source_snapshot_count >= 3`
- summary `node_count >= 3`
- materialized graph contains the three node identities
- `side_effects_performed=false`

### Step 3: Baseline Planner Dry-Run

Use:

- server `GET /api/v1/dgraph/planner/dry-run`
- server `GET /api/v1/dgraph/planner/dry-run/summary`
- `DATUM` `dgraph-snapshot --planner-dry-run`
- `DATUM` `dgraph-snapshot --planner-summary`

Expected result:

- planner output is clean and advisory
- `side_effects_performed=false`
- status may be `empty`, `planned`, `partially_blocked`, or `blocked`

### Step 4: Simulate Degraded Node Metadata

Represent `fog-01` degraded or unavailable using:

- a config-derived snapshot variant, or
- a future fixture mode when supported

Publish the updated snapshot and verify that the graph reflects degraded or unavailable metadata when the schema supports it.

### Step 5: Planner Dry-Run After Degradation

Validate:

- planner output remains advisory only
- placement or reconfiguration alternatives appear when metadata is sufficient
- blocked reasons appear when metadata is insufficient
- no planner decisions are executed

### Step 6: Restore Baseline Metadata

Publish healthy metadata again when needed and verify that the planner returns to the expected baseline behavior.

## Expected Outputs

The first `3-VM` orchestration dry-run should produce:

- three latest snapshots visible on `dserver`
- materialized graph `source_snapshot_count >= 3`
- graph summary `node_count >= 3`
- planner dry-run output with clean JSON and readable viewer output
- planner status that may be `empty`, `planned`, `partially_blocked`, or `blocked`
- `side_effects_performed=false` everywhere
- no deployment or reconfiguration action

Validation errors are tolerated for partial graph data as long as:

- endpoints respond successfully
- planner output remains advisory only
- no side effects occur

## Fatal And Non-Fatal Failures

### Non-Fatal

- graph `validation_errors` caused by partial metadata
- planner `blocked` due to missing candidates
- empty flows when configs do not yet model flow data
- `source_snapshot_count < 3` when one node has not yet published and the situation is clearly reported
- planner warnings

### Fatal

- `side_effects_performed=true`
- `dispatch --execute` is used
- Docker or container side effects are observed
- installation or deployment is executed
- a planner decision is executed
- a real flow reconfiguration is performed
- an artifact is downloaded unexpectedly
- a config file is modified by the test
- local mappings are modified by the test

## Implementation Gaps Before Live 3-VM Smoke

Before a live `3-VM` smoke can be automated, the project still needs:

- per-node config templates for `fog-01`, `fog-02`, and `cloud-01`
- richer flow metadata in `DATUM` config and snapshot generation
- a health or degraded-node reporting mechanism
- optional local snapshot fixture mode for degraded or unavailable scenarios
- a multi-host smoke orchestration script
- a clear environment variable convention for server URL, project ID, node IDs, and config paths
- a decision on Firestore emulator versus real backend for distributed testing
- a stable sample set of modules and flows for repeated runs

## Relation To WASM

- `WASM Runtime v0` is available as an isolated diagnostics and reporting substrate.
- The first `3-VM` test does not execute registered WASM modules.
- Future work may use WASM diagnostics as metadata sources only after policy review.
- WASM must not become a remediation executor without artifact trust, limits, policy, approval, and reporting.

## Safety Boundaries

This `3-VM` plan preserves the current non-destructive boundaries:

- metadata-only publishing
- read-only materialization
- advisory planner dry-run only
- no deployment execution
- no installation execution
- no planner decision execution
- no Docker runtime mutation
- no artifact transfer
- no WASM artifact loading
- no flow rewiring

The first `3-VM` dry-run must prove multi-node visibility and safe orchestration reasoning under degraded conditions before any execution-oriented milestone is considered.
