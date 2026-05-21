# D-Graph Deployment And Distributed Orchestration Entry Plan

## Purpose

This document starts the next macro phase after WASM Runtime v0:

- previous macro phase completed: WASM Runtime v0, `80/80`
- current macro phase: D-Graph Deployment and Distributed Orchestration
- current milestone: `A6/90`

Its role is to define the architectural entry point for deployment and orchestration work before any real deployment execution, installation execution, artifact transfer, or autonomous reconfiguration is introduced.

## What D-Graph Means In This Project

In this project, the d-graph is the distributed operational graph used to describe and reason about the system as a whole.

It must represent at least:

- project
- node
- runtime
- module
- artifact
- flow
- flow edge
- dependency
- placement
- health state
- capability
- observation/report
- action/intent
- deployment plan
- reconfiguration plan

The d-graph is not only a visualization layer. It is the operational model that `dserver` uses to reason about:

- current deployment state
- runtime availability
- module placement
- inter-module flow structure
- dependency constraints
- observed failures or degradations
- possible deployment plans
- possible flow reconfiguration plans

The graph should become the common reasoning surface that connects observations from `DATUM`, global planning on `dserver`, and later controlled local execution under explicit safety policy.

## Server And Client Roles

### `dserver`

`dserver` owns the global d-graph view.

Its responsibilities are:

- receive `DATUM` snapshots, capabilities, and reports
- maintain the project-wide graph state
- derive deployment and orchestration plans
- evaluate placement options
- evaluate flow reconfiguration options
- remain a planning and coordination authority

`dserver` must not execute local actions directly.

### `DATUM`

`DATUM` owns local node observation.

Its responsibilities are:

- report local runtime state
- report local capabilities and constraints
- validate local feasibility for proposed actions
- eventually perform explicitly approved local actions

For now, `DATUM` must remain guarded and non-destructive. The current system is still in the planning, reporting, and metadata-validation stage rather than a deployment executor stage.

## Deployment And Orchestration Progression

### Stage G1: Local D-Graph Model

- single machine
- simulated nodes, modules, and flows
- no deployment execution
- validate graph model and planning logic

Status note:

- `G1/90` adds pure `dserver` d-graph deployment and orchestration model foundations, validation helpers, relationship derivation, and sample local graph coverage without endpoints or execution

This stage should prove that the graph entities and relationships are useful before live distributed behavior is introduced.

### Stage G2: Local Deployment Dry-Run

- generate deployment and reconfiguration plans from the d-graph
- no Docker `run` or `pull`
- no artifact transfer
- no installation execution

Status note:

- `G2/90` adds pure dry-run placement and reconfiguration planning over the d-graph without endpoints, storage, or execution

This stage should stay metadata-only or dry-run only.

### Stage G3: 2-VM Distributed Smoke

- one `dserver`
- two `DATUM` nodes
- publish capabilities and snapshots
- represent modules and flows across nodes
- validate node-scoped plans
- still dry-run or metadata-only initially

This is the first real distributed visibility milestone.

### Stage G4: 3-VM Orchestration Smoke

- one `dserver`
- three `DATUM` nodes
- test placement alternatives
- simulate one module or node failure
- derive flow reconfiguration

This stage should validate distributed planning logic before any real local action execution is considered.

### Stage G5: Controlled Local Action Pilot

- only after explicit safety review
- limited to pre-existing local containers or WASM v0-safe modules
- no automatic remediation loops

This stage is intentionally later. It must follow planning maturity, explicit policy, and local safety boundaries.

## Initial D-Graph Entities And Relationships

This is an initial proposal only. It should stay small enough to implement and test incrementally.

### Entities

- `Node`
- `Runtime`
- `Module`
- `Artifact`
- `Flow`
- `FlowEndpoint`
- `CapabilityProfile`
- `HealthReport`
- `DeploymentIntent`
- `PlacementDecision`
- `ReconfigurationDecision`
- `WasmModuleDescriptor`
- `WasmRuntimeReport`

### Relationships

