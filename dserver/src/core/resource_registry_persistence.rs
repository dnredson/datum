use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::core::{
    resource_binding::{ResourceRoleBindingRecord, RESOURCE_BINDING_SCHEMA_VERSION},
    resource_registry::{
        validate_resource_discovery_request, ResourceRegistryRecord,
        RESOURCE_REGISTRY_SCHEMA_VERSION,
    },
};

pub const RESOURCE_REGISTRY_PERSISTENCE_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceRegistryPersistedState {
    pub persistence_version: String,
    #[serde(default)]
    pub resources: BTreeMap<String, ResourceRegistryRecord>,
    #[serde(default)]
    pub bindings: BTreeMap<String, ResourceRoleBindingRecord>,
}

impl ResourceRegistryPersistedState {
    pub fn empty() -> Self {
        Self {
            persistence_version: RESOURCE_REGISTRY_PERSISTENCE_VERSION.to_string(),
            resources: BTreeMap::new(),
            bindings: BTreeMap::new(),
        }
    }
}

pub fn validate_resource_registry_persisted_state(
    state: &ResourceRegistryPersistedState,
) -> Result<(), String> {
    if state.persistence_version != RESOURCE_REGISTRY_PERSISTENCE_VERSION {
        return Err(format!(
            "unsupported resource registry persistence_version '{}'; expected '{}'",
            state.persistence_version, RESOURCE_REGISTRY_PERSISTENCE_VERSION
        ));
    }

    for (key, resource) in &state.resources {
        if resource.registry_schema_version != RESOURCE_REGISTRY_SCHEMA_VERSION {
            return Err(format!(
                "persisted resource '{}' has unsupported registry_schema_version '{}'",
                resource.resource_id, resource.registry_schema_version
            ));
        }
        validate_resource_discovery_request(&resource.discovery)
            .map_err(|error| format!("persisted resource '{}' is invalid: {error}", resource.resource_id))?;
        let expected_key = registry_key(&resource.project_id, &resource.resource_id);
        if key != &expected_key {
            return Err(format!(
                "persisted resource key '{}' does not match record key '{}'",
                key, expected_key
            ));
        }
        match (resource.state.as_str(), resource.assigned_node_id.as_deref()) {
            ("available_unbound", None) | ("bound", Some(_)) => {}
            (state, assigned) => {
                return Err(format!(
                    "persisted resource '{}' has inconsistent state='{}' assigned_node_id={:?}",
                    resource.resource_id, state, assigned
                ));
            }
        }
    }

    let mut active_nodes = BTreeSet::new();
    for (key, binding) in &state.bindings {
        if binding.binding_schema_version != RESOURCE_BINDING_SCHEMA_VERSION {
            return Err(format!(
                "persisted binding '{}' has unsupported binding_schema_version '{}'",
                binding.binding_id, binding.binding_schema_version
            ));
        }
        let expected_key = binding_key(&binding.project_id, &binding.binding_id);
        if key != &expected_key {
            return Err(format!(
                "persisted binding key '{}' does not match record key '{}'",
                key, expected_key
            ));
        }
        if binding.status == "active" {
            let node_key = format!("{}|{}", binding.project_id.trim(), binding.node_id.trim());
            if !active_nodes.insert(node_key.clone()) {
                return Err(format!(
                    "persisted state contains more than one active binding for node '{}' in project '{}'",
                    binding.node_id, binding.project_id
                ));
            }
            let resource = state
                .resources
                .get(&registry_key(&binding.project_id, &binding.resource_id))
                .ok_or_else(|| {
                    format!(
                        "active binding '{}' references missing resource '{}'",
                        binding.binding_id, binding.resource_id
                    )
                })?;
            if resource.state != "bound"
                || resource.assigned_node_id.as_deref() != Some(binding.node_id.as_str())
            {
                return Err(format!(
                    "active binding '{}' is inconsistent with resource '{}' registry state",
                    binding.binding_id, binding.resource_id
                ));
            }
        }
    }

    for resource in state.resources.values().filter(|resource| resource.state == "bound") {
        let matching = state
            .bindings
            .values()
            .filter(|binding| {
                binding.project_id == resource.project_id
                    && binding.resource_id == resource.resource_id
                    && binding.status == "active"
                    && Some(binding.node_id.as_str()) == resource.assigned_node_id.as_deref()
            })
            .count();
        if matching != 1 {
            return Err(format!(
                "bound resource '{}' requires exactly one matching active binding, found {}",
                resource.resource_id, matching
            ));
        }
    }

    Ok(())
}

