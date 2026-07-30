# D-Graph 2-VM Distributed Test Plan

## Purpose

This document defines the first `2-VM` distributed metadata-only test for `DATUM` and `dserver`.

Status notes:

- `A7/100` documented the first `2-VM` metadata-only distributed test plan.
- `I24/100` adds a parameterized `2-VM` metadata-only smoke script draft.
- `A8/100` adds the follow-up `3-VM` orchestration and reconfiguration dry-run test plan.
- `R4/100` adds a distributed dry-run preparation checkpoint that summarizes live `2-VM` readiness and remaining `3-VM` gaps.
- `I26/100` adds a live preflight checklist and sanitized config templates for `fog-01` and `fog-02`.
- The draft is safe to validate without live VMs by using `-AllowMissingConfigs`.
- The draft does not execute deployment or planner decisions.

`I26/100` still does not run a live `2-VM` smoke as validation. It prepares the first real metadata-only distributed run.

The goal is to validate that:

- two independent `DATUM` node identities can publish metadata to one `dserver`
- `dserver` can materialize one project d-graph from the two latest node snapshots
- the read-only planner dry-run can reason over the two-node graph
- no deployment, installation, planner decision execution, Docker action, or flow rewiring occurs

This milestone is documentation-only. It does not change runtime behavior.

## Preferred Topology

Preferred first test topology:

- one `dserver`
- two `DATUM` node identities
- metadata-only publishing
- no deployment execution

### VM1

- role: `dserver`
- optional role: `DATUM` node `fog-01`
- services:
  - `dserver`
  - Firestore emulator or configured Firestore backend
- endpoint:
  - `http://<server-ip>:8080`

### VM2

- role: `DATUM` node `fog-02`
- services:
  - `DATUM` client
- connects to:
  - `dserver` HTTP endpoint on `VM1`

## Alternative Topology

If the first test environment is easier to prepare with mixed local/remote roles, the acceptable fallback is:

- `VM1`: `dserver` only
- `VM2`: `DATUM` node `fog-01`
- local machine or optional third host used only as `DATUM` node `fog-02`

The preferred topology remains:

- one central `dserver`
- two distinct `DATUM` node IDs
- shared `project_id`
- read-only metadata publishing and read-back only

## Node Identities

Expected node IDs:

- `fog-01`
- `fog-02`

Expected project ID:

- `smart-city-campinas`

Each `DATUM` config must use:

- a unique `node_id`
- the same `project_id`
- the same `dserver` base URL
- different local config-derived modules when practical

## Desired Test Data

The first distributed test should use a minimal two-node d-graph scenario.

### fog-01

- runtime: container metadata derived from config
- module: `chirpstack-primary` or equivalent sample service
- capability: container runtime or mapping support when configured
- health: `unknown` or `ok`

### fog-02

- runtime: container metadata derived from config
- module: `mqtt-adapter` or equivalent sample service
- capability: container runtime or mapping support when configured
- health: `unknown` or `ok`

### Optional Flow

- `flow_id`: `lorawan-ingestion-flow`
- logical edge: `chirpstack-primary -> mqtt-adapter`

If the current `DATUM` config path does not yet expose real cross-node flow data, this flow should be treated as a future enhancement rather than a hard requirement for the first `2-VM` metadata-only test.

## Test Sequence

### Preparation

1. Start `dserver` on `VM1`.
2. Ensure Firestore emulator or configured backend is available.
3. Configure `DATUM` for node `fog-01`.
4. Configure `DATUM` for node `fog-02`.
5. Verify both clients can reach the `dserver` HTTP endpoint.

### Step 1: Publish Capabilities

Publish capabilities from `fog-01` and `fog-02` using the already supported `DATUM` capability reporting flow.

Expected result:

- server capability read-back shows both node identities
- no execution occurs

### Step 2: Publish Local D-Graph Snapshots

Example commands:

```powershell
cargo run -- dgraph-snapshot --config config-fog-01.toml --publish
cargo run -- dgraph-snapshot --config config-fog-02.toml --publish
```

Expected result:

