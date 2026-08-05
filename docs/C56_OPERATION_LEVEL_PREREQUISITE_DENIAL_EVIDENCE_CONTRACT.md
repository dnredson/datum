# C56 Operation-Level Prerequisite Denial Evidence Contract

## Status

Candidate increment at `127/130`. Formatting, focused Rust tests, the complete dserver suite,
repository checks, and strict live validation remain pending on the final branch head.

## Purpose

C56 makes every C46 operation denial traceable to the prerequisite evidence projected by C55.
Each of the eight operation entries now reports the prerequisite counts and the ordered list of
currently blocking gates used by the execution-safety decision.

This is an additive explanation layer only. It does not satisfy any prerequisite, implement the
allow transition, grant authorization, or invoke an executor.

## Existing Read-Only Endpoint

```text
GET /api/v1/dgraph/execution-safety
X-Datum-Project: <project-id>
```

No new endpoint or state source is introduced. The endpoint continues to build readiness evidence,
the C54 prerequisite aggregation, and the C55 prerequisite-backed safety decision in memory.

## Candidate Contract Semantics

Each item in `operations` keeps its existing `operation`, `authorized`, and `reason` fields and
adds:

- `required_gate_count=7`;
- `satisfied_gate_count=0`;
- `blocking_gates` containing the seven unsatisfied gate names in canonical C54 order.

The response metadata additionally reports:

- `operation_denials_are_prerequisite_backed=true`;
- `execution_allow_transition_implemented=false`;
- `execution_performed=false`.

The eight operation classes remain:

1. `placement`;
2. `recovery`;
3. `migration`;
4. `reconfiguration`;
5. `deployment`;
6. `installation`;
7. `dispatch`;
8. `wasm_execution`.

## Fail-Closed Decision

C56 deliberately keeps:

- `decision=denied`;
- `authorization_granted=false`;
- all seven gates unsatisfied;
- all eight operation classes unauthorized;
- all eight operation entries blocked by the same seven canonical prerequisites;
- readiness and planner proposals observational only;
- prerequisite aggregation non-authoritative;
- `side_effects_performed=false`;
- `metadata.execution_performed=false`.

The blocking-gate list explains the current denial; it is not an authorization rule. Even an empty
future blocking list must not imply permission while the explicit allow transition remains
unimplemented.

## Safety Boundary

C56 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- configure, load, or persist execution policy;
- authenticate an operator or accept an approval;
- read, hash, verify, transfer, or execute artifacts;
- load or match allowlists;
- negotiate capabilities;
- reserve resources or enforce limits;
- create checkpoints, perform rollback, or write audit records;
- authorize or execute placement, recovery, migration, reconfiguration, deployment, installation,
  dispatch, or WASM;
- invoke Docker, WASM engines, health checks, collection cycles, AI, or LLM orchestration.

## Planned Validation

Validate the final implementation head with:

- `cargo fmt --check`;
- all 5 focused C56 tests;
- all focused C46/C55/C56 execution-safety tests;
- the complete dserver suite, expected to contain 431 tests;
- `git diff --check origin/main...HEAD`;
- `git status --short`;
- strict live assertions against `execution-prerequisites` and `execution-safety` for
  `smart-city-campinas`.

The live validation must prove that all eight operations remain unauthorized, each reports
`required_gate_count=7`, `satisfied_gate_count=0`, and the same seven ordered blockers projected
from C54/C55, while authorization and all side-effect flags remain false.

The checkpoint remains `127/130` until all validations pass and their evidence is recorded.
