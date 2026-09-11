# DATUM canonical model (Stage 107)

This page is the concise, citable form of the DATUM reference vocabulary and the
v2 contract set that the joint architecture review froze as the **Stage 107
baseline**. The full analysis, the vertical-slice design (VS1) and the stage plan
are in `docs/STAGE107_DATUM_SPEC_GAP_REPORT_AND_VERTICAL_SLICE_PLAN.md`. The
decision records are in [Architecture Decision Records](../adr/README.md).

Stage 107 is specification only. No DServer or SmartSentinel runtime behaviour
changes, no Rust type is renamed, and no API changes. It provides hand-authored
**contract fixtures for the six canonical v2 contract shapes** plus a stdlib
consistency checker. The typed model, its validators and JSON Schemas generated
from those types arrive with the `datum-model` crate in **Stage 108**.

## Canonical concepts

| Concept | Canonical meaning |
|---|---|
| **D-Application** | Placement-independent application logic comprising all pieces of code needed to implement the required functionality of a smart application. Its logically centralized source representation is D-Script; D-Code is generated later as the executable of a D-Serv. Carries an `application_id` distinct from `project_id`. |
| **D-Script** | Monolith-distributed source: a set of D-Functions with non-functional annotations. |
| **D-Function** | A function unit inside a D-Script. |
| **D-Compile** | Compiles a D-Script into a D-Graph and D-Code; maps D-Function → D-Serv (1:1, 1:N, M:1, M:N). |
| **D-Graph** | `G = (V, E)`, `V` = D-Serv, `E` = D-Call. Application-only, placement-free. D-Call cycles are allowed. Schema `datum.dgraph/1`. |
| **D-Serv** | A serverless application service that runs D-Code. |
| **D-Call** | A service invocation `D-Serv_i → D-Serv_j`. This is communication topology, not installation ordering. |
| **D-Code** | The architecture-independent executable of a D-Serv. WASM (ABI `datum-dnode/0`) is the prototype target. **Management WASM is not D-Code.** |
| **D-Continuum** | The supported IoTinuum configuration for a specific D-Application: stages, substages and elements together with their characteristics, constraints and requirements. A D-Map associates a concrete D-Continuum with an application D-Graph. Schema `datum.dcontinuum/1`. The prototype contract also carries `project_id` for deployment / persistence scope — an implementation scoping choice, not part of the canonical definition. |
| **D-Stage / D-Substage / D-Element** | Location class / category within a D-Stage (such as equipment class or geographic coverage) / concrete instance. `element_id` equals the existing logical `node_id`. |
| **D-Map** | *Reference-model meaning:* the mapping / binding of the application and the required platform elements onto the D-Continuum. *Stage 107 governance decision (ADR-0002):* `datum.dmap/1` is the single authoritative source / carrier of placement, materialized only after explicit acceptance of a D-Deploy proposal. Derived objects (such as the per-node operational slice) may repeat or materialize placement information but must never originate an independent placement decision. Explicit operator acceptance is a Stage 107 rule, not a requirement of the DATUM reference model. |
| **D-Controller** | The logically centralized control plane that runs D-Deploy. **DServer** is its implementation. |
| **D-Deploy** | Deployment and dynamic redeployment: triggering, binding, distribution, installation. |
| **D-Platform** | The underlying DATUM platform that provides the environment in which a D-Application executes. It is composed of D-Engine, D-Forward and D-Monitor. *Prototype status:* the repository does not yet represent D-Platform as one first-class runtime object; today it is an architectural grouping of those responsibilities. |
| **D-Engine** | The set of D-Nodes that runs a D-Application. |
| **D-Node** | The stage-specific runtime that executes D-Serv and may provide or support D-Serv communication according to the selected communication model (direct, via the D-Node, or hybrid). Runtime-agnostic: hosted by a container for VS1, by a native host process later, without changing any other contract. ABI `datum-dnode/0` chooses indirect communication through the D-Node ([ADR-0003](../adr/ADR-0003-dnode-abi-v0.md)). |
| **D-Forward** | The IoT-middleware composition that forms the end-to-end data path between D-Serv. Schema `datum.dforward/1`. |
| **D-IoT** | One middleware element of D-Forward (broker, LoRaWAN server, IoT platform). |
| **D-Monitor** | The monitoring sub-platform and lifecycle process. |
| **D-Agent** | **A project extension, not a DATUM-paper term.** The node-local observation, reporting and governed-interaction endpoint. Continues the IoTMan Agent lineage of the IoTManA / IoTManS thesis. **SmartSentinel currently implements D-Agent plus the deployment executor.** |
| **Wave** | A topological-ordering step for installation (the structure currently called `DeploymentDependencyStage` in code). Distinct from a continuum D-Stage. |

