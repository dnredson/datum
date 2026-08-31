# I43 — Deployment Readiness and Governance Evidence

- checkpoint: `I43/149`
- predecessor: `I42/148`
- status: `implementation_in_progress`

I43 consolidates prerequisite evidence required before controlled explicit execution. It does not authorize or trigger execution.

## Readiness boundary

Global hard prerequisites:

- active controlled manifest and source D-Graph;
- observed D-Graph validates with zero errors;
- every targeted logical Sentinel is registered and active;
- every targeted node has a fresh resource observation;
- every targeted node reports `wasm` support;
- every artifact retains validated SHA-256 content identity;
- execution boundary remains explicit/fail-closed;
- governed automatic execution remains unavailable.

Dependency state is represented per assignment. An assignment whose upstream deployment dependencies have not yet produced successful controlled runtime evidence is `waiting_on_dependencies`; this does not invent success or trigger retries.

## Status

```text
ready_for_explicit_execution
blocked
```

`ready_for_explicit_execution` means the global prerequisite envelope is complete and at least the currently dependency-eligible assignments may be explicitly invoked by an operator/Sentinel path. It does not mean automatic execution is allowed.

## Safety constants

```text
automatic_execution_allowed=false
governed_execution_performed=false
readiness_side_effects_performed=false
```

## Planned validation

Healthy three-node fixture:

- all three Sentinels active;
- all three resource observations fresh;
- all three support `wasm`;
- D-Graph/manifest valid;
- stage 0 assignment ready;
- later stages waiting on dependencies;
- global status `ready_for_explicit_execution`;
- automatic execution remains false.

Negative gate:

- expire or omit one prerequisite;
- global status becomes `blocked`;
- deterministic blocking reason names the exact node/prerequisite;
- no report, placement mutation, or execution side effect occurs.
