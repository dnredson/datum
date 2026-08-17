# C67 — Live Distributed Three-Node Deployment

- checkpoint: `C67/150`
- target_progress_after_validation: `139/150`
- predecessor: `C66/150`
- phase: Controlled WASM Deployment
- status: validated; see `C67_LIVE_DISTRIBUTED_THREE_NODE_DEPLOYMENT_FINAL.md`

## Experiment objective

Validate a real distributed deployment from one dserver-owned typed Deployment Manifest across three Sentinel/DATUM execution nodes:

```text
local machine
  dserver
  artifact repository
  Sentinel/DATUM node local-01

VM 1
  Sentinel/DATUM node fog-01

VM 2
  Sentinel/DATUM node cloud-01
```

The local machine is the control point. The two VMs must reach the local machine on the dserver and artifact-repository ports.

C67 proves distributed assignment, remote artifact delivery, independent SHA-256 verification, explicit Wasmtime execution, and centralized runtime-evidence collection. It does not claim that guest WASM modules communicate directly with one another or that dserver autonomously invokes Sentinel/DATUM.

## Global manifest and C64 node slicing

C67 adds a controlled in-memory dserver API:

```text
POST /api/v1/controlled-wasm/deployment-manifest
GET  /api/v1/controlled-wasm/deployment-manifest
GET  /api/v1/controlled-wasm/deployment/for-node/:node_id
POST /api/v1/controlled-wasm/live-reports
GET  /api/v1/controlled-wasm/live-reports
```

Posting a new manifest validates the C63 contract, makes it the active experiment manifest, and clears prior C67 live reports.

The per-node endpoint calls the existing C64 `derive_node_scoped_deployment()` implementation. Node views are therefore not hand-written fixtures.

Expected isolation:

```text
fog-01   -> fog-scale only
local-01 -> local-aggregate only
cloud-01 -> cloud-delta only
```

## Three distinct artifacts

`scripts/c67_prepare_three_node_fixture.py` creates three independent WebAssembly binaries under `.c67-live/artifacts/`:

```text
fog-scale.wasm
  fog_scale(i32 value, i32 factor) -> i32
  fog_scale(12, 2) = 24

local-aggregate.wasm
  local_aggregate(i32 a, i32 b) -> i32
  local_aggregate(24, 6) = 30

cloud-delta.wasm
  cloud_delta(i32 value, i32 reference) -> i32
  cloud_delta(30, 10) = 20
```

The values `24 -> 30 -> 20` make the intended fog -> local -> cloud conceptual sequence visible. C67 does not claim automatic runtime dataflow between the modules; the values are supplied explicitly during deployment validation.

## Artifact repository

Prepare artifacts and the manifest on the local machine:

```bash
python3 scripts/c67_prepare_three_node_fixture.py --artifact-host <LOCAL_IP>
```

Serve the `.c67-live` directory:

```bash
python3 -m http.server 8090 --bind 0.0.0.0 --directory .c67-live
```

The generated manifest uses URLs such as:

```text
http://<LOCAL_IP>:8090/artifacts/fog-scale.wasm
http://<LOCAL_IP>:8090/artifacts/local-aggregate.wasm
http://<LOCAL_IP>:8090/artifacts/cloud-delta.wasm
```

A connectivity check must therefore include `/artifacts/`, for example:

```bash
curl -I http://<LOCAL_IP>:8090/artifacts/fog-scale.wasm
```

All three Sentinel/DATUM executions use C65 network acquisition. Insecure HTTP is explicitly enabled only for the trusted laboratory campaign; production deployment should use HTTPS.

Successful C67 reports contain:

```text
artifact_delivery_performed=true
host_network_fetch_performed=true
persistent_write_performed=false
host_mutation_performed=false
```

## Centralized report evidence

`scripts/c67_run_distributed_node.py` performs one node run:

1. fetches that node's C64 slice from dserver;
2. verifies the requested assignment is present and no assignment targets another node;
3. writes a temporary control-plane slice locally;
4. invokes the C66 `wasm-deploy-live` Sentinel/DATUM command with explicit `--execute`;
5. Sentinel/DATUM acquires the assigned artifact over the network and verifies SHA-256;
6. Sentinel/DATUM executes the assigned function in Wasmtime and emits `LiveWasmExecutionReport`;
7. the runner publishes the report to dserver;
8. the temporary slice file is deleted.

The dserver accepts C67 success evidence only when it matches the active manifest identity and assignment binding and records successful controlled execution, network artifact delivery, no persistent artifact write, no guest side effects, no host mutation, no governed execution, and `timeout_enforced=false`.

## Reproducible live setup

Publish the global manifest:

```bash
curl -sS -X POST \
  http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment-manifest \
  -H 'Content-Type: application/json' \
  --data-binary '@.c67-live/c67-deployment-manifest.json'
```

Inspect the three C64-derived slices:

```bash
curl -sS http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment/for-node/fog-01
curl -sS http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment/for-node/local-01
curl -sS http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment/for-node/cloud-01
```

Run on VM 1:

```bash
python3 scripts/c67_run_distributed_node.py \
  --dserver http://<LOCAL_IP>:8080 \
  --node-id fog-01 \
  --assignment fog-scale \
  --arg i32:12 \
  --arg i32:2 \
  --datum-bin DATUM/target/debug/datum
```

Run on the local machine:

```bash
python3 scripts/c67_run_distributed_node.py \
  --dserver http://<LOCAL_IP>:8080 \
  --node-id local-01 \
  --assignment local-aggregate \
  --arg i32:24 \
  --arg i32:6 \
  --datum-bin DATUM/target/debug/datum
```

Run on VM 2:

```bash
python3 scripts/c67_run_distributed_node.py \
  --dserver http://<LOCAL_IP>:8080 \
  --node-id cloud-01 \
  --assignment cloud-delta \
  --arg i32:30 \
  --arg i32:10 \
  --datum-bin DATUM/target/debug/datum
```

Then validate all distributed evidence:

```bash
python3 scripts/c67_validate_distributed_reports.py \
  --dserver http://<LOCAL_IP>:8080
```

The validator requires exactly three reports; nodes `fog-01`, `local-01`, and `cloud-01`; three distinct artifact content IDs; three distinct execution IDs; expected assignments/results; verified integrity/Wasmtime/guest execution; network artifact acquisition; no persistent write; no guest side effects; no host mutation; no governed execution; and `timeout_enforced=false`.

## Regression validation

C67 adds 10 dserver tests to the C64 baseline. The validated result was:

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check  PASS
cargo test --manifest-path dserver/Cargo.toml           474 passed; 0 failed
cargo build --manifest-path dserver/Cargo.toml          PASS
Python syntax validation                                 PASS
```

## Safety boundary

C67 intentionally keeps the controlled deployment corridor separate from canonical governed `wasm_execution`.

```text
explicit execution = true
safe_to_auto_execute = false
wasm_guest_side_effects = false
governed_execution_performed = false
host guest imports = forbidden
fuel = enforced
timeout = reported but not enforced
```

The node runner's network access and report publication are host-side experiment orchestration, not WASM guest capabilities.

## Final evidence and next increment

The clean three-node live campaign passed. Full evidence and the node-identity hardening observation are recorded in `docs/C67_LIVE_DISTRIBUTED_THREE_NODE_DEPLOYMENT_FINAL.md`.

Next: I34/140 should close the end-to-end external D-Graph -> dserver -> typed manifest -> node slices -> Sentinel/DATUM -> artifact acquisition/execution -> runtime reports -> dserver materialized observed state loop, with less temporary orchestration glue.
