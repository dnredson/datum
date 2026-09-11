# ADR-0001 — Canonical DATUM vocabulary and legacy-structure labels

- Status: Accepted (Stage 107 baseline)
- Decision refs: D-4; working decisions 1–12
- Supersedes nothing. Renames no Rust type.

## Context

The DATUM reference model (Kamienski, *Smart Application Development and Deployment
over the IoT Computing Continuum*, 2026-01) names a family of `D-*` concepts. The
repository implements a governed container deployment pipeline but does not carry
those concepts with their reference meaning, and one name in particular — "D-Graph"
— is used for three different structures:

- `dserver/src/core/operational_dgraph.rs` `OperationalDGraph`: a per-node list of
  desired container services, no edges;
- `dserver/src/core/dgraph.rs` `DGraph`: a composite of application, infrastructure,
  placement and health, from the closed Controlled WASM Deployment phase (C59–I44);
- the observed graph materialized from local snapshots.

The canonical glossary previously defined "D-Graph" as the union of all of these,
which makes the term unusable in specifications and papers.

## Decision

1. Establish the canonical meaning of each concept, independent of the current
   code. The normative list lives in
   `documentation/reference/datum-canonical-model.md` and the glossary
   (`documentation/reference/glossary.md`). Summary:

   | Concept | Canonical meaning |
   |---|---|
   | D-Application | Placement-independent application logic comprising all pieces of code needed to implement the required functionality of a smart application. Its logically centralized source representation is D-Script. D-Code is generated later as the executable of a D-Serv. |
   | D-Script | Monolith-distributed source: a set of D-Functions with non-functional annotations. The logically centralized source representation of a D-Application. |
   | D-Function | A function unit inside a D-Script. |
   | D-Compile | Compiles a D-Script into a D-Graph and D-Code; maps D-Function → D-Serv (1:1, 1:N, M:1, M:N). |
   | D-Graph | `G = (V, E)` with `V` = D-Serv and `E` = D-Call. Application-only, placement-free. Schema `datum.dgraph/1`. |
   | D-Serv | A serverless application service that runs D-Code. |
   | D-Call | A service invocation `D-Serv_i → D-Serv_j`. Communication topology, not installation ordering. |
   | D-Code | The architecture-independent executable of a D-Serv (see ADR-0005). |
   | D-Continuum | The supported IoTinuum configuration for a specific D-Application: stages, substages and elements together with their characteristics, constraints and requirements. A D-Map associates a concrete D-Continuum with an application D-Graph. Schema `datum.dcontinuum/1`. The prototype contract also carries `project_id` for deployment / persistence scope; that is an implementation scoping choice, not part of the canonical definition. |
   | D-Stage / D-Substage / D-Element | Location class / category within a D-Stage (such as equipment class or geographic coverage) / concrete instance. `element_id` = the existing logical `node_id`. |
   | D-Map | The mapping / binding of the application and the required platform elements onto the D-Continuum. Schema `datum.dmap/1`. The Stage 107 governance decision that `datum.dmap/1` is the single authoritative placement carrier, materialized only after explicit acceptance of a D-Deploy proposal, is recorded in ADR-0002 (it is not part of the reference-model meaning). |
   | D-Controller | The logically centralized control plane that runs D-Deploy. DServer is its implementation. |
   | D-Deploy | Deployment and dynamic redeployment: triggering, binding, distribution, installation. |
   | D-Platform | The underlying DATUM platform that provides the environment in which a D-Application executes. Composed of D-Engine, D-Forward and D-Monitor. *Prototype status:* the repository does not yet represent D-Platform as one first-class runtime object; today it is an architectural grouping of those responsibilities. |
   | D-Engine | The set of D-Nodes that runs a D-Application. |
   | D-Node | The stage-specific runtime that executes D-Serv and may provide or support D-Serv communication according to the selected communication model (direct, via the D-Node, or hybrid). See ADR-0004; ABI `datum-dnode/0` specifically chooses indirect communication through the D-Node (ADR-0003). |
   | D-Forward | The IoT-middleware composition that forms the end-to-end data path between D-Serv. Schema `datum.dforward/1`. |
   | D-IoT | One middleware element of D-Forward (broker, LoRaWAN server, IoT platform). |
   | D-Monitor | The monitoring sub-platform and lifecycle process. |
   | D-Agent | **Project extension, not a DATUM-paper term.** The node-local observation, reporting and governed-interaction endpoint. Continues the IoTMan Agent lineage of the IoTManA/IoTManS thesis. SmartSentinel currently implements D-Agent plus the deployment executor. |

2. **Topology is not installation ordering (D-4).** A `datum.dgraph/1` may contain
   D-Call cycles. The deployment-dependency graph derived for installation ordering
   must be acyclic. The two are separate structures; a D-Call cycle alone is never a
   validation failure.

3. **Label the legacy structures by their real role** in documentation only. No
   Rust type is renamed in this phase (see ADR-0007).

   | Current Rust name | Documentation label |
   |---|---|
   | `OperationalDGraph` | operational deployment document v1 |
   | `DGraph` (C59–I44) | controlled composite graph |
   | observed D-Graph | observed runtime graph |

4. Add a `Wave` term for the topological-ordering step currently called
   `DeploymentDependencyStage`, distinct from a continuum `D-Stage`.

## Consequences

- Specifications and the eventual papers have one unambiguous meaning per term.
- Existing code, tests, fixtures and evidence keep their names; only prose changes.
- The canonical application D-Graph is a new contract (`datum.dgraph/1`), typed in
  the `datum-model` crate in Stage 108, not a reinterpretation of `DGraph`.
- VS1 fixtures under `scripts/fixtures/datum-contracts/vs1/` demonstrate the
  vocabulary concretely, including an accepted D-Call cycle.
