# I36 — Persistent Controlled WASM State Final

- checkpoint: `I36/142`
- predecessor: `I35/141`
- validation_date: `2026-08-31`
- status: `validated`

## Closure

I36 removes the dserver restart-state loss observed during I35 by persisting the controlled WASM deployment state to a validated local JSON envelope.

Persisted state contains:

```text
persistence_version
source_dgraph
active_manifest
live_reports
```

Default runtime path:

```text
.datum-controlled-wasm-state.json
```

The lab validation used an explicit disposable path through `DATUM_CONTROLLED_WASM_STATE_PATH`.

## Regression

```text
cargo fmt --manifest-path dserver/Cargo.toml -- --check: PASS
cargo test --manifest-path dserver/Cargo.toml: 488 passed; 0 failed
cargo build --manifest-path dserver/Cargo.toml: PASS
```

Build emitted only the two existing non-blocking dead-code warnings:

```text
EXECUTION_PREREQUISITE_GATE_NAMES
 default_embedded_add_module_descriptor
```

## Restart persistence gate — PASS

The external I34 D-Graph was published into a fresh I36 state file. Before execution:

```text
source_dgraph_id=i34-external-dgraph
manifest_id=manifest:i34-external-dgraph
live_reports count=0
```

The fog Sentinel executed directly through the validated I35 path:

```text
node_id=fog-01
assignment_id=fog-scale
result_i32=24
fuel_consumed=4
report_published=true
execution_id=d80622ca-f534-49c9-a10b-f0e33dc94998
```

Before restart the dserver returned exactly one report with that execution ID.

The dserver process was then stopped and started again without deleting or republishing the persisted state. On the first controlled API access, I36 restored and revalidated the file. After restart:

```text
report_count=1
execution_id=d80622ca-f534-49c9-a10b-f0e33dc94998
node_id=fog-01
result_i32=24
```

PowerShell comparisons returned:

```text
$Before.count -eq $After.count                                  -> True
$Before.reports[0].execution_id -eq $After.reports[0].execution_id -> True
```

`GET /api/v1/controlled-wasm/dgraph` also returned the original full external D-Graph after restart without a new POST.

## Corruption gate — PASS

A separate disposable persisted-state path was filled with deliberately invalid non-JSON content.

On first controlled API access after dserver startup, the runtime emitted:

```text
controlled WASM persisted state was not activated ...
parse controlled WASM persisted state ... expected value at line 1 column 1;
starting controlled state empty
```

The corrupt state was not activated. `GET /api/v1/controlled-wasm/dgraph` returned:

```text
HTTP/1.1 404 Not Found
{"error":"no external controlled D-Graph has been published"}
```

The general dserver remained available. A later valid external D-Graph POST successfully established a new valid controlled state and replaced the corrupt runtime file.

## Validated semantics

I36 therefore demonstrates:

```text
valid state -> persist -> restart -> revalidate -> restore
corrupt state -> reject -> start controlled state empty
empty controlled state -> accept later valid D-Graph -> persist new valid state
```

Restoration revalidates the persistence schema version, source D-Graph, freshly derived manifest equality, live reports, and execution-ID uniqueness before activation.

## Storage scope

I36 uses local file persistence, not a database. This intentionally validates the storage contract before introducing database replication, migrations, or HA concerns.

The persisted JSON does not embed WASM binaries. WASM artifacts remain in the artifact repository; the controlled state retains their URI/content identity and runtime evidence.

## Non-claims

I36 does not provide:

- database-backed or replicated HA state;
- cryptographic storage integrity/authenticity;
- multi-process coordination;
- authenticated physical Sentinel identity;
- automatic Sentinel execution;
- canonical governed execution.

I36 advances the Controlled WASM Deployment phase to `142/150`.