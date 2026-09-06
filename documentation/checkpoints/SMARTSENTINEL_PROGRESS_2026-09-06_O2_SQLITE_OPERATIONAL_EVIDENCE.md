# SmartSentinel O2 checkpoint — durable SQLite operational evidence

Date: 2026-09-06
Branch: `agent/o2-sqlite-operational-evidence`
Baseline main: `8f9ab4c2a90ba59ce199c30558404f5dee506e6a`

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
- 62/100 — IMPLEMENTED, pending compile/live restart validation.

## Step 62 — durable operational evidence in SQLite

Step 62 moves dserver operational evidence from process-local memory into the same embedded SQLite database introduced in step 59 whenever `DATUM_CONTROL_STATE_DB_PATH` is configured.

The external REST contract remains unchanged:

- `POST /api/v1/operational-evidence`;
- `GET /api/v1/operational-evidence`;
- existing project header and filters;
- duplicate `event_id` remains a conflict.

When SQLite is not configured, the pre-step-62 process-local in-memory behavior remains as fallback compatibility.

## SQLite evidence model

New module:

`dserver/src/storage/operational_evidence.rs`

New append-only table:

`operational_evidence`

Columns retain queryable causal/context fields:

- internal SQLite row id;
- `project_id`;
- `event_id`;
- `occurred_at_utc`;
- `resource_id`;
- optional `node_id`;
- optional `dgraph_id`;
- optional `dgraph_revision`;
- `sequence`;
- `event_kind`;
- `side_effects_performed`;
- original event `payload` serialized as JSON.

`UNIQUE(project_id, event_id)` preserves the existing duplicate-event rejection rule across dserver restarts.

Indexes cover project/time, resource, node, D-Graph revision and event kind.

## Storage semantics

The evidence table is intentionally separate from `control_state_history`.

- `control_state_history` answers how accepted dserver control state changed;
- `operational_evidence` answers what operational events/observations/reconciliations were reported.

This avoids conflating desired/governed state history with event/evidence history while keeping both in the same embedded `datum-control-plane.db` file.

## Planned live proof

1. update to `agent/o2-sqlite-operational-evidence`;
2. build/test with the existing locked dependencies;
3. start dserver with `DATUM_CONTROL_STATE_DB_PATH=C:\DatumState\datum-control-plane.db`;
4. POST one clearly marked step-62 validation evidence event for `monitor-f49c8/fog-01`, revision 4;
5. GET/filter it through the existing REST endpoint;
6. POST the same `event_id` again and verify HTTP 409;
7. stop/restart dserver without reposting the event;
8. GET/filter the event again and verify it survived restart;
9. verify Resource Registry binding, D-Graph rev4 and GraphQL current/history queries remain unchanged.

PASS proves operational evidence is durable and independently queryable without altering SmartSentinel observation ownership, D-Graph governance, or GraphQL mutation boundaries.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver receives governed operational evidence -> SQLite preserves accepted local evidence and control-state history; GraphQL remains the read/context plane. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
