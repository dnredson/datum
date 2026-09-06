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
- 62/100 — LIVE PASS.

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

`UNIQUE(project_id, event_id)` preserves duplicate-event rejection across dserver restarts.

Indexes cover project/time, resource, node, D-Graph revision and event kind.

## Live validation

Live validation used project `monitor-f49c8`, node `fog-01`, active D-Graph revision 4 and resource `resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`.

Validation event:

- `event_id=opev:step62-live-1788724605`;
- `event_kind=step62_validation_probe`;
- `dgraph_revision=4`;
- `sequence=1`;
- `side_effects_performed=false`;
- payload recorded `validation_step=62` and `purpose=sqlite_restart_durability`.

Observed proof:

1. POST accepted and returned the expected event identity/context;
2. GET filtered by `event_kind=step62_validation_probe` returned exactly the persisted event;
3. reposting the same `event_id` returned HTTP 409 with duplicate-event error;
4. dserver was stopped and restarted without reposting the event;
5. the same event was returned after restart from SQLite with the same event id, revision, kind and payload.

This proves both event durability and duplicate protection survive dserver process restart.

## Storage semantics

The evidence table is intentionally separate from `control_state_history`.

- `control_state_history` answers how accepted dserver control state changed;
- `operational_evidence` answers what operational events/observations/reconciliations were reported.

This avoids conflating desired/governed state history with event/evidence history while keeping both in the same embedded `datum-control-plane.db` file.

## Architecture statement

`SmartSentinel observes -> Firebase preserves distributed telemetry; dserver receives governed operational evidence -> SQLite preserves accepted local evidence and control-state history; GraphQL remains the read/context plane. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
