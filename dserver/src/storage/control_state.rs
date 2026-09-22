use std::{
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

pub const CONTROL_STATE_DB_PATH_ENV: &str = "DATUM_CONTROL_STATE_DB_PATH";
pub const DEFAULT_CONTROL_STATE_DB_PATH: &str = "datum-control-plane.db";
pub const CONTROL_STATE_SCHEMA_VERSION: &str = "0.1.0";
pub const DOMAIN_RESOURCE_REGISTRY: &str = "resource_registry";
pub const DOMAIN_OPERATIONAL_DGRAPH: &str = "operational_dgraph";
/// Stage 111 (ADR-0018): the D-Deploy proposal/acceptance lifecycle state
/// domain -- proposals, acceptances, active per-application D-Maps, and
/// their derived operational-projection revision counters, persisted as
/// ONE atomic blob per `persist_domain_json` call. Deliberately a NEW,
/// separate domain from `DOMAIN_OPERATIONAL_DGRAPH` (the pre-existing
/// legacy operational graph) -- Stage 111 never writes into that domain.
pub const DOMAIN_DDEPLOY_STATE: &str = "ddeploy_state";
/// D-Node structural authority (D-Node/D-Continuum foundation closure,
/// ADR-0019): the server-owned STRUCTURAL D-Node registry (platform/
/// capacity/D-Node-ABI declarations) DServer derives the authoritative
/// `datum.dcontinuum/1` from. Deliberately a SEPARATE domain from any
/// Sentinel lease/liveness state (`SentinelRegistrationRecord` is never
/// persisted at all today -- see `sentinel_registration.rs`) and from
/// `DOMAIN_RESOURCE_REGISTRY` (an unrelated hardware/OS-identity discovery
/// subsystem whose `resources` payload is an opaque, unresolvable
/// content-addressed `capacity_profile_id` digest, never a typed numeric
/// capacity -- see `dnode_registry.rs`'s module doc for the full audit
/// conclusion on why that subsystem was not reused here).
pub const DOMAIN_DNODE_REGISTRY: &str = "dnode_structural_registry";
/// Canonical D-Serv artifact / D-Code registry (ADR-0021): server-owned
/// canonical METADATA and digest IDENTITY for `datum.dserv-artifact/1`
/// descriptors. The module BYTES themselves are never persisted here -- they
/// live in the D-Node-local content-addressed module store (DATUM
/// `dcode::module_store`, operational, not control-plane state). Deliberately
/// a separate domain from `ServiceArtifact`'s own registry (ADR-0005:
/// Management WASM / container artifacts are never D-Code).
pub const DOMAIN_DCODE_REGISTRY: &str = "dcode_artifact_registry";
/// D-Call operational D-Node transport endpoint registry (ADR-0022):
/// server-owned OPERATIONAL routing configuration mapping a structural
/// `node_id` to a concrete transport address for cross-D-Node D-Call
/// delivery. Deliberately NOT part of `DOMAIN_DNODE_REGISTRY` -- endpoint
/// addresses are explicitly excluded from `CanonicalDNodeV1`/D-Continuum
/// structural authority (that stays host-independent); this is a separate,
/// purely operational domain, exactly analogous to how `dcode_registry`
/// keeps D-Code metadata separate from `ServiceArtifact`.
pub const DOMAIN_DCALL_ENDPOINT_REGISTRY: &str = "dcall_endpoint_registry";
/// D-Call delivery grant state (ADR-0022): short-lived, single-use,
/// server-issued delivery authorizations plus their redemption state. Only
/// `SHA-256(token)` is ever persisted here, never the raw bearer token
/// (`dcall_grant::DCallGrantRecord`'s own doc comment is the authoritative
/// statement of that invariant). Redeemed state is durable and MUST survive
/// a restart -- an already-redeemed grant must never become valid again
/// merely because the process reloaded (see `dcall_grant_replay_audit`/
/// `final-acceptance-gate.txt` in the review package for the regression
/// proving this).
pub const DOMAIN_DCALL_GRANT_STATE: &str = "dcall_grant_state";
/// DIoT runtime-binding registry (Phase 113B1, ADR-0023): server-owned
/// OPERATIONAL record of which DIoT placement a D-Call realization actually
/// resolves to at the transport layer -- broker URI, transport kind, and the
/// exact DMap/2 + DForward + artifact-authority identity the binding was
/// registered under. Deliberately a SEPARATE domain from
/// `DOMAIN_DCALL_ENDPOINT_REGISTRY` (that registry is HTTP-shaped, keyed by
/// `(node_id, transport)`, and has no notion of a DIoT placement or a
/// DForward-governed realization) and from canonical DMap/2 itself (which
/// carries only the caller-declared, format-checked-only
/// `DIoTArtifactBindingV2` -- never a resolved runtime broker address). No
/// MQTT network I/O is implemented against this domain yet; it exists purely
/// to make the governed binding identity durable ahead of that closure.
pub const DOMAIN_DIOT_RUNTIME_BINDING_REGISTRY: &str = "diot_runtime_binding_registry";

#[derive(Debug, Clone, PartialEq)]
pub struct ControlStateHistoryEntry {
    pub id: i64,
    pub domain: String,
    pub payload: Value,
    pub recorded_at_utc: String,
}

pub fn configured_control_state_db_path() -> Option<PathBuf> {
    env::var(CONTROL_STATE_DB_PATH_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn default_control_state_db_path() -> PathBuf {
    configured_control_state_db_path()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONTROL_STATE_DB_PATH))
}
pub fn load_domain_json<T: DeserializeOwned>(
    db_path: &Path,
    domain: &str,
) -> Result<Option<T>, String> {
    let connection = open_database(db_path)?;
    let payload = connection
        .query_row(
            "SELECT payload_json FROM control_state_current WHERE domain = ?1",
            params![domain],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("load SQLite control-state domain '{domain}': {error}"))?;

    payload
        .map(|payload| {
            serde_json::from_str(&payload)
                .map_err(|error| format!("parse SQLite control-state domain '{domain}': {error}"))
        })
        .transpose()
}

pub fn persist_domain_json<T: Serialize>(
    db_path: &Path,
    domain: &str,
    value: &T,
) -> Result<(), String> {
    let payload = serde_json::to_string(value)
        .map_err(|error| format!("serialize SQLite control-state domain '{domain}': {error}"))?;
    let now = Utc::now().to_rfc3339();
    let mut connection = open_database(db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("begin SQLite control-state transaction: {error}"))?;

    transaction
        .execute(
            "INSERT INTO control_state_current(domain, payload_json, updated_at_utc)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(domain) DO UPDATE SET
                 payload_json = excluded.payload_json,
                 updated_at_utc = excluded.updated_at_utc",
            params![domain, &payload, &now],
        )
        .map_err(|error| {
            format!("persist SQLite control-state current domain '{domain}': {error}")
        })?;

    transaction
        .execute(
            "INSERT INTO control_state_history(domain, payload_json, recorded_at_utc)
             VALUES (?1, ?2, ?3)",
            params![domain, &payload, &now],
        )
        .map_err(|error| {
            format!("append SQLite control-state history domain '{domain}': {error}")
        })?;

    transaction
        .commit()
        .map_err(|error| format!("commit SQLite control-state transaction: {error}"))?;
    Ok(())
}

pub fn domain_history_count(db_path: &Path, domain: &str) -> Result<u64, String> {
    let connection = open_database(db_path)?;
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM control_state_history WHERE domain = ?1",
            params![domain],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| {
            format!("count SQLite control-state history domain '{domain}': {error}")
        })?;
    u64::try_from(count).map_err(|_| {
        format!("SQLite control-state history domain '{domain}' returned a negative count")
    })
}

