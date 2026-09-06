# Control-state, telemetry, and unified-context storage architecture

Date: 2026-09-06

## Purpose

This document defines the storage split adopted for the SmartSentinel / dserver architecture after the live durability work in steps 56–58.

The goal is to preserve operational continuity without forcing all distributed observations and all governed control-plane state into one database or one connectivity path.

## Roles

### Firebase / Firestore — distributed observations

Firebase remains the distributed telemetry and observation plane.

SmartSentinel may publish observations to Firebase even when it cannot directly reach dserver. dserver may later consume those observations through its existing Firebase integration.

Firebase therefore answers primarily:

`What has the distributed system observed?`

Examples include:

- raw and latest telemetry;
- latest node observations;
- remotely available evidence used by passive or active analysis;
- observation history that must remain accessible when the Sentinel and dserver are not directly connected.

The existing Firebase client and existing telemetry contracts remain unchanged by step 59.

### SQLite inside dserver — accepted local control-plane state

SQLite is the local durable storage for state that dserver has explicitly accepted as operational/control-plane truth.

SQLite therefore answers primarily:

`What has the control plane accepted?`

Initial domains are:

- Resource Registry + binding state;
- active Operational D-Graph + revision;
- durable revision/binding-scoped authorization leases.

Future domains may include durable operational evidence and materialized context.

SQLite is embedded in the dserver process. No independent database daemon is required. The database location is supplied at runtime through:

`DATUM_CONTROL_STATE_DB_PATH`

Example on Windows:

`C:\Users\dnred\OneDrive\projetos\PD\dserver\datum-control-plane.db`

### GraphQL — unified context query plane

GraphQL is not a database and is not placed in the SmartSentinel reconciliation path.

It will be exposed by dserver as a context/query API capable of composing:

- accepted control-plane state from SQLite;
- distributed/latest telemetry from Firebase;
- later materialized analysis/context.

GraphQL therefore answers primarily:

`Show the accepted desired/control state together with the observed distributed state.`

Primary consumers are expected to be:

- dashboard;
- AI / GNN reasoning components;
- research/debug tooling;
- context extraction for experiments and papers.

## Control path remains unchanged

The operational mutation path remains deterministic and independent of GraphQL:

`SmartSentinel -> dserver REST contracts -> governed D-Graph slice/authorization -> local reconciliation`

GraphQL does not authorize, place, or reconcile workloads.

External AI remains responsible for placement/reasoning decisions. DATUM remains the conceptual framework. dserver remains the governance/control plane.

## Active and passive analysis modes

The storage split preserves both existing dserver analysis modes.

### Passive analysis

1. SmartSentinel publishes a new observation.
2. Firebase makes the observation available to dserver / notification logic.
3. dserver reacts to the new evidence.
4. Accepted control state used for correlation comes from local SQLite.

### Active analysis

1. dserver starts an analysis cycle.
2. dserver traverses the active D-Graph / governed entities from SQLite.
3. dserver queries the latest available telemetry/evidence for those entities from Firebase.
4. dserver evaluates freshness, missing/stale observations, and current desired state.

This preserves the distinction between:

- desired/accepted state;
- observed state;
- missing or stale evidence.

## Step-59 SQLite document-store foundation

Step 59 intentionally starts with a document-oriented SQLite schema rather than immediately normalizing every Rust type into relational tables.

Tables:

### `control_state_current`

One current JSON document per durable domain.

Fields:

- `domain` — primary key;
- `payload_json` — validated serialized state;
- `updated_at_utc`.

### `control_state_history`

Append-only history of every persisted domain update.

Fields:

- monotonic local row ID;
- `domain`;
- `payload_json`;
- `recorded_at_utc`.

### `control_state_meta`

Stores the SQLite control-state schema version.

The database uses:

- WAL journaling;
- `synchronous=FULL`;
- foreign keys enabled;
- bounded busy timeout;
- transactional current-state + history writes.

This foundation preserves the full validated state envelopes while making future historical analysis and schema migration possible.

## Legacy JSON migration and fallback

The step-56/57/58 JSON persistence files remain valid migration/recovery inputs.

When `DATUM_CONTROL_STATE_DB_PATH` is not set:

- legacy JSON persistence remains active exactly as before.

When `DATUM_CONTROL_STATE_DB_PATH` is set:

1. dserver first checks SQLite for the requested durable domain;
2. if SQLite already contains it, SQLite is authoritative;
3. if SQLite is empty and a valid legacy JSON state exists, dserver validates and imports that state into SQLite once;
4. subsequent writes go to SQLite;
5. the old JSON file remains untouched as a migration/recovery artifact.

This allows storage migration without changing the HTTP contracts or the semantics proven in steps 56–58.

## Failure semantics

Loss of Firebase connectivity must not erase or invalidate accepted local control-plane state.

Loss/corruption of the configured SQLite control-state database must fail closed for operations that require durable accepted state; dserver must not invent bindings, D-Graph revisions, or authorizations from telemetry.

A stale/missing Firebase observation must be represented as stale/missing evidence, not as a fabricated resource failure.

## Planned evolution

- 59 — storage abstraction + SQLite document/history backend + legacy migration.
- 60 — live migration/restart validation and normalized control-state read models where useful.
- 61–63 — durable operational evidence/history and context materialization.
- 64 — Firebase telemetry context adapter for explicit active/passive analysis inputs.
- 65 — GraphQL unified-context API and offline/restart validation.

The exact numbering may be refined as live validation exposes implementation boundaries.
