# SmartSentinel O2 checkpoint — hybrid control and runtime artifact gate

Date: 2026-09-04
Branch: `agent/o2-host-capability-discovery`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.

Steps 50 and 51 are closed.

## Step 50 — hybrid time-driven + event-driven reconciliation — LIVE PASS

The operational D-Graph carries an explicit monotonic `revision`.

The dserver rejects revision regression and rejects publishing different content under the same active revision. A higher revision invalidates the previous operational reconciliation authorization. Authorizations are bound to the active D-Graph revision.

The SmartSentinel operational loop uses a lightweight control signature containing:

- `dgraph_id`;
- `dgraph_revision`;
- `authorization_id`;
- authorization revision;
- `allow_host_mutation`.

Reconciliation triggers are hybrid:

1. **event-driven / change-driven** — a D-Graph revision or authorization change triggers reconciliation on the next control poll;
2. **time-driven** — if control is unchanged, periodic drift verification still runs after `--drift-verify-s`;
3. if neither condition applies, reconciliation is skipped without running the full reconciliation path.

Operational evidence records the trigger and D-Graph revision so later analysis can distinguish desired-state changes from time-driven drift detection.

### Live evidence — time-driven drift

With D-Graph revision 1 unchanged, `datum-mqtt` was manually stopped. SmartSentinel later reconciled through `trigger=time_driven_drift_verification` with:

- `wasm_actions=1`;
- `converged_after=1`;
- `host_mutation=true`;
- `snapshot_published=true`;
- `snapshot_due=true`.

Subsequent time-driven verification returned `wasm_actions=0`, `converged_after=1`, and `host_mutation=false`, proving recovery followed by stable convergence.

### Live evidence — event-driven D-Graph revision and authorization

D-Graph revision 2 was published while `datum-mqtt` was stopped. The revision 1 authorization was intentionally invalidated.

Operational evidence for sequence 93 recorded:

- `revision=2`;
- `trigger=event_driven_control_change`;
- `authorized=false`;
- service `execution=blocked_control_plane`;
- `wasm=1`;
- `observed_before=false`;
- `observed_after=false`;
- `host_mutation=false`.

This proves the new desired-state revision was observed and evaluated, while stale authorization was not reused.

After revision 2 received a matching authorization, sequence 116 recorded another `event_driven_control_change` and the operational loop reported:

- runtime prerequisite `status=available`;
- `execution_authorized=true`;
- `side_effects=false` for the runtime prerequisite itself;
- `wasm_actions=1`;
- `converged_after=1`;
- `host_mutation=true`;
- `snapshot_published=true`;
- `snapshot_due=true`.

`docker ps --filter name=datum-mqtt` then showed the existing `datum-mqtt` container `Up`, confirming recovery after the authorization change without waiting for the drift interval.

### Step 50 conclusion

Both trigger classes are live-proven:

- **time-driven:** local drift with unchanged control state -> timed verification -> governed recovery;
- **event-driven:** D-Graph revision or authorization change -> next control poll -> immediate governed reconciliation.

The same operational evidence stream preserves both paths and the D-Graph revision associated with each event.

## Step 51 — trusted runtime artifact prerequisite gate — LIVE PASS

The operational loop checks the runtime prerequisite before container reconciliation when the active slice contains container services.

Runtime installation remains a local trusted artifact concern. The D-Graph does not carry arbitrary shell commands and does not decide package-manager commands.

The trusted Docker runtime artifact contract supports:

- `pre_install_commands`;
- one typed `install_command`;
- `post_install_commands`;
- `execution_user` with the restricted `${SMARTSENTINEL_USER}` placeholder;
- package-manager-specific installers for `apt_get`, `dnf`, `yum`, `pacman`, and `zypper`.

Commands are executed as `program + argv` directly, without invoking a shell.

### Runtime mutation gates

The continuous SmartSentinel loop may execute the runtime artifact only when all required gates hold:

- the resource resolved as a bound node;
- an active operational D-Graph slice exists;
- authorization matches the active D-Graph revision and allows host mutation;
- `--execute` was requested;
- local dispatch/container execution policy allows mutation;
- the trusted runtime artifact is configured;
- required non-interactive privilege is available (root or passwordless sudo).

If a gate is missing, SmartSentinel records evidence and does not install or remediate Docker.

### Runtime states

- Docker `available`: continue to WASM/service reconciliation.
- Docker `missing`: execute trusted `pre -> install -> post` lifecycle only when governed and allowed.
- Docker `installed_but_unavailable`: do not reinstall. When governed and allowed, execute only trusted post-install remediation.

If post-install group membership changes require a new session/service, the report sets `session_refresh_required=true`; SmartSentinel does not pretend that the current process immediately gained supplementary-group access.

### Live evidence — Docker already available on fog-01

