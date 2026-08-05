# C52 Fail-Closed Resource Limits Contract

## Status

Candidate increment at `123/130`. The implementation is prepared for local repository checks, Rust tests, and strict live validation. This record does not advance the validated checkpoint.

## Purpose

C52 formalizes the sixth execution-safety gate without loading or enforcing execution resource limits. It exposes the absence of project-scoped fuel, timeout, memory, and host-capability limits through a deterministic fail-closed contract.

This increment defines an authorization boundary; it does not implement limit storage, runtime instrumentation, resource reservation, timeout enforcement, host-capability enforcement, authorization, or execution.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/resource-limits
X-Datum-Project: <project-id>
```

The project header only scopes and labels the response. The endpoint does not read or write Firebase, snapshots, runtime reports, planner output, local files, network resources, executor state, or host-resource state.

## Candidate Contract Semantics

The candidate response contains:

- `contract_version=0.1.0`;
- `limits_status=not_enforced`;
- configuration and loading flags set to `false`;
- fuel, timeout, and memory configuration flags set to `false`;
- fuel, timeout, and memory limit values set to `null`;
- host-capability limits unloaded and empty;
- `limits_enforced=false`;
- `authorization_granted=false`;
- `side_effects_performed=false`;
- metadata stating that planner metadata is not a resource-limit policy, runtime capability reports are not enforced limits, no host resources were reserved, no state was persisted, no execution occurred, and the C46 resource-limit gate remains unsatisfied.

## Separation From Existing Metadata

Resource fields present in D-Graph snapshots, planner proposals, runtime reports, or node capability metadata describe observations or claims. Their presence does not prove that an execution budget was configured, loaded, bound to an operation, or enforced by an executor.

A future enforcement phase must load explicit project-scoped limits, validate them against trusted target capabilities, bind them to the requested operation and selected target, enforce fuel, time, memory, and host constraints in the executor, and preserve enforcement evidence. Until then, all limit values remain absent or empty and non-authoritative.

## C46 Relationship

C52 does not satisfy `resource_limits`. No project-scoped limits are configured or loaded, and no runtime enforcement occurs. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `resource_limits.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

Even future successful resource-limit enforcement would satisfy only one gate and would not independently authorize an operation.

## Safety Boundary

C52 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- load, derive, persist, or mutate resource limits;
- derive enforceable policy from snapshots, capability reports, planner candidates, module descriptors, runtime reports, or host observations;
- reserve CPU, memory, storage, network, runtime, container, or WASM resources;
- configure fuel, timers, memory limiters, cgroups, quotas, capabilities, or process limits;
- query nodes, runtimes, containers, WASM engines, files, or network endpoints;
- inspect or mutate Docker, WASM, mappings, or configuration;
- execute placement, recovery, migration, reconfiguration, deployment, installation, dispatch, or WASM;
- invoke health checks or collection cycles;
- add AI or LLM orchestration.

## Candidate Validation Checklist

The checkpoint remains `123/130` until all of the following complete:

- `cargo fmt --check`;
- all 5 focused C52 resource-limits tests;
- all 7 focused C46 execution-safety tests;
- the complete dserver suite;
- `git diff --check origin/main...HEAD`;
- a clean `git status --short`;
- strict live resource-limits and execution-safety assertions against `smart-city-campinas` ending with `C52 live validation PASS`.

No test or live result is claimed by this candidate record.
