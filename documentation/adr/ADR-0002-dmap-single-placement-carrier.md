# ADR-0002 — D-Map is the single authoritative placement carrier

- Status: Accepted (Stage 107 baseline)
- Decision refs: F6; "D-Map created through explicit acceptance of a D-Deploy proposal"

## Reference-model meaning

In the DATUM reference model, a **D-Map** represents the mapping / binding of the
application and the required platform elements onto the D-Continuum. The reference
model does **not** prescribe how a D-Map is produced (for example, it does not
require explicit operator acceptance). This ADR records a Stage 107 **governance
decision** layered on top of that meaning.

## Context

Placement information is currently spread across three carriers:

- `dserver/src/core/dgraph.rs` `DGraph.placements` (closed C59–I44 phase);
- `dserver/src/core/service_artifact_registry.rs` `control_plane.target_nodes` /
  `target_stages`;
- the node lists of the operational deployment document (`OperationalDGraph`).

The `/api/v1/dmap` endpoint stores an untyped per-project JSON blob
(`dserver/src/api/dmap.rs`). There is no authoritative, revisioned, validated
object that says which D-Serv runs where.

## Decision

1. **`datum.dmap/1` is the single authoritative source and carrier of placement.**
   Derived objects — most notably the per-node operational slice — may repeat or
   materialize placement information, but must never originate an independent
   placement decision. `datum.dmap/1` references exact revisions (and, from
   Stage 108, digests) of the D-Graph, the D-Continuum and the D-Forward
   specification, and it contains:
   - `dserv_placements` — D-Serv instance → element + `dnode_id`;
   - `dnode_runtime_placements` — `dnode_id` → element + host kind + hosting artifact
     (the only place a host kind appears, see ADR-0004);
   - `diot_placements` — D-IoT element placement;
   - `dcall_realizations` — how each D-Call is carried (for VS1, via a D-IoT
     pub/sub channel);
   - `external_port_bindings` — ingress/egress channel bindings for external ports.

2. **Stage 107 governance decision — not a requirement of the DATUM reference
   model:** a `datum.dmap/1` is materialized only after explicit acceptance of a
   D-Deploy proposal. The proposal may be deterministic or operator-driven for
   VS1; later it may come from an AI mechanism. The acceptance records the actor.
   AI never writes a D-Map directly (consistent with `AI_GNN_EXECUTION_SEMANTICS =
   "proposal_only_no_direct_execution"`).

3. Artifact `target_nodes` / `target_stages` are **demoted to eligibility
   constraints**, never placement. `DGraph.placements` stays frozen with its closed
   phase. `/api/v1/dmap` stays as a legacy endpoint and is not reused.

4. The D-Map validator (D-Deploy v0, Stage 111) rejects, among others: an unplaced
   or duplicated D-Serv; a stage-constraint violation; a `dserv_placement` with no
   matching `dnode_runtime_placement`; an ABI mismatch or unsupported host kind; an
   unrealized D-Call; a **cyclic derived deployment-dependency graph** (a D-Call
   cycle alone is not a rejection); a wildcard or malformed channel; an inactive
   input revision.

## Consequences

- One revisioned object answers "where does this run", which the slice derivation
  (Stage 112) and authorization depend on.
- Moving a D-Node between host kinds is a `dnode_runtime_placements[].host` change
  only; the D-Serv execution contract is unchanged (ADR-0004).
- VS1 fixtures: `dmap.sensor-pipeline.fog-only.json` (VS1a),
  `dmap.sensor-pipeline.fog-cloud.json` (VS1b),
  `dmap.sensor-pipeline.native-dnode.json` (host-invariance pair), plus negative
  fixtures under `negative/`.
