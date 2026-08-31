# I40 — Dependency-Aware Deployment Stages — Final

- checkpoint: `I40/146`
- predecessor: `I39/145`
- status: `validated`
- branch: `agent/i40-dependency-aware-stages`

## Closure

I40 is validated. dserver now derives deterministic deployment stages from explicit D-Graph deployment-dependency semantics without enabling automatic execution.

A flow edge participates in deployment ordering only when the flow has `flow_kind=deployment_dependency` or the edge explicitly carries `metadata.deployment_dependency=true`. Ordinary application/data flows therefore do not silently become deployment prerequisites.

## Regression

```text
dserver fmt: PASS
dserver test: 500 passed; 0 failed
dserver build: PASS
```

## Three-node dependency plan

The validated fixture produced:

```text
assignment_count=3
dependency_edge_count=2
stage_count=3

stage 0 -> a-fog-scale       -> fog-01
stage 1 -> m-local-aggregate -> local-01 -> depends_on a-fog-scale
stage 2 -> z-cloud-delta     -> cloud-01 -> depends_on m-local-aggregate
```

The same assignment/stage/dependency metadata was materialized into the observed D-Graph modules before any WASM execution.

Observed state evidence:

```text
report_count=0
validation_errors=0
fog-scale-module       -> a-fog-scale       -> stage 0
local-aggregate-module -> m-local-aggregate -> stage 1 -> a-fog-scale
cloud-delta-module     -> z-cloud-delta     -> stage 2 -> m-local-aggregate
```

## Fail-closed gates

Cycle fixture:

```text
HTTP 400
error: deployment dependency graph contains a cycle involving assignments: a-assignment, b-assignment
```

Missing deployable dependency fixture:

```text
HTTP 400
error: deployment dependency edge 'missing-upstream' in flow 'missing-deployable-upstream' references upstream module 'external-upstream' without a deployable assignment
```

Controlled live reports remained:

```text
count=0
```

No artifact acquisition, guest execution, lifecycle transition, or governed automatic execution was introduced by I40.

## Materialized contract

D-Graph-derived assignment metadata now carries:

```text
deployment_stage
depends_on_assignments
```

Manifest metadata carries an aggregate `dependency_plan`, and observed D-Graph module/graph metadata preserves the same planning evidence.

## Safety boundary

I40 provides planning and materialization only. It does not provide automatic stage execution, distributed barriers, lifecycle retry, resource-aware placement, readiness authorization, cryptographic Sentinel identity, or canonical governed execution.

## Next

`I41/147` adds deployment lifecycle and explicit retry evidence: discovered/prepared/executing/executed/failed/blocked state, attempt identity, retained prior attempts, and retry lineage without hidden automatic retries.
