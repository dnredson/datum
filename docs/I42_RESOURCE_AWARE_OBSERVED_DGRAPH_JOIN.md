# I42 — Resource-Aware Observed D-Graph Join

- checkpoint: `I42/148`
- predecessor: `I41/147`
- status: `implementation_ready_for_validation`
- branch: `agent/i42-resource-aware-observed-join`

## Goal

I42 correlates the controlled deployment/observed D-Graph with existing Sentinel/local D-Graph snapshot evidence about node resources and capabilities.

The join is evidence-only. It does not replace placements, trigger deployment attempts, retry work, or authorize governed execution.

## Existing observation channel

I42 reuses the existing endpoint:

```text
POST /api/v1/nodes/:node_id/dgraph-snapshot
X-Datum-Project: <project_id>
```

and the existing `LocalDGraphSnapshot` model.

No second telemetry transport is introduced.

## Resource observation metadata

A local snapshot participates in the I42 resource join when it carries:

```json
{
  "metadata": {
    "resource_observation": {
      "observed_at_utc": "RFC3339 timestamp",
      "cpu": {
        "logical_cores": 8,
        "usage_percent": 25.0
      },
      "memory": {
        "total_bytes": 17179869184,
        "available_bytes": 8589934592
      },
      "process_count": 120,
      "container_count": 2
    }
  }
}
```

Supported capability identifiers are taken from the snapshot's existing `capabilities[]` entries where `supported=true`.

The join also records counts of snapshot runtimes, modules, and health entries.

## Validation

The I42 parser requires:

- `observed_at_utc` valid RFC3339;
- `cpu.logical_cores > 0`;
- `0 <= cpu.usage_percent <= 100`;
- `memory.total_bytes > 0`;
- `memory.available_bytes <= memory.total_bytes`;
- unsigned `process_count` and `container_count`.

A snapshot without `metadata.resource_observation` remains a valid local snapshot and is simply ignored by this resource join.

A malformed resource observation fails the controlled observed materialization closed.

## Freshness

I42 uses a fixed evidence freshness threshold:

```text
RESOURCE_OBSERVATION_FRESHNESS_SECONDS = 300
```

Freshness is calculated at observed-D-Graph materialization time from the resource observation's `observed_at_utc`:

```text
age <= 300 s -> fresh
age > 300 s  -> stale
```

The server-assigned local snapshot `reported_at_utc` is retained separately from source observation time.

## Join identity

Resource evidence is joined by:

```text
active manifest project_id
+
D-Graph node_id
```

Snapshots from other projects are ignored. A resource snapshot for a node not present in the active D-Graph is not injected into the graph and is listed in `unmatched_node_ids`.

## Materialized evidence

Each joined `DGraphNode.metadata.resource_observation` carries:

```text
node_id
snapshot_id
observed_at_utc
reported_at_utc
age_seconds
freshness_threshold_seconds
freshness
cpu
memory
process_count
container_count
supported_capabilities
runtime_count
module_count
health_count
```

The controlled observed state and graph metadata also receive:

```text
resource_observations[]
resource_join.observation_count
resource_join.fresh_count
resource_join.stale_count
resource_join.missing_node_ids[]
resource_join.unmatched_node_ids[]
resource_join.freshness_threshold_seconds
resource_join.automatic_placement_performed=false
resource_join.deployment_side_effects_performed=false
resource_join.governed_execution_performed=false
```

## Placement boundary

The existing D-Graph placements remain authoritative evidence from the input graph during I42.

I42 does not compute a new placement from CPU or memory state.

```text
resource evidence -> observed D-Graph / downstream algorithms
resource evidence -X-> automatic placement
resource evidence -X-> automatic execution
```

I43 can consume freshness/capability/resource presence as readiness prerequisites without changing this boundary.

## Focused regression tests

I42 adds six tests on top of the validated I41 dserver baseline of 508 tests:

```text
fresh resource observation parsing
stale resource observation classification
invalid CPU percentage rejection
invalid memory range rejection
node metadata + summary materialization
unmatched node reporting without graph mutation
```

Expected minimum full-suite result:

```text
514 passed; 0 failed
```

## Planned live gate

Use `scripts/fixtures/i42-resource-three-node-dgraph.json`, which keeps the validated I40 dependency stages while using a dedicated `i42-live-project` namespace.

1. publish the I42 D-Graph;
2. publish one local D-Graph snapshot for each of `fog-01`, `local-01`, and `cloud-01`;
3. set fog resource `observed_at_utc` ten minutes in the past;
4. set local and cloud observations to current UTC time;
5. GET the canonical controlled observed D-Graph;
6. require `observation_count=3`, `fresh_count=2`, `stale_count=1`;
7. require no missing or unmatched node IDs;
8. require each D-Graph node to carry CPU, memory, process/container, capability, and freshness evidence;
9. require fog=`stale`, local/cloud=`fresh`;
10. require existing dependency stages and placements to remain unchanged;
11. require `validation_errors=0`;
12. require `automatic_placement_performed=false`, `deployment_side_effects_performed=false`, and `governed_execution_performed=false`.

No artifact server or WASM execution is required for the I42 gate.

## Non-claims

I42 does not provide:

- automatic placement optimization;
- resource reservation;
- CPU/RAM enforcement;
- automatic deployment/retry trigger;
- persistent local snapshot storage beyond the existing Firebase-backed path;
- automatic governed execution;
- readiness authorization (I43).

Checkpoint remains `147/150` until regression and the three-node resource join gate pass.
