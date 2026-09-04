# SmartSentinel O2 checkpoint — service artifact operational probes

Date: 2026-09-04
Branch: `agent/o2-service-artifact-probes`
Baseline main: `ef51332d602387dc10b809886e2ec90035ffbed5`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — IMPLEMENTED, pending compile/live validation.

The validated 51/100 baseline is now on `main`. Step 52 is isolated in a new development branch.

## Step 52 — service artifact operational probes

Step 52 formalizes the distinction:

`execution != health != availability/consumability`

A process or container being reported as running is not sufficient evidence that the application is internally healthy or consumable by a client.

### Service artifact contract

A trusted service artifact now describes:

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

The current validated O2 MQTT container is intentionally useful for the first live proof: Docker reports the container running, while the container was created without a host port mapping. Therefore the expected assessment is execution `passed`, Docker health `not_verifiable` if the image has no HEALTHCHECK, and availability `failed` because a host-local MQTT consumer cannot connect to port 1883.

### New CLI

`smartsentinel-service-probe` loads a trusted service artifact and writes a typed `ServiceOperationalAssessment` JSON report.

Step 52 first validates the semantic/probe layer in isolation. Integration of these assessments into the continuous operational evidence stream and use of deployment declarations for governed drift reconciliation are intentionally deferred to the next step so observation is proven before a new host-mutation class is introduced.

## Architecture statement

`SmartSentinel observes execution -> probes health -> probes consumability -> publishes explicit evidence quality; D-Graph remains desired-state context, dserver remains governance/control plane, and external AI remains responsible for placement/reasoning decisions.`

DATUM remains the conceptual framework; it does not execute host actions.
