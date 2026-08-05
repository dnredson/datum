# C55 Prerequisite-Backed Execution Safety Decision Contract

## Status

Candidate increment at `126/130`. Implementation and documentation are prepared for formatting,
focused Rust tests, the complete dserver suite, repository checks, and strict live validation. This
record does not advance the validated checkpoint before those checks pass.

## Purpose

C55 removes the remaining parallel gate construction between the C46 execution-safety decision and
the C54 execution-prerequisite aggregation. C46 now projects its seven gate results directly from
the C47-C53 evidence aggregated by C54.

This makes the source of every C46 gate explicit and prevents the decision boundary from silently
diverging from the prerequisite boundary. The change does not implement an allow transition:
prerequisite aggregation remains evidence only, and the execution-safety decision remains denied.

## Existing Read-Only Endpoint

```text
GET /api/v1/dgraph/execution-safety
X-Datum-Project: <project-id>
```

The endpoint continues to use the existing readiness read path for observational graph and planner
evidence. C55 additionally calls the pure C54 prerequisite builder for the same project identifier.
It does not load external policy, approval, artifact, allowlist, capability, limit, rollback, or
audit state.

## Candidate Contract Semantics

The C46 response keeps its existing fields and adds:

- `prerequisite_summary.aggregation_status=incomplete`;
- `prerequisite_summary.required_gate_count=7`;
- `prerequisite_summary.satisfied_gate_count=0`;
- `prerequisite_summary.all_prerequisites_satisfied=false`;
- `prerequisite_summary.aggregation_is_authorization=false`;
- `metadata.prerequisite_aggregation_is_authorization=false`;
- `metadata.execution_allow_transition_implemented=false`.

The seven `gates` are projected from the C54 prerequisite entries in the same order and preserve
each gate name, satisfied flag, and deterministic blocking reason. C46 no longer maintains a
separate hard-coded reason table.

## Fail-Closed Decision

C55 deliberately keeps:

- `decision=denied`;
- `authorization_granted=false`;
- all seven gates unsatisfied;
- all eight operation classes unauthorized;
- readiness and planner proposals observational only;
- prerequisite aggregation non-authoritative;
- `side_effects_performed=false`;
- `metadata.execution_performed=false`.

Even a future complete prerequisite summary must not become authorization implicitly. A separate,
explicitly designed and validated allow transition remains required outside this metadata-only
phase.

## Safety Boundary

C55 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

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

## Pending Validation

Validate the candidate implementation head with:

- `cargo fmt --check`;
- `cargo test c55_ -- --nocapture`: expected 5 focused C55 tests;
- `cargo test dgraph_execution_prerequisites -- --nocapture`: expected 5 C54 tests;
- `cargo test dgraph_execution_safety -- --nocapture`: expected 12 C46/C55 tests;
- `cargo test`: expected 426 total dserver tests;
- `git diff --check origin/main...HEAD`;
- `git status --short`;
- strict live assertions against both `execution-prerequisites` and `execution-safety`.

The validated checkpoint remains `126/130` until all offline, repository, and live checks pass.
