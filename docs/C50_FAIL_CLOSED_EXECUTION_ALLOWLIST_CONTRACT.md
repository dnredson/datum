# C50 Fail-Closed Execution Allowlist Contract

## Status

Completed increment `122/130`. The implementation, repository checks, Rust suites, and live fail-closed assertions passed on the final implementation head. Progress is `122/130`.

## Purpose

C50 formalizes the fourth execution-safety gate without loading or enforcing any allowlist. It exposes the absence of project-scoped target, module, runtime, and function allowlists through a deterministic fail-closed contract.

This increment defines an authorization boundary; it does not implement allowlist storage, matching, wildcard expansion, target resolution, module or runtime lookup, function selection, authorization, or execution.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/execution-allowlists
X-Datum-Project: <project-id>
```

The project header only scopes and labels the response. The endpoint does not read or write Firebase, snapshots, configuration, registries, target inventories, module descriptors, runtime reports, function catalogs, local files, network resources, or executor state.

## Contract Semantics

The validated response contains:

- `contract_version=0.1.0`;
- `evaluation_status=not_evaluated`;
- `allowlist_configured=false`;
- target, module, runtime, and function allowlist-loaded flags set to `false`;
- empty target, module, runtime, and function allowlists;
- `authorization_granted=false`;
- `side_effects_performed=false`;
- metadata stating that D-Graph entities are not allowlist entries, wildcard entries are not allowed, no state was persisted, no execution occurred, and the C46 allowlist gate remains unsatisfied.

## Separation From D-Graph Metadata

Nodes, modules, runtimes, flows, capabilities, planner candidates, and artifact references in D-Graph describe observed or desired topology. Their presence does not place them on an execution allowlist.

A future enforcement phase must load explicit project-scoped allowlist entries and match the exact target, module, runtime, and function requested by an operation. Until then, every list remains empty and non-authoritative.

## C46 Relationship

C50 does not satisfy `target_and_function_allowlists`. No target, module, runtime, or function entry is loaded or matched. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `target_and_function_allowlists.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

Even future allowlist matching would satisfy only one gate and would not independently authorize an operation.

## Safety Boundary

C50 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- load or persist allowlist configuration;
- derive permissions from nodes, modules, runtimes, flows, capabilities, planner output, or artifact references;
- resolve targets, inspect installed modules, query runtimes, or enumerate functions;
- support wildcard or implicit allowlist entries;
- inspect or mutate Docker, WASM, files, mappings, or configuration;
- execute placement, recovery, migration, reconfiguration, deployment, installation, dispatch, or WASM;
- invoke health checks or collection cycles;
- add AI or LLM orchestration.

## Validation Evidence

Validated on implementation head `50ab4413fd1f7d935c346c40e2ac929e2a6905de`:

- `cargo fmt --check` passed;
- all 5 focused C50 execution-allowlist tests passed;
- all 7 focused C46 execution-safety tests passed;
- the complete dserver suite passed with 401/401 tests;
- `git diff --check origin/main...HEAD` produced no output;
- `git status --short` confirmed a clean worktree.

Live validation against project `smart-city-campinas` passed through a strict PowerShell block that cleared prior response variables and stopped on request or assertion errors. The allowlist endpoint returned `contract_version=0.1.0`, `evaluation_status=not_evaluated`, `allowlist_configured=false`, all four loaded flags false, all four lists empty, no authorization, no persistence, no execution, and no side effects. D-Graph entities remained non-authoritative and wildcard entries remained disabled.

The paired execution-safety response remained `decision=denied`. Exactly one `target_and_function_allowlists` gate was present and unsatisfied, no gate was satisfied, no operation was authorized, and no execution or side effect was reported.

C50 therefore completes the read-only fail-closed allowlist boundary without adding allowlist enforcement or any execution path. Progress advances to `122/130`.
