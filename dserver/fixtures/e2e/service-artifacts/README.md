# Stage 95 — Distributed E2E service-artifact catalog

This directory defines the declarative service artifacts for the planned distributed IoT
proof: a simulated LoRa device and virtual Semtech UDP traffic path on `local-01`,
ChirpStack Gateway Bridge on `mist-01`, ChirpStack on `fog-01`, and Magistrala plus
Smart Adapter on `cloud-01`.

Stage 95 registers metadata only. It does not clone repositories, build images, render
configuration, start containers, contact a node, or execute WASM. Deployment remains
blocked until the artifact-to-SmartSentinel execution path is implemented and explicitly
authorized in a later stage.

## Files

- `scenario.json`: node addresses, artifact membership, relations, expected data flow and secret references.
- One JSON file per software component: each file can be POSTed to the dserver artifact registry.
- Images from the official Magistrala and ChirpStack compose definitions are recorded in
  the artifacts. The desktop traffic images use the `local/` namespace and must be preloaded as trusted
  local images before deployment; the governed reconciler does not clone repositories,
  build images, pull images, or run Make targets.
- No credential values are stored. `secret_refs` and `bindings` identify values that a
  later governed deployment must resolve.

## Import contract

The dserver request header must select the project scope. The server normalizes the
artifact's control-plane project to that scope and persists it in the configured control
state database. Import order should follow `control_plane.dependencies`; duplicate IDs
return HTTP 409 and can be updated with PUT.

The intended next stages are:

1. validate this catalog and materialize the static operational D-Graph;
2. make dserver distribute only the selected artifact and its resolved bindings;
3. invoke WASM for a decision;
4. let SmartSentinel perform the explicitly authorized local deployment;
5. validate the real data path end to end.

Always expect `safe_to_auto_execute=false` and `side_effects_performed=false`
from the catalog and registry responses.


## Desktop validation

From the repository checkout on the desktop, set the dserver URL and project scope. The following
PowerShell list follows the dependency order and imports all 15 artifacts into the registry:

```powershell
$PROJECT_ID = "monitor-f49c8"
$DSERVER_URL = "http://192.168.10.101:8080"
$ARTIFACT_DIR = Join-Path (Get-Location) "dserver/fixtures/e2e/service-artifacts"

$IMPORT_ORDER = @(
  "magistrala-atom-db.json",
  "magistrala-bootstrap-db.json",
  "magistrala-atom.json",
  "magistrala-bootstrap.json",
  "magistrala-fluxmq-auth.json",
  "magistrala-fluxmq-node1.json",
  "fog-mosquitto.json",
  "chirpstack-postgres.json",
  "chirpstack-redis.json",
  "chirpstack.json",
  "chirpstack-gateway-bridge.json",
  "chirpstack-rest-api.json",
  "lora-packet-forwarder.json",
  "lora-device-simulator.json",
  "sma.json"
)

foreach ($name in $IMPORT_ORDER) {
  $path = Join-Path $ARTIFACT_DIR $name
  curl.exe -fsS `
    -X POST `
    "$DSERVER_URL/api/v1/service-artifacts" `
    -H "Content-Type: application/json" `
    -H "X-Datum-Project: $PROJECT_ID" `
    --data-binary "@$path" |
  jq '{artifact_id, service_id, project_id: .control_plane.project_id, target_nodes: .control_plane.target_nodes}'
}
```

Expected: 15 successful JSON responses, all with `project_id = monitor-f49c8`,
`side_effects_performed` absent or false, and no deployment output. If an artifact was
already imported, HTTP 409 is expected; use PUT only when intentionally replacing that
artifact.

Confirm the registry and scenario coverage:

```powershell
curl.exe -fsS `
  -X GET `
  "$DSERVER_URL/api/v1/service-artifacts" `
  -H "X-Datum-Project: $PROJECT_ID" |
jq '{
  project_id,
  count,
  artifact_ids: [.artifacts[].artifact_id]
}'

Get-Content -Raw (Join-Path $ARTIFACT_DIR "scenario.json") |
jq '{
  scenario_id,
  stage,
  node_count: (.nodes | length),
  artifact_ref_count: ([.nodes[].artifacts[]] | length),
  relation_count: (.relations | length),
  expected_flow,
  side_effects_performed
}'
```

Expected registry `count = 15), scenario `node_count = 3`,
`artifact_ref_count = 15), `relation_count = 3), and
`side_effects_performed = false`. The next stage will consume this catalog to materialize
the static operational D-Graph; SmartSentinel execution is intentionally not part of this
validation.

## Stage 95 topology

The desktop-local traffic generator and virtual packet forwarder send Semtech UDP to
`mist-01:1700`. The Gateway Bridge publishes to the fog broker at
`192.168.10.103:1883`; SMA on the cloud consumes that broker and publishes to Magistrala.
This catalog does not materialize or modify the D-Graph.