## Legacy structures, labelled by their real role

The name "D-Graph" has been used for three different structures. Stage 107 relabels
them **in documentation only**; no Rust type is renamed (ADR-0007).

| Current Rust name / concept | Canonical label | What it actually is |
|---|---|---|
| `OperationalDGraph` (`dserver/src/core/operational_dgraph.rs`) | **operational deployment document v1** | A per-node list of desired container services. No edges, no D-Serv semantics. |
| `DGraph` (`dserver/src/core/dgraph.rs`) | **controlled composite graph** | The closed Controlled WASM Deployment phase (C59–I44) structure: application + infrastructure + placement + health + WASM refs. |
| observed D-Graph (materialized from snapshots) | **observed runtime graph** | Evidence-enriched interpretation of runtime state. |

None of these is the canonical application D-Graph. The canonical D-Graph is the
new `datum.dgraph/1` contract.

## v2 contract set

| Schema id | Purpose |
|---|---|
| `datum.dgraph/1` | Application graph: D-Serv vertices, D-Call edges, placement-free. |
| `datum.dcontinuum/1` | Declared stages, substages, elements, and host-agnostic D-Node runtime descriptors. |
| `datum.dforward/1` | D-IoT elements, their `provides` / `requires` interfaces (with `same_element` / `any_element` scope), and chains. |
| `datum.dmap/1` | The single authoritative placement carrier (ADR-0002): D-Serv, D-Node-runtime and D-IoT placements, D-Call realizations, external port bindings. |
| `datum.operational-slice/2` | The per-node, content-addressed, **derived** deployment slice. Always `execution_pinning_mode = required`. |
| `datum.dserv-artifact/1` | The D-Code artifact: immutable, content-addressed, ABI `datum-dnode/0`, typed ports, limits, provenance. No container fields, no host kind. |

A seventh object, `datum.dserv-execution-contract/1` (`DServExecutionContractV1`),
is embedded in each `dserv_instance` workload of a slice.

Digests are `SHA-256` over the RFC 8785 JSON Canonicalization Scheme (ADR-0006).
Stage 107 fixtures use the placeholder
`sha256:0000…0000`; real digests arrive in Stage 108.

## D-Serv execution-contract host invariance

For every D-Serv instance, `DServExecutionContractV1` is **independent of how its
D-Node is hosted**. Two D-Maps that differ only in
`dnode_runtime_placements[].host` must produce identical execution-contract
digests. Host lifecycle representation (the `dnode_runtime` workload — host kind,
hosting artifact, resolved endpoint addresses) is allowed to differ, and so is
everything derived from it (slice digest, authorization). See
[ADR-0004](../adr/ADR-0004-dnode-runtime-agnostic-host.md).

## D-Code vs Management WASM vs container/runtime artifacts

| Category | Contract | Runs where |
|---|---|---|
| **Application WASM / D-Code** | `datum.dserv-artifact/1` — immutable, content-addressed, ABI `datum-dnode/0` | Inside a D-Node; a fresh instance per invocation |
| **Management WASM** (legacy v1 only) | `ServiceArtifactWasmDecision`, the embedded `desired AND NOT observed` reconciler | The v1 executor only; **never called D-Code** |
| **Container / runtime artifacts** | `ServiceArtifact` + `artifact_class`; `RuntimeInstallArtifact` | Governed container lifecycle / host installation |

See [ADR-0005](../adr/ADR-0005-dcode-and-management-wasm.md).

## Deferred: TD-1 (authorization validity during execution)

The F2 hardening work (Stages 107A / 107B, [ADR-0011](../adr/ADR-0011-f2-execution-content-authorization.md))
carries one **open** design question: what happens when an artifact's execution
generation changes between the moment an agent fetches an authorized context and
the moment it performs the host mutation. Stage 107 records this as **TD-1** and
does **not** choose a solution. Neither snapshot/lease semantics nor immediate
revocation may be claimed until one is specified in 107A design. See
[ADR-0012](../adr/ADR-0012-toctou-authorization-validity.md).
