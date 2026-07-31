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

## Verification

C43 passed the complete Windows validation suite before merge:

- `dserver`: 368/368 tests, `cargo fmt --check`, and build;
- `DATUM`: 353/353 tests, `cargo fmt --check`, and build;
- only the pre-existing DATUM `dead_code` warnings remained.

The metadata-only live read on `main@caa0e46` then confirmed the restored C42 baseline as:

```text
readiness_status       : ready
healthy_targets        : 0
unknown_targets        : 3
degraded_targets       : 0
unavailable_targets    : 0
unhealthy_targets      : 0
warning_count          : 0
blocked_count          : 0
side_effects_performed : False
execution_performed    : False
```

The materialized graph retained 3 snapshots, 3 nodes, 3 node-scoped runtimes, 3 modules, 2 flows, and 14 relationships with zero validation errors. All readiness checks passed.

See [the C43 live validation record](LIVE_C43_READ_ONLY_HEALTH_NORMALIZATION_VALIDATION.md) for the complete evidence.

## Closure And Follow-Up

C43/130 is implementation-, Rust-, and live-validated. Progress advances from `112/130` to `113/130`.

Deriving health from a fresh local service observation remains a separate increment. C43 makes the summary semantics trustworthy before that observation source is introduced.
