# SmartSentinel O2 checkpoint — continuous service evidence and deployment drift

Date: 2026-09-04
Branch: `agent/o2-service-evidence-drift`
Baseline main before closure: `3b266857d92b94a2860d4e4b2f32a1db68038f6f`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.

Step 53 is closed after compile and live A/B validation on fog-01 with local and remote operational evidence.

## Step 53 — continuous service evidence + deployment drift — LIVE PASS

Step 53 extends the service artifact work from an isolated probe into a continuous operational evidence producer.

It preserves two independent questions:

1. **service operational assessment** — is the component executing, healthy when verifiable, and consumable from the configured vantage point?
2. **deployment drift assessment** — does the observed local deployment match the trusted service artifact declaration?

A service failure does not automatically imply deployment drift, and deployment drift does not automatically imply a particular application failure cause.

## Deployment drift contract

`service_deployment_drift.rs` compares the trusted container deployment declaration against Docker inspection evidence.

Step 53 checks:

- expected container existence;
- expected image identity;
- expected host-port publication for declared container ports.

For a declared mapping such as `1883:1883/tcp`, Docker inspection must expose the expected host port. A running container with only an internal `1883/tcp` exposure is therefore deployment drift even when runtime execution is satisfied.

The drift assessment is typed and contains:

- schema version;
- observation timestamp;
- artifact/service identity;
- runtime kind;
- status (`converged` or `drift_detected`);
- `drift_detected` boolean;
- explicit field-level differences with expected and observed values;
- compact observed deployment evidence.

No mutation is performed by this assessment.

## Continuous service evidence publisher

`smartsentinel-service-evidence` continuously loads one trusted service artifact and, on each cycle, emits two evidence streams using the same `OperationalEvidenceEvent` envelope introduced in step 49:

- `service_operational_assessment`;
- `deployment_drift_detected` or `deployment_converged`.

Assessment errors are also explicit:

- `service_operational_assessment_error`;
- `deployment_drift_assessment_error`.

Each event is appended to local JSONL and independently published to dserver. Remote publication failure does not erase the local evidence record.

All step-53 evidence has `side_effects_performed=false`.

The CLI accepts project/resource/node/D-Graph context so the service timeline can be correlated with the same operational graph revision and resource lineage used by reconciliation evidence.

## Compile validation and disk-space incident

The first build attempt reached the linker but failed with `ld terminated with signal 7 [Bus error]`. A subsequent `cargo check`/build exposed the actual host condition: `No space left on device (os error 28)` while Rust attempted to write `.rmeta` and incremental query-cache files.

`cargo clean` reclaimed approximately 13 GB on fog-01. After reclaiming disk space, the step-53 binary compiled and executed normally. The incident was therefore host storage exhaustion, not a Rust semantic/linker regression in step 53.

## Live proof B — corrected deployment — PASS

With `datum-mqtt` running as `eclipse-mosquitto:2` and host mapping `1883:1883/tcp` present, three service-evidence cycles produced:

- `service_operational_assessment` with `operational_state=available`;
- execution `passed`;
- health `not_verifiable` because the image has no Docker HEALTHCHECK state;
- availability `passed` through a real MQTT CONNECT/CONNACK against `127.0.0.1:1883`;
- `deployment_converged` with zero differences;
- local evidence append `ok`;
- remote dserver evidence publication `ok`.

The repeated sequence demonstrated stable converged observation rather than a one-shot result.

## Live proof A — running but unavailable with deployment drift — PASS

The same `datum-mqtt` image was recreated without publishing host port 1883. The trusted service artifact was not changed.

Two service-evidence cycles then produced:

- execution `Passed`;
- health `NotVerifiable`;
- availability `Failed`;
- `operational_state=unavailable`;
- `deployment_drift_detected`;
- `drift_detected=true`;
- exactly one deployment difference;
- local evidence append `ok`;
- remote dserver evidence publication `ok`.

The drift difference was explicit:

- field: `runtime.container.ports.1883/tcp`;
- detail: required host mapping `1883:1883/tcp` is absent;
- expected: container port 1883, host port 1883, protocol tcp;
- observed: no host ports mapped for that declared container port.

This establishes two independent but temporally correlatable evidence streams:

- service consumability changed from `available` to `unavailable`;
- deployment state changed from `converged` to `drift_detected`.

SmartSentinel did not claim that the deployment difference caused the application-level failure; it published both facts without inventing causal reasoning.

## jq validation note

A validation display used `(.payload.drift_detected // null)`. In jq, `//` treats both `null` and `false` as fallback values, so a valid `drift_detected=false` appeared as `null` in that presentation. The service-evidence process output independently confirmed `drift_detected=false` for `deployment_converged`. This was a display-expression artifact, not missing evidence.

## Observation-first boundary

Step 53 intentionally does not repair deployment drift.

SmartSentinel now has sufficient trusted evidence to distinguish:

- runtime execution state;
- health when verifiable;
- consumer-visible availability;
- deployment convergence/drift.

A later governed step may use trusted desired deployment declarations to reconcile container configuration drift. Placement and global reasoning remain outside SmartSentinel.

## Step 53 conclusion

Step 53 is live-proven for both stable convergence and induced deployment drift using the same unchanged service artifact. Evidence was preserved both locally and remotely under the same resource/node/D-Graph revision context, and all observations remained side-effect free.

## Architecture statement

`SmartSentinel observes execution -> probes health -> probes consumability -> compares observed deployment with trusted artifact -> publishes typed operational evidence; dserver governs and preserves control context; D-Graph remains desired-state context; external AI remains responsible for placement/reasoning decisions.`

DATUM remains the conceptual framework; it does not execute host actions.
