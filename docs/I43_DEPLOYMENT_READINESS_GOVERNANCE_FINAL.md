# I43 — Deployment Readiness and Governance Evidence — Final

- checkpoint: `I43/149`
- predecessor: `I42/148`
- status: `validated`
- validation_date: `2026-08-31`

I43 consolidates deterministic deployment-readiness evidence into the canonical controlled observed D-Graph without authorizing or triggering automatic execution.

## Regression

```text
dserver fmt: PASS
dserver test: 520 passed; 0 failed
dserver build: PASS
```

The I43 regression adds six focused readiness/governance tests on top of the validated I42 baseline of 514 tests.

## Healthy live gate

The dedicated `i43-live-project` fixture used three target nodes and dependency stages:

```text
a-fog-scale        -> fog-01   stage 0
m-local-aggregate  -> local-01 stage 1 depends on a-fog-scale
z-cloud-delta      -> cloud-01 stage 2 depends on m-local-aggregate
```

All three logical Sentinels were registered and active. All three target nodes published fresh resource observations with supported `wasm` capability.

Observed readiness matched the intended explicit-execution boundary:

```text
status=ready_for_explicit_execution
assignment_count=3
target_node_count=3
ready_assignment_count=1
waiting_assignment_count=2
completed_assignment_count=0
blocked_assignment_count=0
blocking_reasons=[]
automatic_execution_allowed=false
governed_execution_performed=false
readiness_side_effects_performed=false
```

Assignment evidence remained dependency-aware:

```text
a-fog-scale        ready
m-local-aggregate  waiting_on_dependencies: a-fog-scale
z-cloud-delta      waiting_on_dependencies: m-local-aggregate
```

No live execution report was created.

## Fail-closed lease gate

The same `fog-01` Sentinel identity was re-registered with a one-second lease while local/cloud remained on 300-second leases. After expiry:

```text
active_count=2
stale_count=1
fog-01 status=stale lease_seconds=1
```

Re-materializing the canonical observed D-Graph produced exactly one deterministic blocker:

```text
status=blocked
blocking_reason_count=1
code=sentinel_stale
scope=node
node_id=fog-01
detail="target node 'fog-01' Sentinel lease is stale"
```

The fog resource observation remained fresh, proving that the blocker was specifically Sentinel lease state rather than resource freshness.

Execution/report state stayed unchanged:

```text
live_reports_before=0
live_reports_after=0
automatic_execution_allowed=false
governed_execution_performed=false
readiness_side_effects_performed=false
```

## Boundary preserved

I43 readiness is evidence, not authorization. `ready_for_explicit_execution` means the prerequisite envelope permits the existing explicit operator/Sentinel execution path to be invoked; it never flips automatic execution on.

No placement mutation, retry, artifact execution, governed execution, or hidden side effect is performed by readiness materialization.

## Result

I43/149 is validated. The controlled WASM deployment phase advances to `149/150`.

Next: `I44/150` phase-final integration and reproducibility package.
