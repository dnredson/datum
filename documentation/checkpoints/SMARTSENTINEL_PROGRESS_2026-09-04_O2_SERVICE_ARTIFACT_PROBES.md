# SmartSentinel O2 checkpoint — service artifact operational probes

Date: 2026-09-04
Branch: `agent/o2-service-artifact-probes`
Baseline main before step 52: `ef51332d602387dc10b809886e2ec90035ffbed5`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.

Step 52 is closed after an A/B live proof using the same trusted MQTT service artifact.

## Step 52 — service artifact operational probes — LIVE PASS

Step 52 formalizes the distinction:

`execution != health != availability/consumability`

A process or container being reported as running is not sufficient evidence that the application is internally healthy or consumable by a client.

### Service artifact contract

A trusted service artifact describes:

- service identity;
- runtime kind;
- container deployment identity and desired image;
- declared container/host port exposure;
- execution probe;
- optional health probe;
- optional availability/consumption probe;
- probe vantage point.

Deployment declarations are observational in step 52. Step 52 does not mutate a running deployment to repair port/configuration drift.

### Probe plugins implemented

Execution:

- `docker_container_running`;
- `process_running`;
- `systemd_active`.

Health / availability:

- `docker_health`;
- `tcp_connect`;
- `http`;
- `mqtt_connect`.

The MQTT plugin performs an actual MQTT 3.1.1 CONNECT and requires a successful CONNACK return code, instead of equating an open TCP port with a functioning broker.

No arbitrary shell probe plugin is accepted.

### Assessment semantics

Each dimension returns one of:

- `passed`;
- `failed`;
- `not_verifiable`;
- `error`.

If a health or availability probe is not configured, SmartSentinel reports `not_verifiable`; it does not infer success from execution state.

Derived operational states include:

- `not_running`;
- `unhealthy`;
- `unavailable`;
- `available`;
- `healthy_availability_not_verifiable`;
- `running_not_fully_verifiable`;
- `execution_not_verifiable`;
- `unknown`.

### MQTT fixture

`scripts/fixtures/o2-service-mqtt-artifact.json` declares:

- service `mqtt`;
- container `datum-mqtt`;
- image `eclipse-mosquitto:2`;
- desired exposure `1883:1883/tcp`;
- execution probe `docker_container_running` from `local_runtime`;
- health probe `docker_health` from `local_runtime`;
- availability probe `mqtt_connect` to `127.0.0.1:1883` from `local_host`.

### Live proof A — running but unavailable — PASS

The MQTT container was running without a host port mapping. Running the unchanged service artifact produced:

- `artifact_id=service:mqtt:mosquitto-2`;
- `service_id=mqtt`;
- `operational_state=unavailable`;
- `assessment_basis=execution_health_availability`;
- execution `docker_container_running` -> `passed`;
- health `docker_health` -> `not_verifiable` because the image exposes no Docker HEALTHCHECK state;
- availability `mqtt_connect` -> `failed` because a host-local MQTT client could not connect to `127.0.0.1:1883`.

This proves that `container running = true` does not imply `service available = true`.

### Live proof B — same artifact, consumable deployment — PASS

SmartSentinel continuous reconciliation was stopped while dserver remained running. The same Mosquitto image was recreated as `datum-mqtt` with the artifact-declared exposure:

`-p 1883:1883`

Docker then reported:

- `0.0.0.0:1883->1883/tcp`;
- `[::]:1883->1883/tcp`.

The same unchanged service artifact was evaluated again and produced:

- `artifact_id=service:mqtt:mosquitto-2`;
- `service_id=mqtt`;
- `operational_state=available`;
- `assessment_basis=execution_health_availability`;
- execution `docker_container_running` -> `passed`, with `datum-mqtt` running;
- health `docker_health` -> `not_verifiable`, unchanged from proof A;
- availability `mqtt_connect` -> `passed` with a successful MQTT CONNECT/CONNACK against `127.0.0.1:1883`.

The only material operational difference between proof A and proof B was consumer-visible port exposure. Execution remained satisfied and Docker health remained unverifiable.

Therefore the live A/B transition was:

`execution=passed, health=not_verifiable, availability=failed, state=unavailable`

-> publish the declared `1883:1883/tcp` exposure ->

`execution=passed, health=not_verifiable, availability=passed, state=available`.

This is a direct live proof that runtime execution, internal health evidence, and actual service consumability are distinct dimensions.

### Step 52 conclusion

Step 52 is LIVE PASS.

SmartSentinel can now produce typed evidence that a service is executing but unavailable, can preserve uncertainty through `not_verifiable`, and can verify MQTT consumability through a protocol-level CONNECT/CONNACK rather than through runtime status or a raw TCP-open assumption.

The trusted service artifact also carries the desired deployment declaration that explains the A/B difference (`1883:1883/tcp`), providing the basis for explicit deployment-drift detection in the next step.

### New CLI

`smartsentinel-service-probe` loads a trusted service artifact and writes a typed `ServiceOperationalAssessment` JSON report.

## Next step — 53/100

Integrate service operational assessments into the continuous SmartSentinel operational loop and operational evidence stream, and explicitly distinguish deployment drift from application execution/health/availability failures.

Step 53 should remain observation-first: detect and publish deployment drift, but do not yet introduce automatic port/configuration repair as a new host-mutation class.

## Architecture statement

`SmartSentinel observes execution -> probes health -> probes consumability -> publishes explicit evidence quality; D-Graph remains desired-state context, dserver remains governance/control plane, and external AI remains responsible for placement/reasoning decisions.`

DATUM remains the conceptual framework; it does not execute host actions.
