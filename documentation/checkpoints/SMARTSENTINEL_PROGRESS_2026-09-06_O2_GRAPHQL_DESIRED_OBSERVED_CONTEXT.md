# SmartSentinel O2 checkpoint — GraphQL desired-observed unified context

Date: 2026-09-06
Branch: `agent/o2-graphql-desired-observed-context`
Baseline main: step 64 LIVE PASS

## Progress status

- 59/100 — LIVE PASS.
- 60/100 — LIVE PASS.
- 61/100 — LIVE PASS.
- 62/100 — LIVE PASS.
- 63/100 — LIVE PASS.
- 64/100 — LIVE PASS.
- 65/100 — LIVE PASS.

## Step 65 — expose desired × observed × freshness in the unified GraphQL node context

Step 65 extends the existing read-only GraphQL `node(projectId,nodeId)` query rather than creating a parallel query plane.

The unified node context now exposes Resource Registry state, active binding, accepted Operational D-Graph desired state/revision, current valid authorization, latest Firebase telemetry availability, and an optional typed `assessment` containing the step-64 desired-vs-observed correlation and freshness semantics.

The resolver performs one Firebase latest-snapshot read and reuses that same snapshot both for raw telemetry and for `build_passive_analysis_context()`, so GraphQL does not duplicate the correlation algorithm or risk assessing a different snapshot than the one returned to the client.

## Live validation — PASS

Live project/node:

- project: `monitor-f49c8`;
- node: `fog-01`;
- resource: `resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`;
- active binding: `binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4`;
- D-Graph: `dgraph:o2-mqtt-role-1`, revision `4`.

A single GraphQL `node(...)` query returned:

- resource state `bound`, assigned to `fog-01`, observation count `2`;
- active binding unchanged;
- desired MQTT service `container/running` from D-Graph revision 4;
- `authorization=null`;
- `telemetryAvailable=true`, source `firebase`, no telemetry error;
- assessment schema `0.2.0`;
- freshness policy `120` seconds;
- telemetry observation timestamp `2026-09-05T00:02:52.057683316Z`;
- telemetry age `160120` seconds at validation time;
- `telemetryFreshness=stale`;
- service entity present and bound;
- concrete observed runtime target `container:docker:2787bb511ae943e0eb522fed9237925bf8dd5c9ad166bc9821a96652fc61dac1`;
- last observed runtime state `running`;
- conservative assessment `comparisonStatus=unknown`, `reason=stale_telemetry`;
- `sideEffectsPerformed=false`.

The pre-existing GraphQL control-state history query also remained healthy and returned:

- `operational_dgraph` history count `1`;
- `resource_registry` history count `2`.

PASS proves one GraphQL read can provide accepted local control state, distributed observation, evidence quality and desired-observed assessment without crossing governance, placement or reconciliation boundaries.

## Read-only boundary

Step 65 adds no mutation and no subscription. GraphQL still cannot publish/change a D-Graph, authorize host mutation, mutate bindings, write telemetry, execute reconciliation, select placement, or invoke an AI/GNN decision.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver governs accepted D-Graph/resource state -> SQLite preserves local accepted state/history; dserver correlates desired vs observed under explicit freshness semantics; GraphQL exposes the resulting unified read context to dashboard/AI/GNN consumers. External AI/GNN remains responsible for placement/reasoning; SmartSentinel/WASM reconciles only when authorized; DATUM remains conceptual.`
