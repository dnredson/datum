use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::DateTime;
use serde::{Deserialize, Serialize};

use crate::{
    core::operational_dgraph::{
        validate_operational_dgraph, OperationalDGraph, OperationalReconciliationAuthorization,
        AUTHORIZATION_SCOPE_DURABLE,
    },
    storage::control_state::{
        configured_control_state_db_path, load_domain_json, persist_domain_json,
        DOMAIN_OPERATIONAL_DGRAPH,
    },
};

pub const OPERATIONAL_DGRAPH_PERSISTENCE_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraphPersistedState {
    pub persistence_version: String,
    #[serde(default)]
    pub graphs: BTreeMap<String, OperationalDGraph>,
    #[serde(default)]
    pub authorizations: BTreeMap<String, OperationalReconciliationAuthorization>,
}

impl OperationalDGraphPersistedState {
    pub fn empty() -> Self {
        Self {
            persistence_version: OPERATIONAL_DGRAPH_PERSISTENCE_VERSION.to_string(),
            graphs: BTreeMap::new(),
            authorizations: BTreeMap::new(),
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

    for (key, authorization) in &state.authorizations {
        let expected_key = authorization_key(&authorization.project_id, &authorization.node_id);
        if key != &expected_key {
            return Err(format!(
                "persisted operational authorization key '{}' does not match record key '{}'",
                key, expected_key
            ));
        }
        if authorization.persistence_scope != AUTHORIZATION_SCOPE_DURABLE {
            return Err(format!(
                "persisted operational authorization '{}' must use persistence_scope=durable",
                authorization.authorization_id
            ));
        }
        if !authorization.allow_host_mutation {
            return Err(format!(
                "persisted operational authorization '{}' must allow host mutation",
                authorization.authorization_id
            ));
        }
        let binding_id = authorization
            .binding_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                format!(
                    "persisted operational authorization '{}' requires binding_id",
                    authorization.authorization_id
                )
            })?;
        if binding_id.is_empty() {
            return Err("persisted operational authorization binding_id must not be empty".to_string());
        }
        let expires = authorization.expires_at_utc.as_deref().ok_or_else(|| {
            format!(
                "persisted operational authorization '{}' requires expires_at_utc",
                authorization.authorization_id
            )
        })?;
        DateTime::parse_from_rfc3339(expires).map_err(|error| {
            format!(
                "persisted operational authorization '{}' expires_at_utc is invalid: {error}",
                authorization.authorization_id
            )
        })?;
        DateTime::parse_from_rfc3339(&authorization.authorized_at_utc).map_err(|error| {
            format!(
                "persisted operational authorization '{}' authorized_at_utc is invalid: {error}",
                authorization.authorization_id
            )
        })?;

        let graph = state.graphs.get(&authorization.project_id).ok_or_else(|| {
            format!(
                "persisted operational authorization '{}' references missing project graph '{}'",
                authorization.authorization_id, authorization.project_id
            )
        })?;
        if authorization.dgraph_id != graph.dgraph_id
            || authorization.dgraph_revision != graph.revision
            || !graph
                .nodes
                .iter()
                .any(|node| node.node_id.trim() == authorization.node_id.trim())
        {
            return Err(format!(
                "persisted operational authorization '{}' does not match active graph identity/revision/node",
                authorization.authorization_id
            ));
        }
    }

    Ok(())
}

pub fn load_operational_dgraph_persisted_state(
    path: &Path,
) -> Result<Option<OperationalDGraphPersistedState>, String> {
    if let Some(db_path) = configured_control_state_db_path() {
        if let Some(state) = load_domain_json::<OperationalDGraphPersistedState>(
            &db_path,
            DOMAIN_OPERATIONAL_DGRAPH,
        )? {
            validate_operational_dgraph_persisted_state(&state)?;
            return Ok(Some(state));
        }

        if let Some(legacy) = load_operational_dgraph_json_state(path)? {
            validate_operational_dgraph_persisted_state(&legacy)?;
            persist_domain_json(&db_path, DOMAIN_OPERATIONAL_DGRAPH, &legacy)?;
            return Ok(Some(legacy));
        }
        return Ok(None);
    }

    load_operational_dgraph_json_state(path)
}

