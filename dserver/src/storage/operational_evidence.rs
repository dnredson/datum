use std::{
    fs,
    path::Path,
    time::Duration,
};

use rusqlite::{params, Connection, OptionalExtension};

use crate::core::operational_evidence::{
    validate_operational_evidence_event, OperationalEvidenceEvent,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOperationalEvidenceOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, Default)]
pub struct OperationalEvidenceFilter<'a> {
    pub project_id: &'a str,
    pub resource_id: Option<&'a str>,
    pub node_id: Option<&'a str>,
    pub dgraph_id: Option<&'a str>,
    pub dgraph_revision: Option<u64>,
    pub event_kind: Option<&'a str>,
    pub limit: u32,
}

pub fn append_operational_evidence(
    db_path: &Path,
    event: &OperationalEvidenceEvent,
) -> Result<AppendOperationalEvidenceOutcome, String> {
    validate_operational_evidence_event(event)?;
    let mut connection = open_database(db_path)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("begin SQLite operational evidence transaction: {error}"))?;

    let exists = transaction
        .query_row(
            "SELECT 1 FROM operational_evidence WHERE project_id = ?1 AND event_id = ?2",
            params![&event.project_id, &event.event_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| format!("check duplicate SQLite operational evidence event: {error}"))?
        .is_some();

    if exists {
        return Ok(AppendOperationalEvidenceOutcome::Duplicate);
    }

    let payload_json = serde_json::to_string(&event.payload)
        .map_err(|error| format!("serialize operational evidence payload: {error}"))?;
    let dgraph_revision = event
        .dgraph_revision
        .map(i64::try_from)
        .transpose()
        .map_err(|_| "operational evidence dgraph_revision exceeds SQLite INTEGER range".to_string())?;
    let sequence = i64::try_from(event.sequence)
        .map_err(|_| "operational evidence sequence exceeds SQLite INTEGER range".to_string())?;

    transaction
        .execute(
            "INSERT INTO operational_evidence(
                project_id,
                event_id,
                occurred_at_utc,
                resource_id,
                node_id,
                dgraph_id,
                dgraph_revision,
                sequence,
                event_kind,
                side_effects_performed,
                payload_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                &event.project_id,
                &event.event_id,
                &event.occurred_at_utc,
                &event.resource_id,
                event.node_id.as_deref(),
                event.dgraph_id.as_deref(),
                dgraph_revision,
                sequence,
                &event.event_kind,
                event.side_effects_performed,
                &payload_json,
            ],
        )
        .map_err(|error| format!("append SQLite operational evidence: {error}"))?;

    transaction
        .commit()
        .map_err(|error| format!("commit SQLite operational evidence transaction: {error}"))?;

    Ok(AppendOperationalEvidenceOutcome::Inserted)
}

