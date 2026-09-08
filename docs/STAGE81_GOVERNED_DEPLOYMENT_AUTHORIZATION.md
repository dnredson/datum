# Stage 81 — Governed Deployment Authorization

This stage validates the first real deployment governed by the dserver and SmartSentinel.

The operation requires two independent approvals: an authorization issued by the dserver and the explicit SmartSentinel flag --execute. The authorization is bound to project monitor-f49c8, D-Graph dgraph:o2-mqtt-role-1, revision 5, node fog-01, and the active Resource Registry binding.

Expected deployment:

- service: mqtt
- artifact: fog-mosquitto
- version: 2
- container: datum-fog-mosquitto
- image: eclipse-mosquitto:2
- MQTT port: 1883

Desktop authorization request:

    $AUTH = [ordered]@{
        dgraph_id = "dgraph:o2-mqtt-role-1"
        dgraph_revision = 5
        authorized_by = "operator-stage81"
        allow_host_mutation = $true
        persistence_scope = "process"
        valid_for_seconds = 300
    }
    $AUTH | ConvertTo-Json | Set-Content c81-authorization.json -Encoding utf8
    curl.exe -fsS -X POST `
      "$DSERVER_URL/api/v1/nodes/fog-01/operational-reconciliation/authorize" `
      -H "Content-Type: application/json" `
      -H "X-Datum-Project: $PROJECT_ID" `
      --data-binary "@c81-authorization.json"

The response must contain a non-empty authorization_id and binding_id, with allow_host_mutation=true, node_id=fog-01, and dgraph_revision=5.

Fog execution:

    cargo run --quiet --locked \
      --manifest-path DATUM/Cargo.toml \
      --bin smartsentinel-operational-reconcile \
      -- \
      --config DATUM/config.linux \
      --dserver-url "$DSERVER_URL" \
      --project-id "$PROJECT_ID" \
      --node-id fog-01 \
      --execute \
      --out /tmp/c81-fog-01-deployment.json

Expected successful report:

    execute_requested=true
    control_plane_authorized=true
    execution_status=deployed (or already_running)
    trusted_container_name=datum-fog-mosquitto
    trusted_image=eclipse-mosquitto:2
    observed_after_running=true
    snapshot_published=true
    side_effects_performed=true only when a container was actually started or created

Verify on fog:

    docker ps --filter name=datum-fog-mosquitto
    timeout 5 bash -c '</dev/tcp/127.0.0.1/1883' && echo C81_MQTT_HEALTH_PASS

Without authorization, with a wrong revision/project/node/artifact, or without --execute, SmartSentinel must remain fail-closed and report no host mutation. Cloud-01 remains without desired services.

Automatic destructive rollback is not enabled in this stage; failures must be reported and no success may be claimed. The next stage can add explicit rollback lifecycle handling after this first deployment proof.
