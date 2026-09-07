//! Read-only recovery of the accepted operational control-plane state.
//! A missing or invalid state is never repaired by publishing a new graph.
use std::{env, fs, path::{Path, PathBuf}};

use rusqlite::{Connection, OpenFlags, OptionalExtension};

use crate::{
    core::operational_dgraph_persistence::{
        validate_operational_dgraph_persisted_state, OperationalDGraphPersistedState,
    },
    storage::control_state::{configured_control_state_db_path, CONTROL_STATE_SCHEMA_VERSION, DOMAIN_OPERATIONAL_DGRAPH},
};

pub const ALLOW_EMPTY_STATE_ENV: &str = "DATUM_ALLOW_EMPTY_OPERATIONAL_DGRAPH_STATE";
pub const EXPECTED_PROJECT_ENV: &str = "DATUM_OPERATIONAL_DGRAPH_EXPECTED_PROJECT_ID";
pub const EXPECTED_ID_ENV: &str = "DATUM_OPERATIONAL_DGRAPH_EXPECTED_DGRAPH_ID";
pub const EXPECTED_REVISION_ENV: &str = "DATUM_OPERATIONAL_DGRAPH_EXPECTED_REVISION";

#[derive(Debug, Clone, Default)]
pub struct OperationalDGraphRecoveryPolicy {
    pub allow_empty: bool,
    pub expected_project_id: Option<String>,
    pub expected_dgraph_id: Option<String>,
    pub expected_revision: Option<u64>,
}

impl OperationalDGraphRecoveryPolicy {
    pub fn from_env() -> Result<Self, String> {
        let allow_empty = match env::var(ALLOW_EMPTY_STATE_ENV) {
            Ok(value) if value.trim().eq_ignore_ascii_case("true") => true,
            Ok(value) if value.trim().eq_ignore_ascii_case("false") || value.trim().is_empty() => false,
            Err(env::VarError::NotPresent) => false,
            Err(error) => return Err(format!("read {ALLOW_EMPTY_STATE_ENV}: {error}")),
            Ok(value) => return Err(format!("{ALLOW_EMPTY_STATE_ENV} must be true or false, received '{value}'")),
        };
        let expected_revision = match optional_env(EXPECTED_REVISION_ENV)? {
            Some(value) => Some(value.parse::<u64>().map_err(|error| format!("invalid {EXPECTED_REVISION_ENV}: {error}"))?),
            None => None,
        };
        let policy = Self {
            allow_empty,
            expected_project_id: optional_env(EXPECTED_PROJECT_ENV)?,
            expected_dgraph_id: optional_env(EXPECTED_ID_ENV)?,
            expected_revision,
        };
        policy.validate_configuration()?;
        Ok(policy)
    }

    fn validate_configuration(&self) -> Result<(), String> {
        if self.expected_revision == Some(0) {
            return Err(format!("{EXPECTED_REVISION_ENV} must be greater than zero"));
        }
        if self.expected_project_id.is_none()
            && (self.expected_dgraph_id.is_some() || self.expected_revision.is_some())
        {
            return Err(format!("{EXPECTED_PROJECT_ENV} is required when an expected D-Graph id or revision is configured"));
        }
        if self.allow_empty
            && (self.expected_project_id.is_some() || self.expected_dgraph_id.is_some() || self.expected_revision.is_some())
        {
            return Err(format!("{ALLOW_EMPTY_STATE_ENV} cannot be combined with an expected graph identity"));
        }
        Ok(())
    }

    pub fn validate(&self, state: Option<&OperationalDGraphPersistedState>) -> Result<(), String> {
        self.validate_configuration()?;
        // Validate every persisted record before considering the empty-bootstrap
        // exception. A corrupt state with no graphs must never become valid.
        if let Some(state) = state {
            validate_operational_dgraph_persisted_state(state)?;
        }
        let empty = state.is_none_or(|state| state.graphs.is_empty());
        if empty {
            if !self.allow_empty {
                return Err("operational D-Graph state is missing or empty; recovery refused. Check the configured SQLite/JSON path and restore the accepted state before startup. Empty bootstrap requires DATUM_ALLOW_EMPTY_OPERATIONAL_DGRAPH_STATE=true and no expected graph identity".to_string());
            }
            if state.is_some_and(|state| !state.authorizations.is_empty()) {
                return Err("empty operational D-Graph bootstrap cannot contain authorizations".to_string());
            }
            return Ok(());
        }
        let state = state.expect("nonempty state checked above");
        if let Some(project_id) = &self.expected_project_id {
            let graph = state.graphs.get(project_id).ok_or_else(|| format!("expected operational D-Graph project '{project_id}' is absent"))?;
            if let Some(expected_id) = &self.expected_dgraph_id {
                if &graph.dgraph_id != expected_id { return Err(format!("expected D-Graph id '{expected_id}', restored '{}'", graph.dgraph_id)); }
            }
            if let Some(expected_revision) = self.expected_revision {
                if graph.revision != expected_revision { return Err(format!("expected D-Graph revision {expected_revision}, restored {}", graph.revision)); }
            }
        }
        Ok(())
    }
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(Some(value.trim().to_string())),
        Ok(_) | Err(env::VarError::NotPresent) => Ok(None),
        Err(error) => Err(format!("read {name}: {error}")),
    }
}

