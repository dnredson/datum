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
- 60/100 — LIVE PASS.

## Step 60 — read-only GraphQL unified node context

Step 60 adds GraphQL as a query/context plane inside the existing dserver process. It is not a storage engine and it is not placed in the SmartSentinel reconciliation mutation path.

Storage/context split remains:

- Firebase / Firestore: distributed telemetry and latest observation context;
- SQLite embedded in dserver: accepted local control-plane state from step 59;
- GraphQL: read-only composition layer over local governed state plus remote/shared observation state.

## Runtime surface

New routes on the existing dserver listener:

- `POST /graphql` — GraphQL queries;
- `GET /graphiql` — GraphiQL browser helper page.

No additional daemon, port, container, or GraphQL server process is required. GraphQL is compiled into the existing dserver through `async-graphql`.

The dserver uses `async-graphql` directly with the existing Axum 0.7 JSON handler. The async-graphql 7.0.2 core/derive/parser/value crates are pinned to the same exact release after live Windows compilation exposed an incompatible mixed 7.x dependency resolution.

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

## Live validation — PASS

Windows dserver compile/tests passed after pinning the complete `async-graphql` internal crate family to version `7.0.2` and committing the validated Cargo lockfile.

Live GraphQL proof against `monitor-f49c8` / `fog-01` returned in one query:

- project/node context: `monitor-f49c8` / `fog-01`;
- durable resource: `resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`;
- resource state: `bound` and assigned to `fog-01`;
- observation count: `2`;
- active binding: `binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4`;
- Operational D-Graph: `dgraph:o2-mqtt-role-1`, revision `4`;
- desired service: `mqtt`, runtime `container`, desired state `running`;
- authorization: `null`, preserving the revision-4 invalidation of the former revision-3 durable lease;
- telemetry source: `firebase`;
- `telemetryAvailable=true`;
- latest Firebase snapshot successfully returned the fog-01 host/container/service/network observation document.

A second compact GraphQL query returned the same binding, revision 4, MQTT desired state, null authorization, and Firebase telemetry availability.

`GET /graphiql` returned HTTP 200. The browser helper UI remained on loading, treated as a non-blocking development-UI issue because the real `POST /graphql` endpoint executed live queries successfully. The GraphiQL helper may later be replaced by an offline-safe/local console if useful.

PASS proves GraphQL composes accepted control state and distributed observation context without changing storage ownership or reconciliation authority.

## Next scope

After step 60:

- expose SQLite control-state history/read models through GraphQL;
- persist/query operational evidence history locally;
- formalize Firebase passive event-driven and active graph-scan telemetry analysis modes;
- materialize desired-vs-observed context for AI/GNN consumers.

## Architecture statement

`SmartSentinel observes and publishes telemetry -> Firebase preserves distributed observation context; dserver governs and persists accepted binding/D-Graph/authorization state -> SQLite preserves local control-plane continuity; GraphQL presents a unified read context to AI/GNN/dashboard clients. GraphQL does not place workloads, authorize host mutation, or execute reconciliation. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
