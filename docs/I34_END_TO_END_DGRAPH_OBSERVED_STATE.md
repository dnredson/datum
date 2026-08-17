# I34 — End-to-End External D-Graph to Observed Deployment State

- checkpoint: `I34/140`
- predecessor: `C67/139`
- phase: Controlled WASM Deployment
- status: `implementation_ready_for_validation`

## Objective

I34 closes the controlled end-to-end data/evidence loop that C67 deliberately started from a prebuilt Deployment Manifest:

```text
External D-Graph
  -> dserver validation
  -> derived typed Deployment Manifest
  -> C64 node slices
  -> Sentinel/DATUM agents
  -> bounded artifact acquisition
  -> SHA-256/content identity verification
  -> explicit Wasmtime execution
  -> live runtime reports
  -> dserver
  -> materialized observed D-Graph
```

The external component remains responsible for generating the D-Graph. dserver consumes, validates, interprets, and materializes it; dserver does not become the D-Graph generator.

I34 remains inside the controlled explicit-execution corridor. It does not enable canonical governed automatic `wasm_execution`.

## External D-Graph deployment contract

The controlled I34 bridge derives a C63 `DeploymentManifest` by joining existing canonical D-Graph entities:

```text
DGraphWasmModuleRef
  module_id + function_name
      |
      v
DGraphModule
  artifact_id
      |
      v
DGraphArtifact
  artifact_ref + metadata.sha256

DGraphPlacementDecision
  module_id -> selected_node_id
```

Required source fields for every deployable WASM module are:

- a valid graph-level `metadata.dgraph_id`;
- a `DGraphWasmModuleRef` with `module_id` and `function_name`;
- a matching `DGraphModule` with `artifact_id`;
- a matching `DGraphArtifact` with `artifact_ref` and `metadata.sha256`;
- exactly one `DGraphPlacementDecision` for that module;
- the placement must reference a node present in the D-Graph.

Optional WASM descriptor metadata:

```json
{
  "assignment_id": "fog-scale",
  "fuel": 1000000,
  "timeout_ms": 1000
}
```

If omitted, assignment IDs are derived deterministically and runtime limits default to 1,000,000 fuel units and 1,000 ms timeout metadata.

The derived manifest always uses the currently validated controlled semantics:

```text
strategy=distributed
graph_distribution=node_slice
explicit_invocation_required=true
automatic_execution_allowed=false
governed_execution_authorized=false
```

The derived manifest carries `source_dgraph_id=<metadata.dgraph_id>` for provenance.

## Controlled endpoints

I34 adds two endpoints while retaining all C67 endpoints:

```text
POST /api/v1/controlled-wasm/dgraph
GET  /api/v1/controlled-wasm/dgraph
GET  /api/v1/controlled-wasm/observed-dgraph
```

Publishing an external D-Graph:

1. validates the canonical D-Graph;
2. derives and validates the typed Deployment Manifest;
3. stores the source D-Graph as controlled experiment state;
4. makes the derived manifest active;
5. clears prior controlled live reports.

The existing endpoint then exposes the derived manifest:

```text
GET /api/v1/controlled-wasm/deployment-manifest
```

and C64 continues to derive per-node views:

```text
GET /api/v1/controlled-wasm/deployment/for-node/:node_id
```

Posting a legacy C67 manifest directly remains supported, but explicitly clears the external D-Graph state so the server cannot falsely claim external-D-Graph provenance for that legacy path.

## Materialized observed D-Graph

`GET /api/v1/controlled-wasm/observed-dgraph` clones the original external D-Graph and projects accepted live execution evidence into canonical D-Graph structures.

For every live report, the materializer rechecks:

```text
assignment_id
module_id
function_name
target node
artifact content identity
```

against the active derived manifest.

It then creates a `DGraphWasmRuntimeReportRef` using the live `execution_id` as `report_id`, with metadata containing the execution provenance, result, fuel consumption, artifact content identity, timestamp, and integrity evidence.

The matching `DGraphModule.observed_state` becomes the live execution status (`executed` in the success path), and its metadata records the last execution ID, node, and artifact content identity.