- both publishes succeed
- published snapshots remain metadata-only
- `safe_to_auto_execute=false`
- `side_effects_performed=false`

### Step 3: Verify Latest Snapshots

Validate:

- server project latest d-graph snapshots shows count `>= 2`
- node latest read-back works for `fog-01`
- node latest read-back works for `fog-02`

Expected result:

- each node has an independent latest snapshot
- both snapshots belong to the same project

### Step 4: Materialize Project D-Graph

Use:

- server `GET /api/v1/dgraph/materialized`
- server `GET /api/v1/dgraph/materialized/summary`
- `DATUM` `dgraph-snapshot --materialized`
- `DATUM` `dgraph-snapshot --materialized-summary`

Expected result:

- `source_snapshot_count >= 2`
- `node_count >= 2`
- materialized graph contains both node identities
- `side_effects_performed=false`

### Step 5: Run Planner Dry-Run

Use:

- server `GET /api/v1/dgraph/planner/dry-run`
- server `GET /api/v1/dgraph/planner/dry-run/summary`
- `DATUM` `dgraph-snapshot --planner-dry-run`
- `DATUM` `dgraph-snapshot --planner-summary`
- `DATUM` `dgraph-snapshot --readiness-summary` consolidates the latest-snapshot, materialization, and planner dry-run checks without executing any decision.

Expected result:

- clean advisory output
- `side_effects_performed=false`
- planner status may be `planned`, `partially_blocked`, `blocked`, or `empty`
- placements and reconfigurations may be empty depending on graph completeness

### Step 6: Validate No Execution

Confirm the test does not:

- run `dispatch --execute`
- execute planner decisions
- perform deployment execution
- perform installation execution
- call Docker `run`, `pull`, `rm`, or `inspect`
- load external WASM artifacts
- execute registered WASM modules
- perform migration
- perform flow rewiring
- modify config files

## Expected Outputs

The first `2-VM` distributed metadata-only test should produce:

- two latest `DATUM` snapshots visible on `dserver`
- materialized graph `source_snapshot_count >= 2`
- graph summary `node_count >= 2`
- read-only planner dry-run output with clean JSON and human-readable viewer output
- `side_effects_performed=false` everywhere
- no actual deployment or reconfiguration execution

Validation errors are tolerated for partial graph data as long as:

- endpoints respond successfully
- planner output remains advisory only
- no side effects occur

## Failure Cases To Observe

The following cases should be observed and documented but do not necessarily invalidate the test if the safety contract remains intact:

- one node cannot reach `dserver`
- one node has missing or sparse config fields
- one snapshot contains no modules or no flows
- materialized graph contains `validation_errors`
- planner status is `empty`
- planner status is `blocked`
- planner status is `partially_blocked`
- Firestore or backend is unavailable
- `dserver` health endpoint is unavailable or the wrong service is bound to the expected port

The following cases should fail the test:

- `side_effects_performed=true`
- a config file is modified during the test
- a Docker or container side effect is observed
- `dispatch --execute` is used
- a planner decision is executed
- deployment or installation is executed
- an artifact is downloaded unexpectedly

## 3-VM Follow-Up

Next extension after the first `2-VM` test:

- add `fog-03` or `cloud-01`
- simulate `fog-01` unavailable or degraded
- publish updated health and snapshot metadata
- materialize a three-node project d-graph
- verify planner dry-run proposes alternative placement or reconfiguration when metadata is sufficient
- keep the first `3-VM` phase dry-run only with no execution

## Required Future Implementation Gaps

Before live `2-VM` smoke automation is practical, the project still needs:

- config templates for `fog-01` and `fog-02`
- a manual or scripted remote command path for `DATUM` publish operations
- richer flow configuration in `DATUM` snapshots when cross-node flow validation becomes important
- clearer degraded or unavailable health reporting for nodes and runtimes
- optional local snapshot fixture mode if real configs are too sparse
- a multi-host smoke script variant
- clear environment variables for server URL, project ID, node IDs, and config paths

## Safety Boundaries

This `2-VM` plan preserves the current non-destructive boundaries:

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

The first distributed test must prove visibility and reasoning across nodes before any execution-oriented milestone is considered.
