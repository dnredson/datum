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
- 57/100 — IMPLEMENTED, pending compile/live restart validation.

The validated 56/100 baseline is on `main`. Step 57 is isolated in this branch.

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

This means a restart restores desired state but does not resurrect a previous host-mutation authorization. A restored node slice must therefore carry the desired services and D-Graph revision while `authorization` remains absent until the control plane explicitly authorizes the current revision again.

### Persistence contract

New core module: `dserver/src/core/operational_dgraph_persistence.rs`.

Persistence version: `0.1.0`.

The persisted document contains a map of project ID to validated `OperationalDGraph`.

Validation rejects:

- unsupported persistence versions;
- graph payloads that fail the normal Operational D-Graph validator;
- project map keys that do not match `graph.project_id`.

### File durability

The file lifecycle matches the previously validated dserver persistence discipline:

- pretty JSON serialization;
- sibling `.tmp` write;
- `sync_all` before activation;
- previous primary moved to `.bak` when present;
- temporary file renamed to the primary;
- backup restoration attempt if activation fails;
- successful backup cleanup is best-effort.

Default path:

`.datum-operational-dgraph-state.json`

Optional override:

`DATUM_OPERATIONAL_DGRAPH_STATE_PATH`

Primary, `.tmp`, and `.bak` files are ignored by Git.

### Transaction semantics

`PUT /api/v1/operational-dgraph` now uses a transaction lock and candidate-state flow:

1. validate the incoming graph and project contract;
2. apply existing revision rules against the active graph;
3. build a complete candidate persisted graph map;
4. persist and validate the candidate;
5. only after persistence succeeds, replace in-memory active graphs;
6. if a higher revision replaced an existing graph, invalidate process-local operational authorizations for that project.

The existing revision semantics remain unchanged:

- lower revision -> `409 Conflict`;
- same revision + identical content -> idempotent success;
- same revision + different content -> `409 Conflict`;
- higher revision -> replace active graph and invalidate current authorization.

### Lazy restore

Operational D-Graph persistence is initialized lazily on first Operational D-Graph API use.

On valid restore:

- active graphs are restored;
- process-local authorizations are explicitly cleared.

On missing persisted state:

- active graph state starts empty;
- authorizations start empty.

On invalid/corrupt persisted state:

- persisted state is not trusted;
- active graph and authorization state start empty.

### Compatibility

The existing HTTP contracts remain unchanged:

- `PUT/GET /api/v1/operational-dgraph`;
- `GET /api/v1/nodes/:node_id/operational-dgraph-slice`;
- `POST /api/v1/nodes/:node_id/operational-reconciliation/authorize`.

Resource Registry/binding persistence from 56 remains independent and unchanged.

### Automated validation

The persistence module includes tests for:

- project-key / graph-project consistency;
- persistence round-trip preserving revision 3.

### Planned live restart proof

The intended proof uses the already durable fog-01 Resource Registry/binding state from step 56:

1. stop dserver only when ready to build the step-57 binary;
2. build/test step 57;
3. start dserver with both durable Resource Registry and Operational D-Graph paths configured;
4. publish revision 3 once (no resource/binding rehydration should be needed);
5. authorize revision 3 and confirm the slice carries the authorization before restart;
6. confirm `.datum-operational-dgraph-state.json` records revision 3;
7. stop/restart dserver without publishing or rehydrating any graph/resource/binding state;
8. confirm `GET /api/v1/operational-dgraph` immediately returns the same D-Graph ID and revision 3;
9. confirm the fog-01 slice still contains MQTT desired running and revision 3;
10. confirm the restored slice has no authorization;
11. confirm a revision-2 PUT is rejected with 409 after restart, proving the restored revision participates in normal regression protection;
12. explicitly authorize revision 3 again and confirm the authorization reappears.

PASS means desired state/revision survives the process restart while host-mutation authorization remains fail-closed and requires explicit renewal.

## Architecture statement

`SmartSentinel observes -> dserver durably retains physical-resource binding and active D-Graph desired state -> authorization remains explicit and revision-scoped -> SmartSentinel reconciles only when currently authorized. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
