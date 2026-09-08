# Stage 86 — node-local secret resolver contract

Stage 86 defines the boundary between dserver governance and secret
materialization on the target node.

The dserver returns only opaque references, provider metadata, lookup keys,
and the intended local injection target. Secret values never transit the
control plane and are never included in the contract.

Endpoint:

`GET /api/v1/nodes/{node_id}/deployment-secrets-contract`

The contract remains `proposal_only` and
`execution_allowed=false) whenever references exist. A later SmartSentinel
runtime must resolve the reference locally, verify the result, inject it into
the container without logging it, and report only redacted status plus an
audit identifier.
