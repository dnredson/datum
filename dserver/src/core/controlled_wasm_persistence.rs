use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    controlled_dgraph_deployment::build_controlled_dgraph_activation,
    controlled_wasm_deployment::{
        report_execution_id, validate_controlled_live_report_against_manifest,
    },
    deployment_manifest::{validate_deployment_manifest, DeploymentManifest},
    dgraph::DGraph,
};

pub const CONTROLLED_WASM_PERSISTENCE_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControlledWasmPersistedState {
    pub persistence_version: String,
    #[serde(default)]
    pub source_dgraph: Option<DGraph>,
    #[serde(default)]
    pub active_manifest: Option<DeploymentManifest>,
    #[serde(default)]
    pub live_reports: Vec<Value>,
}

impl ControlledWasmPersistedState {
    pub fn empty() -> Self {
        Self {
            persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
            source_dgraph: None,
            active_manifest: None,
            live_reports: Vec::new(),
        }
    }
}

pub fn validate_controlled_wasm_persisted_state(
    state: &ControlledWasmPersistedState,
) -> Result<(), String> {
    if state.persistence_version != CONTROLLED_WASM_PERSISTENCE_VERSION {
        return Err(format!(
            "unsupported controlled WASM persistence_version '{}'; expected '{}'",
            state.persistence_version, CONTROLLED_WASM_PERSISTENCE_VERSION
        ));
    }

    match (&state.source_dgraph, &state.active_manifest) {
        (Some(source), Some(manifest)) => {
            let activation = build_controlled_dgraph_activation(source)
                .map_err(|error| format!("persisted source D-Graph is invalid: {error}"))?;
            if activation.manifest != *manifest {
                return Err(
                    "persisted active manifest does not match the manifest derived from persisted source D-Graph"
                        .to_string(),
                );
            }
        }
        (Some(_), None) => {
            return Err(
                "persisted source D-Graph requires its derived active manifest".to_string(),
            );
        }
        (None, Some(manifest)) => {
            validate_deployment_manifest(manifest)
                .map_err(|error| format!("persisted active manifest is invalid: {error}"))?;
        }
        (None, None) => {
            if !state.live_reports.is_empty() {
                return Err(
                    "persisted live reports require an active deployment manifest".to_string(),
                );
            }
            return Ok(());
        }
    }

    let manifest = state
        .active_manifest
        .as_ref()
        .expect("validated non-empty persisted state has a manifest");
    let mut execution_ids = HashSet::new();
    for report in &state.live_reports {
        validate_controlled_live_report_against_manifest(manifest, report)
            .map_err(|error| format!("persisted live report is invalid: {error}"))?;
        let execution_id = report_execution_id(report)
            .expect("validated live report always contains execution_id");
        if !execution_ids.insert(execution_id.to_string()) {
            return Err(format!(
                "persisted live report execution_id '{}' is duplicated",
                execution_id
            ));
        }
    }

    Ok(())
}

pub fn load_controlled_wasm_persisted_state(
    path: &Path,
) -> Result<Option<ControlledWasmPersistedState>, String> {
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
            "read controlled WASM persisted state '{}': {error}",
            candidate.display()
        )
    })?;
    let state: ControlledWasmPersistedState = serde_json::from_slice(&bytes).map_err(|error| {
        format!(
            "parse controlled WASM persisted state '{}': {error}",
            candidate.display()
        )
    })?;
    validate_controlled_wasm_persisted_state(&state)?;
    Ok(Some(state))
}