pub fn load_resource_registry_persisted_state(
    path: &Path,
) -> Result<Option<ResourceRegistryPersistedState>, String> {
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
            "read resource registry persisted state '{}': {error}",
            candidate.display()
        )
    })?;
    let state: ResourceRegistryPersistedState = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse resource registry persisted state '{}': {error}",
            candidate.display()
        )
    })?;
    validate_resource_registry_persisted_state(&state)?;
    Ok(Some(state))
}

pub fn persist_resource_registry_state(
    path: &Path,
    state: &ResourceRegistryPersistedState,
) -> Result<(), String> {
    validate_resource_registry_persisted_state(state)?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create resource registry persistence directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let temp = sibling_path(path, ".tmp")?;
    let backup = sibling_path(path, ".bak")?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("serialize resource registry persisted state: {error}"))?;
    bytes.push(b'\n');

    {
        let mut file = fs::File::create(&temp).map_err(|error| {
            format!(
                "create resource registry temporary state '{}': {error}",
                temp.display()
            )
        })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "write resource registry temporary state '{}': {error}",
                temp.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "sync resource registry temporary state '{}': {error}",
                temp.display()
            )
        })?;
    }

    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            format!(
                "remove stale resource registry backup '{}': {error}",
                backup.display()
            )
        })?;
    }

    let had_primary = path.exists();
    if had_primary {
        fs::rename(path, &backup).map_err(|error| {
            format!(
                "move resource registry state '{}' to backup '{}': {error}",
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
            "activate resource registry persisted state '{}' from '{}': {error}",
            path.display(),
            temp.display()
        ));
    }

    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }

    Ok(())
}

fn registry_key(project_id: &str, resource_id: &str) -> String {
    format!("{}|{}", project_id.trim(), resource_id.trim())
}

fn binding_key(project_id: &str, binding_id: &str) -> String {
    format!("{}|{}", project_id.trim(), binding_id.trim())
}

fn sibling_path(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "resource registry persistence path requires a file name".to_string())?;
    Ok(path.with_file_name(format!("{file_name}{suffix}")))
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use crate::core::{
        resource_binding::{build_initial_binding, BindResourceRequest},
        resource_registry::{
            build_resource_registry_record, ResourceDiscoveryRequest, ResourceIdentityEvidence,
        },
    };

    use super::*;

    fn discovery() -> ResourceDiscoveryRequest {
        ResourceDiscoveryRequest {
            schema_version: "0.4.0".to_string(),
            observed_at_utc: Utc::now().to_rfc3339(),
            instance_id: "instance-1".to_string(),
            observation_host_id: "host:test".to_string(),
            stage_hint: None,
            identity: ResourceIdentityEvidence {
                hostname: "fog-test".to_string(),
                sentinel_version: "0.1.0".to_string(),
                hardware_fingerprint_id: None,
                hardware_identity_basis: Vec::new(),
                hardware_identity_confidence: "unknown".to_string(),
                hardware_identity_attested: false,
                os_installation_id: Some(format!("osinst:{}", "a".repeat(64))),
                os_identity_basis: "machine_id".to_string(),
                role_binding_id: None,
                predecessor_binding_id: None,
            },
            platform: json!({"os_family":"linux","arch":"x86_64"}),
            resources: json!({"capacity_profile_id":format!("capacity:{}", "b".repeat(64))}),
            capabilities: Vec::new(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    fn bound_state() -> ResourceRegistryPersistedState {
        let mut resource = build_resource_registry_record(
            "project-a",
            discovery(),
            None,
            Utc::now(),
        )
        .unwrap();
        let binding = build_initial_binding(
            "project-a",
            &resource,
            &BindResourceRequest {
                node_id: "fog-01".to_string(),
                transition_kind: "initial_assignment".to_string(),
            },
            Utc::now(),
        )
        .unwrap();
        resource.state = "bound".to_string();
        resource.assigned_node_id = Some(binding.node_id.clone());

        let mut state = ResourceRegistryPersistedState::empty();
        state.resources.insert(
            registry_key(&resource.project_id, &resource.resource_id),
            resource,
        );
        state.bindings.insert(
            binding_key(&binding.project_id, &binding.binding_id),
            binding,
        );
        state
    }

    #[test]
    fn bound_state_requires_matching_binding() {
        let mut state = bound_state();
        assert!(validate_resource_registry_persisted_state(&state).is_ok());
        state.bindings.clear();
        assert!(validate_resource_registry_persisted_state(&state).is_err());
    }

    #[test]
    fn persisted_state_round_trips() {
        let path = std::env::temp_dir().join(format!(
            "datum-resource-registry-persistence-{}.json",
            Uuid::new_v4()
        ));
        let state = bound_state();
        persist_resource_registry_state(&path, &state).unwrap();
        let restored = load_resource_registry_persisted_state(&path)
            .unwrap()
            .expect("state should exist");
        assert_eq!(restored, state);
        let _ = fs::remove_file(path);
    }
}
