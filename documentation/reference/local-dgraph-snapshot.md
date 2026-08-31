# Local D-Graph snapshot contract

The local D-Graph snapshot is the node-scoped graph/evidence payload accepted by dserver through:

```text
POST /api/v1/nodes/:node_id/dgraph-snapshot
```

The API also requires the `X-Datum-Project` header. dserver normalizes canonical project/node identity and ingestion time instead of blindly trusting identity fields supplied by the client.

Implementation: `dserver/src/core/local_dgraph_snapshot.rs` and `dserver/src/api/local_dgraph_snapshot.rs`.

## Why there are two input types

The API accepts `IncomingDGraphSnapshot`, then converts it to canonical `LocalDGraphSnapshot`.

Identity fields such as `project_id`, `node_id`, `snapshot_id`, and `reported_at_utc` are optional in the incoming request because dserver assigns/overwrites their canonical values from the request context and server clock.

This prevents a client from authoritatively choosing a project/node identity that conflicts with the API path/header.

## Canonical `LocalDGraphSnapshot`

| Field | Meaning | Why it exists |
|---|---|---|
| `project_id` | Canonical DATUM project scope assigned from `X-Datum-Project`. | Keeps snapshots from different projects separated and prevents payload-only project spoofing within this contract. |
| `node_id` | Canonical logical node identity assigned from the URL path. | Guarantees node-scoped ingestion uses the addressed node identity. |
| `snapshot_id` | Server-generated unique snapshot identity. | Gives every ingestion a canonical reference for audit/materialization without trusting a client-supplied ID. |
| `reported_at_utc` | Server-side ingestion/report timestamp. | Provides a canonical control-plane receive/report time. |
| `source` | Provenance label. Normalization currently sets it to `client`. | Makes it explicit that this fragment arrived from a client/node source rather than being native dserver materialization. |
| `generated_at_utc` | Optional producer-side generation time. | Preserves source-time context separately from server `reported_at_utc`, allowing transport/age reasoning later. |
| `runtimes` | Runtime observations represented by `LocalRuntimeSnapshot`. | Describes execution environments visible/configured at the node. |
| `modules` | Module observations represented by `LocalModuleSnapshot`. | Describes application components and desired/observed state at the node. |
| `flows` | Local logical/data-flow descriptions. | Captures module membership and directed flow edges. |
| `health` | Explicit health reports targeting local resources. | Separates health evidence from generic metadata and lets it materialize into D-Graph health reports. |
| `capabilities` | Local capability observations. | Makes supported/unsupported abilities first-class graph evidence. |
| `metadata` | Optional snapshot-level auxiliary data. | Extension point for context not yet promoted to typed fields. |
| `safe_to_auto_execute` | Safety assertion that must currently remain `false`. | Explicitly prevents metadata-only/local snapshot ingestion from being treated as permission for automatic execution. Validation rejects `true`. |
| `side_effects_performed` | Whether producing/ingesting this snapshot performed managed-environment side effects. Must currently remain `false`. | Enforces the architectural boundary that observation/materialization is read-only. Validation rejects `true`. |

## `LocalRuntimeSnapshot`

| Field | Meaning / purpose |
|---|---|
| `runtime_id` | Required identity of the runtime in the graph fragment. Empty values are rejected. |
| `runtime_kind` | Required runtime family/type (e.g. container/WASM/process semantics as defined by producer/domain). Empty values are rejected. |
| `status` | Required producer-reported runtime status. Keeps current condition separate from mere runtime existence. |
| `source` | Provenance of this runtime statement. During D-Graph conversion it is merged into metadata. |
| `metadata` | Runtime-specific auxiliary information. |

## `LocalModuleSnapshot`

