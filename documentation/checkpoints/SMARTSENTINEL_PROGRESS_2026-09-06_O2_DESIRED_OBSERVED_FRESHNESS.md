# SmartSentinel O2 checkpoint — desired vs observed freshness correlation

Date: 2026-09-06
Branch: `agent/o2-desired-observed-freshness`
Baseline main: step 63 LIVE PASS

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.
- 54/100 — LIVE PASS.
- 55/100 — LIVE PASS.
- 56/100 — LIVE PASS.
- 57/100 — LIVE PASS.
- 58/100 — LIVE PASS.
- 59/100 — LIVE PASS.
- 60/100 — LIVE PASS.
- 61/100 — LIVE PASS.
- 62/100 — LIVE PASS.
- 63/100 — LIVE PASS.
- 64/100 — IMPLEMENTED, pending compile/live validation.

## Step 64 — correlate desired state with observed telemetry under an explicit freshness policy

Step 64 extends the passive/active analysis context from step 63. It does not add placement reasoning or reconciliation. It adds an explicit epistemic boundary between a last-known observation and a current-state claim.

### Freshness policy

The analysis context now reports:

- `freshness_max_age_seconds` at context level;
- per-node `telemetry_freshness = fresh | stale | missing`;
- per-node `telemetry_age_seconds` when an observation timestamp is parseable.

The runtime setting is:

`DATUM_TELEMETRY_FRESHNESS_MAX_AGE_SECONDS`

Default: `120` seconds.

A missing or invalid observation timestamp is classified as `missing` for freshness purposes. Future-skewed timestamps clamp age to zero rather than producing negative age.

### Desired vs observed service correlation

For every desired service in the accepted Operational D-Graph, dserver inspects the SmartSentinel snapshot payload.

It correlates:

1. desired service id/runtime/state from D-Graph;
2. matching `service:<service_id>` telemetry entity;
3. service binding information;
4. the bound runtime target id, when present;
5. the runtime target entity state (for example a Docker container `state=running`).

The result exposes:

- `comparison_status = converged | drift | unknown`;
- a machine-readable `reason`;
- whether the service entity was present;
- observed service status;
- observed bound target id;
- observed binding state;
- observed runtime state.

### Conservative freshness semantics

A stale or missing snapshot can still report the last observed runtime state, but dserver will not claim current convergence from stale evidence.

Example:

- desired MQTT: `running`;
- last observed container state: `running`;
- telemetry age: older than freshness policy;
- result: `comparison_status=unknown`, `reason=stale_telemetry`.

With fresh evidence, desired `running` plus bound runtime target `running` is classified `converged`. Fresh decisive unavailability/stopped evidence against desired `running` is classified `drift`.

### Analysis modes remain unchanged

Passive mode still analyzes only the trigger node.

Active mode still traverses all nodes of the accepted active D-Graph and fetches latest Firebase telemetry concurrently.

Both remain context acquisition/assessment only:

`side_effects_performed=false`.

## Planned live proof

1. update to `agent/o2-desired-observed-freshness`;
2. run the new pure context tests and build with locked dependencies;
3. start dserver with the existing SQLite/Firebase configuration and default 120-second freshness policy;
4. call passive context for `monitor-f49c8/fog-01`;
5. verify rev4 and MQTT desired `running` remain unchanged;
6. because the currently stored Firebase snapshot was observed on 2026-09-05, verify it is classified `stale` under the default policy;
7. verify the service correlation still extracts the last observed MQTT runtime target as `running`, while comparison remains `unknown/stale_telemetry`;
8. call active mode and verify the same freshness/correlation semantics with zero side effects;
9. unit tests prove fresh `running` telemetry yields `converged`, while missing telemetry yields `unknown/missing_telemetry`.

PASS proves dserver can distinguish last-known observed state from sufficiently fresh evidence and can correlate desired service state to concrete SmartSentinel runtime observations without taking placement or reconciliation responsibility.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver acquires passive/active context -> explicit freshness policy constrains what can be claimed from observed data -> desired-vs-observed correlation produces evidence for later analysis/AI. External AI/GNN remains responsible for placement/reasoning, dserver governs accepted D-Graph state, SmartSentinel/WASM reconciles only when authorized, and DATUM remains conceptual.`