Existing `build_basic_dgraph_relationships()` is reused, therefore every observed WASM report produces the canonical relationships:

```text
WasmRuntimeReport --OBSERVES--> WasmModuleDescriptor
WasmRuntimeReport --OBSERVES--> Node
```

The resulting graph is validated again with `validate_dgraph()` and summarized with `summarize_dgraph()`.

The response explicitly retains:

```text
side_effects_performed=false
safe_to_auto_execute=false
governed_execution_performed=false
```

## Live fixture

`scripts/i34_prepare_external_dgraph.py` deliberately does not read or derive from a Deployment Manifest. It uses the independently existing C67 WASM files, recomputes SHA-256 from their bytes, and constructs an external D-Graph containing nodes, modules, artifacts, deployment intents, placements, and WASM descriptors.

For the existing laboratory topology:

```text
192.168.10.103 -> fog-01   -> fog-scale-module
192.168.10.107 -> local-01 -> local-aggregate-module
192.168.10.108 -> cloud-01 -> cloud-delta-module
```

The existing artifact repository remains:

```text
192.168.10.107:8090
```

Prepare the source graph from the repository root:

```bash
python3 scripts/i34_prepare_external_dgraph.py \
  --artifact-host 192.168.10.107
```

Expected output file:

```text
.i34-live/i34-external-dgraph.json
```

Publish the external D-Graph:

```bash
curl -sS -X POST \
  http://192.168.10.107:8080/api/v1/controlled-wasm/dgraph \
  -H 'Content-Type: application/json' \
  --data-binary '@.i34-live/i34-external-dgraph.json' \
  | python3 -m json.tool
```

Expected derived identity:

```text
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
project_id=i34-live-project
strategy=distributed
graph_distribution=node_slice
assignment_count=3
```

Inspect the derived manifest and C64 node slices before execution.

## Live execution

DATUM/Sentinel execution code is unchanged from C67. Existing C67-capable binaries on the two VMs can therefore be reused after the dserver branch is updated locally.

The explicit validation invocations remain:

```text
fog-01   -> fog_scale(12,2)       -> 24
local-01 -> local_aggregate(24,6) -> 30
cloud-01 -> cloud_delta(30,10)    -> 20
```

`scripts/c67_run_distributed_node.py` remains the explicit operator harness for this checkpoint. I34 removes the manually authored manifest from that path; later increments can move polling/activation and authenticated node identity fully inside the Sentinel agent.

## Final observed-state validation

After exactly one intended live execution per node:

```bash
python3 scripts/i34_validate_observed_dgraph.py \
  --dserver http://192.168.10.107:8080
```

The validator requires:

- source D-Graph ID `i34-external-dgraph`;
- derived manifest ID `manifest:i34-external-dgraph`;
- project ID `i34-live-project`;
- exactly three accepted live reports;
- canonical D-Graph validation with no errors;
- exactly the three expected nodes;
- all three modules with `observed_state=executed`;
- three materialized `DGraphWasmRuntimeReportRef` records;
- expected results 24, 30, and 20;
- artifact `sha256:` content identities retained in evidence;
- integrity verification retained in report metadata;
- exactly six WASM report `OBSERVES` relationships;
- `side_effects_performed=false`.

## Regression validation

C67 established a dserver baseline of `474 passed; 0 failed`.

I34 adds 8 focused dserver unit tests, so the expected test target is:

```text
482 passed; 0 failed
```

Run from the repository root:

```bash
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml

python3 -m py_compile \
  scripts/i34_prepare_external_dgraph.py \
  scripts/i34_validate_observed_dgraph.py \
  scripts/c67_run_distributed_node.py
```

## Non-claims and follow-ups

I34 does not claim:

- automatic Sentinel polling/execution without explicit operator activation;
- guest-to-guest WASM dataflow;
- authenticated/bound physical Sentinel identity;
- timeout enforcement by wall clock;
- canonical governed execution authorization;
- production HTTPS artifact hosting.

The C67 identity-hardening observation remains open: the final normal Sentinel path must bind `node_id` to the installed/authenticated agent rather than accepting a freely supplied command-line identity.

Once I34 live validation passes, the checkpoint can advance from `139/150` to `140/150`.
