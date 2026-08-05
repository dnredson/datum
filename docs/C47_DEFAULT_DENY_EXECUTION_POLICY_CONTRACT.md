# C47 Default-Deny Execution Policy Contract

## Status

Implemented as the candidate `119/130` increment. Validation is pending, so the project checkpoint
remains `118/130` and this increment must remain draft and unmerged.

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
installation, dispatch, and WASM execution. C46 now imports this canonical list so the safety and
policy contracts cannot silently diverge.

## C46 Relationship

C47 does not satisfy `explicit_execution_policy`. The built-in fallback exists precisely because
no explicit, authenticated, project-scoped policy is configured. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `explicit_execution_policy.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

The C46 gate reason now distinguishes the implemented built-in deny-all fallback from a future
explicit project policy.

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

## Required Validation

From the `dserver` crate on the final branch head:

```text
cargo fmt --check
cargo test dgraph_execution_policy -- --nocapture
cargo test dgraph_execution_safety -- --nocapture
cargo test
git diff --check origin/main...HEAD
git status --short
```

Live validation must confirm both the new policy endpoint and the unchanged C46 denial:

- policy endpoint: fallback source, unconfigured policy, deny-all mode, eight deny rules, execution
  disabled, no authorization, persistence, execution, or side effects;
- safety endpoint: explicit-policy gate still unsatisfied, decision denied, eight unauthorized
  operations, and no execution or side effects.

Only after these checks pass may C47 be recorded as completed and the checkpoint advance from
`118/130` to `119/130`.
