# Stage 81 — Governed Deployment Authorization

This stage validates the first real deployment governed by the dserver and SmartSentinel.

The operation requires two independent approvals: an authorization issued by the dserver and the explicit SmartSentinel flag --execute. The authorization is bound to project monitor-f49c8, D-Graph dgraph:o2-mqtt-role-1, revision 5, node fog-01, and the active Resource Registry binding.

The MQTT artifact is stage-neutral: artifact_id=mosquitto and service_id=mqtt. The deployment stage is selected by the operational D-Graph and the local node mapping, not encoded in the artifact identity.

Expected deployment:

- service: mqtt
- artifact: mosquitto
- version: 2
- container: datum-mosquitto
- image: eclipse-mosquitto:2
- MQTT port: 1883
- allowed target nodes: fog-01 and cloud-01

For desktop authorization, create a process-scoped request without valid_for_seconds:

    $AUTH = [ordered]@{
        dgraph_id = "dgraph:o2-mqtt-role-1"
        dgraph_revision = 5
        authorized_by = "operator-stage81"
        allow_host_mutation = $true
        persistence_scope = "process"
    }

POST it to /api/v1/nodes/fog-01/operational-reconciliation/authorize with X-Datum-Project. The response must contain a non-empty authorization_id and binding_id, allow_host_mutation=true, node_id=fog-01, and dgraph_revision=5.

On fog-01, run smartsentinel-operational-reconcile with --execute. Expected successful report:

    execute_requested=true
    control_plane_authorized=true
    local_container_execution_allowed=true
    artifact_id=mosquitto
    trusted_container_name=datum-mosquitto
    trusted_image=eclipse-mosquitto:2
    execution_status=deployed (or already_running)
    observed_after_running=true
    snapshot_published=true
    side_effects_performed=true only when a container was actually started or created

Verify the MQTT health endpoint on fog-01:

    docker ps --filter name=datum-mosquitto
    timeout 5 bash -c '</dev/tcp/127.0.0.1/1883' && echo C81_MQTT_HEALTH_PASS

On cloud-01, the same artifact may be selected by a future D-Graph assignment, but the current graph has no desired service there. Therefore its reconciliation context must contain zero desired services and zero artifacts.

Without authorization, with a wrong revision/project/node/artifact, or without --execute, SmartSentinel must remain fail-closed and report no host mutation. Automatic destructive rollback is not enabled in this stage; failures must be reported and no success may be claimed.
