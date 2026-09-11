# Glossary

This glossary defines normative terminology for the canonical DATUM documentation.

## Canonical DATUM concepts (Stage 107 baseline)

These meanings were frozen as the Stage 107 baseline by the joint architecture
review. See [DATUM canonical model](datum-canonical-model.md) and the
[Architecture Decision Records](../adr/README.md) for the rationale. Stage 107
changes documentation only; no Rust type is renamed.

| Term | Canonical meaning |
|---|---|
| **D-Application** | Placement-independent application logic comprising all pieces of code needed to implement the required functionality of a smart application. Its logically centralized source representation is D-Script; D-Code is generated later as the executable of a D-Serv. Carries an `application_id` distinct from `project_id`. |
| **D-Script** | Monolith-distributed source: a set of D-Functions with non-functional annotations. |
| **D-Function** | A function unit inside a D-Script. |
| **D-Compile** | Compiles a D-Script into a D-Graph and D-Code; maps D-Function → D-Serv (1:1, 1:N, M:1, M:N). |
| **D-Serv** | A serverless application service that runs D-Code. |
| **D-Call** | A service invocation `D-Serv_i → D-Serv_j`. Communication topology, **not** installation ordering. D-Call cycles are allowed. |
| **D-Code** | The architecture-independent executable of a D-Serv. WASM (ABI `datum-dnode/0`) is the prototype target. **Management WASM is not D-Code.** |
| **D-Continuum** | The supported IoTinuum configuration for a specific D-Application: stages, substages and elements with their characteristics, constraints and requirements. A D-Map associates a concrete D-Continuum with an application D-Graph. Schema `datum.dcontinuum/1`; the prototype contract also carries `project_id` for deployment / persistence scope, which is an implementation scoping choice rather than part of the canonical definition. |
| **D-Stage** | A location class in the continuum (`S1-Thing` … `S6-App`). Not necessarily one physical node. |
| **D-Substage** | A category within a D-Stage, such as equipment class or geographic coverage. |
| **D-Element** | A concrete instance in the continuum. `element_id` equals the existing logical `node_id`. |
| **D-Map** | *Reference-model meaning:* the mapping / binding of the application and the required platform elements onto the D-Continuum. *Stage 107 governance decision (ADR-0002):* `datum.dmap/1` is the single authoritative source / carrier of placement, materialized only after explicit acceptance of a D-Deploy proposal; derived objects such as the operational slice may repeat or materialize placement information but must never originate an independent placement decision. Explicit operator acceptance is a Stage 107 rule, not a requirement of the DATUM reference model. |
| **D-Controller** | The logically centralized control plane that runs D-Deploy. **dserver** is its implementation. |
| **D-Deploy** | Deployment and dynamic redeployment: triggering, binding, distribution, installation. |
| **D-Platform** | The underlying DATUM platform that provides the environment in which a D-Application executes. It is composed of D-Engine, D-Forward and D-Monitor. *Prototype status:* the repository does not yet represent D-Platform as one first-class runtime object; today it is an architectural grouping of those responsibilities. |
| **D-Engine** | The set of D-Nodes that runs a D-Application. |
| **D-Node** | The stage-specific runtime that executes D-Serv and may provide or support D-Serv communication according to the selected communication model (direct, via the D-Node, or hybrid). Runtime-agnostic: hosted by a container for VS1, by a native host process later, without changing any other contract. ABI `datum-dnode/0` chooses indirect communication through the D-Node. |
| **D-Forward** | The IoT-middleware composition that forms the end-to-end data path between D-Serv. Schema `datum.dforward/1`. |
| **D-IoT** | One middleware element of D-Forward (broker, LoRaWAN server, IoT platform). |
| **D-Monitor** | The monitoring sub-platform and lifecycle process. |
| **D-Agent** | **A project extension, not a DATUM-paper term.** The node-local observation, reporting and governed-interaction endpoint. Continues the IoTMan Agent lineage of the IoTManA / IoTManS thesis. **SmartSentinel** currently implements D-Agent plus the deployment executor. |
| **Wave** | A topological-ordering step for installation (the structure currently called `DeploymentDependencyStage` in code). Distinct from a continuum D-Stage. |

### Structures currently called "D-Graph"

The name "D-Graph" has been used for three different structures. Stage 107
relabels them in documentation only; no Rust type is renamed (ADR-0007).

| Current Rust name / concept | Canonical label | What it actually is |
|---|---|---|
| `OperationalDGraph` | **operational deployment document v1** | A per-node list of desired container services. No edges, no D-Serv semantics. |
| `DGraph` (C59–I44 phase) | **controlled composite graph** | Application + infrastructure + placement + health + WASM refs, from the closed Controlled WASM Deployment phase. |
| observed D-Graph (materialized from snapshots) | **observed runtime graph** | Evidence-enriched interpretation of runtime state. |

None of these is the canonical application **D-Graph**, which is the new
`datum.dgraph/1` contract.

## General terminology

