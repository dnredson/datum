# Stage 91 — Dependency-aware container runtime

Stage 91 turns the declarative artifact runtime contract into governed local
container wiring.

## Runtime attachments

A container artifact may declare:

- `runtime.container.network`: a validated Docker network name;
- `runtime.container.volumes`: named volume mounts with an absolute target and
  an optional read-only flag.

DServer copies these fields into the node reconciliation context. SmartSentinel
passes them to Docker only after all existing gates succeed:

1. the embedded WASM reconciler requests the action;
2. the D-Graph revision and operation match the active authorization;
3. the local container policy is enabled;
4. all referenced secrets resolve from the node-local allowlist;
5. every declared dependency is running.

A missing network is created by the authorized node-local Docker operation. Volume
creation is delegated to Docker's `--mount type=volume` behavior. Rollback
removes the governed service container and leaves shared network/volume state
intact for dependent services.

## Dependency semantics

Artifact control-plane dependencies remain separate service/database entities.
Before reconciling a service, SmartSentinel resolves every dependency by
artifact or service ID and observes its trusted local container. A dependency
that is missing, stopped, unobservable, or lacks a trusted local mapping causes
the service to receive `blocked_dependency_unavailable` and prevents host
mutation.

This means a running application container is not sufficient evidence of
availability when its declared database or platform dependency is unavailable.

## E2E fixture

The cloud Magistrala fixtures share `datum-magistrala-net`. The Atom and
Bootstrap PostgreSQL artifacts also declare durable named volumes at
`/var/lib/postgresql/data`. The image pins remain aligned with the legacy SMA
contract:

- Atom `v0.50.0`;
- Bootstrap and FluxMQ `v0.51.0`.

The full legacy users/clients/domains/channels service set remains a later
fixture expansion; this stage establishes the runtime semantics needed before
that expansion.

All execution remains proposal-only by default. `side_effects_performed`
stays false for preflight and becomes true only after an explicitly authorized
node-local mutation.
