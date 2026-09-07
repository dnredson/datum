# C68 — Explicit multi-node D-Graph topology and active complete acquisition

## Status

**C68 — LIVE PASS**

Implementation and laboratory validation completed on `agent/o2-c68-finalization`.

This step is a read-only extension of the existing active analysis endpoint. It does not alter the accepted live Operational D-Graph, publish a new revision, create reconciliation authorizations, execute local reconciliation, or invoke an external AI/GNN.

The finalization hardens the control-plane boundary: persisted operational state is recovered read-only and fail-closed before the HTTP listener starts, and the complete acquisition uses one captured graph revision to materialize all compatible slices.

## Goal

Expose the accepted Operational D-Graph as a complete multi-node object suitable for later AI/GNN consumption while preserving compatibility with the existing per-node Operational D-Graph slices.

## API

Existing endpoint:

```text
POST /api/v1/analysis-context/active
X-Datum-Project: <project-id>
```

Formats:

- no `format` or `format=analysis`: existing analysis schema is preserved;
- `format=ai`: C67 AI export is preserved;
- `format=dgraph`: C68 complete Operational D-Graph acquisition.

The C68 response has schema version `0.1.0`, acquisition mode `active_complete_operational_dgraph`, and consistency model `single_accepted_operational_dgraph_revision_with_node_scoped_compatible_slices`.

## C68 response guarantees

`format=dgraph` returns:

- the complete accepted `operational_dgraph` without rewriting it;
- an explicit `topology.nodes` list containing every graph node;
- explicit `topology.relations` when `operational_dgraph.metadata.relations` is present;
- all non-identity relation attributes preserved under `attributes`;
- one compatible node-scoped Operational D-Graph slice per graph node;
- `slice_compatibility.compatible=true` only after identity/revision/service validation succeeds;
- `side_effects_performed=false`.

A graph without `metadata.relations` remains valid and acquirable. In that legacy-compatible case, topology nodes are still explicit and relations are empty.

## Consistency and validation

The acquisition validates that every returned slice:

1. belongs to the same project;
2. belongs to the same D-Graph id;
3. belongs to the exact accepted D-Graph revision;
4. uses the same Operational D-Graph schema version;
5. identifies one of the graph nodes;
6. preserves that node's `desired_services` exactly.

The acquisition rejects duplicate/missing slices and explicit relations that reference unknown nodes.

## Finalization hardening

The final branch adds the following guarantees to the initial C68 implementation:

- the accepted graph and its authorization view are captured under one control-state transaction;
- all node slices are materialized from that captured graph, preventing a concurrent graph publication from mixing revisions;
- configured SQLite state is read without database creation, migration, legacy fallback, or publication;
- missing, corrupt, schema-incompatible, empty, or identity-incompatible persisted state fails closed unless an explicit unpinned empty bootstrap is configured;
- the previously existing `action-plan/materialized` and `installation-plan/latest` API routes remain intact;
- the C67 AI export compares RFC3339 values by instant and preserves equivalent representations such as `Z` and `+00:00`.

## Architecture boundary

The architecture remains:

- DATUM is the concept/model;
- dserver owns and governs the Operational D-Graph;
- SmartSentinel observes and may execute only locally authorized reconciliation;
- external AI/GNN consumers may read context and later propose decisions, but cannot execute host actions directly.

C68 is therefore acquisition-only. It introduces no autonomous mutation path.

## Unit coverage

The C68 core contains focused tests for:

- explicit two-node topology with preserved relation attributes;
- complete acquisition preserving the original graph and compatible slices;
- legacy graphs without explicit relations;
- rejection of relations that reference unknown nodes;
- rejection of missing or mixed-revision slices.

## Laboratory validation target

Current accepted laboratory baseline before C68 live validation:

- project: `monitor-f49c8`;
- accepted D-Graph: `dgraph:o2-mqtt-role-1`;
- accepted revision: `4`;
- current graph contains only `fog-01`;
- MQTT desired state is `running`;
- the C67 observation of MQTT drift (`observed_service_status=unavailable`, `binding=false`) is evidence, not an exporter failure.

C68 live validation must first prove read-only compatibility against revision 4. A later explicit multi-node graph revision, if intentionally published for the lab, must be a separately authorized laboratory action and is not performed by this implementation.

## PASS criteria

C68 passes implementation validation when:

- the focused C68 unit tests pass;
- the full dserver test suite passes;
- `cargo build --manifest-path dserver/Cargo.toml` passes;
- `format=analysis` retains its prior response contract;
- `format=ai` retains the C67 response contract;
- `format=dgraph` returns the exact accepted graph revision and compatible slice count equal to node count;
- `side_effects_performed=false`;
- no live D-Graph revision or host state changes merely from acquisition.

## Final laboratory validation

The final validation used the real `fog-01` observation:

- cycle id: `9d19f022-99f0-40af-bfb0-1593dc3aaaca`;
- observed at: `2026-09-07T18:17:04.965645695Z`;
- entities: 8;
- edges: 5;
- events: 2;
- actions: 0;
- snapshot ingestion: `accepted=true`.

The active AI export returned:

```text
schema_version=0.1.0
acquisition_mode=active
complete=true
expected_node_count=1
valid_snapshot_count=1
fresh_node_count=1
stale_node_count=0
missing_node_count=0
invalid_snapshot_count=0
usable_node_count=1
side_effects_performed=false
```

The exported `fog-01` cycle id exactly matched the ingested cycle id:

```text
INCOMING=9d19f022-99f0-40af-bfb0-1593dc3aaaca
EXPORTED=9d19f022-99f0-40af-bfb0-1593dc3aaaca
C68_EXACT_CYCLE_PASS
```

The compatibility checks also passed:

- `format=dgraph`: accepted revision 4, one topology node, one compatible slice, no side effects;
- legacy active analysis: schema `0.2.0`, one node, no side effects;
- active AI context: fresh and complete, no side effects.

Build validation completed successfully:

```text
Finished dev profile
DServer warnings: 14
control-state-inspect warnings: 8
errors: 0
```

The warnings are non-blocking compiler warnings and do not invalidate the C68 result.

## FAIL criteria

C68 fails if acquisition mutates the accepted graph, changes authorization state, triggers reconciliation, mixes graph/slice revisions, drops desired services, loses explicit relation attributes, or breaks the existing analysis/AI formats.
