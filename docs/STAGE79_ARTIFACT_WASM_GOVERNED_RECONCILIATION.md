# Stage 79 — artifact-bound WASM reconciliation

Stage 79 connects the registered service-artifact catalog to the existing
SmartSentinel operational reconciliation corridor.

## New dserver contract

For a project-scoped node, dserver now exposes:

```text
GET /api/v1/nodes/{node_id}/operational-reconciliation-context
X-Datum-Project: <project>
```

The response is `datum_artifact_reconciliation_context` and contains:

- the accepted D-Graph identity and revision;
- the node-scoped desired services;
- the resolved artifact ID, version, container name and image for each service;
- non-secret bindings and secret reference names;
- the artifact WASM policy;
- the active reconciliation authorization;
- an execution policy fixed to `proposal_only`.

A desired service without exactly one artifact targeted at that node is rejected.
This prevents the agent from acting on a D-Graph service that has no registered
deployment definition.

## SmartSentinel behavior

`smartsentinel-operational-reconcile` now fetches this context instead of using
only the raw D-Graph slice. Before inspecting or mutating Docker it verifies:

1. project, node, D-Graph revision and reconciler identity;
2. artifact-to-service cardinality;
3. artifact target node, runtime, image and container identity;
4. explicit-only WASM policy and `side_effects_performed=false`;
5. exact agreement between the dserver artifact and the local trusted mapping.

A local mapping whose container name or image differs from the registered artifact
is not trusted. The result is `blocked_local_mapping` and no host mutation.

The existing `--execute` gate, D-Graph authorization, local dispatch policy and
Docker fail-closed behavior remain required. The default command still uses
`--execute=false`.

## Validation

After building dserver and DATUM, with the accepted operational D-Graph and
the Stage 78 artifacts loaded:

```bash
curl -fsS \
  -X GET \
  "$DSERVER_URL/api/v1/nodes/fog-01/operational-reconciliation-context" \
  -H "X-Datum-Project: $PROJECT_ID" |
jq '{
  contract_kind,
  project_id,
  dgraph_id,
  dgraph_revision,
  node_id,
  desired_service_count: (.desired_services | length),
  artifact_count: (.artifacts | length),
  artifacts: [.artifacts[] | {
    artifact_id,
    service_id,
    version,
    container_name,
    image,
    wasm_decision,
    secret_ref_count: (.secret_refs | length)
  }],
  execution_policy,
  side_effects_performed
}'
```

Expected: `contract_kind = datum_artifact_reconciliation_context`,
one artifact per desired service, `execution_policy.mode = proposal_only`,
all WASM permissions false, and `side_effects_performed = false`.

Run the SmartSentinel corridor in dry-run:

```bash
cargo run --quiet --locked \
  --manifest-path DATUM/Cargo.toml \
  --bin smartsentinel-operational-reconcile \
  -- \
  --config DATUM/config.linux \
  --dserver-url "$DSERVER_URL" \
  --project-id "$PROJECT_ID" \
  --node-id fog-01 \
  --out /tmp/c79-fog-01-reconciliation.json
```

Expected: the report contains `artifact_id` and `artifact_version` for
each reconciled service, `execute_requested=false`,
`host_mutation_performed=false), and
`side_effects_performed=false). No Docker start or deployment is permitted
by this validation.
