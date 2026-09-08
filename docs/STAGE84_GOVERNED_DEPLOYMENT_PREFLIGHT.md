# Stage 84 — governed Magistrala deployment preflight

Stage 84 adds a read-only deployment preflight for a node-scoped artifact
assignment.

It validates the same artifact reconciliation context consumed by
SmartSentinel, computes a deterministic dependency order, and reports
unresolved secrets before any Docker command is considered.

The endpoint never invokes Docker, never creates a network or volume, and
never changes the operational D-Graph. It returns
`execution_policy.execution_allowed=false` whenever a dependency is missing,
a dependency cycle exists, or an artifact declares secret references that are
not available to this preflight contract.

Endpoint:

`GET /api/v1/nodes/{node_id}/governed-deployment-preflight`

Required header:

`X-Datum-Project: <project>`

For the current cloud Magistrala graph (revision 6), the expected result is
six services, a dependency order beginning with the database services, and
execution blocked until secret/network/volume/runtime requirements are
represented by an approved execution contract.
