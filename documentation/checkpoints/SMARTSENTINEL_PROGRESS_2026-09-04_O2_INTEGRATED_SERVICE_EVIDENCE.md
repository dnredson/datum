# SmartSentinel O2 checkpoint — integrated service evidence

Date: 2026-09-04
Branch: `agent/o2-integrated-service-evidence`
Baseline main: `7c13e1f950d0f0d7507235ed50341655947750dc`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.
- 54/100 — IMPLEMENTED, pending compile/live validation.

The validated 53/100 baseline is on `main`. Step 54 is isolated in this branch.

## Step 54 — service evidence inside the main SmartSentinel process

Step 54 removes the need to run `smartsentinel-service-evidence` as a second process during normal operation.

The main `smartsentinel --operational` binary now accepts:

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

The evidence envelope therefore carries, when available:

- `resource_id`;
- current `node_id`;
- current `dgraph_id`;
- current `dgraph_revision`.

The payload also includes:

- `integration_source=smartsentinel_operational`;
- `desired_in_active_slice=true|false|null`.

This prevents a locally configured service artifact from silently being treated as current desired state after a future placement change. If the active D-Graph no longer desires that service on the node, the evidence explicitly records that fact rather than letting SmartSentinel make a placement decision.

## Shared history safety

Because the reconciliation loop and service observer may append to the same JSONL concurrently, `append_operational_evidence` now takes an exclusive file lock around each append+flush operation.

This is a storage-safety change only; evidence schema `0.1.0` is unchanged.

## Lifecycle

The internal service observer:

- starts only when `--service-artifact` is configured in operational mode;
- validates the artifact before starting;
- runs at `--service-evidence-s` cadence;
- observes the same stop file;
- receives an explicit shutdown flag after the main operational loop returns;
- is joined before the `smartsentinel` process exits.

The observer does not install runtimes, reconcile containers, alter ports, or execute host mutation.

## Planned live validation

On fog-01, with dserver left running and MQTT restored with `1883:1883/tcp`:

1. build the main `smartsentinel` binary from this branch;
2. run one operational SmartSentinel process with both runtime and service artifacts configured;
3. confirm the normal hybrid reconciliation loop remains active;
4. confirm integrated service events appear in the same operational history with `local=ok remote=ok`;
5. confirm `desired_in_active_slice=true`, `dgraph_revision=2`, MQTT `available`, and deployment `converged`;
6. verify no `smartsentinel-service-evidence` process is required;
7. verify clean observer shutdown when the main SmartSentinel exits.

A later step can use the now-integrated deployment drift evidence as an input to a new governed local deployment-reconciliation class. Step 54 itself remains observation-only for service/deployment state.

## Architecture statement

`SmartSentinel main process observes host/resource state -> polls D-Graph control -> reconciles governed desired execution -> independently probes service execution/health/consumability and deployment drift at service cadence -> publishes all evidence with current resource/node/D-Graph lineage.`

D-Graph remains the desired-state source, dserver remains governance/control plane, and external AI remains responsible for placement/reasoning decisions. DATUM remains the conceptual framework and does not execute host actions.
