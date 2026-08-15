# C64 — Node-Scoped Deployment Assignment

- checkpoint: `C64/150`
- target_progress_after_validation: `136/150`
- predecessor: `C63/150`
- phase: Controlled WASM Deployment
- status: implementation_ready_for_validation

## Objective

Consume the validated typed Deployment Manifest introduced by C63 and derive a deterministic, non-executable deployment view for a specific target node.

C64 does not acquire artifacts, contact DATUM, instantiate WASM, execute functions, mutate D-Graph, install software, dispatch governed actions, or authorize `wasm_execution`.

## Node-scoped contract

`dserver/src/core/deployment_assignment.rs` introduces:

- `NodeDeploymentScope::{Full, NodeSlice, Empty}`;
- `NodeScopedDeployment`;
- `derive_node_scoped_deployment(...)`;
- `validate_node_scoped_deployment(...)`.

Each node-scoped view preserves:

- source manifest version and ID;
- project ID;
- requested `node_id`;
- deployment strategy;
- graph-distribution semantics;
- source D-Graph identity when present;
- source assignment count;
- selected assignment count;
- the selected typed assignments;
- the fail-closed execution boundary from C63.

## Distributed semantics

For:

```text
strategy=distributed
graph_distribution=node_slice
```

only assignments whose `target_node` equals the requested node are included.

Example source manifest:

```text
sensor-monitor -> fog-01
mqtt-adapter  -> fog-02
weather-filter -> fog-01
```

Derived views:

```text
fog-01: delivery_scope=node_slice, selected=2/3
fog-02: delivery_scope=node_slice, selected=1/3
cloud-01: delivery_scope=empty, selected=0/3
```

The validator rejects a node-slice view containing an assignment for any other node.

## Centralized semantics

For:

```text
strategy=centralized
graph_distribution=full
```

C63 already requires all assignments to target the same central node. C64 therefore derives:

- `delivery_scope=full` with every assignment for that central node;
- `delivery_scope=empty` for every other node.

This preserves the C59 meaning of centralized/full without duplicating the full deployment payload to unrelated nodes.

## Empty-view semantics

A node that is not a target receives a valid `empty` view rather than an error. This is intentional: it gives distribution code an explicit, auditable answer that the node has no deployment assignment in the manifest.

An empty view contains no assignments and has `selected_assignment_count=0`.

## Validation order and safety

`derive_node_scoped_deployment(...)` first calls `validate_deployment_manifest(...)`. Invalid C63 manifests are therefore rejected before node scoping.

The derived view is then validated independently. It must preserve:

```text
explicit_invocation_required=true
automatic_execution_allowed=false
governed_execution_authorized=false
```

C64 remains data selection only. A non-empty node view is not authorization to execute.

Artifact identity, SHA-256, function selection, runtime limits, and assignment metadata are cloned from the validated C63 assignments without reinterpretation.

## Tests added

C64 adds 12 dserver unit tests covering:

1. distributed `fog-01` receives only its assignments;
2. distributed `fog-02` receives only its assignment;
3. non-target distributed node receives an empty view;
4. centralized target receives the full assignment set;
5. centralized non-target receives an empty view;
6. invalid source manifest is rejected before scoping;
7. empty node ID is rejected;
8. artifact identity is preserved;
9. fail-closed execution boundary is preserved;
10. selected-assignment count consistency;
11. cross-node assignment leakage is rejected;
12. reserved `node_slice` serde representation round-trip.

With the validated C63 dserver baseline at `452/452`, the expected C64 dserver total is `464` tests.

## Validation required before checkpoint advancement

Run from the repository root on the C64 branch:

```bash
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml
```

Do not advance the phase checkpoint to `136/150` and do not merge until these checks pass.

## Next intended increment

After C64 is validated and integrated, C65 introduces the DATUM-side artifact acquisition/instantiation corridor that consumes an explicit node-scoped assignment, resolves its artifact locally or through a deliberately bounded acquisition path, verifies SHA-256, and prepares bounded Wasmtime execution without opening the governed execution boundary.
