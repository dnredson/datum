# I35 — Sentinel-Bound Deployment Cycle

- checkpoint: `I35/141`
- predecessor: `I34/140`
- phase: Controlled WASM Deployment
- status: `implementation_ready_for_validation`

## Objective

I35 removes the temporary Python orchestration glue from one controlled deployment cycle without enabling automatic governed execution.

The new controlled path is:

```text
external D-Graph
  -> dserver-derived Deployment Manifest
  -> dserver node slice
  -> Sentinel reads its own agent.node_id
  -> Sentinel fetches only its own node slice
  -> Sentinel requires exactly one delivered assignment
  -> Sentinel acquires + verifies the WASM artifact
  -> explicit --execute remains mandatory
  -> Sentinel invokes the assigned WASM function
  -> Sentinel POSTs its LiveWasmExecutionReport directly to dserver
```

The existing C66/C67 file-backed `wasm-deploy-live` mode remains available for regression compatibility.

## Sentinel-bound control-plane mode

`wasm-deploy-live` now enters Sentinel-bound mode when `--deployment` is an `http://` or `https://` dserver base URL.

Example lab invocation:

```bash
DATUM/target/debug/datum wasm-deploy-live \
  --config config.toml \
  --deployment http://192.168.10.101:8080 \
  --assignment auto \
  --arg i32:12 \
  --arg i32:2 \
  --execute \
  --invoked-by i35-sentinel-validation \
  --allow-insecure-http \
  --json
```

There is deliberately no `--node-id` in this mode.

## Bound node identity

I35 treats `agent.node_id` from the Sentinel configuration as the locally bound logical identity for this controlled path.

If `--node-id` is supplied while `--deployment` selects control-plane mode, the command fails before fetching a slice.

This closes the C67 operator-override weakness for the new I35 path. It is still configuration binding, not cryptographic attestation. A future increment must bind identity to authenticated agent credentials/certificates.

## Assignment selection

The operator may not choose an assignment identifier in Sentinel-bound mode.

The CLI requires the literal selector:

```text
--assignment auto
```

and the Sentinel accepts the control-plane view only when its node slice contains exactly one assignment. Zero or multiple assignments fail closed in I35.

This is intentionally narrow. Future scheduling/batching semantics can be added only after they have an explicit execution policy.

## Control-plane transport

HTTPS is allowed by default.

Plain HTTP is rejected unless the operator explicitly supplies:

```text
--allow-insecure-http
```

The same lab opt-in continues to govern HTTP artifact acquisition. Redirects are disabled and the existing bounded request timeout is reused for control-plane requests.

The dserver URL must not embed credentials.

## Direct report publication

After a successful `status=executed` report is produced, the Sentinel POSTs it directly to:

```text
POST /api/v1/controlled-wasm/live-reports
```

The command requires dserver to acknowledge the same `execution_id`; a mismatched acknowledgement fails.

Failed/blocked executions are not presented as successfully published I35 cycles. The current controlled dserver live-report gate still accepts only the validated executed report shape.

## Safety boundary

I35 does not weaken the validated execution boundary:

```text
explicit_invocation_required=true
automatic_execution_allowed=false
governed_execution_authorized=false
```

`--execute` is still mandatory.

Artifact SHA-256/content identity verification, Wasmtime import isolation, scalar ABI validation, fuel enforcement, no persistent artifact write, no guest host imports, and `safe_to_auto_execute=false` remain inherited from C65/C66.

Wall-clock timeout remains metadata-only (`timeout_enforced=false`).

## Regression target

I35 adds 7 focused DATUM unit tests to the existing 401-test DATUM baseline.

Expected target:

```text
408 passed; 0 failed
```

Run:

```bash
cargo fmt --manifest-path DATUM/Cargo.toml -- --check
cargo test --manifest-path DATUM/Cargo.toml
cargo build --manifest-path DATUM/Cargo.toml
```

The focused tests cover:

- control-plane mode recognition;
- rejection of `--node-id` override;
- `--assignment auto` enforcement;
- non-empty bound `agent.node_id`;
- HTTP fail-closed behavior and explicit lab opt-in;
- safe endpoint construction/encoding;
- exactly-one-assignment selection.

## Planned live validation

Reuse the I34 laboratory topology after the regression passes:

```text
192.168.10.101 desktop: dserver + artifact repository + local-01
192.168.10.103 VM: fog-01
192.168.10.108 VM: cloud-01
```

Publish the I34 external D-Graph first, then run the DATUM binary directly on each node. Do not use `scripts/c67_run_distributed_node.py` in the I35 success path.

Expected executions remain:

```text
fog-01   -> fog_scale(12,2)       -> 24
local-01 -> local_aggregate(24,6) -> 30
cloud-01 -> cloud_delta(30,10)    -> 20
```

The final dserver live-report and observed-D-Graph validators must still see exactly three reports and six WASM report `OBSERVES` relationships.

## Non-claims

I35 does not claim:

- cryptographic physical-agent attestation;
- automatic polling/continuous activation;
- multiple-assignment scheduling policy;
- guest-to-guest WASM dataflow;
- wall-clock timeout interruption;
- production authentication/TLS setup;
- canonical governed automatic execution.

Checkpoint remains `140/150` until local regression and the direct-Sentinel three-node live campaign pass.
