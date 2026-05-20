# Assisted Execution Phase Rebaseline

## Purpose

This document rebaselines the DATUM/SmartSentinel assisted execution architecture after completion of the metadata-only control plane and before introducing any side-effect-capable assisted executor.

It is intended to freeze the current contract, make the non-execution boundary explicit, and define the minimum safety gate for any future executor work.

## Phase 1 Completed

Phase 1 is complete:

`Phase 1: Metadata-only control plane and no-op/dry-run execution attempt chain.`

Completed scope:
- DATUM publishes capabilities.
- DATUM publishes snapshots.
- `dserver` builds graph, analysis, action-plan, and dispatch.
- `dserver` annotates actions with capability and feedback.
- `dserver` gates dispatch using capability and feedback.
- DATUM consumes dispatch.
- DATUM publishes action results.
- `dserver` derives `deployment_planning`.
- `dserver` derives `installation_plan`.
- `dserver` materializes `installation_tasks`.
- `dserver` annotates tasks with `installation_policy`.
- DATUM views, materializes, lists, and updates installation tasks.
- `dserver` records operator approval.
- DATUM submits and views approvals.
- `dserver` evaluates assisted readiness.
- DATUM views readiness.
- `dserver` records assisted execution requests.
- DATUM submits and views requests.
- `dserver` records assisted execution claims.
- DATUM submits and views claims.
- `dserver` supports claim lifecycle updates.
- DATUM updates claim lifecycle metadata.
- `dserver` records assisted execution attempts in `no_op` or `dry_run` mode.
- DATUM submits and views assisted execution attempts.
- Smoke coverage validates the complete metadata/no-op chain.

## Current Boundary

The current implementation remains metadata-only.

Explicitly not implemented:
- No real installation execution exists.
- No artifact transfer exists.
- No Docker run or pull exists.
- No WASM runtime exists.
- No migration execution exists.
- No flow reconfiguration execution exists.

Current assisted execution attempts are only `no_op` or `dry_run` metadata reports. They do not perform persistent local changes, installation side effects, artifact movement, container creation, migration application, or runtime execution.

The current smoke path confirms:
- `safe_to_auto_execute == false`
- `side_effects_performed == false`
- no Docker/container/WASM/migration/artifact side effects

## Safety Gate For Future Execution

A future side-effect-capable assisted executor must require all of the following before any side effects are allowed:

- installation task exists
- task status is eligible
- `installation_policy.decision == candidate_for_assisted_execution`
- operator approval exists
- `readiness == ready_for_assisted_execution`
- assisted execution request exists
- active claim exists
- local DATUM config explicitly enables the executor
- execution mode is explicitly selected
- executor-specific local preflight passes
- side effects are reported back to `dserver`
- failure must not create retry loops

Interpretation:
- Approval, readiness, request, or claim alone is not execution permission.
- Server metadata is necessary but still insufficient without explicit local enablement.
- Any first real executor must be opt-in, bounded, and observable.

## Candidate Next Paths

### Path A: Local preflight-only assisted executor

What it does:
- DATUM reads task, request, and claim metadata.
- DATUM validates local state.
- DATUM submits a richer `dry_run` or preflight attempt.
- No persistent side effects occur.

Why it matters:
- Exercises executor decision points without changing runtime behavior.
- Produces better telemetry for readiness, local blockers, and operator review.
- Keeps the rebaseline intact while proving the first executor-shaped control path.

Recommendation:
- Recommended next step.

### Path B: Guarded local mapping preparation

What it does:
- DATUM proposes or validates local config mapping.
- No automatic write occurs initially.
- A later extension could generate a patch file for operator review.

Why it matters:
- Useful for `missing_local_mapping`.
- Creates a narrow, reviewable preparation path before any real execution.

### Path C: WASM runtime v0

What it does:
- DATUM runs a trivial local WASM module in a sandbox.
- No filesystem or network access exists initially.
- Useful for validating future WebAssembly-based D-Code execution.

Boundary:
- It should not yet be connected to installation tasks.

## Recommended Direction

Path A should be the next milestone.

Reason:
- It extends the current metadata/no-op chain instead of jumping across the safety boundary.
- It validates local executor wiring, configuration gates, and preflight reporting first.
- It preserves the rule that no real assisted installation side effects exist yet.

## Proposed Next Milestones

- `C21/60`: Client assisted execution preflight/no-op runner
- `S24/60`: Server attempt-aware assisted execution state summary
- `I12/60`: Smoke test for preflight/no-op assisted execution state
- `C22/60`: Client local mapping preflight validator
- `S25/60`: Server local mapping recommendation contract
- later: WASM runtime v0
- later: artifact transfer and verification
- later: real assisted installation executor

## Rebaseline Summary

The system now has a complete metadata-only assisted execution control chain, plus recorded `no_op` and `dry_run` attempts. That is the end of Phase 1.

The next phase should not introduce real execution immediately. It should first add local preflight/no-op executor behavior, preserve `safe_to_auto_execute = false`, preserve `side_effects_performed = false`, and keep all real installation side effects out of scope until the executor gate, reporting model, and failure handling are proven.
