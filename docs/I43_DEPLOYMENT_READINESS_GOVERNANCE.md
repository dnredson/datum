# I43 — Deployment Readiness and Governance Evidence

- checkpoint: `I43/149`
- predecessor: `I42/148`
- status: `implementation_ready_for_validation`
- branch: `agent/i43-deployment-readiness-governance`

I43 consolidates prerequisite evidence required before controlled explicit execution. It does not authorize or trigger execution.

## Canonical materialization

Readiness is materialized into the existing canonical endpoint:

```text
GET /api/v1/controlled-wasm/observed-dgraph
```

The summary appears at:

```text
metadata.deployment_readiness
graph.metadata.deployment_readiness
```

No parallel readiness store or second observed-state assembly path is introduced.

## Global hard prerequisites

- active controlled manifest and source D-Graph;
- observed D-Graph validates with zero errors;
- every targeted logical Sentinel is registered and active;
- every targeted node has a fresh resource observation;
- every targeted node reports supported capability `wasm`;
- every artifact retains manifest-validated SHA-256 content identity;
- execution boundary requires explicit invocation;
- automatic and governed execution remain disabled.

Deterministic block codes include:

```text
manifest_invalid
observed_dgraph_invalid
observed_identity_mismatch
execution_boundary_invalid
sentinel_missing
sentinel_stale
resource_observation_missing
resource_observation_stale
wasm_capability_missing
```

## Dependency evidence

Dependency state is represented per assignment rather than treated as a global structural failure.

Assignment states:

```text
ready
waiting_on_dependencies
completed
blocked
```

A successful controlled live report marks that assignment completed and can release its downstream dependency. No success is inferred from lifecycle state alone and no retry is triggered.

## Global status

```text
ready_for_explicit_execution
blocked
```

`ready_for_explicit_execution` means the global prerequisite envelope is complete. It does not mean automatic execution is allowed. With no runtime reports, the I43 three-stage fixture should materialize:

```text
a-fog-scale       -> ready
m-local-aggregate -> waiting_on_dependencies(a-fog-scale)
z-cloud-delta     -> waiting_on_dependencies(m-local-aggregate)
```

## Safety constants

Every summary retains:

```text
automatic_execution_allowed=false
governed_execution_performed=false
readiness_side_effects_performed=false
```

## Regression expectation

I43 adds six focused tests on top of the validated I42 dserver baseline of 514 tests:

```text
520 passed; 0 failed
```

Focused coverage:

- healthy envelope ready with downstream dependency waiting;
- stale Sentinel blocks with deterministic node reason;
- stale resource observation blocks;
- missing `wasm` capability blocks;
- successful upstream runtime report releases dependency;
- observed D-Graph validation error blocks.

## Planned live validation

Use isolated fixture:

```text
scripts/fixtures/i43-readiness-three-node-dgraph.json
project=i43-live-project
```

Healthy gate:

- register fog/local/cloud with active leases;
- publish fresh resource observations for all three nodes;
- all three advertise `wasm` in snapshot capabilities;
- GET observed D-Graph;
- readiness status `ready_for_explicit_execution`;
- one ready assignment and two dependency-waiting assignments;
- zero blocking reasons;
- automatic execution remains false;
- reports remain zero.

Negative gate:

- re-register fog with a 1-second lease using the same instance;
- wait for expiry while resource observation remains fresh;
- GET observed D-Graph again;
- global readiness becomes `blocked`;
- deterministic reason contains `sentinel_stale` for `fog-01`;
- no report, placement mutation, retry, or execution side effect occurs.

## Non-claims

I43 does not provide:

- automatic deployment authorization;
- cryptographic Sentinel identity attestation;
- artifact download-time verification in the readiness read itself;
- automatic placement changes from resource values;
- dependency stage execution barriers;
- hidden retry or lifecycle transitions;
- canonical governed automatic execution.

Checkpoint remains `148/150` until regression and live validation pass.
