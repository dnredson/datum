# SmartSentinel progress checkpoint — step 48/100 continuous operational reconciliation

Date: 2026-09-04
Branch: `agent/o2-host-capability-discovery`
Progress index: **48/100 — LIVE PASS**

## Scope

Move the already validated operational reconciliation path from a manually invoked helper binary into the persistent `smartsentinel` process without changing the established responsibility split:

`SmartSentinel observes/publishes → dserver governs/binds/distributes → external D-Graph provides desired state → WASM computes typed local intent → SmartSentinel performs governed host mutation → SmartSentinel publishes resulting evidence`

The SmartSentinel does not perform global placement reasoning.

## Implemented behavior

The `smartsentinel` binary now has an opt-in operational mode. In this mode the long-running process:

1. starts from a role-free configuration;
2. derives stable/provisional resource identity;
3. holds a local singleton lock keyed by resource identity;
4. periodically publishes role-free resource discovery to dserver;
5. remains alive when the resource is unknown or known-but-unbound;
6. retries bootstrap without requiring process restart;
7. once a governed resource→node binding exists, resolves the logical node role;
8. fetches the current operational D-Graph node slice;
9. executes the same sandboxed Wasmtime reconciliation contract already live-validated;
10. executes host mutation only when both control-plane authorization and local execution policy permit it;
11. publishes a post-reconciliation snapshot;
12. repeats and converges to a no-op state.

Legacy O1 mode remains the default. Operational behavior is opt-in so the previously closed O1 observation-only behavior is not silently changed.

## Live validation

Resource identity:

`resource:os:9d564a4d83804cff3b62e84394a8da53d8610c717cd892623a2242ebaabb6e09`

Logical role after binding:

`fog-01`

Operational D-Graph:

`dgraph:o2-mqtt-role-1`

### Phase A — process starts without role

The persistent process started with `requested_node_id=none` and remained alive.

The process autonomously published resource discovery and dserver classified the resource as `KnownUnboundResource` / `available_unbound`.

The operational loop correctly refused reconciliation while unbound and reported `waiting_or_error` without host side effects.

This proves that a SmartSentinel instance can be started before placement/binding exists.

### Phase B — role assigned while SmartSentinel remains running

While the same SmartSentinel process remained alive, dserver state was rehydrated and the resource received the governed initial assignment:

`resource:os:... → fog-01`

The operational D-Graph slice and matching reconciliation authorization were also restored.

The SmartSentinel was not restarted.

The MQTT container had been removed before this transition. After binding, the continuously running SmartSentinel materialized the desired MQTT service again:

- container: `datum-mqtt`
- image: `eclipse-mosquitto:2`
- observed state: running

The helper used to restore dserver state does not deploy the MQTT workload; therefore the workload reappearance is evidence that the persistent operational SmartSentinel path executed the governed reconciliation.

### Subsequent convergence

The final persisted reconciliation report showed:

- `bootstrap_state=known_bound_node`
- `resource_id=resource:os:9d564...`
- `node_id=fog-01`
- `dgraph_id=dgraph:o2-mqtt-role-1`
- `service_id=mqtt`
- `observed_before_running=true`
- `wasm_action_code=0`
- `execution_status=converged`
- `observed_after_running=true`
- `host_mutation_performed=false`
- `side_effects_performed=false`
- `snapshot_published=true`

This is the expected steady-state cycle after deployment.

## Regression property

Resource rediscovery must preserve an existing resource binding. The resource registry implementation retains `state` and `assigned_node_id` from the existing record when a new discovery observation arrives. A regression test was added so periodic discovery cannot silently revert a bound resource to `available_unbound`.

## Evidence gap discovered

The operational report path currently stores only the latest successful reconciliation report. A mutation cycle can therefore be overwritten by the following steady-state `converged` cycle.

This does not invalidate the live step-48 proof, because the MQTT container was absent before binding and present afterward while the persistent SmartSentinel remained running, but durable per-cycle reconciliation evidence should be added next.

## Step 48 closure

**48/100 — PASS.**

Validated transition:

`SmartSentinel starts role-free → publishes itself → waits while unbound → dserver assigns fog-01 while process remains alive → next operational cycle resolves role → receives D-Graph → WASM reconciliation executes → missing MQTT is deployed → later cycle returns action_code=0/converged`

## Next target

**49/100 — preserve per-cycle operational evidence and separate fast control-plane/D-Graph polling from expensive host collection work so a nominal 5 s control loop is not stretched by slow collectors.**
