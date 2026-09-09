# Stage 93 — Bootstrap database runtime contract

This increment prepares the first Bootstrap dependency for governed execution on
`cloud-01`. It does not deploy Bootstrap or FluxMQ yet.

## Scope

- keep the Atom and PostgreSQL validation from Stage 92 unchanged;
- declare the Bootstrap PostgreSQL database and its persistent volume;
- inject the database password only from the cloud-local environment;
- pass the container-native PostgreSQL variables `POSTGRES_DB`,
  `POSTGRES_USER`, and `POSTGRES_PASSWORD`;
- preserve explicit D-Graph authorization and dependency ordering;
- validate the artifact contract before any host mutation.

The Bootstrap service remains gated behind its database and Atom dependencies.
FluxMQ remains a later increment.

## Contract

The artifact `magistrala-bootstrap-db` uses:

- image `postgres:16.2-alpine`;
- container `datum-magistrala-bootstrap-db`;
- network `datum-magistrala-net`;
- named volume `datum-magistrala-bootstrap-db-volume`;
- database `bootstrap`;
- user `datum`;
- local secret reference `magistrala_bootstrap_db_postgres_password`.

The local cloud configuration maps that secret reference to the process-local
`POSTGRES_PASSWORD` environment variable. The value must never be sent to
DServer or committed.

## Validation boundary

This increment is complete only after:

1. focused artifact tests pass;
2. the artifact is updated in DServer;
3. the current D-Graph revision is explicitly authorized;
4. SmartSentinel reports `control_plane_authorized=true` and
   `host_mutation=true`;
5. the Bootstrap DB container is running and accepts
   `pg_isready -U datum -d bootstrap`;
6. the named volume and network attachments are verified.

No Bootstrap application or FluxMQ container is started by this increment.
