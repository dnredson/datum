use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::core::operational_dgraph::{validate_operational_dgraph, OperationalDGraph};

pub const OPERATIONAL_DGRAPH_PERSISTENCE_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraphPersistedState {
    pub persistence_version: String,
    #[serde(default)]
    pub graphs: BTreeMap<String, OperationalDGraph>,
}

impl OperationalDGraphPersistedState {
    pub fn empty() -> Self {
        Self {
            persistence_version: OPERATIONAL_DGRAPH_PERSISTENCE_VERSION.to_string(),
            graphs: BTreeMap::new(),
        }
    }
}

pub fn validate_operational_dgraph_persisted_state(
    state: &OperationalDGraphPersistedState,
) -> Result<(), String> {
    if state.persistence_version != OPERATIONAL_DGRAPH_PERSISTENCE_VERSION {
        return Err(format!(
            "unsupported operational D-Graph persistence_version '{}'; expected '{}'",
            state.persistence_version, OPERATIONAL_DGRAPH_PERSISTENCE_VERSION
        ));
    }

    for (project_key, graph) in &state.graphs {
        validate_operational_dgraph(graph).map_err(|error| {
            format!(
                "persisted operational D-Graph for project '{}' is invalid: {error}",
                project_key
            )
        })?;
        if project_key.trim() != graph.project_id.trim() {
            return Err(format!(
                "persisted operational D-Graph key '{}' does not match graph project_id '{}'",
                project_key, graph.project_id
            ));
        }
    }

    Ok(())
}

pub fn load_operational_dgraph_persisted_state(
    path: &Path,
) -> Result<Option<OperationalDGraphPersistedState>, String> {
    let candidate = if path.exists() {
        path.to_path_buf()
    } else {
        let backup = sibling_path(path, ".bak")?;
        if backup.exists() {
            backup
        } else {
            return Ok(None);
        }
    };

    let bytes = fs::read(&candidate).map_err(|error| {
        format!(
            "read operational D-Graph persisted state '{}': {error}",
            candidate.display()
        )
    })?;
    let state: OperationalDGraphPersistedState = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse operational D-Graph persisted state '{}': {error}",
            candidate.display()
        )
    })?;
    validate_operational_dgraph_persisted_state(&state)?;
    Ok(Some(state))
}

pub fn persist_operational_dgraph_state(
    path: &Path,
    state: &OperationalDGraphPersistedState,
) -> Result<(), String> {
    validate_operational_dgraph_persisted_state(state)?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create operational D-Graph persistence directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let temp = sibling_path(path, ".tmp")?;
    let backup = sibling_path(path, ".bak")?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("serialize operational D-Graph persisted state: {error}"))?;
    bytes.push(b'\n');

    {
        let mut file = fs::File::create(&temp).map_err(|error| {
            format!(
                "create operational D-Graph temporary state '{}': {error}",
                temp.display()
            )
        })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "write operational D-Graph temporary state '{}': {error}",
                temp.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "sync operational D-Graph temporary state '{}': {error}",
                temp.display()
            )
        })?;
    }

    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            format!(
                "remove stale operational D-Graph backup '{}': {error}",
                backup.display()
            )
        })?;
    }

    let had_primary = path.exists();
    if had_primary {
        fs::rename(path, &backup).map_err(|error| {
            format!(
                "move operational D-Graph state '{}' to backup '{}': {error}",
                path.display(),
                backup.display()
            )
        })?;
    }

    if let Err(error) = fs::rename(&temp, path) {
        if had_primary && !path.exists() && backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(format!(
            "activate operational D-Graph persisted state '{}' from '{}': {error}",
            path.display(),
            temp.display()
        ));
    }

    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }

    Ok(())
}

fn sibling_path(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "operational D-Graph persistence path requires a file name".to_string())?;
    Ok(path.with_file_name(format!("{file_name}{suffix}")))
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::core::operational_dgraph::{
        OperationalDesiredService, OperationalNodeDesiredState, OPERATIONAL_DGRAPH_SCHEMA_VERSION,
    };

    use super::*;

    fn graph(revision: u64) -> OperationalDGraph {
        OperationalDGraph {
            schema_version: OPERATIONAL_DGRAPH_SCHEMA_VERSION.to_string(),
            dgraph_id: "dgraph:o2-mqtt-role-1".to_string(),
            revision,
            project_id: "monitor-f49c8".to_string(),
            nodes: vec![OperationalNodeDesiredState {
                node_id: "fog-01".to_string(),
                desired_services: vec![OperationalDesiredService {
                    service_id: "mqtt".to_string(),
                    runtime: "container".to_string(),
                    desired_state: "running".to_string(),
                }],
            }],
            metadata: None,
        }
    }

    #[test]
    fn persisted_graph_key_must_match_project() {
        let mut state = OperationalDGraphPersistedState::empty();
        state.graphs.insert("wrong-project".to_string(), graph(3));
        assert!(validate_operational_dgraph_persisted_state(&state).is_err());
    }

    #[test]
    fn persisted_graph_round_trips_with_revision() {
        let path = std::env::temp_dir().join(format!(
            "datum-operational-dgraph-persistence-{}.json",
            Uuid::new_v4()
        ));
        let mut state = OperationalDGraphPersistedState::empty();
        state
            .graphs
            .insert("monitor-f49c8".to_string(), graph(3));

        persist_operational_dgraph_state(&path, &state).unwrap();
        let restored = load_operational_dgraph_persisted_state(&path)
            .unwrap()
            .expect("state should exist");
        assert_eq!(restored, state);
        assert_eq!(restored.graphs["monitor-f49c8"].revision, 3);
        let _ = fs::remove_file(path);
    }
}
