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
- 59/100 — IMPLEMENTED, pending compile/live storage-migration validation.

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

## Planned live proof

1. stop dserver only when ready to build step 59;
2. build/tests without `--locked` once so the new bundled SQLite dependency updates `dserver/Cargo.lock`;
3. preserve the validated step-58 JSON files containing the durable Resource Registry/binding and revision-4 Operational D-Graph state;
4. configure `DATUM_CONTROL_STATE_DB_PATH` to a new empty database path outside OneDrive;
5. start dserver with the existing legacy JSON paths still configured;
6. query Resource Registry and Operational D-Graph so both domains initialize;
7. verify the SQLite inspector reports both `resource_registry` and `operational_dgraph` domains;
8. stop dserver;
9. temporarily rename the legacy JSON state files so they cannot be used;
10. restart dserver with only the existing SQLite database available;
11. confirm the same resource/binding identity and Operational D-Graph revision 4 are restored;
12. confirm authorization remains null because the revision-3 durable lease was invalidated in step 58;
13. publish an idempotent/current-state update or a later controlled mutation and verify the relevant SQLite history count increases;
14. restore/retain legacy JSON files only as recovery artifacts, not as the active backend.

PASS proves that the storage backend can change from atomic JSON files to embedded SQLite without changing the already validated control-plane semantics.

## Scope boundary

Step 59 does not yet:

- move Firebase telemetry into SQLite;
- persist all operational evidence in SQLite;
- normalize all domain payloads into relational tables;
- expose GraphQL;
- alter SmartSentinel publish/reconciliation contracts.

Those follow after the storage migration itself is live validated.

## Architecture statement

`SmartSentinel publishes distributed observation evidence -> Firebase preserves shared observation context; dserver accepts/governs bindings, D-Graph revisions, and authorization leases -> SQLite preserves accepted local control-plane truth; future GraphQL composes both as a unified query context. External AI remains responsible for placement/reasoning; DATUM remains the conceptual framework.`
