# I35 Validation Checklist

Status: pending local regression and live validation.

## Local regression

```text
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

Expected DATUM test target: `408 passed; 0 failed`.

## Required fail-closed checks

The new control-plane mode must reject:

```text
--node-id <anything>
--assignment <anything other than auto>
http:// dserver without --allow-insecure-http
empty agent.node_id
zero assignments in a node slice
more than one assignment in a node slice
```

`--execute` remains mandatory through the existing C66 command gate.

## Live success criteria

The success path must use the DATUM binary directly, not `scripts/c67_run_distributed_node.py`.

For each node, the Sentinel must:

```text
read agent.node_id
GET its own dserver node slice
select the single delivered assignment
fetch the artifact
verify SHA-256/content identity
execute explicitly in Wasmtime
POST its LiveWasmExecutionReport directly to dserver
receive acknowledgement with the same execution_id
```

Expected results:

```text
fog-01   -> 24
local-01 -> 30
cloud-01 -> 20
```

Final gates:

```text
live report count = 3
observed D-Graph report count = 3
executed module count = 3
WasmRuntimeReport OBSERVES relationships = 6
```

Do not advance to `141/150` until all gates pass.
