# SmartSentinel O2 checkpoint — storage abstraction and SQLite control-state foundation

Date: 2026-09-06
Branch: `agent/o2-storage-abstraction-sqlite-graphql`
Baseline main: `7874eebaba68aba207c0a4082a6b08ce65bc03bd`

## Progress status

- 49/100 — LIVE PASS.
- 50/100 — LIVE PASS.
- 51/100 — LIVE PASS.
- 52/100 — LIVE PASS.
- 53/100 — LIVE PASS.
- 54/100 — LIVE PASS.
- 55/100 — LIVE PASS.
- 56/100 — LIVE PASS.
- 57/100 — LIVE PASS.
- 58/100 — LIVE PASS.
- 59/100 — LIVE PASS.

## Step 59 — storage abstraction + SQLite local control-plane backend

Step 59 begins the storage/context architecture adopted after the JSON durability semantics were proven live.

The architecture separates:

- Firebase / Firestore: distributed telemetry and observations;
- SQLite embedded in dserver: accepted local control-plane state;
- GraphQL: future unified context/query API over both sources, not a database and not part of the reconciliation mutation path.

The existing Firebase client and SmartSentinel/dserver REST contracts are unchanged in step 59.

## SQLite runtime configuration

New environment variable:

`DATUM_CONTROL_STATE_DB_PATH`

When omitted, JSON persistence from steps 56–58 remains active.

When present, Resource Registry and Operational D-Graph persistence route through the configured SQLite database.

The path is intentionally runtime-configurable so a Windows dserver may store the operational database outside the repository or a synchronized OneDrive tree.

Recommended live path:

`C:\DatumState\datum-control-plane.db`

SQLite is embedded through `rusqlite` with the bundled SQLite library. It is not a separate daemon or network service; starting dserver opens/creates the configured database file directly.

## SQLite schema

New module:

`dserver/src/storage/control_state.rs`

Schema version:

`0.1.0`

Tables:

- `control_state_meta` — storage schema metadata;
- `control_state_current` — one current JSON payload per durable domain;
- `control_state_history` — append-only history for every persisted domain write.

Initial domains:

- `resource_registry`;
- `operational_dgraph`.

Each persistence operation transactionally updates the current domain payload and appends the same validated payload to history.

Database settings include:

- WAL journal mode;
- `synchronous=FULL`;
- foreign keys enabled;
- five-second busy timeout.

`rusqlite` uses its bundled SQLite feature so Windows does not require a separately installed SQLite library.

## Compatibility and migration

The validated JSON persistence implementations remain available as fallback/migration code.

With SQLite configured:

1. load the domain from SQLite if it exists;
2. otherwise load and validate the existing JSON state;
3. import the validated legacy state into SQLite;
4. return the imported state to the existing in-memory control-plane logic;
5. subsequent writes go only to SQLite.

Legacy JSON files are not deleted or overwritten during migration.

This means step 59 changes the persistence backend without changing:

- Resource Registry identity/binding semantics;
- active Operational D-Graph revision semantics;
- durable authorization lease semantics;
- revision regression protection;
- REST contracts.

## Inspection utility

New binary:

`control-state-inspect`

Usage:

`cargo run --manifest-path dserver/Cargo.toml --bin control-state-inspect -- --db <path>`

It reports:

- database path;
- storage schema version;
- current durable domains;
- update timestamps;
- history row count per domain.

This avoids requiring a standalone SQLite CLI on the Windows host.

## Live storage-migration validation — PASS

The live Windows migration proof completed successfully on 2026-09-06 using:

`C:\DatumState\datum-control-plane.db`

The validated step-58 Resource Registry/binding state and revision-4 Operational D-Graph state were imported from the existing JSON persistence into a new SQLite database.

After temporarily removing the legacy JSON files from their configured paths and restarting dserver, the control plane restored the same durable state from SQLite alone:

- Resource Registry remained bound to `fog-01`;
- the previously validated binding identity remained intact;
- Operational D-Graph revision 4 remained active;
- MQTT remained desired/running;
- the invalidated revision-3 authorization did not reappear.

The final SQLite inspector output was:

```text
control_state_db=C:\DatumState\datum-control-plane.db
schema_version=0.1.0
current_domain_count=2
domain=operational_dgraph updated_at_utc=2026-09-06T12:50:12.811547300+00:00 history_count=1
domain=resource_registry updated_at_utc=2026-09-06T12:51:25.701613900+00:00 history_count=2
```

The second Resource Registry history entry resulted from a subsequent accepted Resource Registry update, proving that the SQLite backend stores an append-only control-state history in addition to the current snapshot.

PASS therefore proves:

- automatic validated JSON -> SQLite bootstrap works;
- subsequent restart does not require the JSON files;
- control-plane binding and D-Graph semantics are unchanged;
- invalidated authorization remains fail-closed;
- accepted state transitions append to SQLite history.

## Scope boundary

Step 59 does not yet:

- move Firebase telemetry into SQLite;
- persist all operational evidence in SQLite;
- normalize all domain payloads into relational tables;
- expose GraphQL;
- alter SmartSentinel publish/reconciliation contracts.

GraphQL is an API/query layer rather than a third database. It is intentionally introduced only after this storage migration is proven, so it can compose the already stable SQLite control state and Firebase observation state without entering the reconciliation mutation path.

## Architecture statement

`SmartSentinel publishes distributed observation evidence -> Firebase preserves shared observation context; dserver accepts/governs bindings, D-Graph revisions, and authorization leases -> SQLite preserves accepted local control-plane truth; GraphQL composes both as a unified query context. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
