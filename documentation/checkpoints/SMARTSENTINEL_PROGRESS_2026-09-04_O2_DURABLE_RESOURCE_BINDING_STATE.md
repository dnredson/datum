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
- 56/100 — IMPLEMENTED, pending compile/live restart validation.

The validated 55/100 baseline is on `main`. Step 56 is isolated in this branch.

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

Resource discovery and initial binding mutations now use a transaction lock and candidate-state workflow:

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

### Planned live restart proof

Because the currently running dserver predates step 56 and holds its Resource Registry only in process memory, the first restart into the step-56 binary cannot retroactively persist that old memory.

The intended live proof is therefore:

1. stop the old dserver only when ready for the step-56 test;
2. build/start the step-56 dserver;
3. rehydrate the existing fog-01 resource/binding once;
4. confirm `.datum-resource-registry-state.json` contains the bound fog resource and active binding;
5. stop and restart the step-56 dserver a second time without running the rehydrate script;
6. confirm `GET /api/v1/resources` and `GET /api/v1/resource-bindings` immediately restore the same resource/binding identity;
7. confirm Operational D-Graph is absent after the second restart, documenting the intentional step-56 boundary rather than treating it as a failure.

The key PASS criterion is that the same `resource_id`, `assigned_node_id=fog-01`, and `binding_id` survive the second process restart without rehydration.

## Architecture statement

`SmartSentinel discovers physical resource identity -> dserver durably preserves Resource Registry + explicit role binding -> later control-plane state references that durable lineage. dserver remains governance/control plane; D-Graph remains desired-state source; external AI remains responsible for placement/reasoning decisions; DATUM remains the conceptual framework.`
