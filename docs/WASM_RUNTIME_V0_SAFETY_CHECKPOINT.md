# WASM Runtime v0 Safety Checkpoint

## Purpose

This document records the current safety checkpoint for WASM Runtime v0 before any future integration with installation tasks or assisted execution.

It exists to lock in the current boundary, summarize the completed WASM v0 milestones, and define the minimum gates required before any broader execution role is considered.

## Completed WASM v0 Scope

- `W1/80`: DATUM isolated embedded WASM add runner
- `W2/80`: DATUM local WASM runtime report model
- `W3/80`: dserver metadata-only WASM runtime report contract
- `W4/80`: DATUM explicit WASM report publisher and latest report viewers
- `I19/80`: repo-root smoke coverage for the WASM report chain

Current implemented flow:

- DATUM can run an isolated embedded add test locally
- DATUM can render the local WASM runtime report in text or JSON
- DATUM can publish the report to dserver as metadata only
- dserver can store and return the latest report for a node or project
- the repo-root smoke test validates the local report, publish path, server read-back, and DATUM latest-report viewers
- `W5/80` adds explicit execution limits metadata for the embedded runner, including fuel and timeout configuration
- `W6/80` adds a metadata-only WASM module registry contract on dserver
- the runner still uses only embedded test WASM
- no artifact loading or task integration was added
- the registry does not store binaries, fetch artifacts, or execute modules
- all module safety fields remain false in v0

## Current Hard Safety Guarantees

- WASM runtime is DATUM-local only.
- Current WASM module is embedded/test-only.
- No external WASM artifact download exists.
- No filesystem access from WASM.
- No network access from WASM.
- No host command execution from WASM.
- No Docker access from WASM.
- No artifact transfer.
- No installation task integration.
- No assisted execution claim/request integration.
- No migration execution.
- Reports are metadata-only.
- dserver never executes WASM.

All current WASM safety/access flags remain false:

- `safe_to_auto_execute`
- `side_effects_performed`
- `filesystem_access`
- `network_access`
- `host_command_access`
- `docker_access`
- `artifact_access`
- `installation_task_access`
- `assisted_execution_access`

These guarantees mean the current WASM runtime is a sandboxed execution-boundary prototype only. It is not a remediation executor, installer, artifact runner, or host integration surface.

## Out Of Scope For Current WASM v0

- no D-Code deployment
- no dynamic artifact loading
- no signed artifact verification
- no module registry
- no WASI filesystem integration
- no network capability
- no host command capability
- no connection to installation tasks
- no connection to assisted execution attempts
- no real remediation execution

## Required Gates Before Connecting WASM To Assisted Execution

Before WASM is connected to installation tasks, assisted execution state, or any real remediation path, the project needs:

- explicit policy model for WASM-capable tasks
- signed or hashed WASM artifacts
- artifact provenance metadata
- allowlist of permitted module IDs/functions
- resource limits/fuel/timeout
- no default host capabilities
- explicit capability negotiation
- operator approval
- readiness check
- active claim
- no automatic retry loops
- reporting side effects separately
- rollback/compensation plan before real remediation

These are minimum safety gates, not optional enhancements.

## Next Safe Options

### Option W5: WASM runtime execution limits

- add timeout/fuel/resource limit configuration to the local runner
- keep the embedded test module only
- keep task integration out of scope

Risk profile:

- lowest-risk next step
- strengthens the sandbox boundary without broadening capability

### Option W6: WASM module registry metadata contract

- let dserver store metadata about allowed modules
- do not distribute artifacts
- do not execute modules

Risk profile:

- moderate metadata-model expansion
- still safe if artifact distribution and execution remain out of scope

### Option W7: DATUM local WASM fixture loader

- load a local fixture file from a controlled test directory
- keep execution local to DATUM
- keep task integration out of scope

Risk profile:

- higher risk than the embedded module
- introduces local file-based module loading, so it should come after execution limits and policy groundwork

### Option S/W Integration Later

- connect WASM reports to assisted execution state only after policy and artifact model exist

Risk profile:

- should happen later than `W5`
- should not begin until the artifact, policy, approval, and claim model are explicit

## Recommendation

Prefer `W5` next: execution limits, fuel, and timeout controls for the embedded WASM runner.

Do not move to artifact loading or task integration yet.

Reason:

- `W5` improves safety without expanding scope
- it strengthens the existing isolated runtime boundary
- it avoids premature coupling to installation tasks or assisted execution

## Checkpoint Decision

WASM Runtime v0 is now sufficiently documented as an isolated, metadata-only, no-side-effects boundary.

The correct next move is to strengthen runtime limits, not to broaden execution privileges.

The project must not connect WASM to installation tasks, assisted execution claims, or real remediation execution until the gates in this document are implemented and reviewed.
