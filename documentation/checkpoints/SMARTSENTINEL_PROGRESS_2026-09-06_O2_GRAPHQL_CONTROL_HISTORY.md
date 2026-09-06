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
- 61/100 — IMPLEMENTED, pending compile/live history-query validation.

## Step 61 — SQLite control-state history through GraphQL

Step 61 turns the append-only history introduced in step 59 into a read model available through the read-only GraphQL context plane.

No storage ownership changes are introduced:

- SQLite remains the local durable store for accepted dserver control-plane state;
- Firebase remains the distributed telemetry/observation store;
- GraphQL remains read-only and composes/query-exposes those sources.

## New SQLite read capability

`dserver/src/storage/control_state.rs` now exposes a typed `ControlStateHistoryEntry` and a bounded history loader.

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

## Read-only boundary

Step 61 adds no GraphQL mutation or subscription.

It cannot:

- mutate Resource Registry state;
- publish/activate a D-Graph;
- authorize host mutation;
- execute reconciliation;
- modify SQLite history;
- write Firebase telemetry.

## Planned live proof

1. update to `agent/o2-graphql-control-history`;
2. build/test dserver using the already locked GraphQL dependency set;
3. start dserver with `DATUM_CONTROL_STATE_DB_PATH=C:\DatumState\datum-control-plane.db`;
4. query `controlStateDomains` and verify the same counts already observed by `control-state-inspect` (`operational_dgraph` and `resource_registry`);
5. query `controlStateHistory(domain: OPERATIONAL_DGRAPH, limit: 5)` and verify the newest payload contains active Operational D-Graph revision 4;
6. query `controlStateHistory(domain: RESOURCE_REGISTRY, limit: 5)` and verify the newest payload contains the durable fog-01 resource/binding and current observation count;
7. verify ordering is newest-first and the requested limit is honored;
8. repeat the step-60 compact `node(...)` query to ensure unified current context remains unchanged.

PASS proves the dserver can expose both current unified context and durable accepted-state history through one read-only GraphQL plane while preserving Firebase/SQLite ownership boundaries.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed observed context; dserver accepts/governs control state -> SQLite preserves current accepted truth plus append-only history; GraphQL exposes current and historical context to AI/GNN/dashboard clients. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
