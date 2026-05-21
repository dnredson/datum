# Phase 2 Closure And WASM Entry Plan

## Purpose

This note closes Phase 2 of operator-reviewed local remediation and defines the entry plan for mandatory WebAssembly runtime v0 work.

It exists to record what Phase 2 completed, make the safety boundary explicit, and lock in the sequencing rule that real assisted installation execution must wait until after operator-review stability and WASM v0 groundwork.

## Completed Phase 2 Scope

Phase 2 added:

- server local mapping patch proposal contract
- DATUM local mapping patch proposal viewer
- smoke validation for patch proposal visibility
- DATUM local mapping patch proposal file generation
- smoke validation that the generated proposal file is separate and advisory
- DATUM read-only local mapping patch revalidator
- smoke validation that revalidation does not apply changes
- validation that active config remains unchanged across generation and revalidation

Phase 2 therefore completed the full advisory local-remediation loop without crossing into execution, config mutation, or runtime-touching verification.

## Phase 2 Hard Guarantees

- no active config write
- no `dispatch.container_mappings` mutation
- no `service_registry` mutation
- no automatic mapping creation
- no `proposed_config_snippet` application
- no Docker `inspect`, `run`, `pull`, or `rm`
- no artifact transfer
- no installation execution
- no WASM execution yet
- no migration execution
- generated patch files are operator-review artifacts only
- revalidation is read-only

These guarantees mean Phase 2 ended with operator-reviewed visibility and validation only. It did not become a config writer, mapping applier, installer, or runtime executor.

## Why WASM Runtime v0 Comes Next

WebAssembly is part of the project’s intended deployment and maintenance model. It is not an optional future enhancement.

The project now has an operator-review loop for local remediation, but it still lacks a controlled execution substrate. Before any real assisted remediation or installation work can be considered, the system needs a sandboxed runtime boundary that is narrower and safer than direct host execution.

WASM runtime v0 should validate that boundary without touching installation tasks yet. The first WASM milestone should prove that DATUM can host a trivial isolated execution unit while preserving the current non-destructive guarantees.

## WASM Runtime v0 Constraints

`W1` must initially:

- run only inside DATUM
- run a trivial embedded or test WASM module
- have no filesystem access
- have no network access
- have no host command execution
- have no Docker access
- have no artifact download
- not connect to installation tasks
- not modify config
- not perform migration
- report execution result locally or through a future metadata-only report
- be covered by source guard tests

This keeps the first WASM milestone isolated, non-destructive, and suitable as groundwork rather than as an execution feature.

## Recommended Sequence

- `W1/80`: DATUM WASM runtime v0 isolated local runner
- `W2/80`: DATUM WASM runtime report model, still local and no side effects
- `W3/80`: dserver WASM execution report contract, metadata-only
- `W4/80`: DATUM publish WASM no-op execution report
- `I19/80`: smoke test for WASM no-op runtime/report chain
- status: `I19/80` is complete; `W5` is the recommended next safe step
- `W5/80`: add execution limits, fuel, and timeout controls to the embedded DATUM-local runner
- later: connect WASM tasks to assisted execution state only after policy review
- much later: real assisted installation executor with artifact verification and rollback

Real assisted installation execution should not begin until after WASM v0 and the WASM report chain are stable.

## Closure Decision

Phase 2 is complete as an operator-reviewed local remediation phase.

Reason:

- the server can propose advisory local mapping patches
- DATUM can view those proposals
- DATUM can generate a separate operator-review proposal artifact
- DATUM can revalidate that artifact against active config without applying it
- the smoke test now covers proposal visibility, proposal-file generation, proposal revalidation, and active-config unchanged guarantees

The next correct phase entry is mandatory WASM runtime v0 groundwork, not early installation execution.

## Explicit Boundary

- WebAssembly remains mandatory for the project roadmap
- this milestone does not implement WASM
- real assisted installation execution must wait until after operator-review stability and WASM v0 groundwork
- no server or client runtime behavior changes are part of this document checkpoint

## Summary

Phase 2 now closes with a complete advisory-only local remediation loop:

- proposals can be generated and viewed
- review artifacts can be generated explicitly
- review artifacts can be revalidated read-only
- active config remains unchanged across the flow
- no runtime or installation side effects were introduced

That closure makes WASM runtime v0 the next required step. The project should enter WASM with an isolated, non-destructive runner first, then add metadata-only reporting, and only much later consider real assisted installation execution.