| Term | Canonical meaning |
|---|---|
| **DATUM** | Architectural framework and information model for representing, observing, reasoning about, and managing distributed applications across the IoTinuum. DATUM is not itself an executor. |
| **IoTinuum** | Heterogeneous computing continuum in which DATUM application roles/nodes/runtimes operate. It is dynamic and not a rigid one-node-per-layer hierarchy. |
| **D-Graph** | Canonically, the application graph `G = (V, E)` with `V` = D-Serv and `E` = D-Call, placement-free, schema `datum.dgraph/1`. Historically the term has also been used for the three structures listed above; prose should use their canonical labels. |
| **Source D-Graph** | Graph information supplied/configured before runtime evidence enrichment. |
| **Observed D-Graph** | Evidence-enriched control-plane interpretation of graph state (the *observed runtime graph*). |
| **Entity** | Generic graph primitive with identity, kind, attributes, observation time, and TTL. Domain concepts such as Node/Runtime/Module give it semantic meaning. |
| **Edge / Relationship** | Typed directed association between entities; `rel` determines the relationship semantics. |
| **Node** | Logical DATUM participant. `node_id` is the stable identity; hostname/IP are attributes. |
| **Stage** | Architectural role/region (e.g. fog/cloud), not necessarily a physical node. Multiple nodes may share a stage. |
| **Runtime** | Execution environment available on a node, currently modeled through container/WASM/process capability families. |
| **Module** | Application/executable component associated with a runtime/node/flow. |
| **Flow** | Logical application/data-flow relationship. A configured flow is not by itself proof of a real active traffic path. |
| **Capability** | Ability reported/declared by a node/runtime/agent. Capability is not authorization. |
| **Observation** | Something measured/detected/inspected at a point in time. |
| **Evidence** | Observation or outcome information used to support a statement about state/behavior, with provenance and temporal context. |
| **Telemetry** | Structured runtime observations transmitted about a node/environment. Heartbeat is a narrower liveness signal. |
| **Heartbeat** | Lightweight SmartSentinel liveness indication. It does not prove complete host/runtime/service health. |
| **Configured state** | What configuration/source information declares. It must not automatically be treated as runtime truth. |
| **Desired state** | State the operator/system intends to reach or maintain. It is neither configured fact nor observed proof. |
| **Observed state** | Current evidence-backed interpretation of the runtime environment. |
| **Fresh** | Evidence is recent enough for the relevant contract. |
| **Stale** | Evidence exists but is older than the applicable freshness threshold. |
| **Unknown** | Insufficient evidence exists to make a supported current statement. Unknown is not healthy. |
| **Unavailable** | Current evidence indicates the resource/capability/path cannot be used or reached. |
| **Health** | Evidence-derived operational condition. Health is not the same as readiness. |
| **Readiness** | Context-specific evaluation of whether requirements for an operation/phase are satisfied. |
| **Plan** | Structured description of how a goal could be achieved. A plan is not execution. |
| **Proposal** | Advisory candidate operation/adaptation, especially for what-if/failover reasoning. |
| **Dispatch** | Governed transition/classification between planning and executable operations. Dispatch is not proof of execution. |
| **Execution** | Stage in which an authorized operation is actually performed against the environment. |
| **Mutation** | Intentional externally observable state change. Observation, graph materialization, analysis, and planning are not mutations. |
| **Side effect** | Externally observable change caused by execution. |
| **Dry run** | Evaluation mode that intentionally avoids real state-changing side effects. |
| **Adaptation** | Controlled change to running application/environment in response to state, policy, or operator intent. Broader than deployment. |
| **Deployment** | Controlled process of making an application component available in an execution environment. One possible DATUM operation, not DATUM's definition. |
| **Routing** | Real change in the application data-plane path. Metadata-only graph changes must not be called routing. |
| **Control plane** | dserver-centered reasoning/control state and APIs. |
| **Data plane** | Actual path through which application/workload traffic flows. |
| **SmartSentinel** | Distributed node agent for continuous observation and explicitly governed local interaction. |
| **dserver** | Current central DATUM control-plane implementation. It is not synonymous with DATUM. |
| **Snapshot** | Structured result of one local observation cycle: meta, entities, edges, events, actions. |
| **Event** | Structured record of an occurrence/state transition/warning/failure. |
| **Action Record / Action Result** | Evidence describing action lifecycle/outcome. Presence alone is not proof of successful real-world execution. |
| **Provenance** | Information about where evidence/artifacts/state came from. |
| **Correlation ID** | Identifier joining related events/actions/requests/outcomes across stages. |

## Normative distinctions

```text
Configured State != Observed State
Desired State != Observed State
Capability != Authorization
Health != Readiness
Observation != Mutation
Plan != Execution
Proposal != Execution
Dispatch != Execution
Heartbeat != Telemetry
Graph change != Data-plane change
Unknown != Unavailable
Stale != Unknown
Runtime != Module
Stage != Node
Stage != Wave
DATUM != dserver
DATUM != SmartSentinel
D-Graph != Orchestrator
D-Code != Management WASM
D-Serv != D-IoT
D-Call topology != deployment ordering
Local policy narrows or denies, never substitutes execution semantics
```

These distinctions should be preserved in code comments, papers, API documentation, dashboards, and future phases.

## DATUM name expansion

The project has historical material containing more than one expansion of the DATUM name. The canonical documentation intentionally treats **DATUM** as a proper name until one expansion is explicitly selected and normalized across the repository/publications.

The Stage 107 reference paper (Kamienski, *Smart Application Development and Deployment over the IoT Computing Continuum*, 2026-01) uses the expansion "Distributed Applications for the IoTinuum". This is **recorded, not mandated**: no repository-wide rename is implied, and the proper-name treatment above still stands until a normalization decision is taken.