pub fn load_operational_evidence(
    db_path: &Path,
    filter: OperationalEvidenceFilter<'_>,
) -> Result<Vec<OperationalEvidenceEvent>, String> {
    let project_id = filter.project_id.trim();
    if project_id.is_empty() {
        return Err("operational evidence project_id filter must not be empty".to_string());
    }
    if filter.limit == 0 {
        return Ok(Vec::new());
    }

    let connection = open_database(db_path)?;
    let dgraph_revision = filter
        .dgraph_revision
        .map(i64::try_from)
        .transpose()
        .map_err(|_| "operational evidence dgraph_revision filter exceeds SQLite INTEGER range".to_string())?;

    let mut statement = connection
        .prepare(
            "SELECT
                event_id,
                occurred_at_utc,
                project_id,
                resource_id,
                node_id,
                dgraph_id,
                dgraph_revision,
                sequence,
                event_kind,
                side_effects_performed,
                payload_json
             FROM operational_evidence
             WHERE project_id = ?1
               AND (?2 IS NULL OR resource_id = ?2)
               AND (?3 IS NULL OR node_id = ?3)
               AND (?4 IS NULL OR dgraph_id = ?4)
               AND (?5 IS NULL OR dgraph_revision = ?5)
               AND (?6 IS NULL OR event_kind = ?6)
             ORDER BY id DESC
             LIMIT ?7",
        )
        .map_err(|error| format!("prepare SQLite operational evidence query: {error}"))?;

    let rows = statement
        .query_map(
            params![
                project_id,
                normalized(filter.resource_id),
                normalized(filter.node_id),
                normalized(filter.dgraph_id),
                dgraph_revision,
                normalized(filter.event_kind),
                i64::from(filter.limit),
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, bool>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .map_err(|error| format!("query SQLite operational evidence: {error}"))?;

    let mut events = Vec::new();
    for row in rows {
        let (
            event_id,
            occurred_at_utc,
            project_id,
            resource_id,
            node_id,
            dgraph_id,
            dgraph_revision,
            sequence,
            event_kind,
            side_effects_performed,
            payload_json,
        ) = row.map_err(|error| format!("read SQLite operational evidence row: {error}"))?;

        let dgraph_revision = dgraph_revision
            .map(u64::try_from)
            .transpose()
            .map_err(|_| "SQLite operational evidence contains negative dgraph_revision".to_string())?;
        let sequence = u64::try_from(sequence)
            .map_err(|_| "SQLite operational evidence contains negative sequence".to_string())?;
        let payload = serde_json::from_str(&payload_json)
            .map_err(|error| format!("parse SQLite operational evidence payload: {error}"))?;

        let event = OperationalEvidenceEvent {
            schema_version: crate::core::operational_evidence::OPERATIONAL_EVIDENCE_SCHEMA_VERSION
                .to_string(),
            event_id,
            occurred_at_utc,
            project_id,
            resource_id,
            node_id,
            dgraph_id,
            dgraph_revision,
            sequence,
            event_kind,
            side_effects_performed,
            payload,
        };
        validate_operational_evidence_event(&event)?;
        events.push(event);
    }

    events.reverse();
    Ok(events)
}

pub fn operational_evidence_count(db_path: &Path, project_id: &str) -> Result<u64, String> {
    let connection = open_database(db_path)?;
    let count = connection
        .query_row(
            "SELECT COUNT(*) FROM operational_evidence WHERE project_id = ?1",
            params![project_id.trim()],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| format!("count SQLite operational evidence: {error}"))?;
    u64::try_from(count)
        .map_err(|_| "SQLite operational evidence count was negative".to_string())
}

fn normalized(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn open_database(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create SQLite operational evidence directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let connection = Connection::open(path).map_err(|error| {
        format!(
            "open SQLite operational evidence database '{}': {error}",
            path.display()
        )
    })?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| format!("configure SQLite operational evidence busy timeout: {error}"))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA foreign_keys=ON;

             CREATE TABLE IF NOT EXISTS operational_evidence (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 project_id TEXT NOT NULL,
                 event_id TEXT NOT NULL,
                 occurred_at_utc TEXT NOT NULL,
                 resource_id TEXT NOT NULL,
                 node_id TEXT,
                 dgraph_id TEXT,
                 dgraph_revision INTEGER,
                 sequence INTEGER NOT NULL CHECK(sequence > 0),
                 event_kind TEXT NOT NULL,
                 side_effects_performed INTEGER NOT NULL CHECK(side_effects_performed IN (0, 1)),
                 payload_json TEXT NOT NULL,
                 UNIQUE(project_id, event_id)
             );

             CREATE INDEX IF NOT EXISTS idx_operational_evidence_project_time
                 ON operational_evidence(project_id, occurred_at_utc);
             CREATE INDEX IF NOT EXISTS idx_operational_evidence_project_resource
                 ON operational_evidence(project_id, resource_id);
             CREATE INDEX IF NOT EXISTS idx_operational_evidence_project_node
                 ON operational_evidence(project_id, node_id);
             CREATE INDEX IF NOT EXISTS idx_operational_evidence_project_dgraph
                 ON operational_evidence(project_id, dgraph_id, dgraph_revision);
             CREATE INDEX IF NOT EXISTS idx_operational_evidence_project_kind
                 ON operational_evidence(project_id, event_kind);",
        )
        .map_err(|error| format!("initialize SQLite operational evidence schema: {error}"))?;

    Ok(connection)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn event(event_id: &str, sequence: u64, kind: &str) -> OperationalEvidenceEvent {
        OperationalEvidenceEvent {
            schema_version: crate::core::operational_evidence::OPERATIONAL_EVIDENCE_SCHEMA_VERSION
                .to_string(),
            event_id: event_id.to_string(),
            occurred_at_utc: format!("2026-09-06T12:00:0{sequence}Z"),
            project_id: "project-a".to_string(),
            resource_id: "resource:os:abc".to_string(),
            node_id: Some("fog-01".to_string()),
            dgraph_id: Some("dgraph:test".to_string()),
            dgraph_revision: Some(4),
            sequence,
            event_kind: kind.to_string(),
            side_effects_performed: false,
            payload: json!({"sequence": sequence}),
        }
    }

    #[test]
    fn sqlite_operational_evidence_persists_filters_and_rejects_duplicates() {
        let path = std::env::temp_dir().join(format!(
            "datum-operational-evidence-test-{}.db",
            Uuid::new_v4()
        ));
        let first = event("opev:first", 1, "observation");
        let second = event("opev:second", 2, "reconciliation");

        assert_eq!(
            append_operational_evidence(&path, &first).unwrap(),
            AppendOperationalEvidenceOutcome::Inserted
        );
        assert_eq!(
            append_operational_evidence(&path, &second).unwrap(),
            AppendOperationalEvidenceOutcome::Inserted
        );
        assert_eq!(
            append_operational_evidence(&path, &second).unwrap(),
            AppendOperationalEvidenceOutcome::Duplicate
        );
        assert_eq!(operational_evidence_count(&path, "project-a").unwrap(), 2);

        let all = load_operational_evidence(
            &path,
            OperationalEvidenceFilter {
                project_id: "project-a",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all, vec![first.clone(), second.clone()]);

        let filtered = load_operational_evidence(
            &path,
            OperationalEvidenceFilter {
                project_id: "project-a",
                event_kind: Some("reconciliation"),
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(filtered, vec![second]);

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("db-wal"));
        let _ = fs::remove_file(path.with_extension("db-shm"));
    }
}
