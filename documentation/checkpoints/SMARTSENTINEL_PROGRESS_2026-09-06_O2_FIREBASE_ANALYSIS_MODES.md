# SmartSentinel O2 checkpoint — Firebase passive/active analysis context acquisition

Date: 2026-09-06
Branch: `agent/o2-firebase-analysis-modes`
Baseline main: `3e73826f59ea9a6f6609d6149fb9e8aaf50852bc`

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
- 63/100 — IMPLEMENTED, pending compile/live validation.

## Step 63 — explicit passive and active Firebase analysis acquisition modes

Step 63 formalizes the two dserver observation-analysis acquisition modes without adding placement reasoning or reconciliation side effects.

### Passive mode

`POST /api/v1/analysis-context/passive/:node_id`

The caller identifies the node whose new observation triggered analysis. dserver:

1. loads the accepted active Operational D-Graph from local control state;
2. verifies the trigger node belongs to that active D-Graph revision;
3. reads only that node's latest telemetry document from Firebase/Firestore;
4. returns a scoped analysis context containing desired services plus observed telemetry.

This models event-driven/passive analysis: dserver reacts to a notification rather than scanning the graph itself.

### Active mode

`POST /api/v1/analysis-context/active`

dserver:

1. loads the accepted active Operational D-Graph;
2. traverses every node in that graph revision;
3. starts Firebase latest-telemetry reads for all graph nodes concurrently using Tokio tasks;
4. returns one context containing desired state and telemetry acquisition result per graph node.

This models server-initiated/active analysis while avoiding serial network latency across graph nodes.

## Context contract

The response includes:

- `schema_version=0.1.0`;
- `acquisition_mode=passive|active`;
- project, D-Graph id and active revision;
- generation timestamp;
- passive `trigger_node_id` when applicable;
- node count;
- `side_effects_performed=false`;
- per node: desired services, telemetry source, availability, observed timestamp when present, raw latest telemetry, and retrieval error when unavailable.

Firebase failure is fail-soft at node level: accepted local desired state remains present even when telemetry is unavailable.

No freshness classification is invented in step 63. The raw telemetry observation timestamp is exposed so a later explicit policy can classify fresh/stale/missing.

## Architecture boundary

Step 63 does not:

- place workloads;
- choose another node/cloud target;
- mutate D-Graph state;
- authorize host mutation;
- execute reconciliation;
- write telemetry;
- invoke an AI/GNN decision.

It only acquires and composes the context that a later analyzer/AI may consume.

## Planned live proof

1. build/test the new pure analysis-context model;
2. start dserver with the existing step-59+ SQLite database and Firebase configuration;
3. call passive mode for `monitor-f49c8/fog-01` and verify D-Graph revision 4, MQTT desired/running, one node, Firebase telemetry available and `side_effects_performed=false`;
4. call active mode and verify it traverses the current rev4 graph, returns every graph node, preserves desired state and retrieves each latest Firebase telemetry independently;
5. verify both modes report the telemetry observation timestamp when Firestore provides it;
6. verify existing GraphQL current/history and operational evidence endpoints remain unchanged.

PASS proves dserver can obtain observation context in both event-driven and server-initiated forms without collapsing observation, governance and placement responsibilities.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver may react passively to an observation notification or actively traverse the accepted D-Graph to acquire latest telemetry; these modes build context only. External AI/GNN remains responsible for placement/reasoning, dserver governs accepted state, SmartSentinel/WASM reconciles only when authorized, and DATUM remains the conceptual framework.`
