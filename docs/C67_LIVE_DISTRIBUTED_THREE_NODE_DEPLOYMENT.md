# C67 — Live Distributed Three-Node Deployment

- checkpoint: `C67/150`
- target_progress_after_validation: `139/150`
- predecessor: `C66/150`
- phase: Controlled WASM Deployment
- status: implementation_ready_for_validation

## Experiment objective

Validate a real distributed deployment from one dserver-owned typed Deployment Manifest across three DATUM execution nodes:

```text
local machine
  dserver
  artifact repository
  DATUM node local-01

VM 1
  DATUM node fog-01

VM 2
  DATUM node cloud-01
```

The local machine is the control point. The two VMs must reach the local machine on the dserver and artifact-repository ports.

C67 proves distributed assignment, remote artifact delivery, independent SHA-256 verification, explicit Wasmtime execution, and centralized runtime-evidence collection. It does not yet claim that guest WASM modules communicate directly with one another or that dserver autonomously invokes DATUM. Those are later end-to-end orchestration concerns.

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

A view containing another node's assignment remains invalid.

## Three distinct artifacts

`scripts/c67_prepare_three_node_fixture.py` creates three independent WebAssembly binaries:

### fog-01

```text
fog-scale.wasm
fog_scale(i32 value, i32 factor) -> i32
```

The implementation uses `i32.mul`.

Validation input:

```text
fog_scale(12, 2) = 24
```

### local-01

```text
local-aggregate.wasm
local_aggregate(i32 a, i32 b) -> i32
```

The implementation uses `i32.add`.

Validation input:

```text
local_aggregate(24, 6) = 30
```

### cloud-01

```text
cloud-delta.wasm
cloud_delta(i32 value, i32 reference) -> i32
```

The implementation uses `i32.sub`.

Validation input:

```text
cloud_delta(30, 10) = 20
```

The values `24 -> 30 -> 20` make the intended fog -> local/mist -> cloud conceptual pipeline visible. C67 does not claim automatic runtime dataflow between the modules; the values are supplied explicitly during this deployment validation.

The binaries differ by export name and instruction, therefore their SHA-256 content IDs must also be distinct.

## Artifact repository

Artifacts are served from the local machine with a lab HTTP repository:

```bash
python3 -m http.server 8090 --bind 0.0.0.0 --directory .c67-live
```

All three DATUM executions use C65 network acquisition. For the trusted local laboratory, `--allow-insecure-http` is explicitly enabled.

Successful C67 reports must therefore contain:

```text
artifact_delivery_performed=true
host_network_fetch_performed=true
persistent_write_performed=false
host_mutation_performed=false
```

HTTP remains a laboratory-only opt-in. A production deployment should use HTTPS.

## Centralized report evidence

`scripts/c67_run_distributed_node.py` performs one node run:

1. fetches that node's C64 slice from dserver;
2. verifies the requested assignment is present and no assignment targets another node;
3. writes the temporary control-plane slice locally;
4. invokes the existing C66 `wasm-deploy-live` command with explicit `--execute`;
5. lets DATUM acquire the assigned artifact over the network and verify its SHA-256;
6. parses the resulting `LiveWasmExecutionReport`;
7. publishes the report to the local dserver;
8. deletes the temporary node-slice file.

The dserver accepts a C67 report only when it matches the active manifest identity and assignment binding. It also requires successful controlled execution evidence, network artifact delivery, no persistent artifact write, no guest side effects, no host mutation, no governed execution, and `timeout_enforced=false`.

## Reproducible setup

Choose an IP or hostname for the local machine that is reachable from both VMs. The examples below call it `<LOCAL_IP>`.

Prepare artifacts and the global manifest on the local machine:

```bash
python3 scripts/c67_prepare_three_node_fixture.py --artifact-host <LOCAL_IP>
```

Start the artifact repository in a second terminal:

```bash
python3 -m http.server 8090 --bind 0.0.0.0 --directory .c67-live
```

Run dserver on the local machine with its existing configuration. The server must listen on an address reachable from both VMs, normally `0.0.0.0:8080`.

Publish the global manifest:

```bash
curl -sS -X POST \
  http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment-manifest \
  -H 'Content-Type: application/json' \
  --data-binary '@.c67-live/c67-deployment-manifest.json'
```

Inspect the C64-derived slices:

```bash
curl -sS http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment/for-node/fog-01
curl -sS http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment/for-node/local-01
curl -sS http://<LOCAL_IP>:8080/api/v1/controlled-wasm/deployment/for-node/cloud-01
```

## Live executions

Run on VM 1:

```bash
python3 scripts/c67_run_distributed_node.py \
  --dserver http://<LOCAL_IP>:8080 \
  --node-id fog-01 \
  --assignment fog-scale \
  --arg i32:12 \
  --arg i32:2
```

Expected result: `24`.

Run on the local machine:

```bash
python3 scripts/c67_run_distributed_node.py \
  --dserver http://<LOCAL_IP>:8080 \
  --node-id local-01 \
  --assignment local-aggregate \
  --arg i32:24 \
  --arg i32:6
```

Expected result: `30`.

Run on VM 2:

```bash
python3 scripts/c67_run_distributed_node.py \
  --dserver http://<LOCAL_IP>:8080 \
  --node-id cloud-01 \
  --assignment cloud-delta \
  --arg i32:30 \
  --arg i32:10
```

Expected result: `20`.

Then validate all distributed evidence from the local machine:

```bash
python3 scripts/c67_validate_distributed_reports.py \
  --dserver http://<LOCAL_IP>:8080
```

The validator requires:

- exactly three reports;
- exactly the nodes `fog-01`, `local-01`, and `cloud-01`;
- exactly three distinct artifact content IDs;
- exactly three distinct execution IDs;
- expected assignments for each node;
- expected results `24`, `30`, and `20`;
- successful integrity/Wasmtime/guest-execution evidence;
- network artifact acquisition on every node;
- no persistent artifact write;
- no guest side effects;
- no host mutation;
- no governed execution;
- `timeout_enforced=false`.

## Regression validation before live deployment

C67 adds 10 dserver unit tests to the validated C64 dserver baseline of `464 passed; 0 failed`, so the expected dserver total is `474 passed; 0 failed`.

Run:

```bash
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml

python3 -m py_compile \
  scripts/c67_prepare_three_node_fixture.py \
  scripts/c67_run_distributed_node.py \
  scripts/c67_validate_distributed_reports.py
```

DATUM execution code is unchanged from validated C66, but each execution host must successfully build or already have a current C66-capable DATUM binary.

## Safety boundary

C67 intentionally keeps the controlled deployment corridor separate from canonical governed `wasm_execution`.

The live success path still requires:

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

## Next intended increment

After C67 proves the distributed three-node execution, I34/140 should close the end-to-end demonstration loop: external D-Graph -> dserver -> typed manifest -> node slices -> DATUM -> artifact acquisition -> runtime reports -> dserver materialized state, with less manual orchestration and evidence suitable for the live dashboard.
