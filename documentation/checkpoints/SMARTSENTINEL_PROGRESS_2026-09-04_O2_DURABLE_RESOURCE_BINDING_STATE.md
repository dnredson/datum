# SmartSentinel O2 checkpoint — durable Resource Registry and binding state

Date: 2026-09-04
Branch: `agent/o2-durable-control-plane-state`
Baseline main: `1bf29169a0bc4ea903fcf753ab2914b480fbe738`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.
- 54/100 — LIVE PASS.
- 55/100 — LIVE PASS.
- 56/100 — LIVE PASS.

Step 56 is closed after compile/test plus live restart validation on the real dserver/fog-01 environment.

## Step 56 — durable Resource Registry + role-binding state

Step 56 begins the durable-control-plane phase by persisting the state that establishes physical-resource identity and its explicit logical node binding.

This directly removes the previous requirement to recreate Resource Registry and binding state after every dserver restart.

### Scope boundary

Step 56 persists only:

- Resource Registry records;
- role-binding records/history.

Operational D-Graph, operational reconciliation authorizations, and remote operational evidence remain process-local in step 56. Their persistence belongs to following steps in the 56–65 durable-control-plane phase.

This boundary is intentional so restart validation can prove exactly which state became durable.

### Persistence contract

New core module: `dserver/src/core/resource_registry_persistence.rs`.

Persistence version: `0.1.0`.

The persisted document contains Resource Registry and binding maps together. Validation rejects inconsistent durable state, including:

- unsupported persistence/schema versions;
- map keys that do not match record project/resource/binding identity;
- `available_unbound` resources carrying an assigned node;
- `bound` resources without an assigned node;
- active bindings referencing missing resources;
- active bindings whose node does not match the resource registry assignment;
- bound resources without exactly one matching active binding;
- more than one active resource binding to the same project/node.

Resource discovery payloads are revalidated when durable state is loaded.

### Transaction semantics

Resource discovery and initial binding mutations use a transaction lock and candidate-state workflow:

1. clone current in-memory state;
2. apply and validate the candidate mutation;
3. persist the complete candidate state;
4. only after persistence succeeds, replace the in-memory state;
5. return the API result.

A persistence failure therefore cannot leave memory ahead of durable state.

### File durability

The implementation follows the already validated Controlled WASM persistence discipline:

- serialize pretty JSON;
- write a sibling `.tmp` file;
- `sync_all` the temporary file;
- move the previous primary to `.bak` when present;
- activate the temporary file as the new primary;
- restore the backup if activation fails;
- remove backup best-effort after successful commit.

Default path:

`.datum-resource-registry-state.json`

Optional override:

`DATUM_RESOURCE_REGISTRY_STATE_PATH`

Primary, `.tmp`, and `.bak` files are ignored by Git.

### Lazy restore

The Resource Registry API lazily initializes persistence on first use after dserver startup.

If a valid persisted file exists, Resource Registry and bindings are restored together before serving the request.

If persisted state is invalid/corrupt, it is not trusted; the resource registry starts empty, which is fail-closed for later operational authorization because no binding can be inferred from stale node snapshots.

### Compatibility

The HTTP contracts remain unchanged:

- `POST /api/v1/resources/discovery`;
- `GET /api/v1/resources`;
- `GET /api/v1/resources/:resource_id`;
- `GET /api/v1/resource-bindings`;
- `POST/GET /api/v1/resources/:resource_id/binding`.

`resource_registry_snapshot()` now reads restored durable state, so existing operational authorization checks automatically benefit from persistent binding state.

### Automated validation

The persistence module includes tests for:

- bound-resource/binding consistency validation;
- JSON persistence round-trip through the atomic file lifecycle.

## Live restart validation — PASS

The step-56 dserver was seeded once after moving from the pre-persistence binary. The fog-01 identity before the durability restart was:

- resource: `resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`;
- assigned node: `fog-01`;
- binding: `binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4`.

After stopping and restarting the step-56 dserver without running the rehydration script:

- `GET /api/v1/resources` immediately returned one resource with `bound_count=1`;
- the same resource ID remained `state=bound` and `assigned_node_id=fog-01`;
- `GET /api/v1/resource-bindings` immediately returned one active binding;
- the restored binding ID was exactly `binding:af8d1eb7-10e4-499a-b0f5-248e4bbe30f4`;
- the explicit before/after comparison printed `BINDING_PERSISTENCE_PASS`.

This proves Resource Registry and explicit role-binding identity survived a real dserver process restart without rehydration.

The same post-restart session intentionally queried `GET /api/v1/operational-dgraph` and received HTTP 404 with `operational D-Graph not found`. This is also a PASS result for the step boundary: Operational D-Graph remained process-local exactly as documented and becomes the next durable-control-plane target.

### Closure boundary after step 56

After a dserver restart:

- Resource Registry — durable / restored;
- role bindings — durable / restored with the same binding identity;
- Operational D-Graph — still process-local;
- operational reconciliation authorization — still process-local;
- remote operational evidence — still process-local.

Step 56 is therefore CLOSED / LIVE PASS. Step 57 should persist the active Operational D-Graph and revision while retaining the existing revision/idempotency/invalidation semantics.

## Architecture statement

`SmartSentinel discovers physical resource identity -> dserver durably preserves Resource Registry + explicit role binding -> later control-plane state references that durable lineage. dserver remains governance/control plane; D-Graph remains desired-state source; external AI remains responsible for placement/reasoning decisions; DATUM remains the conceptual framework.`
