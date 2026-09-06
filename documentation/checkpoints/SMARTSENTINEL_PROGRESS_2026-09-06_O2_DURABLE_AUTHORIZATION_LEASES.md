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
- 58/100 — IMPLEMENTED, pending compile/live restart validation.

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

### Live validation plan

The planned live proof starts from the durable revision-3 graph and durable fog-01 binding already validated in steps 56–57.

1. build/test the step-58 dserver without deleting existing persistence files;
2. confirm the old step-57 file migrates additively and still restores revision 3;
3. issue a legacy/process authorization without the new fields and confirm `persistence_scope=process` plus current `binding_id`;
4. restart dserver and confirm that process authorization is absent;
5. issue a durable revision-3 authorization with a finite lease and record its `authorization_id`, `binding_id`, and expiry;
6. restart dserver before expiry and confirm the exact same authorization is restored in the fog-01 slice;
7. publish revision 4 using `scripts/fixtures/o2-operational-mqtt-dgraph-rev4.json`;
8. confirm the slice immediately returns no authorization;
9. restart again and confirm revision 4 persists while the old revision-3 authorization does not reappear.

PASS means durable authorization survives only when explicitly requested, finite, unexpired, revision-matching, and bound to the same physical-resource role binding.

## Architecture statement

`dserver durably retains resource binding + desired D-Graph state; optional host-mutation authority is a finite lease explicitly scoped to the exact graph revision and binding lineage. SmartSentinel consumes that governed slice and never creates placement or authorization itself. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
