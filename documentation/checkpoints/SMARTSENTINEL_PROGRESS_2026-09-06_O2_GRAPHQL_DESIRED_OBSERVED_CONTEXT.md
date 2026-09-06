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
- 65/100 — IMPLEMENTED, pending compile/live GraphQL validation.

## Step 65 — expose desired × observed × freshness in the unified GraphQL node context

Step 65 extends the existing read-only GraphQL `node(projectId,nodeId)` query rather than creating a parallel query plane.

The existing node context already exposes:

- Resource Registry resource identity/state;
- active binding;
- accepted Operational D-Graph desired state/revision;
- current valid authorization when one exists;
- latest Firebase telemetry document;
- telemetry availability/error.

Step 65 adds an optional typed `assessment` field containing the step-64 desired-vs-observed correlation and freshness semantics.

## GraphQL assessment fields

`assessment` exposes:

- analysis schema version;
- assessment generation timestamp;
- freshness policy max age;
- `telemetryFreshness = fresh | stale | missing`;
- telemetry age when known;
- telemetry observation timestamp;
- typed service comparisons;
- `sideEffectsPerformed=false`.

Each service comparison exposes:

- service id;
- runtime;
- desired state;
- `comparisonStatus = converged | drift | unknown`;
- machine-readable reason;
- service entity presence;
- observed service status;
- observed concrete runtime state;
- observed target id;
- observed binding state.

## Single-snapshot consistency

The GraphQL node resolver performs one Firebase latest-snapshot read. That same result is used both for:

- the existing raw `latestTelemetry` field; and
- the typed step-64 assessment.

The assessment calls the existing `build_passive_analysis_context()` core used by the REST analysis context. GraphQL does not implement a second desired-observed correlation algorithm.

This avoids intra-query races where raw telemetry and assessment could otherwise come from different snapshots.

## Failure semantics

If Firebase latest telemetry cannot be retrieved but the accepted D-Graph node exists:

- local resource/binding/desired state remains available;
- `latestTelemetry=null`;
- `telemetryAvailable=false`;
- the typed assessment can still report `telemetryFreshness=missing` and service comparisons `unknown/missing_telemetry`.

If no accepted Operational D-Graph exists for the node, the assessment is absent rather than inventing a desired state.

## Read-only boundary

Step 65 adds no mutation and no subscription.

GraphQL still cannot:

- publish or change a D-Graph;
- authorize host mutation;
- mutate resource bindings;
- write Firebase telemetry;
- execute reconciliation;
- select placement;
- invoke an AI/GNN decision.

## Planned live proof

1. update to `agent/o2-graphql-desired-observed-context`;
2. build/test with locked dependencies;
3. start dserver with the existing SQLite/Firebase configuration and 120-second freshness policy;
4. issue one GraphQL `node(projectId:"monitor-f49c8",nodeId:"fog-01")` query requesting binding, desired state, telemetry availability and assessment;
5. verify binding remains `binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4`;
6. verify desired D-Graph revision remains 4 and MQTT desired `running`;
7. verify assessment reports the current old Firebase snapshot as `stale`;
8. verify the service comparison still reports the last observed concrete container state `running` but conservatively returns `unknown/stale_telemetry`;
9. verify `sideEffectsPerformed=false`;
10. verify the pre-existing GraphQL control-state history query still works.

PASS proves one GraphQL read can provide accepted local control state, distributed observation, evidence quality and desired-observed assessment without crossing the governance/placement/reconciliation boundaries.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver governs accepted D-Graph/resource state -> SQLite preserves local accepted state/history; dserver correlates desired vs observed under explicit freshness semantics; GraphQL exposes the resulting unified read context to dashboard/AI/GNN consumers. External AI/GNN remains responsible for placement/reasoning; SmartSentinel/WASM reconciles only when authorized; DATUM remains conceptual.`
