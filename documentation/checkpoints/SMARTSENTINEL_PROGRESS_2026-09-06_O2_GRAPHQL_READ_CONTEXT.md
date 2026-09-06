# SmartSentinel O2 checkpoint — GraphQL read-only unified context

Date: 2026-09-06
Branch: `agent/o2-graphql-read-context`
Baseline main: `f77c3c682f11b46beeb7e19bc3e16b755a6c4a12`

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
- 60/100 — IMPLEMENTED, pending compile/live GraphQL validation.

## Step 60 — read-only GraphQL unified node context

Step 60 adds GraphQL as a query/context plane inside the existing dserver process. It is not a storage engine and it is not placed in the SmartSentinel reconciliation mutation path.

Storage/context split remains:

- Firebase / Firestore: distributed telemetry and latest observation context;
- SQLite embedded in dserver: accepted local control-plane state from step 59;
- GraphQL: read-only composition layer over local governed state plus remote/shared observation state.

## Runtime surface

New routes on the existing dserver listener:

- `POST /graphql` — GraphQL queries;
- `GET /graphiql` — interactive GraphiQL browser UI.

No additional daemon, port, container, or GraphQL server process is required.

The dserver uses `async-graphql` directly with the existing Axum 0.7 JSON handler rather than introducing another Axum integration layer.

## Initial schema

The first query is:

`node(projectId: String!, nodeId: String!): NodeContextView!`

It composes:

- `resource` — Resource Registry identity/current binding state;
- `binding` — active durable Resource Registry binding;
- `desired` — active Operational D-Graph slice and revision;
- `authorization` — only the authorization that is valid for the current graph revision and binding lineage;
- `latestTelemetry` — raw latest Firebase/Firestore telemetry document as GraphQL JSON;
- `telemetryAvailable`;
- `telemetrySource`;
- `telemetryError`.

The desired-state and authorization portions reuse the same in-memory control-plane stores and authorization validity rules used by the existing REST endpoints.

## Failure semantics

Firebase telemetry retrieval is fail-soft for GraphQL context composition.

If Firebase is unavailable or the latest telemetry document cannot be loaded:

- local Resource Registry/binding state remains queryable;
- local Operational D-Graph desired state remains queryable;
- valid local authorization remains queryable;
- `latestTelemetry=null`;
- `telemetryAvailable=false`;
- `telemetryError` contains the retrieval error.

The GraphQL query therefore does not convert loss of observation connectivity into loss of accepted control-plane knowledge.

Errors reading the local governed stores remain GraphQL errors because those stores are the source of accepted local control-plane truth.

## Read-only boundary

Step 60 exposes no GraphQL mutations and no subscriptions.

GraphQL cannot in this step:

- publish a D-Graph;
- create/renew authorization;
- mutate Resource Registry bindings;
- request SmartSentinel reconciliation;
- write telemetry;
- invoke placement/reasoning.

Existing REST/control contracts remain authoritative for mutation paths.

## Planned live proof

1. update the branch and run Cargo without `--locked` once to resolve the newly pinned `async-graphql` dependency;
2. build/test the dserver;
3. start dserver with `DATUM_CONTROL_STATE_DB_PATH` pointing to the step-59 SQLite database;
4. verify `GET /graphiql` returns HTTP 200 and GraphiQL HTML;
5. execute the GraphQL `node(projectId:"monitor-f49c8", nodeId:"fog-01")` query;
6. verify Resource Registry state returns the same fog-01 resource and active binding proven in steps 56/59;
7. verify desired state returns Operational D-Graph revision 4 and MQTT desired/running;
8. verify authorization is null because the revision-3 durable authorization was invalidated by revision 4 in step 58;
9. inspect `telemetryAvailable/latestTelemetry` from the existing Firebase latest snapshot path;
10. verify the existing REST endpoints still return the same resource binding and Operational D-Graph revision after GraphQL queries;
11. optional fail-soft proof: query a node with no Firebase latest document (or otherwise induce only Firebase lookup failure) and verify local control state remains present while telemetry reports unavailable.

PASS proves GraphQL composes accepted control state and distributed observation context without changing storage ownership or reconciliation authority.

## Next scope

After step 60, likely follow-on work is:

- expose SQLite control-state history/read models through GraphQL;
- persist/query operational evidence history locally;
- formalize Firebase passive event-driven and active graph-scan telemetry analysis modes;
- materialize desired-vs-observed context for AI/GNN consumers.

## Architecture statement

`SmartSentinel observes and publishes telemetry -> Firebase preserves distributed observation context; dserver governs and persists accepted binding/D-Graph/authorization state -> SQLite preserves local control-plane continuity; GraphQL presents a unified read context to AI/GNN/dashboard clients. GraphQL does not place workloads, authorize host mutation, or execute reconciliation. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
