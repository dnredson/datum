# C64 — Node-Scoped Deployment Assignment

- checkpoint: `C64/150`
- progress: `136/150`
- predecessor: `C63/150`
- phase: Controlled WASM Deployment
- status: complete

## Objective

Consume the validated typed Deployment Manifest introduced by C63 and derive a deterministic, non-executable deployment view for a specific target node.

C64 does not acquire artifacts, contact DATUM, instantiate WASM, execute functions, mutate D-Graph, install software, dispatch governed actions, or authorize `wasm_execution`.

## Node-scoped contract

`dserver/src/core/deployment_assignment.rs` introduces:

- `NodeDeploymentScope::{Full, NodeSlice, Empty}`;
- `NodeScopedDeployment`;
- `derive_node_scoped_deployment(...)`;
- `validate_node_scoped_deployment(...)`.

Each node-scoped view preserves source manifest/version/project context, requested node ID, deployment strategy, graph-distribution semantics, source D-Graph identity when present, source/selected assignment counts, selected typed assignments, artifact identity and limits, and the fail-closed execution boundary from C63.

## Distributed semantics

For `strategy=distributed` and `graph_distribution=node_slice`, only assignments whose `target_node` equals the requested node are included.

Example source manifest:

```text
sensor-monitor  -> fog-01
mqtt-adapter    -> fog-02
weather-filter  -> fog-01
```

Derived views:

```text
fog-01: delivery_scope=node_slice, selected=2/3
fog-02: delivery_scope=node_slice, selected=1/3
cloud-01: delivery_scope=empty, selected=0/3
```

The validator rejects a node-slice view containing an assignment for any other node.

## Centralized semantics

For `strategy=centralized` and `graph_distribution=full`, C63 already requires all assignments to target the same central node. C64 therefore derives `delivery_scope=full` with every assignment for that central node and `delivery_scope=empty` for every other node.

## Empty-view semantics

A node that is not a target receives a valid `empty` view rather than an error. This provides an explicit, auditable answer that the node has no deployment assignment in the manifest. An empty view contains no assignments and has `selected_assignment_count=0`.

## Validation order and safety

`derive_node_scoped_deployment(...)` first calls `validate_deployment_manifest(...)`; invalid C63 manifests are rejected before node scoping. The derived view is then validated independently and must preserve:

```text
explicit_invocation_required=true
automatic_execution_allowed=false
governed_execution_authorized=false
```

C64 remains data selection only. A non-empty node view is not authorization to execute.

## Tests added

C64 adds 12 dserver unit tests covering distributed node selection, non-target empty views, centralized full/empty behavior, invalid-source rejection, empty node IDs, artifact-identity preservation, fail-closed execution-boundary preservation, assignment-count consistency, cross-node leakage rejection, and serde representation/round-trip behavior.

## Linux validation — 2026-08-15

```bash
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml
```

Observed:

- formatting check: PASS;
- dserver tests: `464 passed; 0 failed`;
- dserver build: PASS;
- no dependency or lockfile changes;
- no governed execution allow transition;
- no execution or deployment side effects introduced.

C64 is complete at `136/150`.

## Next intended increment

C65 introduces the DATUM-side artifact acquisition/instantiation corridor that consumes an explicit node-scoped assignment, resolves its artifact through a deliberately bounded acquisition path, verifies SHA-256, and prepares bounded Wasmtime execution without opening the governed execution boundary.