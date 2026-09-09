# Stage 94 — Governed Atom bootstrap job

This increment adds the official Magistrala `atom-bootstrap:v0.51.0` one-shot job to the SmartSentinel service catalog. It is separate from the long-running legacy `bootstrap:v0.51.0` HTTP service.

## Scope

- keep the validated Atom and Bootstrap PostgreSQL runtimes unchanged;
- declare `magistrala-atom-bootstrap` as a governed container job;
- require the Atom dependency to be running before the job is started;
- resolve the Atom administrator secret only from the cloud-local environment;
- support `desired_state = "completed"` for successful one-shot containers;
- treat a stopped container with exit code `0` as converged and avoid rerunning it;
- treat a non-zero exit code as a failed job requiring explicit operator recovery;
- keep FluxMQ and the legacy Bootstrap HTTP service out of this increment.

The job uses the official image `ghcr.io/absmach/magistrala/atom-bootstrap:v0.51.0`. It calls the healthy Atom service at `http://magistrala-atom:8080` and installs the Magistrala action applicability and assignment guardrails. The job has no exposed host port.

## Secret boundary

The artifact references `magistrala_atom_bootstrap_admin_secret`. The cloud configuration maps that reference to `ATOM_ADMIN_SECRET`. The value must be the same administrator secret used by the running Atom, but it must remain process local and must not be committed or sent to DServer.

The existing Atom runtime binding `ADMIN_SECRET` remains unchanged. Keeping the job binding separate prevents the Atom container contract from being changed while adding the job.

## D-Graph execution contract

```json
{
  "service_id": "magistrala-atom-bootstrap",
  "runtime": "container",
  "desired_state": "completed"
}
```

The first authorized reconcile creates the container. After the job exits with code `0`, a later reconcile observes the stopped container as converged. A non-zero exit code is reported as `failed`; the executor does not silently delete or retry it.

## Validation boundary

The increment is complete only after:

1. focused artifact and reconciliation tests pass;
2. the new artifact is registered in DServer;
3. a new D-Graph revision explicitly adds only `magistrala-atom-bootstrap` with `desired_state = "completed"`;
4. the revision receives explicit host-mutation authorization;
5. SmartSentinel reports local secret resolution and governed host mutation;
6. the job exits with code `0`;
7. a second reconcile reports the job as converged without host mutation.

No Bootstrap HTTP or FluxMQ container is started by this increment.
