# I40 — Dependency-Aware Deployment Stages

- checkpoint: `I40/146`
- predecessor: `I39/145`
- status: `implementation_ready_for_validation`
- branch: `agent/i40-dependency-aware-stages`

## Goal

I40 derives deterministic deployment stages from explicit D-Graph dependency semantics rather than treating every D-Graph flow as deployment ordering.

Execution remains explicit. I40 plans and materializes ordering; it does not automatically execute stages.

## D-Graph dependency semantics

A flow edge is interpreted as a deployment dependency only when either:

```text
flow.flow_kind == "deployment_dependency"
```

or:

```json
{"metadata":{"deployment_dependency":true}}
```

This distinction is intentional. Ordinary MQTT/data/application flows are not silently converted into deployment prerequisites.

For a marked edge:

```text
from_module_id -> to_module_id
```

both modules must map to a unique deployable assignment. The downstream assignment depends on the upstream assignment.

If either endpoint has no deployable assignment, the D-Graph activation fails closed as an unsatisfied deployment dependency.

## Deterministic stage planning

The planner uses deterministic topological stages. Every assignment with no unsatisfied predecessor is placed in the current stage, sorted by `assignment_id`. All assignments in a stage are logically parallel from the dependency-plan perspective.

Example:

```text
fog-scale -> local-aggregate -> cloud-delta
```

becomes:

```text
stage 0: a-fog-scale
stage 1: m-local-aggregate
stage 2: z-cloud-delta
```

A cycle leaves no ready assignment and is rejected fail-closed.

## Materialized evidence

Each D-Graph-derived `DeploymentAssignment.metadata` receives:

```text
deployment_stage
depends_on_assignments
```

`DeploymentManifest.metadata.dependency_plan` contains the aggregate plan:

```text
plan_version
assignment_count
dependency_edge_count
stage_count
stages
assignments
automatic_execution_allowed=false
governed_execution_performed=false
```

When the observed D-Graph is materialized, each module also receives:

```text
deployment_assignment_id
deployment_stage
depends_on_assignments
```

and the aggregate plan is copied into graph/observation metadata.

## Compatibility

I39 manually posted manifests remain accepted. Dependency stages are derived only when dserver creates a manifest from an external D-Graph.

No schema field was added to the public assignment struct, so existing Sentinel deserialization remains compatible.

## Focused regression tests

I40 adds five dserver tests:

```text
acyclic three-node plan -> deterministic 3 stages
cycle -> rejected
marked dependency with missing deployable assignment -> rejected
ordinary data flow -> ignored for deployment ordering
edge metadata can explicitly mark a dependency in a mixed flow
```

Existing controlled D-Graph tests also assert that stage metadata is present in the derived manifest and observed D-Graph without increasing the test count.

Validated dserver baseline before I40 is `495 passed; 0 failed`, so expected minimum is:

```text
500 passed; 0 failed
```

## Fixtures

### Three-node acyclic

`scripts/fixtures/i40-dependency-three-node-dgraph.json`

```text
fog-scale-module
    ↓
local-aggregate-module
    ↓
cloud-delta-module
```

Expected assignment stages:

```text
stage 0 -> a-fog-scale       -> fog-01
stage 1 -> m-local-aggregate -> local-01
stage 2 -> z-cloud-delta     -> cloud-01
```

### Cycle

`scripts/fixtures/i40-dependency-cycle-dgraph.json`

Expected activation: HTTP 400 with cycle reason.

### Missing deployable dependency

`scripts/fixtures/i40-dependency-missing-dgraph.json`

Expected activation: HTTP 400 explaining that the marked upstream module has no deployable assignment.

## Planned live gate

Use a fresh disposable dserver controlled state. No artifact server is needed because I40 does not acquire or execute artifacts.

1. POST the three-node acyclic fixture to `/api/v1/controlled-wasm/dgraph`;
2. require activation success and manifest dependency plan `stage_count=3`, `dependency_edge_count=2`;
3. require assignment metadata stages `0,1,2` and expected dependency IDs;
4. GET `/api/v1/controlled-wasm/observed-dgraph` before any execution and require module stage metadata plus zero reports;
5. reset to fresh state and POST cycle fixture -> HTTP 400;
6. reset to fresh state and POST missing-dependency fixture -> HTTP 400;
7. require no live WASM reports and no governed execution.

## Non-claims

I40 does not provide:

- automatic stage execution;
- distributed barrier synchronization between Sentinels;
- lifecycle/retry state (I41);
- resource-aware placement (I42);
- readiness/governance authorization (I43);
- canonical automatic governed execution.

Checkpoint remains `145/150` until regression and the dependency-stage gates pass.