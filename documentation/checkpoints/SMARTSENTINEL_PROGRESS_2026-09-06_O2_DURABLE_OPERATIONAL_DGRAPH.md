# SmartSentinel O2 checkpoint — durable Operational D-Graph state

Date: 2026-09-06
Branch: `agent/o2-durable-operational-dgraph`
Baseline main: `95fa481ffd114b7bb37cd7c9a506432397f14ec1`

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

The validated step-57 state is ready to advance to `main`.

## Step 57 — durable active Operational D-Graph

Step 57 persists the active Operational D-Graph per project so desired service state and D-Graph revision survive dserver process restarts.

### Scope boundary

Step 57 persists:

- active Operational D-Graph per project;
- graph identity and content;
- active graph revision.

Step 57 deliberately does **not** persist:

- operational reconciliation authorizations;
- remote operational evidence.

This means a restart restores desired state but does not resurrect a previous host-mutation authorization. A restored node slice therefore carries the desired services and D-Graph revision while `authorization` remains absent until the control plane explicitly authorizes the current revision again.

### Persistence contract

New core module: `dserver/src/core/operational_dgraph_persistence.rs`.

Persistence version: `0.1.0`.

The persisted document contains a map of project ID to validated `OperationalDGraph`.

Validation rejects unsupported persistence versions, invalid Operational D-Graph payloads, and project map keys that do not match `graph.project_id`.

### File durability

The file lifecycle matches the previously validated dserver persistence discipline: pretty JSON serialization, sibling `.tmp` write, `sync_all`, previous primary moved to `.bak`, activation by rename, backup restoration attempt on activation failure, and best-effort cleanup after success.

Default path: `.datum-operational-dgraph-state.json`.

Optional override: `DATUM_OPERATIONAL_DGRAPH_STATE_PATH`.

### Transaction semantics

`PUT /api/v1/operational-dgraph` now uses a transaction lock and candidate-state flow: validate, apply revision rules, construct complete candidate, persist candidate, then replace in-memory graphs. A higher revision invalidates current process-local operational authorizations only after the durable graph commit succeeds.

The existing revision semantics remain unchanged:

- lower revision -> `409 Conflict`;
- same revision + identical content -> idempotent success;
- same revision + different content -> `409 Conflict`;
- higher revision -> replace active graph and invalidate current authorization.

### Lazy restore

Operational D-Graph persistence is initialized lazily on first Operational D-Graph API use. Valid graphs are restored, while process-local authorizations are explicitly cleared. Missing or invalid persisted state starts fail-closed with empty graph/authorization memory.

### Live restart validation — PASS

The live proof used the already durable fog-01 Resource Registry/binding state from step 56.

After publishing and authorizing D-Graph revision 3, dserver was stopped and restarted without republishing or rehydrating graph/resource/binding state.

Observed after restart:

- dserver logged restoration of `.datum-operational-dgraph-state.json`;
- dserver also restored `.datum-resource-registry-state.json`;
- `GET /api/v1/operational-dgraph` immediately returned `dgraph:o2-mqtt-role-1`, `revision=3`, `project_id=monitor-f49c8`;
- `GET /api/v1/nodes/fog-01/operational-dgraph-slice` immediately returned revision 3 and MQTT `container/running` desired state;
- the restored slice returned `authorization=null`, proving the prior host-mutation authorization did not survive restart;
- attempting to publish the revision-2 fixture returned HTTP `409` with `operational D-Graph revision regression: active revision is 3, received 2`.

These results prove the restored revision is active control-plane state, not merely archived JSON, while authorization remains fail-closed across restart.

## Architecture statement

`SmartSentinel observes -> dserver durably retains physical-resource binding and active D-Graph desired state -> authorization remains explicit and revision-scoped -> SmartSentinel reconciles only when currently authorized. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
