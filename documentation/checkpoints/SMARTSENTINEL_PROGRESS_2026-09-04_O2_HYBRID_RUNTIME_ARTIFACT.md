# SmartSentinel O2 checkpoint — hybrid control and runtime artifact gate

Date: 2026-09-04
Branch: `agent/o2-host-capability-discovery`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — IMPLEMENTED, pending compile/live validation.
- 51/100 — IMPLEMENTED, pending compile/live validation.

No step beyond 49 is considered closed until the combined validation is executed.

## Step 50 — hybrid time-driven + event-driven reconciliation

The operational D-Graph now carries an explicit monotonic `revision`.

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
3. if neither condition applies, reconciliation is skipped without running Wasmtime or Docker inspection.

Operational evidence records the trigger and D-Graph revision so later analysis can distinguish desired-state changes from time-driven drift detection.

## Step 51 — trusted runtime artifact prerequisite gate

The operational loop now checks the runtime prerequisite before container reconciliation when the active slice contains container services.

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
- Docker `installed_but_unavailable`: do not reinstall. When governed and allowed, execute only trusted post-install remediation (for example starting/enabling Docker and configuring the execution user's Docker group).

If post-install group membership changes require a new session/service, the report sets `session_refresh_required=true`; SmartSentinel does not pretend that the current process immediately gained supplementary-group access.

### Evidence event classes

The operational evidence stream can now include:

- `runtime_prerequisite`;
- `runtime_prerequisite_action`;
- `runtime_prerequisite_blocked`;
- `runtime_prerequisite_error`.

Runtime artifact action reports preserve phase, command program, argv, privilege requirement, success/detail, before/after runtime probe, side effects, and session refresh requirement.

## Governance hardening

The standalone/manual operational reconciliation path now also deserializes and validates `dgraph_revision`. Host-mutation authorization is accepted only when its revision matches the active D-Graph slice revision. This prevents an older authorization from being accepted by an alternate reconciliation path.

## Planned combined validation

When live testing resumes, validate steps 50 and 51 together:

1. compile/test dserver and DATUM at the checkpoint HEAD;
2. start dserver and rehydrate revision 1 + matching authorization;
3. run SmartSentinel with a short control poll and longer drift interval plus the trusted Docker runtime artifact;
4. confirm control-unchanged cycles are skipped;
5. stop `datum-mqtt` without changing the D-Graph and confirm time-driven drift verification starts it after the drift interval;
6. publish D-Graph revision 2 and confirm event-driven control-change detection before the drift timer;
7. confirm revision 1 authorization is not reused for revision 2;
8. authorize revision 2 and confirm the authorization change triggers event-driven reconciliation;
9. confirm runtime prerequisite evidence is emitted and Docker-available state passes through without runtime mutation;
10. separately, on a host with Docker missing or daemon access unavailable, validate the runtime artifact install/remediation lifecycle and its session-refresh behavior.

## Architecture statement

`SmartSentinel observes -> D-Graph contextualizes desired state -> dserver governs/distributes -> external AI reasons/updates D-Graph -> SmartSentinel detects control changes or timed drift -> trusted runtime artifact satisfies local prerequisites -> WASM reconciles governed local desired state -> SmartSentinel publishes evidence.`

DATUM remains the conceptual framework; it does not execute host actions.
