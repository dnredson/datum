# SmartSentinel progress checkpoint — step 49/100 operational evidence and cadence

Date: 2026-09-04
Branch: `agent/o2-host-capability-discovery`
Progress index: **49/100 — LIVE PASS**

## Scope

Step 49 separates fast control-plane reconciliation from expensive host collection and preserves a durable local operational history while publishing the same evidence stream to dserver.

The operational evidence model is intended to support later temporal reasoning over the complete system history, including discovery, binding, D-Graph changes, WASM decisions, host mutations, failures, recovery, and subsequent convergence.

## Live validation result

Resource:

`resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`

Node after binding:

`fog-01`

D-Graph:

`dgraph:o2-mqtt-role-1`

### Fast loop before binding

The persistent SmartSentinel started unbound and published cached/periodically refreshed resource discovery while keeping the control loop responsive.

Observed wait-cycle durations:

- sequence 1: ~27 ms
- sequence 2: ~17 ms

Both cycles produced `reconciliation_waiting` evidence with no node_id, no dgraph_id, and no side effects.

This preserves the epistemic distinction between a known resource and an actually bound logical node.

### Binding and real reconciliation action

While SmartSentinel remained alive, dserver restored the governed binding, operational D-Graph, and reconciliation authorization.

At sequence 3, SmartSentinel resolved `fog-01`, executed the Wasmtime reconciliation, and recreated the missing MQTT container.

The preserved action evidence records:

- `event_kind=reconciliation_action`
- `node_id=fog-01`
- `dgraph_id=dgraph:o2-mqtt-role-1`
- `side_effects_performed=true`
- `execution_status=deployed`
- `observed_before_running=false`
- `wasm.action_code=1`
- `wasm.host_imports_allowed=false`
- `wasm.guest_side_effects_performed=false`
- `host_mutation_performed=true`
- `observed_after_running=true`
- `snapshot_published=true`

The mutation cycle took ~9.94 s because host mutation forced an immediate heavy post-reconciliation snapshot.

### Steady-state performance

Subsequent converged cycles no longer performed the heavy snapshot and completed substantially faster:

- sequence 4: ~1.64 s
- sequence 5: ~1.96 s
- sequence 6: ~1.18 s

Each returned:

- `wasm_actions=0`
- `execution_status=converged`
- `host_mutation=false`
- `snapshot_published=false`
- `snapshot_due=false`

Sequence 7 refreshed the expensive resource discovery and therefore took ~7.39 s. This demonstrates that the control loop and host collection cadence are now separated rather than every 5 s control cycle paying the full collector cost.

## Local append-only history

The live run produced 9 JSONL records in `smartsentinel-operational-history.jsonl`.

The sequence preserved the complete transition instead of overwriting the mutation event with the final steady-state report:

1. resource discovery
2. reconciliation waiting
3. reconciliation waiting
4. reconciliation action / deployed / WASM action 1 / snapshot true
5. reconciliation / converged / WASM action 0
6. reconciliation / converged / WASM action 0
7. reconciliation / converged / WASM action 0
8. resource discovery refresh
9. reconciliation / converged / WASM action 0

The ordinary JSON report remains a latest-state convenience view; the JSONL stream is the append-only local operational history.

## Remote operational evidence stream

The same events were published successfully to dserver through `/api/v1/operational-evidence`.

The remote query for the tested resource returned the same 9 events.

Filtering by `event_kind=reconciliation_action` returned exactly one preserved mutation event containing the full reconciliation payload, including D-Graph identity, node identity, trusted image/container mapping, Wasmtime fuel usage, action code, execution result, and observed post-action state.

The remote O2 evidence store is still process-local. Durable control-plane persistence remains intentionally scheduled for the later persistence block.

## AI / historical reasoning intent

The evidence stream is designed so future reasoning can analyze not only a current graph snapshot but an ordered causal history such as:

`resource discovery → binding → desired state → deploy → steady state → degradation → failure evidence → D-Graph update → reconciliation action → observed recovery`

This allows a placement/reasoning AI to evaluate the full history around an event rather than infer causality from isolated latest-state snapshots.

SmartSentinel records observations and governed actions; it does not itself perform global placement reasoning.

## Step 49 closure

**49/100 — LIVE PASS.**

Validated properties:

- fast control loop separated from expensive discovery refresh;
- heavy snapshot separated from steady-state reconciliation;
- mutation forces immediate post-action snapshot;
- latest report retained for convenience;
- append-only local JSONL history preserves every evidence event;
- same evidence envelope published to dserver;
- remote evidence can be filtered by resource/node/D-Graph/event kind;
- action history survives later steady-state reconciliations without being overwritten.

## Next target

**50/100 — make D-Graph revision/change detection explicit and avoid unnecessary full reconciliation work when desired state has not changed and the last convergence proof is still fresh, while preserving periodic drift verification and the operational evidence stream.**