pub fn load_configured_operational_dgraph_state(path: &Path) -> Result<Option<OperationalDGraphPersistedState>, String> {
    read_only_operational_dgraph_state(path, configured_control_state_db_path().as_deref())
}

/// No database creation, schema migration, legacy import, or state publication.
/// A configured SQLite database is authoritative: a missing domain is not
/// silently replaced with a different legacy JSON state. SQLite's normal
/// read-only mode is used, not immutable mode, so committed WAL data is visible.
pub fn read_only_operational_dgraph_state(
    json_path: &Path,
    db_path: Option<&Path>,
) -> Result<Option<OperationalDGraphPersistedState>, String> {
    let state = if let Some(db_path) = db_path {
        if !db_path.is_file() { return Ok(None); }
        let connection = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|error| format!("open operational control-state SQLite '{}' read-only: {error}", db_path.display()))?;
        connection.pragma_update(None, "query_only", "ON")
            .map_err(|error| format!("configure read-only SQLite: {error}"))?;
        connection.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| format!("configure SQLite read timeout: {error}"))?;
        // Pin the schema and payload reads to one SQLite snapshot.
        connection.execute_batch("BEGIN DEFERRED TRANSACTION")
            .map_err(|error| format!("begin read-only control-state snapshot: {error}"))?;
        let version: String = connection.query_row(
            "SELECT value FROM control_state_meta WHERE key = 'schema_version'", [], |row| row.get(0),
        ).map_err(|error| format!("read SQLite control-state schema version: {error}"))?;
        if version != CONTROL_STATE_SCHEMA_VERSION {
            return Err(format!("unsupported SQLite control-state schema_version '{version}'; expected '{CONTROL_STATE_SCHEMA_VERSION}'"));
        }
        let payload: Option<String> = connection.query_row(
            "SELECT payload_json FROM control_state_current WHERE domain = ?1",
            [DOMAIN_OPERATIONAL_DGRAPH], |row| row.get(0),
        ).optional().map_err(|error| format!("read SQLite operational D-Graph domain: {error}"))?;
        payload.map(|payload| serde_json::from_str(&payload)
            .map_err(|error| format!("parse SQLite operational D-Graph state: {error}"))).transpose()?
    } else {
        let candidate = if json_path.exists() { Some(json_path.to_path_buf()) } else {
            let backup = sibling_backup_path(json_path)?;
            backup.exists().then_some(backup)
        };
        match candidate {
            Some(candidate) => {
                let bytes = fs::read(&candidate).map_err(|error| format!("read operational D-Graph state '{}': {error}", candidate.display()))?;
                Some(serde_json::from_slice(&bytes).map_err(|error| format!("parse operational D-Graph state '{}': {error}", candidate.display()))?)
            }
            None => None,
        }
    };
    if let Some(state) = &state { validate_operational_dgraph_persisted_state(state)?; }
    Ok(state)
}

