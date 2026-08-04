# C45/130 — Enriched Planner Advisory Proposals

## Status

C45 is complete. The D-Graph planner dry-run now exposes structured advisory
proposals while preserving existing response fields and the unchanged summary
endpoint contract. Project progress is 116/130.

## Additive contract

The full GET /api/v1/dgraph/planner/dry-run response gains:

~~~text
planner.advisory_proposals: []
planner.safe_to_auto_execute: false
~~~

Each proposal has a deterministic identifier, category, summary, structured reason,
graph-backed evidence, related flow/node/candidate IDs, limitations,
advisory_only=true, and executable=false.

For example, a real degraded flow can yield:

~~~json
{
  "proposal_id": "advisory:degraded-flow:flow-mqtt-adapter-to-magistrala-core",
  "category": "degraded_flow_observation",
  "reason": {"code": "degraded_flow_observed"},
  "related_flow_ids": ["flow-mqtt-adapter-to-magistrala-core"],
  "candidate_node_ids": ["cloud-01", "fog-01"],
  "advisory_only": true,
  "executable": false
}
~~~

The example reflects only entities present in the materialized graph. Candidate
nodes are sorted observations, not selected destinations. A healthy baseline has
an empty proposal collection. Missing modules, unknown health, or absent candidates
are limitations; they are never converted into claimed availability, compatibility,
or health facts. Proposal summaries and reasons preserve the flow status observed in
the graph (for example, `warning` remains `warning` rather than being stated as
`degraded`).

## Determinism and safety

- Proposals, evidence, related IDs, candidate IDs, and limitations are sorted and
  deduplicated.
- Invalid graphs continue through the existing blocked response and emit no inferred
  corrective proposal.
- The planner does not mutate the graph while constructing proposals.
- Proposals are not actions, placement decisions, commands, or executable plans.
- advisory_only=true, executable=false, safe_to_auto_execute=false, and
  side_effects_performed=false are explicit.
- No dispatch, recovery, deployment, installation, Docker, WASM, migration,
  placement, or reconfiguration is invoked.

## Validation

The dserver suite validates healthy baselines, degraded flows, multiple flows and
candidates, incomplete data, deterministic IDs/order, graph-reference integrity,
additive serialization, no graph mutation, and both safety flags. DATUM parsing
remains compatible with the additive fields.
