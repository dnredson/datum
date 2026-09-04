# Glossary

This glossary defines normative terminology for the canonical DATUM documentation.

| Term | Canonical meaning |
|---|---|
| **DATUM** | Architectural framework and information model for representing, observing, reasoning about, and managing distributed applications across the IoTinuum. DATUM is not itself an executor. |
| **IoTinuum** | Heterogeneous computing continuum in which DATUM application roles/nodes/runtimes operate. It is dynamic and not a rigid one-node-per-layer hierarchy. |
| **D-Graph** | Graph-based representation correlating application, infrastructure, runtime, capability, flow, and observed-state information. |
| **Source D-Graph** | Graph information supplied/configured before runtime evidence enrichment. |
| **Observed D-Graph** | Evidence-enriched control-plane interpretation of graph state. |
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
DATUM != dserver
DATUM != SmartSentinel
D-Graph != Orchestrator
```

These distinctions should be preserved in code comments, papers, API documentation, dashboards, and future phases.

## DATUM name expansion

The project has historical material containing more than one expansion of the DATUM name. The canonical documentation intentionally treats **DATUM** as a proper name until one expansion is explicitly selected and normalized across the repository/publications.
