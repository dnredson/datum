# SmartSentinel O2 checkpoint — governed deployment reconciliation

Date: 2026-09-04
Branch: `agent/o2-governed-deployment-reconciliation`
Baseline main before closure: `95be3aff1020622f031d861c850e3ad6dda2a2cc`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.
- 54/100 — LIVE PASS.
- 55/100 — LIVE PASS.

Step 55 is closed after compile/test plus fail-closed and authorized-action live validation on fog-01.

## Step 55 — governed service deployment drift reconciliation — LIVE PASS

Step 55 introduces a new local host-mutation class for deployment configuration drift described by a trusted service artifact.

It does not make placement decisions. It only reconciles local deployment details after the control plane and local policy already authorize the current D-Graph desire.

### Ownership boundary

The existing operational/WASM reconciler remains responsible for execution lifecycle:

- missing container;
- stopped container;
- desired running vs observed running.

The service deployment reconciler is responsible only for configuration/deployment drift on an existing container, currently:

- image mismatch;
- required host-port publication mismatch.

A missing container is reported as `deferred_execution_reconciler` rather than being deployed by both mechanisms.

### Required gates

A deployment repair may mutate the host only when all are true:

1. the service artifact is trusted local input;
2. the service is still desired in the active D-Graph slice;
3. the slice authorization allows host mutation and its authorization revision matches the active D-Graph revision;
4. SmartSentinel was started with `--execute`;
5. local dispatch policy is enabled and `allow_container_execution=true`.

Gate outcomes are explicit:

- `blocked_not_desired`;
- `suggested` when `--execute` is absent;
- `blocked_control_plane`;
- `blocked_local_policy`;
- `authorized`.

### Typed mutation

The artifact executor does not use a shell.

For supported configuration drift it performs typed Docker argv operations:

1. `docker rm -f <trusted container_name>`;
2. `docker run -d --name <trusted container_name> [-p <trusted host:container/protocol> ...] <trusted image>`.

The MQTT artifact reconstructs the desired deployment as:

`docker run -d --name datum-mqtt -p 1883:1883/tcp eclipse-mosquitto:2`

Command phase, argv, success and detail are captured in the reconciliation report.

### Evidence

When integrated service observation detects deployment drift, the same SmartSentinel process may emit:

- `service_deployment_reconciliation_blocked`;
- `service_deployment_reconciliation`;
- `service_deployment_reconciliation_action`;
- `service_deployment_reconciliation_error`.

The report carries:

- `execute_requested`;
- `desired_in_active_slice`;
- `control_plane_authorized`;
- `local_container_execution_allowed`;
- `execution_authorized`;
- before/after drift assessment;
- command results;
- `side_effects_performed`.

If a deployment action occurs, SmartSentinel immediately attempts to publish an operational role snapshot and records `snapshot_published` / `snapshot_publish_error` in the action evidence.

### Concurrency boundary

The deployment reconciler does not act on missing execution. This avoids competing with the existing execution reconciler. The live case used a running MQTT container whose host port mapping was wrong: the execution reconciler saw it running and performed no Docker mutation, while the service artifact reconciler owned the port-binding correction.

### Automated validation

`DATUM/tests/smartsentinel_o2_service_deployment_reconciliation.rs` passed and validates without touching Docker:

- fail-closed gate ordering;
- fully authorized gate;
- typed Docker argv construction from the MQTT service artifact contract.

### Live validation A — fail-closed control-plane gate — PASS

A revision-three D-Graph fixture kept MQTT desired on `fog-01` while invalidating the prior revision-two authorization. The running `datum-mqtt` container was recreated without `-p 1883:1883`, preserving execution while introducing deployment and availability drift.

Repeated integrated evidence showed:

- `deployment_drift_detected` with `revision=3` and `status=drift_detected`;
- `execute_requested=true`;
- `desired_in_active_slice=true`;
- `control_plane_authorized=false`;
- `local_container_execution_allowed=true`;
- `service_deployment_reconciliation_blocked` with `status=blocked_control_plane`;
- `side_effects_performed=false`.

Docker inspection after the blocked cycles still showed only internal `1883/tcp` exposure and no host publication. This proves that `--execute` plus local policy are insufficient without current-revision control-plane authorization.

### Live validation B — authorized artifact reconciliation — PASS

Revision 3 was then explicitly authorized with `allow_host_mutation=true` while the D-Graph desired MQTT placement remained unchanged.

The next integrated reconciliation produced:

- `service_deployment_reconciliation_action`;
- `revision=3`;
- `status=reconciled`;
- `execute_requested=true`;
- `desired_in_active_slice=true`;
- `control_plane_authorized=true`;
- `local_container_execution_allowed=true`;
- `side_effects_performed=true`;
- `snapshot_published=true`.

Typed command evidence showed both operations succeeding:

1. `docker rm -f datum-mqtt`;
2. `docker run -d --name datum-mqtt -p 1883:1883/tcp eclipse-mosquitto:2`.

Docker inspection then showed:

- `0.0.0.0:1883->1883/tcp`;
- `[::]:1883->1883/tcp`.

The service evidence timeline proved convergence:

- sequence 1: `service_operational_assessment` = `unavailable`, availability `failed`;
- sequence 1: `deployment_drift_detected` = true;
- sequence 2 onward: `service_operational_assessment` = `available`, availability `passed`;
- sequence 2 onward: `deployment_converged`, drift=false.

This closes the full observation -> governed mutation -> proof-of-convergence path without any SmartSentinel placement decision or D-Graph modification.

## Architecture statement

`SmartSentinel observes -> correlates with current D-Graph desire and authorization -> applies only locally governed artifact reconciliation -> publishes action/result evidence. dserver remains governance/control plane; D-Graph remains desired-state source; external AI remains responsible for placement/reasoning decisions; DATUM remains the conceptual framework.`
