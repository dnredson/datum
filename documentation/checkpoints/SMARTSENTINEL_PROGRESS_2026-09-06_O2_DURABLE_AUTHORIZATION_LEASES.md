# SmartSentinel O2 checkpoint — durable operational authorization leases

Date: 2026-09-06
Branch: `agent/o2-durable-authorization-leases`
Baseline main: `dfe626e2aaa5ea7717a52e57bbfff85949441e0e`

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

## Step 58 — durable, finite, revision- and binding-scoped authorization leases

Step 58 adds optional durable host-mutation authorization without turning every existing authorization into persistent permission.

### Backward-compatible default

Existing authorization requests remain process-scoped by default. If `persistence_scope` is omitted, the authorization behaves as before and disappears on dserver restart.

New request fields are optional:

- `persistence_scope`: `process` or `durable`;
- `valid_for_seconds`: required only for `durable`.

### Durable lease contract

A durable authorization requires:

- active Resource Registry binding for the node;
- matching active D-Graph ID and revision;
- explicit `allow_host_mutation=true`;
- `persistence_scope=durable`;
- finite `valid_for_seconds` between 1 and 86400 seconds.

The resulting authorization records:

- `binding_id`;
- `persistence_scope`;
- `expires_at_utc`.

The lease is therefore scoped simultaneously to:

- project;
- D-Graph ID;
- D-Graph revision;
- node ID;
- active binding ID;
- finite wall-clock validity.

### Runtime validity

A slice exposes an authorization only if all current context still matches and the lease has not expired.

A process-scoped authorization has no persisted expiry and remains valid only for the current process.

A durable authorization without a valid future `expires_at_utc` is never accepted as current authorization.

### Persistence

The existing `.datum-operational-dgraph-state.json` format is extended additively with an `authorizations` map using `#[serde(default)]`. Step-57 files without that field remain readable and restore with an empty durable-authorization set.

Only `persistence_scope=durable` authorizations may be written to the durable state file.

Persisted authorization validation requires:

- matching project/node key;
- matching active persisted D-Graph ID/revision/node;
- `allow_host_mutation=true`;
- non-empty `binding_id`;
- RFC3339 `authorized_at_utc` and `expires_at_utc`;
- `persistence_scope=durable`.

On restore, dserver additionally checks the current durable Resource Registry binding and current time before reactivating a persisted lease.

### Authorization replacement semantics

Any newly issued authorization replaces the previous authorization for the node.

- durable -> candidate state persists the new lease before memory is updated;
- process -> if the current authorization was durable, the persisted durable lease is removed before the process-scoped authorization is installed in memory.

This prevents an older durable authorization from being hidden by a process authorization and unexpectedly returning after restart.

### D-Graph revision invalidation

Publishing a higher D-Graph revision removes authorization state for that project from both the persisted candidate and memory after the new graph is durably committed.

A lease for revision 3 therefore cannot authorize revision 4.

### Binding lineage

Operational authorization now references the active Resource Registry `binding_id`, not only `node_id`.

The Resource Registry API exposes an internal active-binding lookup backed by the durable step-56 state. A restored authorization is ignored if its binding ID no longer matches the current active binding.

### Automated validation

Core tests cover:

- process authorization remains binding/revision scoped;
- durable authorization requires finite validity;
- durable lease expiration;
- persistence round-trip containing graph revision and durable authorization;
- rejection of process-scoped authorizations in persisted state.

### Live validation — PASS

The live restart proof completed successfully on 2026-09-06.

Observed durable lease before restart:

- `authorization_id=opauth:fe8478f9-1376-4514-9a53-3bd39fcbf9a7`;
- `dgraph_revision=3`;
- `node_id=fog-01`;
- `binding_id=binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4`;
- `persistence_scope=durable`;
- `expires_at_utc=2026-09-06T12:49:25.404885400+00:00`;
- `allow_host_mutation=true`.

Desktop inspection confirmed the same authorization persisted in `.datum-operational-dgraph-state.json`.

After restarting dserver before lease expiry, the fog-01 slice restored the exact same authorization ID and binding ID. The explicit comparison returned:

`DURABLE_AUTH_RESTART_PASS`

Publishing Operational D-Graph revision 4 then immediately removed the revision-3 authorization from the active slice while retaining MQTT as desired/running. A subsequent dserver restart restored revision 4 and did not restore the old authorization.

PASS criteria therefore hold:

- durable authorization survives restart only when explicitly requested and unexpired;
- exact `authorization_id` survives restart;
- binding lineage survives and remains checked;
- a new D-Graph revision invalidates the prior durable lease immediately;
- invalidated authorization does not reappear after another restart.

## Architecture statement

`dserver durably retains resource binding + desired D-Graph state; optional host-mutation authority is a finite lease explicitly scoped to the exact graph revision and binding lineage. SmartSentinel consumes that governed slice and never creates placement or authorization itself. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
