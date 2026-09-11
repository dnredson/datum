# ADR-0009 — `application_id` distinct from `project_id`

- Status: Accepted (Stage 107 baseline)
- Decision refs: Round 2 "Identity"

## Context

`project_id` currently scopes the operational D-Graph, service artifacts, resource
bindings and evidence together
(`dserver/src/core/operational_dgraph.rs`, `service_artifact_registry.rs`). It
therefore plays installation, application and tenant at once. The same application
cannot be described once and then deployed to two installations, or compared under
two D-Maps.

## Decision

1. The canonical application D-Graph carries an **`application_id`** that is
   distinct from `project_id`.
2. `project_id` keeps its current meaning exactly: installation / tenant scope,
   authorization scope, and control-state persistence key (Stage 106). No existing
   record changes.
3. A project references one or more application D-Graph revisions. The D-Graph
   itself never references a project, a node, an address, a middleware product, a
   container image or a stage (except abstract stage *constraints*).

## Consequences

- `datum.dgraph/1` fixtures carry both `application_id` (`app:sensor-pipeline`) and
  `dgraph_id` (`dgraph:sensor-pipeline`); `datum.dmap/1` and
  `datum.operational-slice/2` carry `project_id`.
- Reusing one D-Graph across installations, and comparing two D-Maps of one
  application (VS1b), become expressible.
