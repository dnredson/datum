# Stage 80 — Artifact Mapping Preflight

## Goal

Make the local SmartSentinel mapping explicit and compatible with the artifact selected by
the dserver before enabling any governed deployment.

This stage does not deploy a service. It establishes the preflight contract for the next
stage:

- the operational D-Graph selects a desired service;
- the dserver resolves that service to one registered artifact;
- the SmartSentinel receives the artifact-bound reconciliation context;
- the local mapping must match the artifact's trusted container name and image;
- WASM remains explicit-only and side-effect free;
- `--execute` remains blocked until the control plane authorizes execution.

## Artifact and local mapping

For the current E2E fixture, the desired MQTT service resolves as follows:

| Field | Expected value |
| --- | --- |
| D-Graph service | `mqtt` |
| Artifact | `fog-mosquitto` |
| Artifact version | `2` |
| Container name | `datum-fog-mosquitto` |
| Image | `eclipse-mosquitto:2` |
| Target node | `fog-01` |

`DATUM/config.linux` uses the same container name in the Docker collector,
dispatch mapping, and service implementation mapping.

## Expected reconciliation behavior

With the artifact context available and the local mapping aligned:

- dry-run (`execute_requested=false`) returns `execution_status=suggested`;
- the report carries the trusted container and image mapping;
- WASM executes only the deterministic decision function;
- no host mutation or side effect occurs.

If the local mapping does not match the artifact:

- the service is reported as `blocked_local_mapping`;
- no container operation is attempted;
- `host_mutation_performed=false`;
- `side_effects_performed=false`.

Even with `--execute`, the control-plane authorization gate remains mandatory. An
unauthorized execution request is reported as `blocked_control_plane`.

## Desktop — verify the artifact-bound context

From the repository root on the desktop:

`@powershell
$PROJECT_ID = "monitor-f49c8"
$DSERVER_URL = "http://192.168.10.101:8080"
$CONTEXT_PATH = Join-Path (Get-Location) "c80-fog-01-context.json"

curl.exe -fsS `
  -X GET `
  "$DSERVER_URL/api/v1/nodes/fog-01/operational-reconciliation-context" `
  -H "X-Datum-Project: $PROJECT_ID" `
  > $CONTEXT_PATH

Get-Content -Raw $CONTEXT_PATH |
  jq '{
    contract_kind,
    node_id,
    dgraph_revision,
    desired_service_count: (.desired_services | length),
    artifact_count: (.artifacts | length),
    artifacts: [
      .artifacts[] | {
        artifact_id,
        service_id,
        version,
        container_name,
        image
      }
    ],
    execution_policy,
    side_effects_performed
  }'
`@

Expected:

`@text
contract_kind = datum_artifact_reconciliation_context
node_id = fog-01
desired_service_count = 1
artifact_count = 1

artifact_id = fog-mosquitto
service_id = mqtt
version = 2
container_name = datum-fog-mosquitto
image = eclipse-mosquitto:2

execution_policy.mode = proposal_only
execution_policy.explicit_invocation_required = true
execution_policy.host_imports_allowed = false
execution_policy.side_effects_allowed = false
execution_policy.safe_to_auto_execute = false
side_effects_performed = false
`@

## Fog — run the preflight

On `fog-01`:

`@bash
cd ~/datum-i38

export PROJECT_ID="monitor-f49c8"
export DSERVER_URL="http://192.168.10.101:8080"

cargo run --quiet --locked `
  --manifest-path DATUM/Cargo.toml `
  --bin smartsentinel-operational-reconcile `
  -- `
  --config DATUM/config.linux `
  --dserver-url "$DSERVER_URL" `
  --project-id "$PROJECT_ID" `
  --node-id fog-01 `
  --out /tmp/c80-fog-01-preflight.json

jq '{
  bootstrap_state,
  node_id,
  dgraph_id,
  dgraph_revision,
  execute_requested,
  control_plane_authorized,
  local_container_execution_allowed,
  services: [
    .services[] | {
      service_id,
      artifact_id,
      artifact_version,
      trusted_container_name,
      trusted_image,
      wasm_status: .wasm.status,
      execution_status,
      host_mutation_performed
    }
  ],
  side_effects_performed,
  snapshot_published
}' /tmp/c80-fog-01-preflight.json
`@

Expected for the aligned mapping:

`@text
execute_requested = false
control_plane_authorized = false
local_container_execution_allowed = true

service_id = mqtt
artifact_id = fog-mosquitto
artifact_version = 2
trusted_container_name = datum-fog-mosquitto
trusted_image = eclipse-mosquitto:2
wasm_status = executed
execution_status = suggested
host_mutation_performed = false

side_effects_performed = false
snapshot_published = true
`@

The absence of a running container is intentional in this preflight. The next stage will
introduce an explicit authorization path and test the first governed deployment.

## Cloud — confirm the node remains unaffected

No reconciliation is requested for `cloud-01` in this fixture. Verify that the node
does not receive the fog service assignment:

`@bash
cd ~/datum-i38

export PROJECT_ID="monitor-f49c8"
export DSERVER_URL="http://192.168.10.101:8080"

curl -fsS `
  -X GET `
  "$DSERVER_URL/api/v1/nodes/cloud-01/operational-reconciliation-context" `
  -H "X-Datum-Project: $PROJECT_ID" |
jq '{
  contract_kind,
  node_id,
  dgraph_revision,
  desired_service_count: (.desired_services | length),
  artifact_count: (.artifacts | length),
  side_effects_performed
}'
`@

Expected:

`@text
node_id = cloud-01
desired_service_count = 0
artifact_count = 0
side_effects_performed = false
`@

## Acceptance criteria

- Artifact and local mapping use the same trusted container name and image.
- A valid artifact-bound context is accepted by SmartSentinel.
- A mapping mismatch is fail-closed as `blocked_local_mapping`.
- Dry-run produces a suggestion without host mutation.
- Cloud remains outside the fog service assignment.
- No WASM host imports or side effects are enabled.
