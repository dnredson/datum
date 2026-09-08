# Stage 82 — Post-Deployment Verification and Governed Rollback

This stage verifies the result of the first governed deployment and defines a fail-closed rollback procedure. Verification is read-only. Rollback is never automatic and requires a separate explicit authorization.

## Verification on fog-01

    docker inspect datum-mosquitto --format 'running={{.State.Running}} status={{.State.Status}}'
    docker port datum-mosquitto
    timeout 5 bash -c '</dev/tcp/127.0.0.1/1883' && echo C82_MQTT_HEALTH_PASS || echo C82_MQTT_HEALTH_FAIL

Expected: running=true, port 1883 published, and C82_MQTT_HEALTH_PASS.

The SmartSentinel reconciliation report must preserve the deployment identity: artifact mosquitto, service mqtt, image eclipse-mosquitto:2, container datum-mosquitto, and the active D-Graph revision.

## Verification on desktop

    curl.exe -fsS -X GET "$DSERVER_URL/api/v1/observed-dgraph" -H "X-Datum-Project: $PROJECT_ID" | jq '{revision, nodes, side_effects_performed}'

The post-deployment snapshot must not claim success when the container is stopped or the MQTT port is unreachable.

## Rollback policy

A failed health check produces evidence for rollback but does not remove the container. A rollback request must be separately authorized for the same project, node, D-Graph identity, revision, and active Resource Registry binding. The rollback operation must be explicit, bounded, auditable, and report host_mutation_performed accurately.

Until the rollback endpoint and lifecycle command are implemented, rollback is manual and operator-controlled. No automatic docker rm, image replacement, or topology change is permitted.

## Acceptance criteria

- Verification is read-only and independent from deployment.
- MQTT health is checked through the published host port.
- A stopped or unhealthy container is reported as degraded/failed.
- No rollback occurs without a separate authorization.
- No automatic destructive operation is performed.
- Evidence preserves project, node, artifact, revision, and side-effect flags.
