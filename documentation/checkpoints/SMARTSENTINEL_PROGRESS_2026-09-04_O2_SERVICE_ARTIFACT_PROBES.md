# SmartSentinel O2 checkpoint — service artifact operational probes

Date: 2026-09-04
Branch: `agent/o2-service-artifact-probes`
Baseline main: `ef51332d602387dc10b809886e2ec90035ffbed5`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — IMPLEMENTED; first live availability-discrimination proof PASS; A/B recovery proof pending.

The validated 51/100 baseline is on `main`. Step 52 remains isolated in the development branch.

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

### Live proof A — running but unavailable — PASS

The current O2 MQTT container was running but had no host port mapping. Running the unchanged service artifact produced:

- `artifact_id=service:mqtt:mosquitto-2`;
- `service_id=mqtt`;
- `operational_state=unavailable`;
- `assessment_basis=execution_health_availability`;
- execution `docker_container_running` -> `passed` with detail that `datum-mqtt` is running;
- health `docker_health` -> `not_verifiable` because the container image exposes no Docker HEALTHCHECK state;
- availability `mqtt_connect` -> `failed` because a host-local MQTT client could not connect to `127.0.0.1:1883`;
- execution vantage point `local_runtime`;
- availability vantage point `local_host`.

This is the intended counterexample to any rule that equates runtime execution with service availability:

`container running = true` did not imply `service available = true`.

The live result reproduces the operational issue observed during thesis experimentation: a component can be executing correctly from the runtime perspective while still being unusable by its consumer.

### Remaining A/B proof for step 52

Without changing the service artifact, recreate the same Mosquitto container with the declared `1883:1883/tcp` host exposure and rerun the same probe. The expected transition is:

- execution remains `passed`;
- health remains `not_verifiable` for the same image;
- availability changes `failed -> passed`;
- `operational_state` changes `unavailable -> available`.

This A/B test isolates availability from execution and proves that the service artifact describes a meaningful consumer-visible contract.

### New CLI

`smartsentinel-service-probe` loads a trusted service artifact and writes a typed `ServiceOperationalAssessment` JSON report.

Step 52 first validates the semantic/probe layer in isolation. Integration of these assessments into the continuous operational evidence stream and use of deployment declarations for governed drift reconciliation are intentionally deferred to the next step so observation is proven before a new host-mutation class is introduced.

## Architecture statement

`SmartSentinel observes execution -> probes health -> probes consumability -> publishes explicit evidence quality; D-Graph remains desired-state context, dserver remains governance/control plane, and external AI remains responsible for placement/reasoning decisions.`

DATUM remains the conceptual framework; it does not execute host actions.
