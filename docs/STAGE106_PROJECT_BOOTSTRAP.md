# Stage 106 — Project-scoped operational D-Graph bootstrap

Stage 106 changes DServer startup semantics so one DServer can manage multiple
project-scoped operational D-Graphs.

## Startup behavior

DServer may start with an empty project registry. This is a valid first-use
state, not a replacement for an existing project graph.

For an existing project, recovery remains fail-closed when the configured
expected identity is missing or the persisted state is invalid. The server
does not silently import a JSON file, create a replacement graph, or authorize
host mutation.

## Explicit project creation

Create a project explicitly:

```http
POST /api/v1/projects
Content-Type: application/json

{
  "project_id": "monitor-f49c8",
  "dgraph_id": "dgraph:o2-mqtt-role-1"
}
```

If `dgraph_id` is omitted, DServer uses `dgraph:<project_id>`.

The response is an inert bootstrap graph:

- revision `0`;
- no nodes;
- no desired services;
- no host-mutation authorization;
- metadata `bootstrap_mode=empty`.

A later operational graph must be published with a higher revision through the
existing governed publication path. Reconciliation authorization remains a
separate explicit operation.

List registered projects:

```http
GET /api/v1/projects
```

## Existing project preservation

The current project `monitor-f49c8` and graph
`dgraph:o2-mqtt-role-1` remain unchanged. Stage 106 changes only the handling
of a new DServer with no projects; it does not reset or reconstruct accepted
operational state.

The persisted state is still keyed by project identity in the operational
state payload. Future work can extend the registry to support multiple named
graphs per project without changing the fail-closed deployment boundary.
