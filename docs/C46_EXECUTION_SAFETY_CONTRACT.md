# C46 Execution Safety Contract

## Status

Implementation draft for the `118/130` increment. The project checkpoint remains `117/130`
until the Rust test suite and contract validation pass on a checkout with the Rust toolchain.

## Purpose

C46 creates a separate, fail-closed contract between read-only D-Graph planning and any future
executed orchestration. It prevents a healthy readiness result, a candidate node, a placement
proposal, or a reconfiguration proposal from being interpreted as execution authorization.

This increment does not add an executor. It records why execution is denied.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/execution-safety
X-Datum-Project: <project-id>
```

The endpoint reads the latest local D-Graph snapshots through the existing read path and derives
readiness and planner evidence. It does not persist the response and does not mutate snapshots,
the materialized graph, the planner output, dispatch, tasks, claims, mappings, or configuration.

## Contract Semantics

The response contains:

- `contract_version=0.1.0`;
- `decision=denied`;
- `authorization_granted=false`;
- observational evidence copied from the readiness/planner summaries;
- an explicit list of unsatisfied execution gates;
- an explicit denial for every in-scope operation class;
- `side_effects_performed=false`;
- metadata stating that readiness and planner proposals are not authorization and that no
  execution occurred.

The contract is fail-closed for empty, incomplete, ready, ready-with-warnings, and blocked graph
states. A `readiness_status=ready` is evidence that the metadata pipeline is coherent; it is not
evidence that an operational action is safe or authorized.

## Explicitly Denied Operations

- placement;
- recovery;
- migration;
- reconfiguration;
- deployment;
- installation;
- dispatch execution;
- WASM execution.

## Unsatisfied Gates

C46 reports these gates as unsatisfied rather than attempting to implement them:

- explicit execution policy;
- authenticated, project-scoped operator approval;
- artifact integrity and provenance verification;
- target, module, runtime, and function allowlists;
- execution capability negotiation;
- fuel, timeout, and host-capability limits;
- rollback guarantees and execution audit trail.

No individual gate can authorize an action in C46. There is no allow transition, override,
approval write, or execution method in this increment.

## Safety Boundary

C46 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- call `dispatch --execute`;
- execute planner decisions;
- start placement, recovery, migration, or reconfiguration;
- deploy or install modules;
- inspect, pull, start, stop, or remove Docker resources;
- load or execute WASM artifacts;
- transfer artifacts;
- write local mappings or configuration;
- run health checks or collection cycles;
- add AI or LLM orchestration.

## Validation Required Before `118/130`

- run the dserver Rust tests for `dgraph_execution_safety`;
- run the complete dserver test suite;
- run `cargo fmt --check`;
- run `git diff --check`;
- inspect the endpoint JSON for both an empty project and the restored three-node baseline;
- confirm `decision=denied`, `authorization_granted=false`, every operation `authorized=false`,
  every gate `satisfied=false`, and `side_effects_performed=false`;
- confirm no state changes before and after each read.

Only after those checks pass should the checkpoint advance from `117/130` to `118/130`.
