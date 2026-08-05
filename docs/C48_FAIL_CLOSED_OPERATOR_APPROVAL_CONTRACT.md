# C48 Fail-Closed Operator Approval Contract

## Status

Completed and validated increment `120/130`.

On the final implementation head `5f7cdd7a446d6c25e3785aec480068de4f302a8e`,
formatting, focused and full Rust tests, repository checks, and live endpoint validation passed.
The contract remains fail-closed, read-only, evaluation-only, and side-effect-free.

## Purpose

C48 formalizes the second execution-safety gate without authenticating an operator or granting
execution authority. It exposes the effective absence of an authenticated, project-scoped execution
approval through a deterministic fail-closed contract.

This increment defines an approval boundary; it does not implement authentication, approval
administration, approval persistence, authorization, or an executor.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/operator-approval
X-Datum-Project: <project-id>
```

The endpoint requires the project header only to scope and label the response. It does not read or
write Firebase, snapshots, configuration, identities, credentials, tokens, approvals, tasks,
claims, mappings, artifacts, or local files.

## Contract Semantics

The response contains:

- `contract_version=0.1.0`;
- `authentication_status=not_evaluated`;
- `identity_authenticated=false`;
- no operator identity;
- `approval_status=missing`;
- `approval_present=false`;
- no approval identifier;
- `approval_valid=false`;
- an empty approval scope;
- `authorization_granted=false`;
- `side_effects_performed=false`;
- metadata stating that no project-scoped approval was loaded, nothing was persisted, no execution
  occurred, and the C46 operator-approval gate remains unsatisfied.

## Separation From Legacy Installation-Task Approvals

The existing installation-task approval records belong to the earlier assisted-execution workflow.
They can carry operator-provided labels, but they do not prove an authenticated identity and are
scoped to an individual installation task. C48 therefore does not read, translate, aggregate, or
reuse them as D-Graph execution authorization.

The contract reports both
`legacy_installation_task_approvals_evaluated=false` and
`legacy_installation_task_approvals_are_authorization=false`.

## C46 Relationship

C48 does not satisfy `authenticated_operator_approval`. No authentication source or valid
project-scoped approval is evaluated. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `authenticated_operator_approval.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

Even a future valid approval would satisfy only one gate and would not independently authorize an
operation.

## Safety Boundary

C48 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- accept credentials, tokens, signatures, or identity assertions;
- authenticate an operator;
- accept, persist, revoke, or expire an execution approval;
- reuse legacy installation-task approval records as global authorization;
- execute placement, recovery, migration, or reconfiguration;
- deploy, install, dispatch, load, or execute modules;
- inspect or mutate Docker, WASM, files, mappings, or configuration;
- transfer artifacts;
- invoke health checks or collection cycles;
- add AI or LLM orchestration.

## Completed Validation

The final implementation head `5f7cdd7a446d6c25e3785aec480068de4f302a8e` passed:

- `cargo fmt --check`;
- all 5 focused C48 operator-approval tests;
- all 7 focused C46 execution-safety tests;
- all 391 dserver tests;
- `git diff --check origin/main...HEAD`;
- a clean `git status --short`;
- live operator-approval and execution-safety assertions, ending with
  `C48 live validation PASS`.

The live C48 response reported no evaluated authentication, no authenticated identity, no
operator or approval identifier, no approval presence or validity, an empty approval scope, no
authorization, no persisted state, and no side effects. The live C46 response remained
`decision=denied`; all seven gates were unsatisfied, all eight operations were unauthorized, and
no execution or side effects were reported.

Progress is `120/130`. No later increment is started by this validation record.
