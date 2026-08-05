# C51 Fail-Closed Capability Negotiation Contract

## Status

Candidate increment for `123/130`. Implementation and validation are pending on the candidate branch. The validated checkpoint remains `122/130`.

## Purpose

C51 formalizes the fifth execution-safety gate without loading capability requirements, reading capability advertisements, evaluating compatibility, or performing negotiation. It exposes the absence of project-scoped execution capability negotiation through a deterministic fail-closed contract.

This increment defines an authorization boundary; it does not implement requirement storage, capability discovery, protocol exchange, matching, target selection, authorization, or execution.

## Read-Only Endpoint

```text
GET /api/v1/dgraph/capability-negotiation
X-Datum-Project: <project-id>
```

The project header only scopes and labels the response. The endpoint does not read or write Firebase, snapshots, node capability reports, module descriptors, runtime reports, planner output, local files, network resources, or executor state.

## Candidate Contract Semantics

The candidate response contains:

- `contract_version=0.1.0`;
- `negotiation_status=not_negotiated`;
- requirement-loaded and advertisement-loaded flags set to `false`;
- empty required, advertised, and matched capability collections;
- `compatibility_evaluated=false`;
- `negotiation_succeeded=false`;
- `authorization_granted=false`;
- `side_effects_performed=false`;
- metadata stating that D-Graph capability metadata is not a negotiation, legacy node capability reports are not execution authority, no network exchange occurred, no state was persisted, no execution occurred, and the C46 capability gate remains unsatisfied.

## Separation From Existing Capability Metadata

Capabilities present in D-Graph snapshots or exposed through the existing node capability APIs describe reported metadata. Their presence does not prove that an operation's requirements were loaded, that a target advertised a compatible execution environment, or that a negotiation succeeded.

A future enforcement phase must load explicit project-scoped requirements and trusted target advertisements, evaluate exact compatibility, bind the result to the requested operation and target, and preserve evidence of the negotiation. Until then, all capability collections remain empty and non-authoritative.

## C46 Relationship

C51 does not satisfy `capability_negotiation`. No requirements or advertisements are loaded, no compatibility is evaluated, and no negotiation succeeds. C46 therefore remains:

- `decision=denied`;
- `authorization_granted=false`;
- `capability_negotiation.satisfied=false`;
- all eight operations unauthorized;
- evaluation-only and side-effect-free.

Even future successful capability negotiation would satisfy only one gate and would not independently authorize an operation.

## Safety Boundary

C51 remains metadata-only, read-only, evaluation-only, and dry-run. It does not:

- load or persist capability requirements or advertisements;
- derive negotiation success from D-Graph snapshots, node capability reports, planner candidates, module descriptors, or runtime reports;
- query nodes, runtimes, containers, WASM engines, files, or network endpoints;
- perform handshakes, matching, protocol exchange, reservation, or target binding;
- inspect or mutate Docker, WASM, mappings, or configuration;
- execute placement, recovery, migration, reconfiguration, deployment, installation, dispatch, or WASM;
- invoke health checks or collection cycles;
- add AI or LLM orchestration.

## Validation Plan

Validation must confirm:

- `cargo fmt --check`;
- all 5 focused C51 capability-negotiation tests;
- all 7 focused C46 execution-safety tests;
- the complete dserver suite;
- `git diff --check origin/main...HEAD` with no output;
- clean `git status --short`;
- strict live capability-negotiation and execution-safety assertions against `smart-city-campinas`.

The checkpoint advances to `123/130` only after repository, Rust, and strict live validation complete successfully.
