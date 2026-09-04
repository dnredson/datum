# Architecture overview

DATUM separates **model**, **observation**, **reasoning**, **control**, and **execution evidence**.

## Component roles

| Component / concept | Role |
|---|---|
| DATUM | Architecture and information model. |
| IoTinuum | Heterogeneous continuum environment in which distributed application roles execute. |
| D-Graph | Graph representation correlating application/infrastructure relationships and state. |
| SmartSentinel | Distributed node agent for observation and explicitly governed local interaction. |
| dserver | Central control plane that receives evidence, materializes state, reasons, plans, and coordinates governed operations. |
| Runtime/executor mechanisms | Perform concrete local operations when authorization/contracts allow them. |

## End-to-end information flow

```mermaid
sequenceDiagram
    participant SS as SmartSentinel
    participant DS as dserver
    participant DG as D-Graph state
    participant PL as Planner/Gates
    participant EX as Local execution

    SS->>DS: snapshot / capability / heartbeat / result evidence
    DS->>DG: validate + store + materialize
    DG->>PL: current configured/observed state
    PL-->>DS: readiness / dry-run / proposal / dispatch decision
    DS-->>SS: explicitly governed control (when applicable)
    SS->>EX: local operation (when authorized)
    EX-->>SS: outcome
    SS->>DS: action/lifecycle evidence
    DS->>DG: update observed interpretation
```

## Observation is not control

Collectors and SmartSentinel can report that something changed without causing the change. dserver can update the observed graph without mutating the environment. This is the normal observation path.

## Reasoning is not execution

Analysis, readiness, planning, dry-run, proposals, and dispatch all represent progressively stronger control-plane decisions, but none should be presented as successful real-world mutation without execution/outcome evidence.

## Identity boundary

Logical `node_id` is the principal cross-component identity. Hostname, host ID, IP address, and physical/virtual machine identity are evidence/attributes that may change independently.

## Temporal boundary

DATUM data is time-sensitive. Timestamps, TTL, freshness thresholds, heartbeat sequence, and latest/history collections exist because a stored value can be valid evidence historically while no longer being safe to treat as current truth.

## Persistence boundary

The current dserver uses Firebase collections for persistence. Firebase is an implementation choice, not a defining DATUM concept. Domain semantics should remain stable if the persistence backend changes.

## Historical phase boundaries

The codebase contains validated phase-specific behavior such as Controlled WASM Deployment and the newer SmartSentinel continuous observation loop. New phases must not silently rewrite the claims or safety properties of closed historical phases.

For example, the O1 SmartSentinel loop explicitly disables mutation even if older configuration contains desired-state or auto-recovery settings. That lets observation evolve independently from deployment automation.