pub fn persist_controlled_wasm_state(
    path: &Path,
    state: &ControlledWasmPersistedState,
) -> Result<(), String> {
    validate_controlled_wasm_persisted_state(state)?;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "create controlled WASM persistence directory '{}': {error}",
                parent.display()
            )
        })?;
    }

    let temp = sibling_path(path, ".tmp")?;
    let backup = sibling_path(path, ".bak")?;
    let mut bytes = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("serialize controlled WASM persisted state: {error}"))?;
    bytes.push(b'\n');

    {
        let mut file = fs::File::create(&temp).map_err(|error| {
            format!(
                "create controlled WASM temporary state '{}': {error}",
                temp.display()
            )
        })?;
        file.write_all(&bytes).map_err(|error| {
            format!(
                "write controlled WASM temporary state '{}': {error}",
                temp.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "sync controlled WASM temporary state '{}': {error}",
                temp.display()
            )
        })?;
    }

    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| {
            format!(
                "remove stale controlled WASM state backup '{}': {error}",
                backup.display()
            )
        })?;
    }

    let had_primary = path.exists();
    if had_primary {
        fs::rename(path, &backup).map_err(|error| {
            format!(
                "move controlled WASM state '{}' to backup '{}': {error}",
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
            "activate controlled WASM persisted state '{}' from '{}': {error}",
            path.display(),
            temp.display()
        ));
    }

    // The primary state is already committed at this point. Backup cleanup is
    // deliberately best-effort so cleanup failure cannot make memory lag a
    // successfully persisted primary state.
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
        .ok_or_else(|| "controlled WASM persistence path requires a file name".to_string())?;
    Ok(path.with_file_name(format!("{file_name}{suffix}")))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use crate::core::deployment_manifest::{
        build_deployment_artifact_identity, DeploymentAssignment, DeploymentExecutionBoundary,
        DeploymentManifest, DeploymentRuntimeLimits, DeploymentStrategy, GraphDistribution,
        DEPLOYMENT_MANIFEST_VERSION,
    };

    use super::*;

    const SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn manifest() -> DeploymentManifest {
        DeploymentManifest {
            manifest_version: DEPLOYMENT_MANIFEST_VERSION.to_string(),
            manifest_id: "manifest-i36".to_string(),
            project_id: "project-i36".to_string(),
            strategy: DeploymentStrategy::Distributed,
            graph_distribution: GraphDistribution::NodeSlice,
            source_dgraph_id: None,
            assignments: vec![DeploymentAssignment {
                assignment_id: "fog-scale".to_string(),
                module_id: "fog-scale-module".to_string(),
                function_name: "fog_scale".to_string(),
                artifact: build_deployment_artifact_identity(
                    "fog-scale-artifact",
                    "http://127.0.0.1:8090/artifacts/fog-scale.wasm",
                    SHA,
                )
                .unwrap(),
                target_node: "fog-01".to_string(),
                limits: DeploymentRuntimeLimits {
                    fuel: Some(1_000_000),
                    timeout_ms: Some(1_000),
                },
                metadata: None,
            }],
            execution_boundary: DeploymentExecutionBoundary::default(),
            metadata: None,
        }
    }

    fn report(execution_id: &str) -> Value {
        json!({
            "execution_id": execution_id,
            "manifest_id": "manifest-i36",
            "project_id": "project-i36",
            "node_id": "fog-01",
            "assignment_id": "fog-scale",
            "module_id": "fog-scale-module",
            "function_name": "fog_scale",
            "artifact_content_id": format!("sha256:{SHA}"),
            "explicit_execute": true,
            "status": "executed",
            "integrity_verified": true,
            "wasmtime_compiled": true,
            "wasmtime_instantiated": true,
            "function_invocation_attempted": true,
            "guest_execution_performed": true,
            "wasm_guest_side_effects": false,
            "governed_execution_performed": false,
            "host_mutation_performed": false,
            "safe_to_auto_execute": false,
            "timeout_enforced": false,
            "acquisition": {
                "artifact_delivery_performed": true,
                "host_network_fetch_performed": true,
                "persistent_write_performed": false,
                "host_mutation_performed": false
            }
        })
    }

    fn temp_path() -> PathBuf {
        std::env::temp_dir().join(format!("datum-i36-{}.json", Uuid::new_v4()))
    }

    #[test]
    fn empty_state_is_valid() {
        validate_controlled_wasm_persisted_state(&ControlledWasmPersistedState::empty()).unwrap();
    }

    #[test]
    fn direct_manifest_and_report_round_trip() {
        let path = temp_path();
        let state = ControlledWasmPersistedState {
            persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
            source_dgraph: None,
            active_manifest: Some(manifest()),
            live_reports: vec![report("exec-1")],
        };
        persist_controlled_wasm_state(&path, &state).unwrap();
        let restored = load_controlled_wasm_persisted_state(&path)
            .unwrap()
            .expect("state should exist");
        assert_eq!(restored, state);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn duplicate_execution_id_is_rejected() {
        let state = ControlledWasmPersistedState {
            persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
            source_dgraph: None,
            active_manifest: Some(manifest()),
            live_reports: vec![report("exec-1"), report("exec-1")],
        };
        let error = validate_controlled_wasm_persisted_state(&state)
            .expect_err("duplicate execution id must fail");
        assert!(error.contains("duplicated"));
    }

    #[test]
    fn report_without_manifest_is_rejected() {
        let state = ControlledWasmPersistedState {
            persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
            source_dgraph: None,
            active_manifest: None,
            live_reports: vec![report("exec-1")],
        };
        assert!(validate_controlled_wasm_persisted_state(&state).is_err());
    }

    #[test]
    fn corrupt_primary_fails_closed() {
        let path = temp_path();
        fs::write(&path, b"{not-json").unwrap();
        let error = load_controlled_wasm_persisted_state(&path)
            .expect_err("corrupt primary must not restore");
        assert!(error.contains("parse controlled WASM persisted state"));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn backup_is_used_when_primary_is_missing() {
        let path = temp_path();
        let backup = sibling_path(&path, ".bak").unwrap();
        let state = ControlledWasmPersistedState {
            persistence_version: CONTROLLED_WASM_PERSISTENCE_VERSION.to_string(),
            source_dgraph: None,
            active_manifest: Some(manifest()),
            live_reports: Vec::new(),
        };
        let mut bytes = serde_json::to_vec_pretty(&state).unwrap();
        bytes.push(b'\n');
        fs::write(&backup, bytes).unwrap();
        let restored = load_controlled_wasm_persisted_state(&path)
            .unwrap()
            .expect("backup state should restore");
        assert_eq!(restored, state);
        let _ = fs::remove_file(backup);
    }
}
