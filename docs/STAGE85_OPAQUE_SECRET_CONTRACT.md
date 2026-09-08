# Stage 85 — opaque secret-reference contract

Stage 85 introduces a read-only secret preflight for node-scoped artifacts.

The contract carries only opaque reference names such as
`magistrala_atom_jwt_secret`. It never accepts, stores, resolves, or exposes
secret values. Each reference is reported as `unresolved` until a separately
approved runtime resolver exists.

Endpoint:

`GET /api/v1/nodes/{node_id}/deployment-secrets-preflight`

The endpoint is fail-closed for execution: when references exist,
`execution_policy.execution_allowed=false`, while
`side_effects_performed=false` is always preserved.

This is intentionally a contract boundary, not a secret manager. The next
runtime stage must define the provider, injection mechanism, audit trail, and
rollback behavior before any Magistrala container receives secret material.
