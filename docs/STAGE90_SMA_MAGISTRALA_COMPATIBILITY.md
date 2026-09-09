# Stage 90 — SMA/Magistrala compatibility baseline

## Decision

The end-to-end fixture uses the legacy Magistrala contract implemented by the
Smart Adapter (SMA):

- Magistrala: `v0.51.0`
- Atom: `v0.50.0`
- FluxMQ: `v0.51.0`
- SMA: commit `914e22fd0384c499b4e2147bca7c49aa55c13f04`

The fixture no longer uses mutable `latest` tags for these release-pinned
images.

## Why this baseline is required

The SMA registry invokes the legacy CLI surface:

- `users token`
- `clients create/get/connect/secret`
- domain and channel identifiers

Magistrala `v0.51.0` still exposes the `users`, `clients`, `domains`, and
`channels` model and CLI. The `v1.0.0` line changes the model to
workspaces/devices and changes the provisioning flow, so it is not the
compatibility target for this adapter.

## Deployment model

The D-Graph keeps each database and service as a separate software entity.
Dependencies remain explicit in the artifact control-plane metadata, for
example:

- `magistrala-atom` depends on `magistrala-atom-db`;
- `magistrala-bootstrap` depends on `magistrala-bootstrap-db` and
  `magistrala-atom`;
- `magistrala-fluxmq-node1` depends on `magistrala-fluxmq-auth`.

The next runtime increment must materialize those dependencies into the local
container network, persistent volumes, and readiness assessment. A running
container whose database dependency is unavailable must not be reported as an
available Magistrala service.

## External references

- [SMA](https://github.com/dnredson/SMA)
- [Magistrala v0.51.0](https://github.com/absmach/magistrala/tree/v0.51.0)
- [Magistrala v1.0.0](https://github.com/absmach/magistrala/tree/v1.0.0)
- [Magistrala v0.51.0 CLI](https://github.com/absmach/magistrala/blob/v0.51.0/cli/README.md)
