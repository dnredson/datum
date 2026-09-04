# SmartSentinel O2 checkpoint — continuous service evidence and deployment drift

Date: 2026-09-04
Branch: `agent/o2-service-evidence-drift`
Baseline main: `3b266857d92b94a2860d4e4b2f32a1db68038f6f`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — IMPLEMENTED, pending compile/live validation.

The validated 52/100 baseline is on `main`. Step 53 is isolated in this branch.

## Step 53 — continuous service evidence + deployment drift

Step 53 extends the service artifact work from an isolated probe into a continuous operational evidence producer.

It preserves two independent questions:

1. **service operational assessment** — is the component executing, healthy when verifiable, and consumable from the configured vantage point?
2. **deployment drift assessment** — does the observed local deployment match the trusted service artifact declaration?

A service failure does not automatically imply deployment drift, and deployment drift does not automatically imply a particular application failure cause.

## Deployment drift contract

`service_deployment_drift.rs` compares the trusted container deployment declaration against Docker inspection evidence.

Step 53 currently checks:

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

Each event is appended to local JSONL first and independently published to dserver. Remote publication failure does not erase the local evidence record.

All step-53 evidence has `side_effects_performed=false`.

The CLI accepts project/resource/node/D-Graph context so the service timeline can be correlated with the same operational graph revision and resource lineage used by reconciliation evidence.

## Observation-first boundary

Step 53 intentionally does not repair deployment drift.

For example, if the trusted MQTT artifact declares `1883:1883/tcp` but Docker exposes only the container-internal port, SmartSentinel may now publish both:

- `service_operational_assessment` with `operational_state=unavailable` and MQTT availability failure;
- `deployment_drift_detected` with the missing `1883:1883/tcp` mapping.

That is evidence, not a placement decision and not yet a new host-mutation class.

A later step can use governed desired deployment declarations to decide whether and how the local executor may recreate/reconfigure the container.

## Planned live proof

1. compile the new drift module and service evidence binary;
2. with the currently corrected MQTT deployment (`1883:1883` published), run a few cycles and confirm:
   - service assessment `available`;
   - `deployment_converged`;
   - local and remote evidence both succeed;
3. stop the service evidence process only;
4. recreate the same `datum-mqtt` container without the host port publication;
5. run the same unchanged artifact and confirm:
   - execution remains `passed`;
   - availability changes to `failed`;
   - service state becomes `unavailable`;
   - deployment event changes to `deployment_drift_detected`;
   - drift difference identifies the missing `1883:1883/tcp` host mapping;
6. inspect local JSONL and dserver history to verify both timelines are preserved under the same artifact/service/resource/D-Graph context.

## Architecture statement

`SmartSentinel observes execution -> probes health -> probes consumability -> compares observed deployment with trusted artifact -> publishes typed operational evidence; dserver governs and preserves control context; D-Graph remains desired-state context; external AI remains responsible for placement/reasoning decisions.`

DATUM remains the conceptual framework; it does not execute host actions.
