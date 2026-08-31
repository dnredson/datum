# I41 — Deployment Lifecycle and Explicit Retry Evidence

- checkpoint: `I41/147`
- predecessor: `I40/146`
- status: `implementation_ready_for_validation`
- branch: `agent/i41-deployment-lifecycle-retry`

## Goal

I41 models controlled deployment attempts as explicit lifecycle evidence rather than treating a final live report as the only state retained around execution.

The lifecycle is evidence and coordination state. It does not create automatic execution or automatic retry behavior.

## Lifecycle states

```text
discovered
  -> prepared
      -> executing
          -> executed
          -> failed
      -> failed
      -> blocked
  -> blocked
```

Terminal states are immutable:

```text
executed
failed
blocked
```

A transition outside the allowed graph fails closed.

## Attempt identity and lineage

Each attempt records:

```text
attempt_id
root_attempt_id
retry_of_attempt_id
attempt_number
manifest_id
project_id
assignment_id
node_id
requested_by
status
execution_id
created_at_utc
updated_at_utc
history[]
automatic_retry_performed=false
governed_execution_performed=false
```

Initial attempts use `attempt_number=1` and `root_attempt_id=attempt_id`.

An explicit retry can be created only from a `failed` or `blocked` attempt. The retry receives:

```text
retry_of_attempt_id=<previous attempt>
root_attempt_id=<original root>
attempt_number=<previous + 1>
status=discovered
```

The fixed I41 lineage bound is:

```text
MAX_ATTEMPTS_PER_LINEAGE=3
```

This is a representation and enforcement of bounded retry. No retry is triggered automatically.

## Live-report binding

Transitioning an attempt to `executed` requires an `execution_id`.

The dserver requires that this ID already exist in the controlled live-report store and that the report match the lifecycle attempt's assignment and node. The same `execution_id` cannot be linked to two lifecycle attempts.

This means lifecycle evidence cannot invent successful execution independently from existing controlled runtime evidence.

Existing live-report duplicate protection remains unchanged.

## API

```text
POST /api/v1/controlled-wasm/deployment-attempts
GET  /api/v1/controlled-wasm/deployment-attempts
GET  /api/v1/controlled-wasm/deployment-attempts/:attempt_id
POST /api/v1/controlled-wasm/deployment-attempts/:attempt_id/transition
POST /api/v1/controlled-wasm/deployment-attempts/:attempt_id/retry
```

Publishing a new controlled D-Graph or manual deployment manifest clears lifecycle attempts because they belong to the previous active manifest.

## Observed D-Graph evidence

The canonical controlled observed D-Graph endpoint remains:

```text
GET /api/v1/controlled-wasm/observed-dgraph
```

I41 enriches this materialization with:

```text
graph.metadata.deployment_lifecycle_attempt_count
graph.metadata.deployment_attempts[]
metadata.deployment_lifecycle_attempt_count
metadata.deployment_attempts[]
```

For modules with lifecycle attempts, module metadata also carries:

```text
deployment_lifecycle_current_attempt_id
deployment_lifecycle_current_status
deployment_attempts[]
```

Thus current and prior attempt evidence remain visible in the observed graph during the active dserver process.

## Persistence boundary

The I36 persisted envelope continues to persist source D-Graph, active manifest, and live reports. I41 lifecycle attempts are intentionally in-memory in this increment and are cleared on a new active manifest. Extending persistence to lifecycle evidence can be included in the phase-final reproducibility consolidation if required.

This scope avoids silently changing the I36 persisted-state compatibility contract inside the lifecycle increment.

## Focused regression tests

I41 adds eight core tests on top of the validated I40 dserver baseline of 500 tests:

```text
initial attempt is discovered and explicit
full success lifecycle is valid
invalid state jump is rejected
terminal attempt cannot transition
explicit retry links to failed attempt
retry before failed/blocked is rejected
retry lineage bound is enforced
duplicate lifecycle execution IDs are rejected
```

Expected minimum full-suite result:

```text
508 passed; 0 failed
```

The observed full-suite count remains authoritative.

## Planned live gate

Use the validated I40 three-node D-Graph fixture and artifact repository.

1. publish `i40-dependency-three-node-dgraph.json`;
2. create `fog-attempt-1` for assignment `a-fog-scale`;
3. transition `discovered -> prepared -> failed` with an explicit reason;
4. require failed attempt retained in the attempt list;
5. explicitly create retry `fog-attempt-2` from `fog-attempt-1`;
6. require `retry_of_attempt_id=fog-attempt-1`, `root_attempt_id=fog-attempt-1`, `attempt_number=2`, `automatic_retry_performed=false`;
7. transition retry `discovered -> prepared -> executing`;
8. explicitly execute the existing fog assignment through the Sentinel and publish its live report;
9. transition `fog-attempt-2` to `executed` using that real `execution_id`;
10. require two retained attempts: first failed, second executed;
11. GET observed D-Graph and require both attempts plus module current lifecycle status `executed`;
12. prove an unrelated attempt cannot reuse the same execution ID;
13. preserve `governed_execution_performed=false` and no hidden retry.

## Non-claims

I41 does not provide:

- hidden/automatic retries;
- automatic lifecycle transitions from the Sentinel;
- retry backoff scheduling;
- distributed stage barriers;
- persisted lifecycle attempts across dserver restart;
- resource-aware observed state (I42);
- readiness authorization (I43);
- canonical automatic governed execution.

Checkpoint remains `146/150` until regression and live lifecycle/retry validation pass.