pub fn load_domain_history(
    db_path: &Path,
    domain: &str,
    limit: u32,
) -> Result<Vec<ControlStateHistoryEntry>, String> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let connection = open_database(db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT id, domain, payload_json, recorded_at_utc
             FROM control_state_history
             WHERE domain = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )
        .map_err(|error| format!("prepare SQLite control-state history query: {error}"))?;

    let rows = statement
        .query_map(params![domain, i64::from(limit)], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| {
            format!("query SQLite control-state history domain '{domain}': {error}")
        })?;

    let mut entries = Vec::new();
    for row in rows {
        let (id, row_domain, payload_json, recorded_at_utc) = row.map_err(|error| {
            format!("read SQLite control-state history domain '{domain}': {error}")
        })?;
        let payload = serde_json::from_str(&payload_json).map_err(|error| {
            format!("parse SQLite control-state history domain '{domain}' entry {id}: {error}")
        })?;
        entries.push(ControlStateHistoryEntry {
            id,
            domain: row_domain,
            payload,
            recorded_at_utc,
        });
    }

    Ok(entries)
}

pub fn list_current_domains(db_path: &Path) -> Result<Vec<(String, String)>, String> {
    let connection = open_database(db_path)?;
    let mut statement = connection
        .prepare(
            "SELECT domain, updated_at_utc
             FROM control_state_current
             ORDER BY domain",
        )
        .map_err(|error| format!("prepare SQLite control-state domain list: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("query SQLite control-state domain list: {error}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read SQLite control-state domain list: {error}"))
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create SQLite control-state directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let connection = Connection::open(path).map_err(|error| {
        format!(
            "open SQLite control-state database '{}': {error}",
            path.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("configure SQLite control-state busy timeout: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;

             CREATE TABLE IF NOT EXISTS control_state_meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS control_state_current (
                 domain TEXT PRIMARY KEY,
                 payload_json TEXT NOT NULL,
                 updated_at_utc TEXT NOT NULL
             );

             CREATE TABLE IF NOT EXISTS control_state_history (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 domain TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 recorded_at_utc TEXT NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_control_state_history_domain_time
                 ON control_state_history(domain, recorded_at_utc);",
        )
        .map_err(|error| format!("initialize SQLite control-state schema: {error}"))?;

    connection
        .execute(
            "INSERT OR IGNORE INTO control_state_meta(key, value) VALUES ('schema_version', ?1)",
            params![CONTROL_STATE_SCHEMA_VERSION],
        )
        .map_err(|error| format!("initialize SQLite control-state schema version: {error}"))?;

    let version = connection
        .query_row(
            "SELECT value FROM control_state_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("read SQLite control-state schema version: {error}"))?;
    if version != CONTROL_STATE_SCHEMA_VERSION {
        return Err(format!(
            "unsupported SQLite control-state schema_version '{version}'; expected '{CONTROL_STATE_SCHEMA_VERSION}'"
        ));
    }

    Ok(connection)
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use uuid::Uuid;

    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct SampleState {
        revision: u64,
        label: String,
    }

    #[test]
    fn sqlite_control_state_round_trips_and_keeps_history() {
        let path =
            std::env::temp_dir().join(format!("datum-control-state-test-{}.db", Uuid::new_v4()));
        let first = SampleState {
            revision: 1,
            label: "first".to_string(),
        };
        let second = SampleState {
            revision: 2,
            label: "second".to_string(),
        };

        persist_domain_json(&path, "sample", &first).unwrap();
        persist_domain_json(&path, "sample", &second).unwrap();

        let restored: SampleState = load_domain_json(&path, "sample")
            .unwrap()
            .expect("sample state should exist");
        assert_eq!(restored, second);
        assert_eq!(domain_history_count(&path, "sample").unwrap(), 2);

        let history = load_domain_history(&path, "sample", 10).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].payload["revision"], 2);
        assert_eq!(history[0].payload["label"], "second");
        assert_eq!(history[1].payload["revision"], 1);
        assert_eq!(history[1].payload["label"], "first");

        let latest_only = load_domain_history(&path, "sample", 1).unwrap();
        assert_eq!(latest_only.len(), 1);
        assert_eq!(latest_only[0].payload["revision"], 2);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("db-wal"));
        let _ = fs::remove_file(path.with_extension("db-shm"));
    }
}
