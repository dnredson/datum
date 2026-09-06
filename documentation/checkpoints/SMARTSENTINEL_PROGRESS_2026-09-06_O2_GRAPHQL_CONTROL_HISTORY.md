# SmartSentinel O2 checkpoint — GraphQL control-state history

Date: 2026-09-06
Branch: `agent/o2-graphql-control-history`
Baseline main: `f6f7d6207b4ca5d3ca2b1b4daca4222c8baa90f5`

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

## Step 61 — SQLite control-state history through GraphQL

Step 61 turns the append-only history introduced in step 59 into a read model available through the read-only GraphQL context plane.

No storage ownership changes are introduced:

- SQLite remains the local durable store for accepted dserver control-plane state;
- Firebase remains the distributed telemetry/observation store;
- GraphQL remains read-only and composes/query-exposes those sources.

## New SQLite read capability

`dserver/src/storage/control_state.rs` exposes a typed `ControlStateHistoryEntry` and a bounded history loader.

History reads:

- filter by one durable domain;
- order newest-first by append-only SQLite row id;
- return the original persisted JSON payload without reinterpreting it;
- support a bounded result limit.

The existing append/current-state persistence semantics are unchanged.

## New GraphQL queries

`controlStateDomains`

Returns each current SQLite control-state domain with:

- domain name;
- last update timestamp;
- append-only history count.

`controlStateHistory(domain: ControlStateDomain!, limit: Int)`

Allowed domains are explicit GraphQL enum values:

- `RESOURCE_REGISTRY`;
- `OPERATIONAL_DGRAPH`.

The optional limit defaults to 20 and must be between 1 and 100.

Each history entry returns:

- SQLite history row id;
- domain;
- recorded timestamp;
- original persisted payload as GraphQL JSON.

When `DATUM_CONTROL_STATE_DB_PATH` is absent, these SQLite-history queries fail explicitly rather than silently reading legacy JSON files.

## Live validation — PASS

The Windows dserver used the step-59 SQLite database at `C:\DatumState\datum-control-plane.db` and the existing locked GraphQL dependency set.

`controlStateDomains` returned the same durable domains and counts previously shown by the SQLite inspector:

- `operational_dgraph`: `historyCount=1`, updated at `2026-09-06T12:50:12.811547300+00:00`;
- `resource_registry`: `historyCount=2`, updated at `2026-09-06T12:51:25.701613900+00:00`.

`controlStateHistory(domain: OPERATIONAL_DGRAPH, limit: 5)` returned newest history row `id=2` with:

- `dgraph_id=dgraph:o2-mqtt-role-1`;
- active `revision=4`;
- no GraphQL errors.

`controlStateHistory(domain: RESOURCE_REGISTRY, limit: 5)` returned newest-first rows `id=3` then `id=1`, proving append-order history. Both rows retained the same durable binding:

`binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4 -> fog-01 -> active`

while the resource observation history evolved from:

- row `id=1`: `observation_count=1`;
- row `id=3`: `observation_count=2`.

The same resource identity remained bound throughout:

`resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`.

This proves the GraphQL read plane can retrieve both current unified context and historical accepted control-plane state without changing the Firebase/SQLite ownership boundary.

## Read-only boundary

Step 61 adds no GraphQL mutation or subscription.

It cannot:

- mutate Resource Registry state;
- publish/activate a D-Graph;
- authorize host mutation;
- execute reconciliation;
- modify SQLite history;
- write Firebase telemetry.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed observed context; dserver accepts/governs control state -> SQLite preserves current accepted truth plus append-only history; GraphQL exposes current and historical context to AI/GNN/dashboard clients. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
