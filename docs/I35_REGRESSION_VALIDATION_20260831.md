# I35 — Local Regression Validation — 2026-08-31

- checkpoint target: `I35/141`
- branch: `agent/i35-sentinel-bound-deployment-cycle`
- status: `PASS`

## Result

The local DATUM regression gate passed on the Windows desktop after implementing the Sentinel-bound control-plane deployment cycle.

```text
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
PASS

cargo test --manifest-path DATUM/Cargo.toml
419 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

cargo build --manifest-path DATUM/Cargo.toml
PASS
```

The previously documented `408` target was only an estimate based on the historical C65 `401`-test baseline plus the 7 new focused I35 tests. The actual current branch contains additional DATUM tests accumulated after that baseline. The authoritative observed regression result for I35 is therefore:

```text
419 passed; 0 failed
```

## I35 focused coverage

The new tests cover the Sentinel-bound path requirements:

- control-plane mode recognition;
- rejection of `--node-id` override;
- mandatory `--assignment auto` selection policy;
- non-empty `agent.node_id` binding;
- fail-closed plain HTTP control-plane access unless explicitly enabled for the lab;
- bounded/encoded control-plane endpoint construction;
- exactly-one-assignment acceptance.

## Remaining gate

This regression PASS does not advance the phase checkpoint by itself.

I35 remains at `140/150` until the direct-Sentinel three-node live campaign passes without `scripts/c67_run_distributed_node.py`, including:

```text
fog-01   -> result 24
local-01 -> result 30
cloud-01 -> result 20
live report count = 3
observed module count = 3 executed
WasmRuntimeReport OBSERVES relationships = 6
```

Canonical governed automatic `wasm_execution` remains fail-closed.