- `Node HAS_RUNTIME Runtime`
- `Node HOSTS Module`
- `Module USES Artifact`
- `Flow CONTAINS Module`
- `Flow CONNECTS Module -> Module`
- `Module REQUIRES CapabilityProfile`
- `Node PROVIDES CapabilityProfile`
- `Module PLACED_ON Node`
- `HealthReport OBSERVES Node/Module`
- `DeploymentIntent TARGETS Flow/Module`
- `PlacementDecision SELECTS Node`
- `ReconfigurationDecision REPLACES edge/module placement`
- `WasmModuleDescriptor DESCRIBES Module or Artifact`
- `WasmRuntimeReport OBSERVES WasmModuleDescriptor`

This proposal is enough to support early planning, dry-run placement, and reconfiguration reasoning without overdesigning a full graph ontology too early.

## Initial Failure And Reconfiguration Scenarios

### Scenario 1: Missing Local Mapping

The current local remediation chain already diagnoses missing or ambiguous local mappings and produces advisory proposals. That existing chain should become one graph-informed input rather than be replaced.

### Scenario 2: Module Missing On Selected Node

- `dserver` detects a module gap
- the d-graph marks the selected placement as unsatisfied
- a deployment or installation task is proposed
- no automatic execution occurs

### Scenario 3: Node Unavailable

- `dserver` marks node health as degraded or unavailable
- the d-graph suggests alternative placement
- a flow reconfiguration plan is generated

This remains planning-only in early stages.

### Scenario 4: Runtime Unavailable

- `DATUM` reports runtime unavailable
- `dserver` blocks or defers actions for that node
- an alternative runtime or node is considered

### Scenario 5: WASM Diagnostic Available

- a `WasmModuleDescriptor` exists
- `DATUM` may eventually run an isolated diagnostic under future policy
- this is not part of immediate deployment execution

WASM remains relevant to the long-term system design, but this macro phase does not grant new WASM execution privileges by itself.

## First Implementation Sequence

Recommended next milestones:

- `G1/90`: `dserver` d-graph deployment model draft, pure models only
- `G2/90`: `dserver` local d-graph planner dry-run for simulated nodes, modules, and flows
- `C30/90`: `DATUM` local node/module/flow snapshot extension
- `S30/90`: `dserver` ingest extended deployment snapshots into the d-graph
- `I21/90`: repo-root smoke for local simulated d-graph deployment dry-run
- `G3/90`: 2-VM distributed test plan document
- later: 2-VM live smoke
- later: 3-VM live orchestration and reconfiguration test

This sequence keeps modeling, ingestion, and smoke validation ahead of real side effects.

Status note:

- `C30/90` adds `DATUM` local d-graph snapshot reporting for node, runtime, module, flow, health, and capability metadata. It remains local and read-only, and it does not inspect Docker state, execute WASM, deploy modules, or reconfigure flows.
- `S30/90` adds `dserver` ingestion and read-back contracts for `DATUM` local d-graph snapshots. It does not materialize a full project d-graph yet, run the planner, or execute deployment behavior.

## Safety Boundaries

The early G milestones must preserve these boundaries:

- no automatic deployment execution
- no installation execution
- no Docker `run`, `pull`, or `rm`
- no artifact transfer until artifact verification exists
- no WASM artifact loading until the module registry and artifact trust model are stronger
- no autonomous flow rewiring without explicit plan, report, and review
- `dserver` plans; `DATUM` validates and eventually executes only with explicit local policy
- all side effects must be reported

These boundaries are critical because the macro phase changes the planning model from local node reasoning toward distributed orchestration, which increases coordination power before execution safety is ready.

## Validation Expectations

Every milestone in this macro phase should preserve:

- independent validation for `PD/dserver` and `PD/DATUM`
- `cargo fmt`
- `cargo test`
- `cargo build`
- repo-root smoke updates for integration milestones
- git hygiene with no `target/`, no generated artifacts, and no credentials
- commit and push checkpoint after each milestone

This mirrors the validation discipline already used in the safe assisted control-plane, local remediation, and WASM Runtime v0 phases.

## Entry Decision

The next macro phase should begin with graph modeling and dry-run reasoning, not with execution expansion.

The immediate goal is to define a d-graph that can represent deployment state, placement options, and reconfiguration choices across nodes. Only after that model is stable should the project move into simulated planning, distributed smoke tests, and eventually tightly controlled local execution pilots.

The intended growth path is:

1. local graph model
2. local dry-run planning
3. 2-VM distributed visibility
4. 3-VM orchestration and failure simulation
5. later controlled local action pilots

This keeps the system aligned with the long-term d-graph-driven orchestration target without bypassing safety design.