The combined step 50/51 live run repeatedly recorded:

- `event_kind=runtime_prerequisite`;
- `status=available`;
- `runtime=docker`;
- `artifact_id=runtime:docker`;
- `ready=true`;
- `execute_requested=true`;
- `execution_authorized=true` when the active revision was authorized;
- `side_effects=false`;
- `session_refresh_required=false`.

This proves the trusted runtime artifact is integrated into the continuous loop and that an already-satisfied prerequisite passes through without unnecessary installation or remediation.

### Live evidence — clean cloud-01 host with Docker missing

Cloud-01 started with:

- user `vboxuser`;
- Docker executable absent;
- `apt-get` available;
- `sudo -n true` rejected because interactive authentication was required.

Dry-run artifact reconciliation produced:

- `before=missing`;
- `package_manager=apt_get`;
- `execute_requested=false`;
- `execution_authorized=false`;
- `status=suggested`;
- `side_effects_performed=false`;
- no command results.

A second run requested execution and host mutation authorization without non-interactive privilege. It correctly produced:

- `before=missing`;
- `execute_requested=true`;
- `execution_authorized=true`;
- `status=privilege_required`;
- `side_effects_performed=false`;
- no command results.

This proves fail-closed privilege handling with no interactive password prompt and no partial mutation.

The same trusted artifact was then explicitly executed under `sudo`, with `execution_user=vboxuser`. The report recorded:

- `before=missing`;
- `package_manager=apt_get`;
- `selected_installer=apt_get`;
- `status=installed_available`;
- `side_effects_performed=true`;
- `session_refresh_required=true`;
- `after=available`.

Every lifecycle command succeeded:

1. `pre_install`: `apt-get update`;
2. `install`: `apt-get install -y docker.io`;
3. `post_install`: `systemctl enable --now docker`;
4. `post_install`: `usermod -aG docker vboxuser`.

Immediately afterward, the existing `vboxuser` session showed:

- Docker CLI installed;
- daemon socket inaccessible to that old session;
- `id` still omitted the Docker group.

After logout/login, `id` included group `docker` and `docker info --format '{{.ServerVersion}}'` returned `29.1.3`.

This validates the artifact's `session_refresh_required=true` semantics and proves that SmartSentinel does not falsely claim that supplementary-group changes affect the already-running user session.

### Step 51 conclusion

The trusted runtime artifact lifecycle is live-proven across all required paths:

- prerequisite already satisfied -> pass through without mutation;
- Docker missing + no execute -> suggestion only;
- Docker missing + authorized execute but insufficient privilege -> fail closed without side effects;
- Docker missing + explicit privilege -> typed `pre -> install -> post` lifecycle;
- session refresh requirement -> explicitly reported and confirmed by logout/login;
- refreshed user session -> Docker available to the SmartSentinel execution user.

## Evidence event classes

The operational evidence stream can include:

- `runtime_prerequisite`;
- `runtime_prerequisite_action`;
- `runtime_prerequisite_blocked`;
- `runtime_prerequisite_error`.

Runtime artifact action reports preserve phase, command program, argv, privilege requirement, success/detail, before/after runtime probe, side effects, and session refresh requirement.

## Governance hardening

The standalone/manual operational reconciliation path also deserializes and validates `dgraph_revision`. Host-mutation authorization is accepted only when its revision matches the active D-Graph slice revision. This prevents an older authorization from being accepted by an alternate reconciliation path.

## Next architectural step — 52/100

Step 52 should formalize service artifacts and operational probes so SmartSentinel distinguishes:

- execution state (`running` / not running);
- health state (component internally healthy/responding);
- availability or consumability (a real consumer can use the exposed service).

A running process/container must not automatically imply service availability.

Probe definitions should live in the trusted service artifact and select typed probe plugins such as:

- Docker/process/systemd execution checks;
- TCP checks;
- HTTP/REST checks;
- MQTT connect / publish / subscribe checks;
- database checks.

When no meaningful health or consumption probe exists, SmartSentinel should report an explicit `not_verifiable` state rather than infer availability from execution.

This also provides the basis for later peer/vantage-point checks, where a service can be locally available but unreachable from an actual dependent node.

The MQTT proof motivating this step is concrete: a container may be `Up` and the Mosquitto process may be running while an external consumer still cannot reach the broker because host port `1883` was not published. Therefore `running != healthy != available`.

## Architecture statement

`SmartSentinel observes -> D-Graph contextualizes desired state -> dserver governs/distributes -> external AI reasons/updates D-Graph -> SmartSentinel detects control changes or timed drift -> trusted runtime artifact satisfies local prerequisites -> WASM reconciles governed local desired state -> SmartSentinel publishes evidence.`

DATUM remains the conceptual framework; it does not execute host actions.
