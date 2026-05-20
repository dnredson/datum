# Safe Assisted Control Plane Checkpoint

## Purpose

This document records the completed DATUM/SmartSentinel safe assisted control-plane checkpoint after the 60/60 assisted metadata, advisory, and smoke-coverage phase.

It exists to freeze what is complete, make the non-execution boundary explicit, and provide a clear handoff point before any future side-effect-capable execution phase is considered.

## Completed Scope

The completed phase provides:

- capability publishing
- snapshot publishing
- graph and analysis materialization
- capability-aware action plans
- feedback-aware dispatch policy
- action result reporting
- deployment planning
- installation planning
- installation task lifecycle
- installation policy
- operator approval
- assisted readiness
- assisted execution request
- assisted execution claim and lease handling
- assisted claim lifecycle updates
- assisted execution attempts in `no_op` and `dry_run` mode
- assisted execution state summary
- local mapping recommendations
- local mapping preflight
- local mapping preflight report publishing
- local mapping context enrichment in assisted execution state
- full repo-root smoke test coverage for the non-destructive chain

## Current Hard Guarantees

- `dserver` does not execute local actions.
- `DATUM` does not execute assisted installation.
- assisted attempts are `no_op` and `dry_run` metadata only.
- `safe_to_auto_execute` remains false across the chain.
- `side_effects_performed` remains false in the current flow.
- no Docker `run`, `pull`, `rm`, or `inspect` is part of assisted installation.
- no artifact transfer exists.
- no local config write exists.
- no automatic mapping creation exists.
- no WASM runtime exists.
- no migration execution exists.

These guarantees mean the current assisted control plane is orchestration metadata plus advisory visibility only. It is not yet a side-effect-capable installer.

## Smoke Test Coverage

`scripts/smoke-client-server-contract.ps1` validates the non-destructive control path end to end:

- capabilities
- snapshot, graph, and analysis flow
- action-plan and dispatch flow
- action-results reporting
- deployment planning
- installation plan
- installation tasks
- installation policy
- approval
- readiness
- assisted execution request
- claim
- claim lifecycle
- `no_op` and `dry_run` attempt recording
- assisted execution state summary
- local mapping recommendations
- local mapping preflight
- local mapping preflight publishing
- local mapping context in assisted execution state

The smoke test remains non-destructive:

- it does not call `dispatch --execute`
- it does not perform deployment or installation execution
- it does not perform Docker/container/WASM/migration side effects
- it does not write local config
- it does not modify local mappings

## Known Limitations

- live smoke depends on a running compatible `dserver` and Firestore runtime
- the current health probe may return `404` if the wrong server/runtime is active
- `DATUM` still has pre-existing dead-code warnings
- no real execution is implemented
- no artifact verification is implemented
- no rollback is implemented
- no WASM sandbox is implemented
- no automatic remediation is implemented

## Control-Plane Interpretation Notes

- assisted execution state is informational and read-only
- local mapping recommendations are advisory only
- local mapping preflight is advisory only
- local mapping preflight publishing records advisory metadata only
- local mapping context in assisted execution state is advisory only
- `local_mapping_status=already_mapped` does not imply execution permission
- `local_mapping_status=missing_mapping` or `local_mapping_status=ambiguous` requires operator or config review, not automatic remediation

## Candidate Next Phases

### Option A: Local mapping operator patch generation

- Generate a proposed config patch file for operator review.
- Still no automatic write.
- Good next step for `missing_local_mapping`.

Assessment:
- Recommended.
- Extends the local mapping workflow without crossing the side-effect boundary.

### Option B: Docker inspect-only runtime verification

- Could verify that mapped containers exist without starting or stopping anything.
- Must be explicit and guarded because it introduces Docker inspect.
- No `run`, `pull`, or `rm`.

Assessment:
- Recommended with caution.
- Useful for improving runtime visibility before any executor is introduced.

### Option C: WASM runtime v0

- Local sandbox with a trivial module.
- No filesystem or network access.
- Initially disconnected from installation tasks.

Assessment:
- Reasonable exploratory track.
- Better treated as separate runtime groundwork than as the immediate next assisted-installation step.

### Option D: Real guarded assisted installation executor

- Not recommended as the immediate next step.
- Requires artifact verification, rollback planning, stronger policy, and explicit local config enablement.

Assessment:
- Defer until narrower preparatory phases are complete.

## Recommendation

Option A or Option B should come before any real executor work.

Reason:

- both preserve the current safe control-plane boundary
- both improve operator visibility or reviewability
- neither requires immediate side-effect-capable assisted installation
- both reduce risk before artifact handling, rollback, policy hardening, and local opt-in are designed

## Checkpoint Summary

The safe assisted control-plane phase is complete.

The system now has a full non-destructive chain for:

- planning
- tasking
- policy
- approvals
- readiness
- request and claim metadata
- no-op and dry-run assisted attempt recording
- local mapping advisory workflows
- state-summary visibility
- smoke validation of the whole read-only and metadata-only path

The next phase should continue to respect this checkpoint and avoid introducing real execution until narrower, explicitly guarded preparatory steps have been completed.
