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

The current cloud graph contains six Magistrala services with unresolved
secret references. Because reconciliation fails closed for the whole batch,
the canary must be tested in an isolated temporary graph.

1. Import the canary artifact into the project registry.
2. Save the current revision-6 graph locally.
3. Temporarily replace the graph with revision 7 containing the existing fog
   service and only `stage88-secret-canary` on `cloud-01`.
4. Authorize cloud reconciliation for revision 7.
5. Export a disposable value only in the cloud SmartSentinel process
   environment:

   `export STAGE88_CANARY_SECRET=stage88-local-only-value`

6. Run reconciliation with `--execute`.
7. Confirm the report says `secret_resolution_status=resolved_locally`,
   `execution_status=deployed`, and
   `side_effects_performed=true`, without printing the value.
8. Check only the presence of the environment variable inside the container,
   never its value, and verify port `18088`.
9. Create a revision-7 rollback authorization and run explicit rollback.
10. Confirm the canary container and port are gone.
11. Restore the original graph as revision 8, without the canary, and unset the
    environment variable.

The positive test is intentionally separate from the six Magistrala services.
It proves local secret injection and governed rollback before any real
credentials or multi-service deployment are attempted.