| Field | Meaning | Why it exists |
|---|---|---|
| `module_id` | Required module identity. | Flow membership/edges and artifact mapping refer to modules by this ID. Empty IDs are rejected. |
| `module_kind` | Optional application-component category. | Allows semantic classification without requiring one fixed module taxonomy; conversion falls back to `unknown` if absent. |
| `desired_state` | Optional intended module state. | Keeps operator/config intent separate from what is actually observed. |
| `observed_state` | Optional evidence-backed current state. | Lets the graph carry configuration and runtime truth side by side. |
| `runtime` | Optional runtime reference/kind association. | Connects the module to its execution environment semantics. |
| `artifact_ref` | Optional artifact reference. | Allows materialization to create a D-Graph artifact linked to the module when an artifact exists. |
| `local_mapping_key` | Optional key used to map logical module intent to local concrete resources. | Preserves the boundary between portable logical identity and host-specific mapping. |
| `local_container_name` | Optional concrete local container name. | Carries local execution mapping evidence without redefining the logical module ID. |
| `source` | Provenance for module information. |
| `metadata` | Auxiliary module-specific information. |

### Artifact conversion

When `artifact_ref` is present and non-empty, conversion creates a D-Graph artifact with an ID derived from node/module identity and stores local mapping/container information in artifact metadata. This keeps an external artifact reference separate from the graph's internal artifact identity.

## `LocalFlowSnapshot`

| Field | Meaning / purpose |
|---|---|
| `flow_id` | Required flow identity. Empty IDs are rejected. |
| `flow_kind` | Optional flow category. |
| `modules` | Module IDs participating in the flow. Every non-empty reference must exist in this snapshot's `modules`. |
| `edges` | Directed module-to-module flow relationships. Endpoints must reference modules present in the snapshot. |
| `status` | Optional flow-level status. |
| `source` | Flow provenance. |
| `metadata` | Auxiliary flow information. |

The cross-reference validation is important: a local flow cannot silently refer to a module that is absent from the same graph fragment.

## `LocalFlowEdgeSnapshot`

| Field | Meaning / purpose |
|---|---|
| `edge_id` | Required relationship identity. |
| `from_module_id` | Required source module; must exist in the module set. |
| `to_module_id` | Required destination module; must exist in the module set. |
| `protocol` | Optional protocol semantic (for example a producer may use HTTP/MQTT categories). |
| `status` | Optional edge/connection state. |
| `metadata` | Relationship-specific auxiliary information. |

A flow edge is a **logical/model relationship**. Its presence alone must not be described as proof that packets currently traverse that path.

## `LocalHealthSnapshot`

| Field | Meaning / purpose |
|---|---|
| `target_kind` | Required category of the resource being assessed. |
| `target_id` | Required identity of the resource being assessed. |
| `status` | Required health classification/value. |
| `reason` | Optional explanation of why the target has that status. |
| `source` | Provenance of the health evidence. |
| `metadata` | Additional structured health context. |

During conversion to D-Graph, each health item receives a generated report ID derived from node, target kind, target ID, and position.

## `LocalCapabilitySnapshot`

| Field | Meaning / purpose |
|---|---|
| `capability_id` | Required capability identity. |
| `capability_kind` | Required capability category/type. |
| `supported` | Explicit positive/negative capability support statement. |
| `source` | Provenance. |
| `metadata` | Auxiliary capability information. |

`capability_id` and `capability_kind` must be non-empty. `supported=false` is explicit negative evidence and is semantically different from the capability item being absent altogether.

## `LocalDGraphSnapshotList`

| Field | Purpose |
|---|---|
| `project_id` | Project scope used to retrieve/list the snapshots. |
| `count` | Number of returned latest node snapshots. |
| `snapshots` | Node snapshots, sorted by node ID and reported time by the API helper. |

## Identity and normalization rules

The POST API takes identity from:

```text
X-Datum-Project header -> project_id
URL :node_id           -> node_id
server UUID            -> snapshot_id
server UTC clock       -> reported_at_utc
normalizer             -> source = client
```

Producer-provided versions of those canonical fields are not trusted as final values.

This is a strong control-plane design choice: **request context establishes identity; payload describes the node-local graph contents**.

## Read-only safety invariants

Current validation rejects any local D-Graph snapshot with:

```text
safe_to_auto_execute = true
side_effects_performed = true
```

Therefore this contract currently means:

```text
"publish node-local graph/evidence for materialization"
```

not:

```text
"authorize dserver or the agent to automatically execute the graph"
```

That distinction must remain visible in API docs, dashboards, papers, and future automation phases.
