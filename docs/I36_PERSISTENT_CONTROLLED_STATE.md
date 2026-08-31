# I36 — Persistent Controlled WASM State

- checkpoint: `I36/142`
- predecessor: `I35/141`
- status: `implementation_ready_for_validation`

## Objective

I36 removes the dserver restart-state loss observed during the I35 campaign.

The controlled dserver state now has a persisted envelope containing:

```text
persistence_version
source_dgraph
active_manifest
live_reports
```

The default operator-local state path is:

```text
.datum-controlled-wasm-state.json
```

It can be overridden with:

```text
DATUM_CONTROLLED_WASM_STATE_PATH
```

The runtime state file and its `.tmp`/`.bak` siblings are ignored by Git.

## Validation before restoration

Persisted state is not trusted merely because it was previously written.

Before restoration I36 revalidates:

- persistence schema version;
- source D-Graph through the existing I34 activation/derivation contract;
- exact equality between the persisted active manifest and the manifest freshly re-derived from the persisted D-Graph;
- standalone direct manifests through the existing manifest validator;
- every persisted live report against the active manifest;
- uniqueness of every persisted `execution_id`;
- absence of reports when no active manifest exists.

Malformed or semantically invalid persisted state is not activated. Controlled in-memory state starts empty and the server can accept a later valid publication that replaces the corrupt runtime file.

## Mutation ordering

For the three state-changing controlled endpoints:

```text
POST /api/v1/controlled-wasm/dgraph
POST /api/v1/controlled-wasm/deployment-manifest
POST /api/v1/controlled-wasm/live-reports
```

I36 uses:

```text
validate candidate
  -> persist candidate
  -> activate candidate in memory
```

A persistence failure therefore prevents the candidate mutation from becoming the official in-memory controlled state.

A transaction mutex serializes controlled state mutations for this laboratory control plane.

## Replacement discipline

Persistence uses a temporary file and backup replacement sequence:

```text
serialize + validate
write <state>.tmp
sync temporary file
move current primary to <state>.bak when present
move temporary file to primary
best-effort remove backup
```

If activation of the temporary primary fails, the previous backup is restored when possible.

If the primary is missing after an interrupted replacement but a `.bak` exists, the loader can restore from the backup. A corrupt existing primary does not silently fall back to older data; it fails closed.

## Lazy initialization

To keep I36 independent from the wider dserver/Firebase startup path, persisted controlled state is restored before the first `controlled-wasm` API operation in the process.

This is sufficient for restart persistence semantics: after a restart, the first controlled GET/POST sees the validated restored state before handling the request.

## Focused tests

I36 adds six core tests:

```text
empty persisted state accepted
manifest + report round-trip
reject duplicate execution_id
reject reports without active manifest
corrupt primary fails closed
backup used when primary is absent
```

The I34 dserver baseline was `482 passed; 0 failed`, so the expected minimum after these six focused tests is `488 passed; 0 failed`. The observed full-suite count is authoritative if other tests have accumulated.

## Local regression

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check
cargo test --manifest-path dserver/Cargo.toml
cargo build --manifest-path dserver/Cargo.toml
```

## Live restart gate

Use a fresh local persistence file and the existing I34/I35 lab fixture.

1. start I36 dserver;
2. publish the external D-Graph;
3. confirm the state file exists;
4. execute one direct Sentinel assignment and confirm `live-reports count=1`;
5. stop dserver without deleting the state file;
6. start dserver again;
7. GET the manifest, source D-Graph, node slice, and live reports;
8. confirm the same manifest/report/execution ID is restored without republishing the D-Graph;
9. optionally complete the remaining two direct Sentinel assignments and validate the observed D-Graph.

Required restart PASS:

```text
same source_dgraph_id
same manifest_id
same pre-restart report execution_id
no D-Graph republish required after restart
```

## Corruption gate

With dserver stopped and using a disposable persistence path, write invalid JSON to the persisted state file and restart.

Expected behavior:

```text
invalid persisted state is not activated
controlled GET returns empty/not-found state
health/general dserver remains available
later valid D-Graph POST can establish a new persisted state
```

Do not corrupt the state file used for the main I36 live evidence; use a separate path through `DATUM_CONTROLLED_WASM_STATE_PATH`.

## Non-claims

I36 does not add:

- database-backed/high-availability state replication;
- authenticated storage encryption;
- multi-process state coordination;
- automatic Sentinel execution;
- cryptographic agent identity;
- canonical governed execution.

Checkpoint remains `141/150` until regression and restart validation pass.