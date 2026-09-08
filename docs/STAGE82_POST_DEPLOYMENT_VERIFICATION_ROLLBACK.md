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

The SmartSentinel report also exposes observed_before_running, observed_after_running,
execution_status, host_mutation_performed, and side_effects_performed. A successful
deployment is therefore distinguishable from a read-only suggestion, a control-plane block,
a local mapping block, and a failed Docker operation.

## Rollback policy

A failed health check produces evidence for rollback but does not remove the container. A rollback request must be separately authorized for the same project, node, D-Graph identity, revision, and active Resource Registry binding. The rollback operation must be explicit, bounded, auditable, and report host_mutation_performed accurately.

The rollback authorization endpoint is:

    POST /api/v1/nodes/:node_id/operational-reconciliation/rollback-authorize

Its JSON request is bound to the active project, D-Graph id/revision, node, and Resource
Registry binding. It must include operation: "rollback" and allow_host_mutation: true.
persistence_scope: "process" is the short-lived default; durable authorization additionally
requires valid_for_seconds.

After that authorization, rollback remains an explicit local command:

    cargo run --quiet --locked \\
      --manifest-path DATUM/Cargo.toml \\
      --bin smartsentinel-operational-reconcile \\
      -- \\
      --config DATUM/config.linux \\
      --dserver-url "$DSERVER_URL" \\
      --project-id "$PROJECT_ID" \\
      --node-id fog-01 \\
      --execute \\
      --rollback \\
      --out /tmp/c82-fog-01-rollback.json

The command removes only the trusted mapped container, reports rolled_back, and publishes
a post-rollback snapshot. Missing authorization, stale revision, wrong operation, absent
binding, missing local mapping, or disabled local policy fail closed. No automatic
rollback, image replacement, or topology change is permitted.

## Acceptance criteria

- Verification is read-only and independent from deployment.
- MQTT health is checked through the published host port.
- A stopped or unhealthy container is reported as degraded/failed.
- No rollback occurs without a separate authorization.
- No automatic destructive operation is performed.
- Evidence preserves project, node, artifact, revision, and side-effect flags.
