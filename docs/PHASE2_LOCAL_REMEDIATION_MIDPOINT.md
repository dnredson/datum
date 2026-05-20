# Phase 2 Local Remediation Midpoint

## Purpose

This note records the Phase 2 midpoint decision after local mapping patch proposals became visible on the server, visible in `DATUM`, and covered by the repo-root smoke test.

It exists to make the current safety boundary explicit, compare the next available paths, and lock in the sequencing decision before any future runtime or executor work.

## Current Phase 2 Status

Phase 2 has already achieved:

- server local mapping patch proposal contract
- client local mapping patch proposal viewer
- smoke test coverage for patch proposals
- advisory `proposed_config_snippet` visibility
- no config write
- no mapping application
- no Docker `inspect`, `run`, `pull`, or `rm`
- no installation execution

The current system still does not write config, does not apply mappings, and does not execute installation work.

## Current Guarantees

- patch proposals are operator-reviewed suggestions only
- `proposed_config_snippet` must not be auto-applied
- `safe_to_auto_apply` remains false
- `side_effects_performed` remains false
- `requires_operator` remains true
- no local config or mapping files are modified
- no Docker/container/WASM/migration/artifact side effects occur

These guarantees mean the current local remediation flow is advisory visibility only. It is not yet a config writer, mapping applier, runtime verifier, or installer.

## Next Options

### Option A1: Patch file generation for operator review

- `DATUM` generates a separate proposal file under a dedicated output location such as `generated/` or `reports/`
- it does not modify `config.toml`
- the operator can review it and manually copy from it if desired
- this is the most natural continuation of the current Phase 2 operator-review loop

Assessment:
- lowest-risk continuation
- preserves the current no-write boundary for live config
- improves operator usability without introducing runtime side effects

### Option A2: Config diff/revalidator

- `DATUM` validates a proposed mapping patch against the current config
- it still performs no write
- it is more useful after patch file generation exists

Assessment:
- strong follow-on step after A1
- helps operators confirm whether a reviewed proposal still matches current config state

### Option B: Docker inspect-only verification

- `DATUM` verifies whether an explicitly mapped container exists
- this uses `docker inspect` only
- it is higher risk than pure patch generation because it touches Docker runtime state
- it must remain explicit and guarded

Assessment:
- useful, but not the best immediate next step
- should wait until the operator review and revalidation loop is stable

### Option C: WASM runtime v0

- mandatory roadmap item
- implement a trivial isolated WASM runner
- no filesystem access
- no network access
- initially disconnected from installation tasks
- should be scheduled soon after the local remediation operator-review loop is stable

Assessment:
- mandatory, not optional
- should begin soon as groundwork, but not by collapsing the current advisory local-remediation boundary

## Recommended Sequence

- `C28/70`: DATUM local mapping patch file generator, explicit-only, writes only to a proposal/output file, never to `config.toml`
- `I17/70`: smoke test for patch file generation, verifying `config.toml` is unchanged
- `C29/70`: DATUM config diff/revalidator for proposed mapping patch
- `I18/70`: smoke test for config diff/revalidator
- `W1`: WASM runtime v0 isolated proof-of-execution, no host access and disconnected from installation tasks

## Decision

The next Phase 2 sequence should stay inside the operator-review loop first, then begin WASM groundwork.

Reason:

- patch file generation extends the existing advisory proposal model without modifying live config
- revalidation strengthens operator confidence before any future execution-capable work is considered
- Docker inspect-only verification is useful but introduces runtime-touching behavior earlier than necessary
- WASM is mandatory for the long-term project vision and must be scheduled soon, but this midpoint does not justify skipping the remaining operator-review steps

## Explicit Boundary

- WASM is not optional
- this milestone does not implement WASM
- real assisted installation executor work should wait until after patch proposal generation, patch revalidation, and WASM v0 groundwork
- no server or client runtime execution behavior changes are part of this midpoint decision

## Midpoint Summary

Phase 2 now has a complete advisory path for local mapping patch proposals:

- proposal generation on the server
- proposal visibility in `DATUM`
- smoke validation of server and client contract behavior
- explicit visibility of advisory config snippets without applying them

The correct next move is to deepen operator-reviewed remediation first, while preserving WebAssembly as a mandatory near-term roadmap track rather than a deferred optional idea.
