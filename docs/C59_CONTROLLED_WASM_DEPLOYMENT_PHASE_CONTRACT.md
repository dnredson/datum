# C59 — Controlled WASM Deployment Phase Contract

- checkpoint: `C59/150`
- progress: `131/150`
- predecessor: `C58/130`
- runtime_changes_in_c59: none
- execution_policy_changes_in_c59: none

C59 opens a controlled WebAssembly deployment-readiness phase without changing runtime behavior.

## D-Graph boundary

The D-Graph is external input. SmartSentinel/dserver does not generate it in this phase. A separate upstream component may produce the graph; dserver may validate, materialize, inspect, plan from, and distribute that supplied representation.

D-Graph state, readiness, health observations, planner output, capability metadata, node assignment, deployment intent, and artifact presence remain evidence only and are not authorization.

## Initial deployment modes

Two deployment modes are reserved for the next increments:

- centralized: `strategy=centralized`, `graph_distribution=full`;
- distributed: `strategy=distributed`, `graph_distribution=node_slice`.

The first distributed implementation may reuse existing node-scoped snapshot and target-assignment mechanisms. C59 does not require a general graph partitioning or placement algorithm.

## Controlled WASM sandbox

The existing isolated Wasmtime test will evolve into an explicitly invoked, artifact-backed sandbox corridor. It must remain bounded by explicit artifact/function selection, bounded fuel, bounded timeout when enforceable support is added, no filesystem/network/host-command imports, no Docker access, no installation-task access, no assisted-execution access, and no implicit host mutation.

This corridor remains separate from governed D-Graph execution. The canonical operation classes `placement`, `recovery`, `migration`, `reconfiguration`, `deployment`, `installation`, `dispatch`, and `wasm_execution` remain fail-closed under C46–C57. C59 implements no allow transition.

## Demonstration vertical slice

Target flow:

`external D-Graph -> deployment intent -> centralized/full or distributed/node_slice assignment -> verified WASM artifact -> target DATUM -> bounded Wasmtime instantiation -> explicit function invocation -> runtime report -> dserver`

The demo must show a real external `.wasm` artifact assigned to one or more DATUM nodes, executed in the bounded sandbox, and reported without opening the governed execution boundary.

## Sequence to demo milestone

- C60 / 132: external `.wasm` artifact representation and loader;
- C61 / 133: SHA-256 verification before instantiation;
- C62 / 134: generic module/function WASM runner;
- C63 / 135: typed deployment manifest;
- C64 / 136: node-scoped deployment assignment;
- C65 / 137: DATUM artifact acquisition/instantiation corridor;
- C66 / 138: live single-node validation;
- C67 / 139: live two-node distributed validation;
- I34 / 140: end-to-end demo validation and evidence.

## C59 exit criteria

C59 is complete when the external D-Graph boundary, deployment modes, sandbox boundary, and preserved fail-closed guarantees are documented; no runtime code or side-effect path is changed; and C60 is the next implementation increment.
