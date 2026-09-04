# Operational WASM Reconciliation

This document defines the O2 vertical slice that connects Resource Registry role binding, an externally supplied D-Graph node slice, sandboxed WebAssembly reconciliation, and guarded SmartSentinel host execution.

It does **not** reopen or modify the previously closed Controlled WASM Deployment phase. The historical controlled-WASM contracts remain unchanged. This is a new operational SmartSentinel path built on top of the already validated Wasmtime capability.

## Responsibility split

```text
external D-Graph
    describes desired state

        |
        v

dserver
    validates and stores the operational D-Graph
    correlates the resource binding
    exposes only the node slice for the resolved role
    explicitly authorizes host mutation for a specific dgraph_id + node_id

        |
        v

SmartSentinel
    bootstraps from resource identity
    resolves its bound role
    fetches the role's D-Graph slice
    observes local state

        |
        v

reconciler.wasm / Wasmtime
    receives normalized desired/observed facts
    returns a typed action code
    has no host imports
    has no filesystem, network, shell, or Docker access

        |
        v

SmartSentinel Rust guard
    checks D-Graph membership
    checks dserver authorization
    checks local execution policy
    resolves only a trusted local service mapping

        |
        v

host executor
    performs the permitted Docker mutation
    observes again and records convergence evidence
```

The operational architecture is therefore:

```text
SmartSentinel observes
  -> D-Graph contextualizes
  -> dserver governs
  -> WASM reconciles
  -> SmartSentinel executes
  -> SmartSentinel proves result
```

## Operational D-Graph

The operational D-Graph is an external artifact. dserver does not invent desired workloads.

O2 schema version `0.1.0` supports a deliberately narrow desired-state contract:

```json
{
  "schema_version": "0.1.0",
  "dgraph_id": "dgraph:o2-mqtt-role-1",
  "project_id": "monitor-f49c8",
  "nodes": [
    {
      "node_id": "fog-01",
      "desired_services": [
        {
          "service_id": "mqtt",
          "runtime": "container",
          "desired_state": "running"
        }
      ]
    }
  ]
}
```

The first vertical slice intentionally accepts only:

- runtime `container`;
- desired state `running`;
- unique node IDs;
- unique service IDs within each node.

The live fixture is `scripts/fixtures/o2-operational-mqtt-dgraph.json` and contains only MQTT for `fog-01`.

## Node slice as authority boundary

SmartSentinel never receives authority to manage arbitrary services merely because it is bound to a role.

For a resource bound to `fog-01`, dserver exposes:

```text
GET /api/v1/nodes/fog-01/operational-dgraph-slice
```

The returned slice contains only workloads explicitly present in the external D-Graph for that role.

The WASM ABI does not accept or return a service name. The host invokes the reconciler once per desired service from the validated slice. Therefore the guest cannot manufacture a request for an unrelated service such as `postgres` when only `mqtt` is present in the slice.

## Sandboxed reconciler

The embedded module is identified as:

```text
module_id = smartsentinel-reconciler-v1
function = reconcile_service
ABI = (desired_running:i32, observed_running:i32) -> action_code:i32
```

Action codes:

```text
0 = no host action required
1 = ensure the current D-Graph service is running
```

The module is compiled and executed by Wasmtime with fuel accounting and no imports. The report explicitly distinguishes guest execution from host mutation:

```text
WASM guest:
  host_imports_allowed = false
  guest_side_effects_performed = false

SmartSentinel host guard/executor:
  host_mutation_performed = true|false
```

## Double authorization

A WASM decision alone is never sufficient to change the host.

Host mutation requires both:

1. a current dserver authorization tied to the active `dgraph_id` and resolved `node_id`;
2. an explicit local `--execute` invocation plus `dispatch.allow_container_execution=true`.

Replacing the active operational D-Graph with a different `dgraph_id` invalidates previous operational reconciliation authorizations.

The authorization endpoint also requires an active Resource Registry binding for the target role. An unbound resource cannot be authorized to reconcile a logical node.

## Trusted local mapping

The D-Graph names the logical service (`mqtt`). It does not provide an arbitrary Docker command or image to execute.

The local SmartSentinel configuration maps the logical service to a trusted concrete deployment:

```toml
[dispatch]
enabled = true
allow_container_execution = true

[[dispatch.container_mappings]]
service_name = "mqtt"
container_name = "datum-mqtt"
artifact = "eclipse-mosquitto:2"
```

The `artifact` field is the trusted local image for this O2 vertical slice.

For a permitted `ensure_service_running(mqtt)` intent:

```text
container running -> converged / no mutation
container stopped -> docker start datum-mqtt
container missing -> docker run -d --name datum-mqtt eclipse-mosquitto:2
mapping absent -> blocked_local_mapping
Docker inaccessible -> docker_unavailable
```

No shell command received from dserver or WASM is executed.

## API

O2 adds:

```text
PUT  /api/v1/operational-dgraph
GET  /api/v1/operational-dgraph
GET  /api/v1/nodes/:node_id/operational-dgraph-slice
POST /api/v1/nodes/:node_id/operational-reconciliation/authorize
```

All calls require `X-Datum-Project`.

## Live acceptance sequence

The intended live proof is:

```text
T0  VM starts without a DATUM role; MQTT absent
T1  SmartSentinel discovers the resource
T2  dserver binds resource -> fog-01
T3  external D-Graph says fog-01 desired_services=[mqtt]
T4  dserver exposes the fog-01 node slice and authorizes that dgraph_id
T5  SmartSentinel executes smartsentinel-reconciler-v1 in Wasmtime
T6  WASM emits typed ensure_service_running for the current slice service
T7  SmartSentinel guard validates slice + authorization + trusted local mapping
T8  host executor creates/starts datum-mqtt when --execute is explicit
T9  SmartSentinel re-observes the container and reports convergence
```

A useful negative test is to leave Docker inaccessible to the SmartSentinel user. The expected result is still successful WASM execution with `action_code=1`, but host execution is reported as `docker_unavailable` and no side effect is claimed.

## Persistence boundary

Operational D-Graph state and authorization are process-local in this first O2 vertical slice, like the current Resource Registry proof path. Restarting dserver clears resource discovery, bindings, the operational graph, and operational authorization; the live validation must republish them after a restart.

Durable history belongs to the later persistence work and must preserve resource identity, role-binding lineage, D-Graph identity, authorizations, reconciliation outcomes, and observed convergence without overwriting history.
