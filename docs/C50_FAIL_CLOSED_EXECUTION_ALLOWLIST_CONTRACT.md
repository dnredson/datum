# C50 Fail-Closed Execution Allowlist Contract

## Status

Candidate increment `122/130`. Implementation is published for local and live validation. Progress remains `121/130` until the complete validation evidence is recorded.

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

The candidate response contains:

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

## Planned Validation

Validate on the final implementation head with:

- `cargo fmt --check`;
- all 5 focused C50 execution-allowlist tests;
- all 7 focused C46 execution-safety tests;
- the complete dserver suite;
- `git diff --check origin/main...HEAD`;
- a clean `git status --short`;
- live execution-allowlist assertions confirming empty lists, no configuration, no authorization, persistence, execution, or side effects;
- live execution-safety assertions confirming `decision=denied`, the allowlist gate unsatisfied, all seven gates unsatisfied, all eight operations unauthorized, and no execution or side effects.

No completion or progress advancement is claimed before this evidence passes.
