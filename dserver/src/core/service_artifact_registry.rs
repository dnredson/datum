use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::storage::control_state::{
    configured_control_state_db_path, load_domain_json, persist_domain_json,
};

pub const SERVICE_ARTIFACT_SCHEMA_VERSION: &str = "0.1.0";
pub const SERVICE_ARTIFACT_REGISTRY_DOMAIN: &str = "service_artifact_registry";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceArtifact {
    pub schema_version: String,
    pub artifact_id: String,
    pub service_id: String,
    pub runtime: ServiceRuntimeSpec,
    pub probes: ServiceProbeSet,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub control_plane: ServiceArtifactControlPlane,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceRuntimeSpec {
    pub kind: String,
    #[serde(default)]
    pub container: Option<ContainerDeploymentSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerDeploymentSpec {
    pub container_name: String,
    pub image: String,
    #[serde(default)]
    pub ports: Vec<ContainerPortMapping>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub volumes: Vec<ContainerVolumeMount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerVolumeMount {
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerPortMapping {
    pub container_port: u16,
    #[serde(default)]
    pub host_port: Option<u16>,
    #[serde(default = "default_tcp")]
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceProbeSet {
    pub execution: ServiceProbeSpec,
    #[serde(default)]
    pub health: Option<ServiceProbeSpec>,
    #[serde(default)]
    pub availability: Option<ServiceProbeSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceProbeSpec {
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub vantage_point: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ServiceArtifactControlPlane {
    pub project_id: String,
    pub name: String,
    pub version: String,
    pub artifact_kind: String,
    #[serde(default)]
    pub target_nodes: Vec<String>,
    #[serde(default)]
    pub target_stages: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub acquisition: ServiceArtifactAcquisition,
    #[serde(default)]
    pub lifecycle: ServiceArtifactLifecycle,
    #[serde(default)]
    pub health_checks: Vec<ServiceArtifactHealthCheck>,
    #[serde(default)]
    pub wasm_decision: ServiceArtifactWasmDecision,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
    #[serde(default)]
    pub created_at_utc: String,
    #[serde(default)]
    pub updated_at_utc: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceArtifactAcquisition {
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceArtifactLifecycle {
    #[serde(default)]
    pub pre_install: Vec<ServiceArtifactCommand>,
    #[serde(default)]
    pub install: Vec<ServiceArtifactCommand>,
    #[serde(default)]
    pub configure: Vec<ServiceArtifactCommand>,
    #[serde(default)]
    pub start: Vec<ServiceArtifactCommand>,
    #[serde(default)]
    pub stop: Vec<ServiceArtifactCommand>,
    #[serde(default)]
    pub post_install: Vec<ServiceArtifactCommand>,
    #[serde(default)]
    pub rollback: Vec<ServiceArtifactCommand>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceArtifactCommand {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceArtifactHealthCheck {
    pub kind: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub expected_status: Option<u16>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceArtifactWasmDecision {
    #[serde(default = "default_true")]
    pub explicit_invocation_required: bool,
    pub module_id: String,
    pub function_name: String,
    #[serde(default)]
    pub host_imports_allowed: bool,
    #[serde(default)]
    pub side_effects_allowed: bool,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub max_fuel: Option<u64>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

impl Default for ServiceArtifactWasmDecision {
    fn default() -> Self {
        Self {
            explicit_invocation_required: true,
            module_id: String::new(),
            function_name: String::new(),
            host_imports_allowed: false,
            side_effects_allowed: false,
            safe_to_auto_execute: false,
            max_fuel: None,
            timeout_ms: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ServiceArtifactRegistryState {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub artifacts_by_project: BTreeMap<String, Vec<ServiceArtifact>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceArtifactList {
    pub schema_version: String,
    pub project_id: String,
    pub count: usize,
    pub artifacts: Vec<ServiceArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceArtifactRegistryError {
    Invalid(String),
    Conflict(String),
    NotFound(String),
    Storage(String),
}

impl ServiceArtifactRegistryError {
    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(message)
            | Self::Conflict(message)
            | Self::NotFound(message)
            | Self::Storage(message) => message,
        }
    }
}

#[derive(Clone)]
pub struct ServiceArtifactRegistry {
    state: Arc<RwLock<ServiceArtifactRegistryState>>,
    db_path: Option<PathBuf>,
}

impl ServiceArtifactRegistry {
    pub fn load() -> Result<Self, String> {
        let path = configured_control_state_db_path().ok_or_else(|| {
            "DATUM_CONTROL_STATE_DB_PATH must be configured for durable service artifact registry persistence"
                .to_string()
        })?;
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self, String> {
        let state = load_domain_json::<ServiceArtifactRegistryState>(
            path,
            SERVICE_ARTIFACT_REGISTRY_DOMAIN,
        )?
        .unwrap_or_else(|| ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            ..Default::default()
        });
        validate_registry_state(&state).map_err(|error| error.message().to_string())?;
        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            db_path: Some(path.to_path_buf()),
        })
    }

    pub fn from_state(state: ServiceArtifactRegistryState) -> Self {
        Self {
            state: Arc::new(RwLock::new(state)),
            db_path: None,
        }
    }

    pub fn list(&self, project_id: &str) -> Result<ServiceArtifactList, String> {
        let state = self
            .state
            .read()
            .map_err(|_| "service artifact registry lock poisoned".to_string())?;
        let artifacts = state
            .artifacts_by_project
            .get(project_id)
            .cloned()
            .unwrap_or_default();
        Ok(ServiceArtifactList {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            project_id: project_id.to_string(),
            count: artifacts.len(),
            artifacts,
        })
    }

    pub fn get(
        &self,
        project_id: &str,
        artifact_id: &str,
    ) -> Result<ServiceArtifact, ServiceArtifactRegistryError> {
        let state = self.state.read().map_err(|_| {
            ServiceArtifactRegistryError::Storage("registry lock poisoned".to_string())
        })?;
        state
            .artifacts_by_project
            .get(project_id)
            .and_then(|items| items.iter().find(|item| item.artifact_id == artifact_id))
            .cloned()
            .ok_or_else(|| {
                ServiceArtifactRegistryError::NotFound(format!(
                    "service artifact '{artifact_id}' not found for project '{project_id}'"
                ))
            })
    }

    pub fn create(
        &self,
        project_id: &str,
        artifact: ServiceArtifact,
    ) -> Result<ServiceArtifact, ServiceArtifactRegistryError> {
        let artifact = normalize_artifact(project_id, artifact)?;
        self.mutate(|state| {
            let items = state
                .artifacts_by_project
                .entry(project_id.to_string())
                .or_default();
            if items
                .iter()
                .any(|item| item.artifact_id == artifact.artifact_id)
            {
                return Err(ServiceArtifactRegistryError::Conflict(format!(
                    "service artifact '{}' already exists for project '{}'",
                    artifact.artifact_id, project_id
                )));
            }
            items.push(artifact.clone());
            Ok(artifact)
        })
    }

    pub fn update(
        &self,
        project_id: &str,
        artifact_id: &str,
        mut artifact: ServiceArtifact,
    ) -> Result<ServiceArtifact, ServiceArtifactRegistryError> {
        if !artifact.artifact_id.trim().is_empty() && artifact.artifact_id != artifact_id {
            return Err(ServiceArtifactRegistryError::Invalid("project_id must not be empty".to_string()));
        }
        artifact.artifact_id = artifact_id.to_string();
        let artifact = normalize_artifact(project_id, artifact)?;

        self.mutate(|state| {
            let items = state
                .artifacts_by_project
                .get_mut(project_id)
                .ok_or_else(|| ServiceArtifactRegistryError::NotFound(format!(
                    "service artifact '{artifact_id}' not found for project '{project_id}'"
                )))?;
            let index = items
                .iter()
                .position(|item| item.artifact_id == artifact_id)
                .ok_or_else(|| ServiceArtifactRegistryError::NotFound(format!(
                    "service artifact '{artifact_id}' not found for project '{project_id}'"
                )))?;
            let created_at = items[index].control_plane.created_at_utc.clone();
            let mut updated = artifact.clone();
            updated.control_plane.created_at_utc = created_at;
            items[index] = updated.clone();
            Ok(updated)
        })
    }

    pub fn delete(
        &self,
        project_id: &str,
        artifact_id: &str,
    ) -> Result<(), ServiceArtifactRegistryError> {
        self.mutate(|state| {
            let remove_project;
            {
                let items = state
                    .artifacts_by_project
                    .get_mut(project_id)
                    .ok_or_else(|| ServiceArtifactRegistryError::NotFound(format!(
                        "service artifact '{artifact_id}' not found for project '{project_id}'"
                    )))?;
                let index = items
                    .iter()
                    .position(|item| item.artifact_id == artifact_id)
                    .ok_or_else(|| ServiceArtifactRegistryError::NotFound(format!(
                        "service artifact '{artifact_id}' not found for project '{project_id}'"
                    )))?;
                items.remove(index);
                remove_project = items.is_empty();
            }
            if remove_project {
                state.artifacts_by_project.remove(project_id);
            }
            Ok(())
        })
    }

    fn mutate<T>(
        &self,
        operation: impl FnOnce(
            &mut ServiceArtifactRegistryState,
        ) -> Result<T, ServiceArtifactRegistryError>,
    ) -> Result<T, ServiceArtifactRegistryError> {
        let mut state = self.state.write().map_err(|_| {
            ServiceArtifactRegistryError::Storage("registry lock poisoned".to_string())
        })?;
        let mut candidate = state.clone();
        let result = operation(&mut candidate)?;
        candidate.schema_version = SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
        if let Some(path) = &self.db_path {
            persist_domain_json(path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &candidate)
                .map_err(ServiceArtifactRegistryError::Storage)?;
        }
        *state = candidate;
        Ok(result)
    }
}

fn normalize_artifact(
    project_id: &str,
    mut artifact: ServiceArtifact,
) -> Result<ServiceArtifact, ServiceArtifactRegistryError> {
    if project_id.trim().is_empty() {
        return Err(ServiceArtifactRegistryError::Invalid("project_id must not be empty".to_string()));
    }
    artifact.control_plane.project_id = project_id.trim().to_string();
    if artifact.control_plane.name.trim().is_empty() {
        artifact.control_plane.name = artifact.service_id.clone();
    }
    if artifact.control_plane.version.trim().is_empty() {
        artifact.control_plane.version = "unversioned".to_string();
    }
    if artifact.control_plane.artifact_kind.trim().is_empty() {
        artifact.control_plane.artifact_kind = "container".to_string();
    }
    if artifact.control_plane.created_at_utc.trim().is_empty() {
        artifact.control_plane.created_at_utc = now_utc();
    }
    artifact.control_plane.updated_at_utc = now_utc();
    validate_artifact(&artifact)?;
    Ok(artifact)
}

fn validate_registry_state(
    state: &ServiceArtifactRegistryState,
) -> Result<(), ServiceArtifactRegistryError> {
    if state.schema_version != SERVICE_ARTIFACT_SCHEMA_VERSION {
        return Err(ServiceArtifactRegistryError::Invalid(format!(
            "unsupported service artifact registry schema_version '{}'",
            state.schema_version
        )));
    }
    for (project_id, artifacts) in &state.artifacts_by_project {
        for artifact in artifacts {
            if artifact.control_plane.project_id != *project_id {
                return Err(ServiceArtifactRegistryError::Invalid(format!(
                    "artifact '{}' project_id does not match registry scope",
                    artifact.artifact_id
                )));
            }
            validate_artifact(artifact)?;
        }
    }
    Ok(())
}

fn validate_artifact(artifact: &ServiceArtifact) -> Result<(), ServiceArtifactRegistryError> {
    if artifact.schema_version != SERVICE_ARTIFACT_SCHEMA_VERSION {
        return invalid("unsupported service artifact schema_version");
    }
    for (label, value) in [
        ("artifact_id", &artifact.artifact_id),
        ("service_id", &artifact.service_id),
        ("runtime.kind", &artifact.runtime.kind),
        ("control_plane.project_id", &artifact.control_plane.project_id),
        ("control_plane.name", &artifact.control_plane.name),
        ("control_plane.version", &artifact.control_plane.version),
        ("control_plane.artifact_kind", &artifact.control_plane.artifact_kind),
    ] {
        if value.trim().is_empty() {
            return invalid(format!("{label} must not be empty"));
        }
    }
    if !matches!(
        artifact.control_plane.artifact_kind.as_str(),
        "container" | "git" | "command_bundle"
    ) {
        return invalid("control_plane.artifact_kind must be container, git, or command_bundle");
    }
    if artifact.control_plane.target_nodes.is_empty()
        && artifact.control_plane.target_stages.is_empty()
    {
        return invalid("at least one control_plane.target_nodes or target_stages entry is required");
    }
    ensure_unique_non_empty(
        &artifact.control_plane.target_nodes,
        "control_plane.target_nodes",
    )?;
    ensure_unique_non_empty(
        &artifact.control_plane.target_stages,
        "control_plane.target_stages",
    )?;
    ensure_unique_non_empty(
        &artifact.control_plane.dependencies,
        "control_plane.dependencies",
    )?;
    if artifact
        .control_plane
        .dependencies
        .iter()
        .any(|item| item == &artifact.artifact_id)
    {
        return invalid("artifact cannot depend on itself");
    }

    let container = artifact
        .runtime
        .container
        .as_ref()
        .ok_or_else(|| ServiceArtifactRegistryError::Invalid(
            "container runtime requires runtime.container".to_string(),
        ))?;
    if container.container_name.trim().is_empty() || container.image.trim().is_empty() {
        return invalid("runtime.container requires container_name and image");
    }
    for port in &container.ports {
        if port.container_port == 0 || port.host_port == Some(0) {
            return invalid("runtime container ports must be non-zero");
        }
        if !matches!(port.protocol.as_str(), "tcp" | "udp") {
            return invalid("runtime container port protocol must be tcp or udp");
        }
    }

    validate_container_attachments(container)?;
    validate_probe("execution", &artifact.probes.execution)?;
    if let Some(probe) = &artifact.probes.health {
        validate_probe("health", probe)?;
    }
    if let Some(probe) = &artifact.probes.availability {
        validate_probe("availability", probe)?;
    }

    if artifact.control_plane.acquisition.kind.trim().is_empty() {
        return invalid("control_plane.acquisition.kind must not be empty");
    }
    if matches!(
        artifact.control_plane.artifact_kind.as_str(),
        "git" | "command_bundle"
    ) && artifact
        .control_plane
        .acquisition
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        return invalid(
            "control_plane.acquisition.source is required for git and command_bundle artifacts",
        );
    }
    if let Some(sha256) = &artifact.control_plane.acquisition.sha256 {
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return invalid(
                "control_plane.acquisition.sha256 must be a 64-character hexadecimal digest",
            );
        }
    }

    for commands in [
        &artifact.control_plane.lifecycle.pre_install,
        &artifact.control_plane.lifecycle.install,
        &artifact.control_plane.lifecycle.configure,
        &artifact.control_plane.lifecycle.start,
        &artifact.control_plane.lifecycle.stop,
        &artifact.control_plane.lifecycle.post_install,
        &artifact.control_plane.lifecycle.rollback,
    ] {
        for command in commands {
            validate_command(command)?;
        }
    }

    for check in &artifact.control_plane.health_checks {
        if !matches!(check.kind.as_str(), "tcp" | "http" | "command") {
            return invalid("control_plane health check kind must be tcp, http, or command");
        }
        if check.kind == "tcp" && check.port.unwrap_or(0) == 0 {
            return invalid("tcp health checks require a non-zero port");
        }
        if check.kind == "http"
            && check.path.as_deref().map(str::trim).unwrap_or("").is_empty()
        {
            return invalid("http health checks require path");
        }
    }

    let wasm = &artifact.control_plane.wasm_decision;
    if wasm.module_id.trim().is_empty() || wasm.function_name.trim().is_empty() {
        return invalid(
            "control_plane.wasm_decision.module_id and function_name are required",
        );
    }
    if !wasm.explicit_invocation_required
        || wasm.host_imports_allowed
        || wasm.side_effects_allowed
        || wasm.safe_to_auto_execute
    {
        return invalid(
            "control_plane.wasm_decision must require explicit invocation and forbid imports, effects, and auto-execution",
        );
    }
    ensure_unique_non_empty(&artifact.control_plane.secret_refs, "control_plane.secret_refs")?;
    for (key, value) in artifact
        .control_plane
        .bindings
        .iter()
        .chain(artifact.control_plane.secret_refs.iter().map(|value| (value, value)))
    {
        if key.trim().is_empty()
            || value.trim().is_empty()
            || key.contains('\0')
            || value.contains('\0')
        {
            return invalid("bindings and secret_refs must be non-empty and NUL-free");
        }
    }
    Ok(())
}

fn validate_container_attachments(
    container: &ContainerDeploymentSpec,
) -> Result<(), ServiceArtifactRegistryError> {
    if let Some(network) = container.network.as_deref() {
        if !is_safe_docker_name(network) {
            return invalid("runtime.container.network must be a non-empty Docker network name");
        }
    }
    for volume in &container.volumes {
        if !is_safe_docker_name(&volume.name) {
            return invalid("runtime.container.volumes name must be a non-empty Docker volume name");
        }
        if !volume.target.starts_with('/')
            || volume.target.chars().any(char::is_whitespace)
            || volume.target.contains(',')
        {
            return invalid(
                "runtime.container.volumes target must be an absolute path without whitespace or commas",
            );
        }
    }
    Ok(())
}

fn is_safe_docker_name(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-'))
        && value.chars().next().is_some_and(|character| character.is_ascii_alphanumeric())
}

fn validate_probe(
    dimension: &str,
    probe: &ServiceProbeSpec,
) -> Result<(), ServiceArtifactRegistryError> {
    let allowed = match dimension {
        "execution" => matches!(
            probe.kind.as_str(),
            "docker_container_running" | "process_running" | "systemd_active"
        ),
        "health" => {
            matches!(
                probe.kind.as_str(),
                "docker_health" | "tcp_connect" | "http" | "mqtt_connect"
            )
        }
        "availability" => {
            matches!(probe.kind.as_str(), "tcp_connect" | "http" | "mqtt_connect")
        }
        _ => false,
    };
    if !allowed {
        return invalid(format!(
            "unsupported {dimension} probe kind '{}'",
            probe.kind
        ));
    }
    Ok(())
}

fn validate_command(command: &ServiceArtifactCommand) -> Result<(), ServiceArtifactRegistryError> {
    if command.program.trim().is_empty() {
        return invalid("lifecycle command program must not be empty");
    }
    if command.program.contains('\0')
        || command.program.contains('/')
        || command.program.contains('\\')
    {
        return invalid("lifecycle command program must be an allowlist key, not a path");
    }
    if command.args.iter().any(|arg| arg.contains('\0')) {
        return invalid("lifecycle command arguments must be NUL-free");
    }
    Ok(())
}

fn ensure_unique_non_empty(
    values: &[String],
    label: &str,
) -> Result<(), ServiceArtifactRegistryError> {
    let mut seen = BTreeSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            return invalid(format!("{label} entries must not be empty"));
        }
        if !seen.insert(value.to_string()) {
            return invalid(format!("{label} entries must be unique"));
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> Result<(), ServiceArtifactRegistryError> {
    Err(ServiceArtifactRegistryError::Invalid(message.into()))
}

fn default_schema_version() -> String {
    SERVICE_ARTIFACT_SCHEMA_VERSION.to_string()
}

fn default_tcp() -> String {
    "tcp".to_string()
}

fn default_true() -> bool {
    true
}

fn now_utc() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_artifact() -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: "sma".to_string(),
            service_id: "sma".to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-sma".to_string(),
                    image: "datum/sma:dev".to_string(),
                    ports: Vec::new(),
                    network: None,
                    volumes: Vec::new(),
                }),
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "docker_container_running".to_string(),
                    params: BTreeMap::new(),
                    vantage_point: Some("local_host".to_string()),
                },
                health: None,
                availability: None,
            },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                project_id: "wrong-project".to_string(),
                name: "Smart Adapter".to_string(),
                version: "main".to_string(),
                artifact_kind: "git".to_string(),
                target_nodes: vec!["cloud-01".to_string()],
                target_stages: vec!["cloud".to_string()],
                dependencies: Vec::new(),
                acquisition: ServiceArtifactAcquisition {
                    kind: "git".to_string(),
                    source: Some("https://github.com/dnredson/SMA".to_string()),
                    revision: Some("main".to_string()),
                    sha256: None,
                },
                lifecycle: ServiceArtifactLifecycle {
                    install: vec![ServiceArtifactCommand {
                        program: "clone_git".to_string(),
                        args: vec!["SMA".to_string()],
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                health_checks: Vec::new(),
                wasm_decision: ServiceArtifactWasmDecision {
                    module_id: "reconcile_service".to_string(),
                    function_name: "decide".to_string(),
                    ..Default::default()
                },
                bindings: BTreeMap::new(),
                secret_refs: vec!["magistrala_api_token".to_string()],
                created_at_utc: String::new(),
                updated_at_utc: String::new(),
            },
        }
    }

    fn empty_registry() -> ServiceArtifactRegistry {
        ServiceArtifactRegistry::from_state(ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            ..Default::default()
        })
    }

    #[test]
    fn create_normalizes_project_and_timestamps() {
        let created = empty_registry().create("project-a", valid_artifact()).unwrap();
        assert_eq!(created.control_plane.project_id, "project-a");
        assert!(!created.control_plane.created_at_utc.is_empty());
        assert!(!created.control_plane.updated_at_utc.is_empty());
        assert_eq!(empty_registry().list("project-a").unwrap().count, 0);
    }

    #[test]
    fn unsafe_wasm_decision_is_rejected() {
        let mut artifact = valid_artifact();
        artifact.control_plane.wasm_decision.safe_to_auto_execute = true;
        assert!(matches!(
            empty_registry().create("project-a", artifact),
            Err(ServiceArtifactRegistryError::Invalid(_))
        ));
    }

    #[test]
    fn duplicate_target_nodes_are_rejected() {
        let mut artifact = valid_artifact();
        artifact.control_plane.target_nodes.push("cloud-01".to_string());
        assert!(matches!(
            normalize_artifact("project-a", artifact),
            Err(ServiceArtifactRegistryError::Invalid(_))
        ));
    }

    #[test]
    fn path_like_lifecycle_program_is_rejected() {
        let mut artifact = valid_artifact();
        artifact.control_plane.lifecycle.start.push(ServiceArtifactCommand {
            program: "/bin/sh".to_string(),
            ..Default::default()
        });
        assert!(matches!(
            normalize_artifact("project-a", artifact),
            Err(ServiceArtifactRegistryError::Invalid(_))
        ));
    }

    #[test]
    fn registry_persists_and_round_trips_with_sqlite() {
        let path = std::env::temp_dir().join(format!("datum-artifact-registry-{}.db", Uuid::new_v4()));
        let registry = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        registry.create("project-a", valid_artifact()).unwrap();

        let restored = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        assert_eq!(restored.list("project-a").unwrap().count, 1);
        assert_eq!(
            restored.get("project-a", "sma").unwrap().control_plane.acquisition.source.as_deref(),
            Some("https://github.com/dnredson/SMA")
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
