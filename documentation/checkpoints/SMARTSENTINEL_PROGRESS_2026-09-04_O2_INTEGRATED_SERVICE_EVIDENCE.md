# SmartSentinel O2 checkpoint — integrated service evidence

Date: 2026-09-04
Branch: `agent/o2-integrated-service-evidence`
Baseline main before closure: `7c13e1f950d0f0d7507235ed50341655947750dc`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.
- 54/100 — LIVE PASS.

Step 54 is closed after build and live validation on fog-01 with one operational SmartSentinel process.

## Step 54 — service evidence inside the main SmartSentinel process — LIVE PASS

Step 54 removes the need to run `smartsentinel-service-evidence` as a second process during normal operation.

The main `smartsentinel --operational` binary accepts:

- `--service-artifact <path>`;
- `--service-evidence-s <seconds>` with default 15 seconds.

When a service artifact is configured, the SmartSentinel process starts an internal read-only service observer thread. This is part of the same process lifecycle, not a second daemon.

The existing reconciliation loop remains unchanged. Service observation has its own cadence so execution/health/availability probes do not force the full reconciliation path on every fast control poll.

## Integrated evidence

The internal observer reuses the trusted service-artifact and deployment-drift evaluators validated in steps 52 and 53.

On each service-evidence cycle it emits into the same `OperationalEvidenceEvent` stream and same local JSONL history used by the operational reconciler:

- `service_operational_assessment`;
- `deployment_drift_detected` or `deployment_converged`;
- explicit assessment-error event kinds when evaluation fails.

All integrated service evidence remains observational and sets `side_effects_performed=false`.

## Control-plane correlation

Before each service assessment the observer resolves the current resource binding with the existing fail-closed bootstrap path and, when bound, fetches the active operational D-Graph slice.

The evidence envelope carries, when available:

- `resource_id`;
- current `node_id`;
- current `dgraph_id`;
- current `dgraph_revision`.

The payload includes:

- `integration_source=smartsentinel_operational`;
- `desired_in_active_slice=true|false|null`.

This prevents a locally configured service artifact from silently being treated as current desired state after a future placement change. If the active D-Graph no longer desires that service on the node, the evidence records that fact rather than letting SmartSentinel make a placement decision.

## Shared history safety

Because the reconciliation loop and service observer append to the same JSONL concurrently, `append_operational_evidence` takes an exclusive file lock around each append+flush operation.

The live history was successfully parsed line-by-line with `jq` after concurrent reconciliation and service-evidence writes, proving that the shared JSONL remained valid.

## Live validation — PASS

The live run on fog-01 used one `smartsentinel --operational` process with both runtime and service artifacts configured.

Observed integrated service timeline:

- `service_operational_assessment` repeated with `dgraph_revision=2`;
- `operational_state=available`;
- execution `passed`;
- availability `passed`;
- `desired_in_active_slice=true`;
- `integration_source=smartsentinel_operational`;
- `side_effects_performed=false`;
- matching `deployment_converged` events with `drift_detected=false`.

At the same time, the same history preserved the existing hybrid reconciliation timeline:

- initial `reconciliation` under revision 2;
- repeated `reconciliation_skipped` while drift proof remained fresh;
- periodic `drift_verification`;
- no service-observer side effects.

No separate `smartsentinel-service-evidence` process was present after the run (`pgrep -af smartsentinel-service-evidence` returned no process).

The main command returned normally after the configured max cycles, so the internal observer lifecycle completed with the main process rather than becoming a second daemon.

## Architecture statement

`SmartSentinel main process observes host/resource state -> polls D-Graph control -> reconciles governed desired execution -> independently probes service execution/health/consumability and deployment drift at service cadence -> publishes all evidence with current resource/node/D-Graph lineage.`

D-Graph remains the desired-state source, dserver remains governance/control plane, and external AI remains responsible for placement/reasoning decisions. DATUM remains the conceptual framework and does not execute host actions.

## Next step

55/100 introduces governed local deployment-drift reconciliation using the trusted service artifact. It must remain strictly gated by current D-Graph desire, current-revision authorization, local execution policy, and explicit `--execute`; it must not make placement decisions.
