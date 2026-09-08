# Stage 88 — positive local secret injection canary

Stage 88 validates the positive path of the node-local secret resolver with a
disposable canary. It does not use a Magistrala credential.

The fixture
`dserver/fixtures/e2e/validation/stage88-secret-canary.json` describes one
cloud service:

- image: `nginx:alpine`;
- container: `datum-stage88-secret-canary`;
- host port: `18088`;
- opaque secret reference: `stage88_canary_secret`;
- local environment variable: `STAGE88_CANARY_SECRET`.

The value is intentionally absent from Git, dserver, the D-Graph, reports,
logs, and Docker command arguments.

## Validation sequence

1. Import the canary artifact into the project registry.
2. Add `stage88-secret-canary` to the cloud desired services and advance the
   operational D-Graph revision.
3. Authorize cloud reconciliation for that revision.
4. Export a disposable value only in the cloud SmartSentinel process
   environment:

   `export STAGE88_CANARY_SECRET=stage88-local-only-value`

5. Run reconciliation with `--execute`.
6. Confirm the canary is running and that port `18088` is reachable.
7. Confirm the report says `resolved_locally`, `deployed`, and
   `side_effects_performed=true`, without printing the value.
8. Check only the presence of the environment variable inside the container,
   never its value.
9. Revoke the authorization and perform the explicit rollback; confirm the
   container and port are gone.
10. Unset the environment variable.

The positive test is intentionally separate from the six Magistrala services.
It proves local secret injection and rollback before any real credentials or
multi-service deployment are attempted.
