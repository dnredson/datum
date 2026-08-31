# I40 — Dependency-Aware Deployment Stages

- checkpoint: `I40/146`
- predecessor: `I39/145`
- status: `validated`
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

## Regression — PASS

```text
dserver fmt: PASS
dserver test: 500 passed; 0 failed
dserver build: PASS
```

## Live validation — PASS

Three-node acyclic fixture:

```text
assignment_count=3
dependency_edge_count=2
stage_count=3
stage 0 -> a-fog-scale       -> fog-01
stage 1 -> m-local-aggregate -> local-01 -> a-fog-scale
stage 2 -> z-cloud-delta     -> cloud-01 -> m-local-aggregate
```

Observed D-Graph materialized the same assignment/stage/dependency metadata with `report_count=0` and zero validation errors.

Fail-closed fixtures:

```text
cycle -> HTTP 400
missing deployable upstream dependency -> HTTP 400
```

Final controlled live-report count remained `0`.

## Focused regression tests

I40 adds five dserver tests:

```text
acyclic three-node plan -> deterministic 3 stages
cycle -> rejected
marked dependency with missing deployable assignment -> rejected
ordinary data flow -> ignored for deployment ordering
edge metadata can explicitly mark a dependency in a mixed flow
```

## Fixtures

- `scripts/fixtures/i40-dependency-three-node-dgraph.json`
- `scripts/fixtures/i40-dependency-cycle-dgraph.json`
- `scripts/fixtures/i40-dependency-missing-dgraph.json`

## Non-claims

I40 does not provide automatic stage execution, distributed barrier synchronization between Sentinels, lifecycle/retry state, resource-aware placement, readiness/governance authorization, or canonical automatic governed execution.

Closure evidence: `docs/I40_DEPENDENCY_AWARE_DEPLOYMENT_STAGES_FINAL.md`.
