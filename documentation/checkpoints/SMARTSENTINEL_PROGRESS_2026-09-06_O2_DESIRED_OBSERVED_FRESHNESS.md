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
- 64/100 — LIVE PASS.

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

## Live validation — PASS

Live validation used project `monitor-f49c8`, node `fog-01`, active D-Graph `dgraph:o2-mqtt-role-1` revision 4, and freshness policy `120` seconds.

Passive context returned:

- `schema_version=0.2.0`;
- `acquisition_mode=passive`;
- `dgraph_revision=4`;
- `freshness_max_age_seconds=120`;
- `side_effects_performed=false`;
- `telemetry_available=true`;
- `telemetry_freshness=stale`;
- telemetry observation timestamp `2026-09-05T00:02:52.057683316Z`;
- telemetry age approximately `159352` seconds at validation time;
- desired MQTT service `runtime=container`, `desired_state=running`;
- concrete observed target `container:docker:2787bb511ae943e0eb522fed9237925bf8dd5c9ad166bc9821a96652fc61dac1`;
- `observed_binding_bound=true`;
- last observed runtime state `running`;
- comparison conservatively remained `unknown` with `reason=stale_telemetry`.

Active context returned the same revision, freshness policy, desired service and runtime observation for the current one-node rev4 graph, with:

- `acquisition_mode=active`;
- `node_count=1`;
- telemetry age approximately `159358` seconds;
- `comparison_status=unknown`;
- `reason=stale_telemetry`;
- `side_effects_performed=false`.

The six-second difference in reported telemetry age between passive and active calls is expected because age is evaluated at request time.

PASS proves dserver distinguishes last-known observed state from sufficiently fresh evidence and correlates desired service state to concrete SmartSentinel runtime observations without taking placement or reconciliation responsibility.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver acquires passive/active context -> explicit freshness policy constrains what can be claimed from observed data -> desired-vs-observed correlation produces evidence for later analysis/AI. External AI/GNN remains responsible for placement/reasoning, dserver governs accepted D-Graph state, SmartSentinel/WASM reconciles only when authorized, and DATUM remains conceptual.`
