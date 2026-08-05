# C47 Default-Deny Execution Policy Contract

## Status

Completed and validated as increment `119/130`.

The final implementation head `884d2e460bc716af4ef675a2f578eafe44fe71b0` passed formatting,
focused and full Rust tests, repository checks, and live validation. The subsequent record update changes
documentation only and does not alter runtime semantics.

## Purpose

C47 formalizes the first execution-safety gate without granting execution authority. It exposes
the effective fallback policy that applies while no authenticated, project-scoped execution policy
exists. The fallback is built in, deterministic, and deny-only.

This increment defines a policy contract; it does not implement policy administration, policy
persistence, an allow decision, or an executor.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/execution-policy
X-Datum-Project: <project-id>
```

The endpoint requires the project header only to scope and label the response. It does not read or
write Firebase, snapshots, configuration, approvals, claims, tasks, mappings, artifacts, or local
files.

## Contract Semantics

The response contains:

- `contract_version=0.1.0`;
- `policy_source=builtin_fail_closed_fallback`;
- `policy_configured=false`;
- `policy_mode=deny_all`;
- `default_decision=deny`;
- `execution_enabled=false`;
- one explicit `deny` rule for each of the eight C46 operation classes;
- `authorization_granted=false`;
- `side_effects_performed=false`;
- metadata stating that no project policy was loaded, nothing was persisted, no execution occurred,
  and the C46 explicit-policy gate remains unsatisfied.

The shared operation classes are placement, recovery, migration, reconfiguration, deployment,
installation, dispatch, and WASM execution. C46 imports this canonical list so the safety and
policy contracts cannot silently diverge.

## C46 Relationship

C47 does not satisfy `explicit_execution_policy`. The built-in fallback exists precisely because
no explicit, authenticated, project-scoped policy is configured. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `explicit_execution_policy.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

The C46 gate reason distinguishes the implemented built-in deny-all fallback from a future explicit
project policy.

## Safety Boundary

C47 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- accept or persist policy definitions;
- support an `allow` rule or an enable transition;
- authenticate an operator or record approval;
- execute placement, recovery, migration, or reconfiguration;
- deploy, install, dispatch, load, or execute modules;
- inspect or mutate Docker, WASM, files, mappings, or configuration;
- transfer artifacts;
- invoke health checks or collection cycles;
- add AI or LLM orchestration.

## Validation Evidence

Validation on the final implementation head passed:

- `cargo fmt --check`;
- all 5 focused C47 execution-policy tests;
- all 7 focused C46 execution-safety tests;
- all 386 dserver tests;
- `git diff --check origin/main...HEAD`;
- clean `git status --short`.

Live validation used project `smart-city-campinas` against the restored three-node baseline. The
policy endpoint returned the built-in fallback with `policy_configured=false`, `deny_all`,
`default_decision=deny`, execution disabled, eight deny rules, no authorization, and no side
effects. The safety endpoint remained `decision=denied`, with all seven gates unsatisfied, all
eight operations unauthorized, and no reported execution or side effects. The PowerShell
assertions completed with `C47 live validation PASS`.

C47 is therefore complete at `119/130`. No later increment is started by this record.