fn sibling_backup_path(path: &Path) -> Result<PathBuf, String> {
    let name = path.file_name().ok_or_else(|| "operational D-Graph state path requires a file name".to_string())?;
    let mut backup = name.to_os_string();
    backup.push(".bak");
    Ok(path.with_file_name(backup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operational_dgraph::{OperationalDGraph, OperationalNodeDesiredState, OPERATIONAL_DGRAPH_SCHEMA_VERSION};
    use crate::core::operational_dgraph_persistence::OPERATIONAL_DGRAPH_PERSISTENCE_VERSION;
    use rusqlite::params;
    use uuid::Uuid;

    fn state(revision: u64) -> OperationalDGraphPersistedState {
        let mut state = OperationalDGraphPersistedState::empty();
        state.graphs.insert("monitor-f49c8".to_string(), OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: "dgraph:o2-mqtt-role-1".to_string(), revision,
            project_id: "monitor-f49c8".to_string(),
            nodes: vec![OperationalNodeDesiredState { node_id: "fog-01".to_string(), desired_services: vec![] }],
            metadata: None,
        });
        state
    }
    fn strict() -> OperationalDGraphRecoveryPolicy {
        OperationalDGraphRecoveryPolicy { expected_project_id: Some("monitor-f49c8".to_string()), expected_dgraph_id: Some("dgraph:o2-mqtt-role-1".to_string()), expected_revision: Some(4), ..Default::default() }
    }
    #[test]
    fn c68_recovery_rejects_missing_empty_wrong_identity_and_revision() {
        assert!(strict().validate(None).is_err());
        assert!(strict().validate(Some(&OperationalDGraphPersistedState::empty())).is_err());
        assert!(strict().validate(Some(&state(3))).is_err());
        let mut wrong = state(4);
        wrong.graphs.get_mut("monitor-f49c8").unwrap().dgraph_id = "wrong".to_string();
        assert!(strict().validate(Some(&wrong)).is_err());
        assert!(strict().validate(Some(&state(4))).is_ok());
        assert!(OperationalDGraphRecoveryPolicy::default().validate(None).is_err());
    }
    #[test]
    fn c68_recovery_allows_only_explicit_unpinned_empty_bootstrap() {
        let policy = OperationalDGraphRecoveryPolicy { allow_empty: true, ..Default::default() };
        assert!(policy.validate(None).is_ok());
        assert!(policy.validate(Some(&OperationalDGraphPersistedState::empty())).is_ok());
        assert!(strict().validate(None).is_err());
    }
    #[test]
    fn c68_recovery_rejects_invalid_empty_state_and_ambiguous_policy() {
        let policy = OperationalDGraphRecoveryPolicy { allow_empty: true, ..Default::default() };
        let mut invalid = OperationalDGraphPersistedState::empty();
        invalid.persistence_version = "invalid".to_string();
        assert!(policy.validate(Some(&invalid)).is_err());
        invalid.persistence_version = OPERATIONAL_DGRAPH_PERSISTENCE_VERSION.to_string();
        invalid.authorizations.clear();
        assert!(policy.validate(Some(&invalid)).is_ok());
        let mut ambiguous = strict();
        ambiguous.allow_empty = true;
        assert!(ambiguous.validate(None).is_err());
        assert!(ambiguous.validate(Some(&state(4))).is_err());
        let mut no_project = OperationalDGraphRecoveryPolicy { expected_revision: Some(4), ..Default::default() };
        assert!(no_project.validate(None).is_err());
        no_project.expected_revision = Some(0);
        assert!(no_project.validate(Some(&state(4))).is_err());
    }
    #[test]
    fn c68_recovery_reads_sqlite_without_creating_or_migrating_state() {
        let dir = std::env::temp_dir().join(format!("datum-c68-recovery-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("control.db");
        let json = dir.join("legacy.json");
        let original = state(4);
        fs::write(&json, serde_json::to_vec(&original).unwrap()).unwrap();
        assert!(read_only_operational_dgraph_state(&json, Some(&db)).unwrap().is_none());
        assert!(!db.exists());
        {
            let connection = Connection::open(&db).unwrap();
            connection.execute_batch("CREATE TABLE control_state_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE control_state_current (domain TEXT PRIMARY KEY, payload_json TEXT NOT NULL, updated_at_utc TEXT NOT NULL);").unwrap();
            connection.execute("INSERT INTO control_state_meta VALUES ('schema_version', ?1)", [CONTROL_STATE_SCHEMA_VERSION]).unwrap();
            connection.execute("INSERT INTO control_state_current VALUES (?1, ?2, 'test')", params![DOMAIN_OPERATIONAL_DGRAPH, serde_json::to_string(&original).unwrap()]).unwrap();
        }
        let before = fs::read(&db).unwrap();
        let restored = read_only_operational_dgraph_state(&json, Some(&db)).unwrap().unwrap();
        strict().validate(Some(&restored)).unwrap();
        assert_eq!(restored, original);
        assert_eq!(fs::read(&db).unwrap(), before);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn c68_recovery_rejects_corrupt_database_without_falling_back_to_json() {
        let dir = std::env::temp_dir().join(format!("datum-c68-corrupt-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("control.db");
        let json = dir.join("legacy.json");
        fs::write(&db, b"not a sqlite database").unwrap();
        fs::write(&json, serde_json::to_vec(&state(4)).unwrap()).unwrap();
        assert!(read_only_operational_dgraph_state(&json, Some(&db)).is_err());
        assert_eq!(fs::read(&db).unwrap(), b"not a sqlite database");
        fs::remove_dir_all(dir).unwrap();
    }
}