pub fn persist_operational_dgraph_state(
    path: &Path,
    state: &OperationalDGraphPersistedState,
) -> Result<(), String> {
    validate_operational_dgraph_persisted_state(state)?;

    if let Some(db_path) = configured_control_state_db_path() {
        return persist_domain_json(&db_path, DOMAIN_OPERATIONAL_DGRAPH, state);
    }

    persist_operational_dgraph_json_state(path, state)
}

fn load_operational_dgraph_json_state(
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

fn persist_operational_dgraph_json_state(
    path: &Path,
    state: &OperationalDGraphPersistedState,
) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
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

pub fn authorization_key(project_id: &str, node_id: &str) -> String {
    format!("{}|{}", project_id.trim(), node_id.trim())
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
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use crate::core::operational_dgraph::{
        OperationalDesiredService, OperationalNodeDesiredState,
        OperationalReconciliationAuthorization, OPERATIONAL_DGRAPH_SCHEMA_VERSION,
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

    fn durable_authorization() -> OperationalReconciliationAuthorization {
        let now = Utc::now();
        OperationalReconciliationAuthorization {
            authorization_id: "opauth:test".to_string(),
            project_id: "monitor-f49c8".to_string(),
            dgraph_id: "dgraph:o2-mqtt-role-1".to_string(),
            dgraph_revision: 3,
            node_id: "fog-01".to_string(),
            binding_id: Some("binding:test".to_string()),
            resource_id: Some("resource:test".to_string()),
            authorized_by: "operator:test".to_string(),
            authorized_at_utc: now.to_rfc3339(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            persistence_scope: AUTHORIZATION_SCOPE_DURABLE.to_string(),
            expires_at_utc: Some((now + Duration::minutes(10)).to_rfc3339()),
            artifact_bindings: Vec::new(),
            execution_pinning_mode: "required".to_string(),
            snapshot: Vec::new(),
        }
    }

    #[test]
    fn persisted_graph_key_must_match_project() {
        let mut state = OperationalDGraphPersistedState::empty();
        state.graphs.insert("wrong-project".to_string(), graph(3));
        assert!(validate_operational_dgraph_persisted_state(&state).is_err());
    }

    #[test]
    fn persisted_json_graph_round_trips_with_revision_and_durable_auth() {
        let path = std::env::temp_dir().join(format!(
            "datum-operational-dgraph-persistence-{}.json",
            Uuid::new_v4()
        ));
        let mut state = OperationalDGraphPersistedState::empty();
        state.graphs.insert("monitor-f49c8".to_string(), graph(3));
        let auth = durable_authorization();
        state
            .authorizations
            .insert(authorization_key(&auth.project_id, &auth.node_id), auth.clone());

        persist_operational_dgraph_json_state(&path, &state).unwrap();
        let restored = load_operational_dgraph_json_state(&path)
            .unwrap()
            .expect("state should exist");
        assert_eq!(restored, state);
        assert_eq!(restored.graphs["monitor-f49c8"].revision, 3);
        assert_eq!(restored.authorizations.len(), 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn process_scoped_authorization_cannot_be_persisted() {
        let mut state = OperationalDGraphPersistedState::empty();
        state.graphs.insert("monitor-f49c8".to_string(), graph(3));
        let mut auth = durable_authorization();
        auth.persistence_scope = "process".to_string();
        auth.expires_at_utc = None;
        state
            .authorizations
            .insert(authorization_key(&auth.project_id, &auth.node_id), auth);
        assert!(validate_operational_dgraph_persisted_state(&state).is_err());
    }
}
