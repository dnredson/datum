# Stage 83 — Cloud Magistrala Deployment Preparation

Stage 83 prepares the cloud node for the distributed E2E platform. The target is cloud-01, and the registered artifacts are the Magistrala foundation, API, bootstrap, and FluxMQ services.

## Scope

This stage is deliberately a governed preflight:

- the cloud node receives a dedicated node-scoped configuration;
- the operational D-Graph assigns only cloud-targeted Magistrala services to cloud-01;
- every assignment resolves by service_id and target node;
- WASM remains mandatory and host imports/automatic execution remain disabled;
- no container is started until dependencies, network, volumes, secrets, and lifecycle configuration are supported by the runtime.

The file DATUM/config.cloud.linux contains trusted local container mappings and health ports for the cloud services. It is not a request to start the full Magistrala stack by itself.

## Expected cloud assignments

The D-Graph slice for cloud-01 should eventually contain these services in dependency order:

1. magistrala-atom-db
2. magistrala-bootstrap-db
3. magistrala-atom
4. magistrala-bootstrap
5. magistrala-fluxmq-auth
6. magistrala-fluxmq-node1

The current Stage 83 validation uses --execute only after an operator has reviewed the generated report. Until the runtime supports the required dependency/network/secret contract, the recommended validation is dry-run (--execute omitted), with no host mutation.

## Safety invariants

A valid preflight must report:

- control_plane_authorized=false unless a fresh explicit authorization exists;
- host_mutation_performed=false;
- side_effects_performed=false;
- one artifact per desired service;
- artifact target node equal to cloud-01;
- trusted image and container mapping present;
- WASM executed with no host imports and no guest side effects.

A failed mapping, missing artifact, wrong target node, stale revision, absent binding, or unsafe WASM policy must fail closed.

## Next runtime increment

The next increment must add a governed container dependency contract covering Docker network, persistent volumes, environment bindings, secret references, health checks, and ordered start/rollback. Only then should the Magistrala containers be deployed on cloud-01.
