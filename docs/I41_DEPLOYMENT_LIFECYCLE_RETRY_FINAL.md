# I41 — Deployment Lifecycle and Explicit Retry Evidence — Final

- checkpoint: `I41/147`
- predecessor: `I40/146`
- status: `validated`
- branch: `agent/i41-deployment-lifecycle-retry`
- PR: `#38`

## Regression — PASS

```text
dserver fmt: PASS
dserver test: 508 passed; 0 failed
dserver build: PASS
```

## Live lifecycle and retry gate — PASS

The validated campaign used the I40 three-node dependency D-Graph and the existing fog WASM artifact.

Observed lifecycle:

```text
fog-attempt-1
discovered -> prepared -> failed

explicit retry
      |
      v
fog-attempt-2
discovered -> prepared -> executing -> executed
```

The first attempt was deliberately failed before execution, leaving the live-report count at zero. Retry creation was explicit and produced a new attempt with preserved lineage:

```text
attempt_number=2
retry_of_attempt_id=fog-attempt-1
root_attempt_id=fog-attempt-1
automatic_retry_performed=false
governed_execution_performed=false
```

The second attempt was moved explicitly to `executing`, then the real Sentinel path executed `a-fog-scale` on `fog-01`, published a controlled live report, and returned the expected scalar result `24`.

Transitioning `fog-attempt-2` to `executed` succeeded only after binding the lifecycle record to the real live-report `execution_id` for the same assignment and node.

## Observed D-Graph evidence — PASS

The canonical observed D-Graph retained both attempts. For `fog-scale-module`, the current lifecycle evidence resolved to:

```text
current_attempt_id=fog-attempt-2
current_status=executed
```

Both the failed predecessor and successful retry remained available in lifecycle metadata.

## Fail-closed execution-evidence reuse gate — PASS

A separate attempt was advanced to `executing` and then attempted to reuse the successful retry's `execution_id`.

Observed behavior:

```text
HTTP 409
execution_id already linked to another deployment attempt
```

The negative-gate attempt remained `executing`; it was not promoted to `executed`.

## Safety and scope

Validated properties:

- lifecycle transitions are explicit and constrained;
- terminal attempts cannot transition again;
- retries can originate only from `failed` or `blocked` attempts;
- retry lineage is bounded to `MAX_ATTEMPTS_PER_LINEAGE=3`;
- retries are never automatically triggered;
- `executed` lifecycle evidence cannot be invented without an existing matching live report;
- an execution ID cannot be linked to two lifecycle attempts;
- `automatic_retry_performed=false`;
- `governed_execution_performed=false`;
- no canonical automatic governed execution was enabled.

## Persistence boundary

I41 lifecycle-attempt state remains intentionally in-memory. The I36 persisted envelope for source D-Graph, active manifest, and controlled live reports is unchanged. This non-claim remains explicit for the phase-final package.

## Result

I41/147 is validated and closed.

Next checkpoint: `I42/148 — resource-aware observed D-Graph join`.
