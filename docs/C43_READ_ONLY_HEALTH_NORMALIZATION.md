# C43/130 — Read-Only Health Normalization

## Purpose

C43 corrects the materialized D-Graph health summary so that an unobserved target is not reported as unhealthy.

The change remains metadata-only and read-only. It normalizes health reports that already exist in local D-Graph snapshots; it does not execute checks or collect new runtime observations.

## Previous Behavior

Before C43, `summarize_dgraph` classified every health status other than `healthy`, `ok`, or `running` as unhealthy. Consequently, the C42 three-node metadata-only baseline reported:

- `unknown_targets` was not available;
- `unhealthy_targets=3` for three module reports whose status was `unknown`.

Those values remain valid historical evidence for the pre-C43 contract, but `unknown` did not represent an operational failure.

## Normalized Contract

Health status matching is case-insensitive and ignores surrounding whitespace.

| Output field | Normalized input statuses |
| --- | --- |
| `healthy_targets` | `healthy`, `ok`, `running` |
| `degraded_targets` | `degraded`, `warning` |
| `unavailable_targets` | `unavailable`, `down`, `failed`, `critical` |
| `unknown_targets` | `unknown`, empty, or any unrecognized status |
| `unhealthy_targets` | `degraded_targets + unavailable_targets` |

The conservative fallback to `unknown` prevents a new or malformed status from being interpreted as a failure without explicit semantics.

## Server Changes

`dserver` now:

- classifies each `DGraphHealthReport` into exactly one normalized category;
- exposes the four category counts in `DGraphSummary`;
- retains `unhealthy_targets` as a backward-compatible aggregate;
- excludes `unknown` from `unhealthy_targets`;
- preserves zero defaults when an older serialized summary omits the new fields.

The C42-equivalent three-node materializer regression now expects:

- `healthy_targets=0`;
- `unknown_targets=3`;
- `degraded_targets=0`;
- `unavailable_targets=0`;
- `unhealthy_targets=0`.

## DATUM Changes

DATUM now:

- deserializes all four normalized category counts as optional fields;
- remains able to parse an older server response that does not contain the new fields;
- displays the normalized counts in the human-readable materialized summary;
- preserves the existing JSON viewer contract.

## Smoke Contract

The client-server smoke contract now requires all normalized health fields from both materialization endpoints and asserts:

```text
unhealthy_targets == degraded_targets + unavailable_targets
```

All health counters must be non-negative.

## Safety Boundary

C43 does not:

- run service, HTTP, TCP, MQTT, process, or container checks;
- inspect, start, stop, pull, or remove Docker resources;
- execute planner proposals, recovery, dispatch, deployment, installation, or reconfiguration;
- load or execute WASM artifacts;
- ingest or publish telemetry;
- mutate local mappings or active configuration.

The existing `side_effects_performed=false` and `execution_performed=false` guarantees remain unchanged.

## Verification Required Before Merge

Run from the repository root:

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

The current implementation workspace did not provide a Rust toolchain, so these commands must pass before C43 is marked complete and promoted from `112/130` to `113/130`.

## Follow-Up

After the contract is verified, a metadata-only live read can confirm that the restored C42 baseline changes from the historical `unhealthy_targets=3` to:

```text
healthy_targets=0
unknown_targets=3
degraded_targets=0
unavailable_targets=0
unhealthy_targets=0
```

Deriving health from a fresh local service observation remains a separate increment. C43 only makes the summary semantics trustworthy before that observation source is introduced.
