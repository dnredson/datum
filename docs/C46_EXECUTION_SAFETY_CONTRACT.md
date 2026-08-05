# C46 Execution Safety Contract

## Status

Completed and validated as the `118/130` increment on 2026-08-04. The Rust formatting check,
the seven focused execution-safety tests, the complete 381-test dserver suite, the live restored
three-node baseline check, and the repository checks passed. The contract remains fail-closed and
does not authorize or execute any operation.

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

## Validation Evidence

The final validated branch head was:

```text
5eea32d3d3cde0b7e64ad65c3a1348e8614b6d1d
```

Offline validation on Windows from the `dserver` crate passed:

- `cargo fmt --check`;
- `cargo test dgraph_execution_safety -- --nocapture`: 7 passed, 0 failed;
- `cargo test`: 381 passed, 0 failed;
- `git diff --check origin/main...HEAD`;
- `git status --short`: clean worktree.

The focused tests covered the empty-project fail-closed case, explicit denial of every operation
class, preservation of the non-authoritative readiness/planner semantics, and source guards against
execution or mutation paths.

The endpoint was then checked live against the restored `smart-city-campinas` three-node baseline:

```powershell
$headers = @{ "X-Datum-Project" = "smart-city-campinas" }
$c = Invoke-RestMethod `
  -Uri "http://192.168.10.101:8080/api/v1/dgraph/execution-safety" `
  -Headers $headers
```

The response reported `readiness_status=ready`, `source_snapshot_count=3`, three placement
proposals, zero warnings, blockers, degraded flows, or reconfiguration proposals, while still
returning:

- `decision=denied`;
- `authorization_granted=false`;
- all seven gates with `satisfied=false`;
- all eight operations with `authorized=false`;
- `evidence.planner_output_is_advisory=true`;
- `side_effects_performed=false`;
- `metadata.execution_performed=false`;
- `metadata.readiness_is_authorization=false`;
- `metadata.planner_proposals_are_authorization=false`.

The PowerShell assertions completed with `Baseline C46 PASS`. The live response was collected on
the immediately preceding implementation head; the final commit only applied the exact `rustfmt`
layout change, and the full offline validation was repeated successfully on the final head above.

## Completed Validation Checklist

- dserver Rust tests for `dgraph_execution_safety`: passed;
- complete dserver test suite: passed;
- `cargo fmt --check`: passed;
- `git diff --check`: passed;
- empty-project fail-closed behavior: passed in the focused Rust test;
- restored three-node baseline endpoint JSON: passed live;
- every authorization, gate, operation, and side-effect invariant: passed;
- source-level guards confirm that the read endpoint has no store, executor, or mutation path.

This evidence advances the project checkpoint from `117/130` to `118/130`. No later increment is
started by this record.
