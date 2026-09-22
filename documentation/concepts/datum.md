# DATUM

DATUM is an architectural framework and information model for representing, observing, reasoning about, and managing distributed applications across the computing continuum, referred to in this project as the **IoTinuum**.

DATUM provides a common representation for application components, computational resources, execution environments, capabilities, communication flows, runtime observations, and the relationships between them. Its central structural representation is the **D-Graph**.

## Why DATUM exists

Distributed IoT applications can span devices, edge/fog nodes, local infrastructure, cloud hosts, container runtimes, WebAssembly runtimes, processes, services, modules, and heterogeneous network paths. Those environments change while the application is running.

A static deployment description can tell us what is configured, but not necessarily what is true now. DATUM therefore separates four concerns:

1. **Configured / source information** — what the model says should exist or be related.
2. **Observed evidence** — what agents and runtime inspection report now.
3. **Reasoning** — readiness, analysis, planning, proposals, and policy decisions.
4. **Controlled execution** — explicitly governed actions that may mutate the real environment.

## DATUM is not an executor

DATUM itself is not a deployment engine and is not synonymous with either dserver or SmartSentinel.

- **D-Graph** is the structural information model.
- **dserver** is the current central control-plane implementation.
- **SmartSentinel** is the current distributed node agent.
- **Execution mechanisms** perform only actions that pass the relevant contracts and gates.

This distinction matters because a planner can produce a valid proposal without executing it, and the D-Graph can change because new evidence arrived without any mutation of the managed environment.

## Canonical model vs current structures (Stage 107)

The [DATUM canonical model](../reference/datum-canonical-model.md) page and the
[Architecture Decision Records](../adr/README.md) freeze the reference meaning of
every `D-*` concept as the **Stage 107 baseline**. Two points matter for reading
the current codebase:

- **"D-Graph" names three different structures today.** `OperationalDGraph` is the
  *operational deployment document v1* (a per-node desired-services list, no
  edges). `DGraph` is the *controlled composite graph* from the closed
  Controlled WASM Deployment phase. The snapshot-materialized graph is the
  *observed runtime graph*. None of these is the canonical application D-Graph,
  which is the new `datum.dgraph/1` contract (D-Serv vertices, D-Call edges,
  placement-free). Stage 107 relabels the structures in prose; it renames no Rust
  type.
- **SmartSentinel currently implements *D-Agent* plus the *deployment executor*.**
  "D-Agent" is a project extension, not a DATUM-paper term; it continues the
  IoTMan Agent lineage of the IoTManA / IoTManS thesis. A separate **D-Node**
  runtime — which executes application code through the governed Phase-111 D-Code
  path — now exists; full middleware and engine orchestration remain later phases.

## D-Code and Management WASM

Three uses of "WASM" must be kept apart (ADR-0005):

- **D-Code** is the executable of an application service (D-Serv). WASM is the
  prototype target (ABI `datum-dnode/0`). Phase 111 executes D-Code as a governed
  service invocation; Phase 112 realizes one semantic DCall edge.
- **Management WASM** is the small `desired AND NOT observed` reconciler embedded
  in the agent, plus the artifact `wasm_decision` policy. It is legacy v1
  compatibility only and **is not D-Code**.
- **Container / runtime artifacts** are middleware and host images governed
  through the container lifecycle.

## Core concepts

### Node

A logical computational participant in the IoTinuum. A node can correspond to a VM, physical host, edge/fog machine, workstation, cloud host, or another execution participant. `node_id` is the stable DATUM identity; hostname and IP address are attributes, not substitutes for that identity.

### Runtime

An execution environment available on a node. The current capability model explicitly represents `container`, `wasm`, and `process` runtime families.

### Module

An application/executable component associated with a runtime, node, role, or flow. A module is not the same thing as the runtime that executes it.

### Flow

A logical application or data-flow relationship. A configured flow describes intended structure; it must not be confused with proof of the actual live data-plane path.

### Capability

A declared or observed ability. Capability and authorization are intentionally separate. For example, a container runtime may be available while `allow_execution` remains false.

### Observation / Evidence

An observation is something measured or inspected at a point in time. It becomes evidence when used to support a statement about system state. Evidence needs temporal context and provenance; old evidence can become stale.

## Configured state vs. observed state

DATUM never assumes that configured existence proves runtime health.

```mermaid
flowchart LR
    C[Configured / source state]
    E[Runtime evidence]
    O[Observed state]
    C --> O
    E --> O
```

A configured runtime may be absent at runtime. A configured module may be stopped. A configured link may be unreachable. Conversely, the absence of evidence should be represented as **unknown**, not silently promoted to healthy.

## Reasoning and execution pipeline

```mermaid
flowchart LR
    O[Observation] --> G[Observed D-Graph]
    G --> A[Analysis / readiness]
    A --> P[Plan / proposal]
    P --> V[Policy + safety gates]
    V --> X[Controlled execution]
    X --> R[Outcome evidence]
    R --> O
```

Not every cycle reaches execution. Read-only observation, what-if planning, dry-run planning, readiness evaluation, and advisory proposals are valid end states.

## Current implementation boundary

In the current codebase, DATUM's local snapshot primitive is intentionally generic: a `Snapshot` contains metadata, entities, edges, events, and action records. Domain meaning such as node/runtime/module/flow is represented through entity kinds, attributes, relationships, and higher-level materialization rather than by one hard-coded Rust type for every domain concept.

The current SmartSentinel service loop runs continuously for a logical `node_id`, emits heartbeat/liveness information, executes collection cycles, and in the O1 baseline explicitly disables mutation-oriented desired-state/auto-recovery behavior. That is an architectural signal: **the agent is useful even when no deployment is happening**.

## Naming note

The expansion of the name **DATUM** is intentionally not frozen here yet because historical project material contains more than one expansion. Until one is selected and normalized across the project, DATUM is treated as the proper name of the architecture.

The Stage 107 reference paper uses the expansion "Distributed Applications for the IoTinuum". That is recorded for traceability; it does not mandate a repository-wide rename.
