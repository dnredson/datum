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
    default_control_state_db_path, load_domain_json, persist_domain_json,
};

/// Per-artifact record shape version. Unchanged by the Stage 107A migration:
/// the new fields on `ServiceArtifact` / `ServiceArtifactControlPlane` are all
/// `#[serde(default)]`, and an old record is only ever reached after the
/// registry-envelope migration below has already run.
pub const SERVICE_ARTIFACT_SCHEMA_VERSION: &str = "0.1.0";
pub const SERVICE_ARTIFACT_REGISTRY_DOMAIN: &str = "service_artifact_registry";

/// `ServiceArtifactRegistryState` envelope schema version, Stage 107A.
/// Distinct from `SERVICE_ARTIFACT_SCHEMA_VERSION` on purpose (ADR-0011 §3
/// migration design): bumping this constant, while keeping
/// `validate_registry_state`'s strict-equality check, is what makes a
/// pre-107A binary refuse to load a migrated registry rather than silently
/// re-serializing it without the generation / high-water data.
pub const SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION: &str = "0.2.0";
/// The pre-Stage-107A envelope schema version, recognised on load so the
/// one-time migration can run.
const LEGACY_REGISTRY_STATE_SCHEMA_VERSION: &str = "0.1.0";

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
    /// Control-plane-declared Docker network aliases (Stage 107A, finding
    /// F2c). In `execution_pinning_mode = required` the agent must publish
    /// exactly these aliases and must not substitute a node-local alias
    /// (previously `mapping.service_name`); in `legacy_partial` an empty list
    /// here still permits the local substitution, recorded in evidence.
    #[serde(default)]
    pub network_aliases: Vec<String>,
    #[serde(default)]
    pub volumes: Vec<ContainerVolumeMount>,
    #[serde(default)]
    pub configuration_mounts: Vec<ContainerConfigurationMount>,
    #[serde(default)]
    pub command: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerVolumeMount {
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerConfigurationMount {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
    /// Expected exact-byte SHA-256 of the mounted file's content (Stage 107A,
    /// Section G). When present, the agent hashes the actual resolved bytes
    /// before mounting and blocks (`config_content_mismatch`) on any
    /// difference; path/target equality alone is never sufficient. When
    /// absent or present, the mutable bind mount remains an unpinned external
    /// input: current-byte verification does not close bind-mount TOCTOU.
    #[serde(default)]
    pub declared_content_sha256: Option<String>,
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
    /// Control-plane-declared secret target bindings (Stage 107A, finding
    /// F2c). Replaces node-local origination of the target environment
    /// variable name for `execution_pinning_mode = required`. Kept alongside
    /// `secret_refs` for backward compatibility; when a `secret_refs` entry
    /// has no corresponding `secret_bindings` entry, `required` mode reports
    /// it as an unpinned/substituted input rather than silently trusting the
    /// node-local `DispatchSecretBinding.environment_variable`.
    #[serde(default)]
    pub secret_bindings: Vec<ServiceArtifactSecretBinding>,
    #[serde(default)]
    pub created_at_utc: String,
    #[serde(default)]
    pub updated_at_utc: String,
    /// Server-owned (Stage 107A, D-7a). Always recomputed by the registry on
    /// create/update; any client-supplied value is ignored, never trusted.
    #[serde(default)]
    pub execution_generation: u64,
    /// Server-owned (Stage 107A). SHA-256 (hex) of the RFC 8785 JCS
    /// serialization of this artifact's `ArtifactExecutionProjectionV1`.
    /// Always recomputed by the registry; any client-supplied value is
    /// ignored, never trusted.
    #[serde(default)]
    pub execution_projection_digest: String,
}

/// A control-plane-declared secret target binding (Stage 107A, Section D/G).
/// The `secret_ref` identifies which secret; `target_environment_variable`
/// names where its value is injected; `version`, when the secret backend
/// supports it, pins the secret's identity/version. The **value** is never
/// carried here, hashed, logged, or projected.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceArtifactSecretBinding {
    pub secret_ref: String,
    pub target_environment_variable: String,
    #[serde(default)]
    pub version: Option<String>,
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
    /// Registry-envelope schema version. Stage 107A: `"0.1.0"` on-load
    /// triggers the migration to `SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION`
    /// in `ServiceArtifactRegistry::load_from_path`; every in-memory state
    /// this process produces (fresh, migrated, or ordinarily mutated) carries
    /// the current envelope version.
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub artifacts_by_project: BTreeMap<String, Vec<ServiceArtifact>>,
    /// Durable per-`(project_id, artifact_id)` generation ceiling (Stage
    /// 107A, D-7a / DC-1). Survives DELETE so a later re-CREATE of the same
    /// `(project_id, artifact_id)` can never resurrect an earlier
    /// `execution_generation`. Never decreases.
    #[serde(default)]
    pub execution_generation_high_water: BTreeMap<String, BTreeMap<String, u64>>,
    /// Project-level `execution_pinning_mode`, changed only through an
    /// explicit recorded operator action (`set_project_pinning_mode`).
    /// A project with no entry here is treated as `required` (ADR-0011 N-2:
    /// "no silent legacy default").
    #[serde(default)]
    pub project_pinning_modes: BTreeMap<String, ProjectPinningModeRecord>,
}

/// Records the project's current `execution_pinning_mode` and the operator
/// action that set it (ADR-0011 N-2: "changed only through explicit recorded
/// operator action").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectPinningModeRecord {
    pub execution_pinning_mode: String,
    pub changed_by: String,
    pub changed_at_utc: String,
}

pub const EXECUTION_PINNING_MODE_LEGACY_PARTIAL: &str = "legacy_partial";
pub const EXECUTION_PINNING_MODE_REQUIRED: &str = "required";

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
        let path = default_control_state_db_path();
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &std::path::Path) -> Result<Self, String> {
        let loaded = load_domain_json::<ServiceArtifactRegistryState>(
            path,
            SERVICE_ARTIFACT_REGISTRY_DOMAIN,
        )?;
        let state = match loaded {
            None => {
                // No row exists yet: initialise directly at the current
                // envelope version. No migration is applicable.
                ServiceArtifactRegistryState {
                    schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
                    ..Default::default()
                }
            }
            Some(state) if state.schema_version == LEGACY_REGISTRY_STATE_SCHEMA_VERSION => {
                // Stage 107A migration (ADR-0011 §3): normalise every
                // artifact's execution_generation / execution_projection_digest
                // deterministically, seed the durable high-water map, bump the
                // envelope to the current schema version, and persist the
                // result IMMEDIATELY -- before this function returns and
                // before the server can serve any endpoint that could issue a
                // Stage-107A authorization. Idempotent: re-running against an
                // already-migrated state is a no-op (the guard above only
                // matches the legacy version).
                let migrated = migrate_legacy_registry_state(state)?;
                validate_registry_state(&migrated).map_err(|error| error.message().to_string())?;
                persist_domain_json(path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &migrated)?;
                migrated
            }
            Some(state) => state,
        };
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

    /// The project's current `execution_pinning_mode` (Stage 107A, N-2). A
    /// project with no recorded operator action defaults to `required` --
    /// "no silent legacy default" (ADR-0011 N-2).
    pub fn pinning_mode(&self, project_id: &str) -> Result<String, String> {
        self.with_stable_view(|state| pinning_mode_from_state(state, project_id))
    }

    /// P0-1 (review): holds the registry's READ lock for the entire
    /// duration of `f`. `create` / `update` / `delete` / `set_pinning_mode`
    /// all go through `mutate()`, which requires the WRITE lock -- under
    /// `std::sync::RwLock` semantics a writer blocks until every reader
    /// (including a `with_stable_view` caller) releases the lock, so no
    /// artifact mutation or pinning-mode change can be committed while an
    /// authorization built from a stable view is still being resolved.
    ///
    /// LOCK ORDERING (must be preserved by every caller): if a caller also
    /// holds `dserver::api::operational_dgraph::state_transaction()`, it
    /// MUST acquire that mutex FIRST and call this method while holding it
    /// -- never acquire this registry lock first and then reach for
    /// `state_transaction()`. `authorize_operational_reconciliation` is the
    /// only caller that combines the two and follows this order; nothing
    /// inside `mutate()` (the registry's own write path) ever touches
    /// `state_transaction()`, so this ordering cannot deadlock as long as
    /// it stays one-directional.
    pub fn with_stable_view<T>(
        &self,
        f: impl FnOnce(&ServiceArtifactRegistryState) -> T,
    ) -> Result<T, String> {
        let guard = self
            .state
            .read()
            .map_err(|_| "service artifact registry lock poisoned".to_string())?;
        Ok(f(&guard))
    }

    /// The SQLite control-state path this registry is backed by, when it
    /// has one (P0-2: used to decide `required`-mode split-persistence
    /// eligibility against the operational-authorization domain's own
    /// backend identity). `None` for an in-memory-only registry
    /// (`from_state`, used by tests), which is never persistence-eligible.
    pub fn backend_path(&self) -> Option<PathBuf> {
        self.db_path.clone()
    }

    /// Explicit recorded operator action changing a project's
    /// `execution_pinning_mode` (ADR-0011 N-2: "changed only through explicit
    /// recorded operator action"). Never silently invoked as a side effect of
    /// any other operation.
    pub fn set_pinning_mode(
        &self,
        project_id: &str,
        mode: &str,
        changed_by: &str,
    ) -> Result<ProjectPinningModeRecord, ServiceArtifactRegistryError> {
        if !matches!(mode, EXECUTION_PINNING_MODE_LEGACY_PARTIAL | EXECUTION_PINNING_MODE_REQUIRED) {
            return Err(ServiceArtifactRegistryError::Invalid(format!(
                "unsupported execution_pinning_mode '{mode}'; expected '{EXECUTION_PINNING_MODE_LEGACY_PARTIAL}' or '{EXECUTION_PINNING_MODE_REQUIRED}'"
            )));
        }
        if changed_by.trim().is_empty() {
            return Err(ServiceArtifactRegistryError::Invalid(
                "set_pinning_mode requires a non-empty changed_by (explicit recorded operator action)".to_string(),
            ));
        }
        let record = ProjectPinningModeRecord {
            execution_pinning_mode: mode.to_string(),
            changed_by: changed_by.trim().to_string(),
            changed_at_utc: now_utc(),
        };
        self.mutate(|state| {
            state
                .project_pinning_modes
                .insert(project_id.to_string(), record.clone());
            Ok(record.clone())
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
        let mut artifact = normalize_artifact(project_id, artifact)?;
        // Server-owned (D-7a, DC-5): any client-supplied generation/digest is
        // ignored, never trusted.
        let (_, digest) = crate::core::execution_projection::compute_projection_and_digest(&artifact)
            .map_err(ServiceArtifactRegistryError::Invalid)?;
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
            // D-7a / DC-1: generation is high_water(project, artifact_id) + 1,
            // even if this artifact_id was previously deleted -- the
            // high-water map is never cleared by delete, so a re-create can
            // never resurrect an earlier generation.
            let project_high_water = state
                .execution_generation_high_water
                .entry(project_id.to_string())
                .or_default();
            let generation = checked_next_generation(
                project_high_water.get(&artifact.artifact_id).copied().unwrap_or(0),
            )?;
            artifact.control_plane.execution_generation = generation;
            artifact.control_plane.execution_projection_digest = digest.clone();
            project_high_water.insert(artifact.artifact_id.clone(), generation);
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
            // P2 cleanup: this error previously reported "project_id must
            // not be empty", which is unrelated to the actual condition
            // (a body artifact_id that disagrees with the path artifact_id).
            return Err(ServiceArtifactRegistryError::Invalid(format!(
                "request body artifact_id '{}' does not match the path artifact_id '{artifact_id}'",
                artifact.artifact_id
            )));
        }
        artifact.artifact_id = artifact_id.to_string();
        let artifact = normalize_artifact(project_id, artifact)?;
        // Server-owned (D-7a, DC-5): compute the new content's projection
        // digest before touching generation bookkeeping; whether it changed
        // relative to the stored artifact decides increment vs. no-op.
        let (_, new_digest) = crate::core::execution_projection::compute_projection_and_digest(&artifact)
            .map_err(ServiceArtifactRegistryError::Invalid)?;

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
            let existing_digest = items[index].control_plane.execution_projection_digest.clone();
            let existing_generation = items[index].control_plane.execution_generation;

            let mut updated = artifact.clone();
            updated.control_plane.created_at_utc = created_at;

            if new_digest == existing_digest && existing_generation > 0 {
                // Semantic no-op PUT (D-7a): execution-relevant content is
                // unchanged (e.g. only control_plane.version, or only a
                // non-projected wasm_decision field, changed). Generation and
                // digest are carried over unchanged, not recomputed.
                updated.control_plane.execution_generation = existing_generation;
                updated.control_plane.execution_projection_digest = existing_digest;
            } else {
                // Content-changing PUT: increment exactly once, from the
                // durable high-water mark (never from the possibly-stale
                // in-place value alone), and advance the high-water mark.
                let project_high_water = state
                    .execution_generation_high_water
                    .entry(project_id.to_string())
                    .or_default();
                let baseline = project_high_water
                    .get(artifact_id)
                    .copied()
                    .unwrap_or(existing_generation);
                let generation = checked_next_generation(baseline.max(existing_generation))?;
                updated.control_plane.execution_generation = generation;
                updated.control_plane.execution_projection_digest = new_digest.clone();
                project_high_water.insert(artifact_id.to_string(), generation);
            }

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
                // DC-1: retain the high-water mark for (project_id,
                // artifact_id) even though the artifact record itself is
                // removed, so a later re-create cannot resurrect an earlier
                // generation. Defensive `max` in case the in-place generation
                // ever outran the tracked high-water (should not happen, but
                // the invariant must hold regardless).
                let removed_generation = items[index].control_plane.execution_generation;
                let project_high_water = state
                    .execution_generation_high_water
                    .entry(project_id.to_string())
                    .or_default();
                let current = project_high_water.get(artifact_id).copied().unwrap_or(0);
                project_high_water.insert(artifact_id.to_string(), current.max(removed_generation));

                items.remove(index);
                remove_project = items.is_empty();
            }
            if remove_project {
                // Only the artifact list entry is dropped. The project's
                // execution_generation_high_water sub-map is NEVER cleared
                // here -- that is the durable tombstone (DC-1).
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
        candidate.schema_version = SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string();
        if let Some(path) = &self.db_path {
            persist_domain_json(path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &candidate)
                .map_err(ServiceArtifactRegistryError::Storage)?;
        }
        *state = candidate;
        Ok(result)
    }
}

/// Pure lookup shared by `ServiceArtifactRegistry::pinning_mode` (its own
/// stable-view read) and by any other caller (e.g. the authorization handler,
/// P0-1) that already holds a `&ServiceArtifactRegistryState` from
/// `with_stable_view` and must read the pinning mode from that SAME view
/// rather than taking a second, separately-timed lock.
pub(crate) fn pinning_mode_from_state(state: &ServiceArtifactRegistryState, project_id: &str) -> String {
    state
        .project_pinning_modes
        .get(project_id)
        .map(|record| record.execution_pinning_mode.clone())
        .unwrap_or_else(|| EXECUTION_PINNING_MODE_REQUIRED.to_string())
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

/// Stage 107A migration (ADR-0011 §3): normalise a legacy ("0.1.0") registry
/// envelope. Deterministic and safe to run only against a state whose
/// envelope `schema_version` is exactly `LEGACY_REGISTRY_STATE_SCHEMA_VERSION`
/// (the caller in `load_from_path` guarantees this), so it is idempotent in
/// practice: it never runs twice against the same on-disk state, because the
/// immediate persist in `load_from_path` moves the envelope to
/// `SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION` before this function
/// could be reached again.
/// P2 cleanup: checked generation increment. `execution_generation` reaching
/// `u64::MAX` is astronomically unlikely in practice, but silently wrapping
/// to `0` would violate the D-7a monotonicity invariant this entire module
/// exists to protect (a wrapped-to-0 generation could then be resurrected by
/// a later, legitimately low generation) -- fail closed instead of wrapping.
fn checked_next_generation(current: u64) -> Result<u64, ServiceArtifactRegistryError> {
    current.checked_add(1).ok_or_else(|| {
        ServiceArtifactRegistryError::Invalid(
            "execution_generation exhausted (u64::MAX); refusing to wrap and violate D-7a monotonicity"
                .to_string(),
        )
    })
}

fn migrate_legacy_registry_state(
    mut state: ServiceArtifactRegistryState,
) -> Result<ServiceArtifactRegistryState, String> {
    let ServiceArtifactRegistryState {
        artifacts_by_project,
        execution_generation_high_water,
        ..
    } = &mut state;

    for (project_id, artifacts) in artifacts_by_project.iter_mut() {
        for artifact in artifacts.iter_mut() {
            let (_, digest) =
                crate::core::execution_projection::compute_projection_and_digest(artifact)?;
            let project_high_water = execution_generation_high_water
                .entry(project_id.clone())
                .or_default();
            let generation = checked_next_generation(
                project_high_water.get(&artifact.artifact_id).copied().unwrap_or(0),
            )
            .map_err(|error| error.message().to_string())?;
            artifact.control_plane.execution_generation = generation;
            artifact.control_plane.execution_projection_digest = digest;
            project_high_water.insert(artifact.artifact_id.clone(), generation);
        }
    }

    state.schema_version = SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string();
    Ok(state)
}

fn validate_registry_state(
    state: &ServiceArtifactRegistryState,
) -> Result<(), ServiceArtifactRegistryError> {
    if state.schema_version != SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION {
        return Err(ServiceArtifactRegistryError::Invalid(format!(
            "unsupported service artifact registry schema_version '{}'; expected '{}' (a pre-Stage-107A binary refuses a migrated registry rather than silently rewriting it)",
            state.schema_version, SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION
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
            // P2 cleanup: defensive D-7a monotonicity check. By the time
            // `validate_registry_state` runs (always after migration, if
            // any, already ran -- see `load_from_path`), every live artifact
            // must have a non-zero generation, and the durable high-water
            // mark for it must be at least that generation. A violation
            // here means the persisted state was corrupted or tampered with
            // outside the normal create/update/delete/migrate paths (all of
            // which maintain this invariant); refuse to load rather than
            // silently trusting a state that could let a later delete/
            // recreate resurrect an old generation.
            let generation = artifact.control_plane.execution_generation;
            if generation == 0 {
                return Err(ServiceArtifactRegistryError::Invalid(format!(
                    "artifact '{}' has execution_generation=0, which is never valid after migration",
                    artifact.artifact_id
                )));
            }
            let high_water = state
                .execution_generation_high_water
                .get(project_id)
                .and_then(|per_project| per_project.get(&artifact.artifact_id))
                .copied()
                .unwrap_or(0);
            if high_water < generation {
                return Err(ServiceArtifactRegistryError::Invalid(format!(
                    "artifact '{}' execution_generation={generation} exceeds its own durable high-water mark {high_water}; refusing to load a state that could resurrect an old generation",
                    artifact.artifact_id
                )));
            }
            let stored_digest = &artifact.control_plane.execution_projection_digest;
            let (_, recomputed_digest) = crate::core::execution_projection::compute_projection_and_digest(artifact)
                .map_err(ServiceArtifactRegistryError::Invalid)?;
            if !crate::core::jcs::is_canonical_digest_format(stored_digest) || *stored_digest != recomputed_digest {
                return invalid("execution_projection_digest must be a matching canonical sha256:<64 lowercase hex> object digest");
            }
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
    // P2 cleanup: `resolve_secret_target_environment_variable`
    // (DATUM/src/agent/stage107a_enforcement.rs) looks up a `secret_ref`'s
    // declared target with `.find(...)`, which silently returns only the
    // FIRST match. Two `secret_bindings` entries for the same `secret_ref`
    // -- whether they agree on `target_environment_variable` or not -- would
    // be an ambiguous, order-dependent declaration; reject it outright
    // rather than letting the agent's first-match resolution silently
    // decide which one "wins".
    {
        let mut seen_secret_refs = BTreeSet::new();
        for binding in &artifact.control_plane.secret_bindings {
            if binding.secret_ref.trim().is_empty() {
                return invalid("control_plane.secret_bindings entries require a non-empty secret_ref");
            }
            if binding.target_environment_variable.trim().is_empty() {
                return invalid("control_plane.secret_bindings entries require a non-empty target_environment_variable");
            }
            if !seen_secret_refs.insert(binding.secret_ref.trim().to_string()) {
                return invalid(format!(
                    "control_plane.secret_bindings has more than one entry for secret_ref '{}'; declare exactly one target_environment_variable per secret_ref",
                    binding.secret_ref
                ));
            }
        }
        // Two DIFFERENT secret_refs targeting the SAME environment variable
        // is equally ambiguous: `docker run --env VAR` would be pushed
        // twice with two different secret values, and which one the
        // container actually observes depends on argument order, not
        // declared intent.
        let mut seen_targets = BTreeSet::new();
        for binding in &artifact.control_plane.secret_bindings {
            let target = binding.target_environment_variable.trim().to_string();
            if artifact.control_plane.bindings.keys().any(|key| key.trim() == target) {
                return invalid(format!("secret target '{target}' collides with a non-secret control_plane.bindings key"));
            }
            if !seen_targets.insert(target.clone()) {
                return invalid(format!(
                    "control_plane.secret_bindings declares target_environment_variable '{target}' for more than one secret_ref"
                ));
            }
        }
    }
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
    if container.command.len() > 64 || container.command.iter().any(|arg| arg.len() > 4096 || arg.contains('\0')) {
        return invalid("invalid container command");
    }
    for mount in &container.configuration_mounts {
        if !is_safe_configuration_source(&mount.source) {
            return invalid(
                "runtime.container.configuration_mounts source must be a relative path without traversal",
            );
        }
        if !mount.target.starts_with('/')
            || mount.target.chars().any(char::is_whitespace)
            || mount.target.contains(',')
        {
            return invalid(
                "runtime.container.configuration_mounts target must be an absolute path without whitespace or commas",
            );
        }
        if !mount.read_only {
            return invalid("runtime.container.configuration_mounts must be read-only");
        }
    }
    Ok(())
}

fn is_safe_configuration_source(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 512
        && !value.starts_with('/')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '.')
        })
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
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

/// Serde-level default for `ServiceArtifactRegistryState.schema_version`,
/// used only when a deserialized payload is missing the key outright (should
/// not occur in practice; the field has always been written). Defaults to the
/// LEGACY envelope version, the safe choice: an ambiguous/malformed blob is
/// still routed through the Stage 107A migration in `load_from_path` rather
/// than being assumed already-current.
fn default_schema_version() -> String {
    LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string()
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
                    network_aliases: Vec::new(),
                    volumes: Vec::new(),
                    configuration_mounts: Vec::new(),
                    command: Vec::new(),
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
                secret_bindings: Vec::new(),
                created_at_utc: String::new(),
                updated_at_utc: String::new(),
                execution_generation: 0,
                execution_projection_digest: String::new(),
            },
        }
    }

    fn empty_registry() -> ServiceArtifactRegistry {
        ServiceArtifactRegistry::from_state(ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
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

    // ---- P2 cleanups ---------------------------------------------------- //

    #[test]
    fn update_reports_the_real_artifact_id_mismatch_not_a_misleading_project_id_error() {
        let registry = empty_registry();
        registry.create("project-a", valid_artifact()).unwrap();
        let mut mismatched = valid_artifact();
        mismatched.artifact_id = "different".to_string();
        let error = registry.update("project-a", "sma", mismatched).unwrap_err();
        let message = error.message();
        assert!(message.contains("does not match the path artifact_id"), "{message}");
        assert!(!message.contains("project_id must not be empty"), "{message}");
    }

    #[test]
    fn checked_next_generation_normal_case() {
        assert_eq!(checked_next_generation(0).unwrap(), 1);
        assert_eq!(checked_next_generation(41).unwrap(), 42);
    }

    #[test]
    fn checked_next_generation_fails_closed_at_u64_max() {
        assert!(checked_next_generation(u64::MAX).is_err());
    }

    #[test]
    fn duplicate_secret_binding_for_the_same_secret_ref_is_rejected() {
        let mut artifact = valid_artifact();
        artifact.control_plane.secret_refs = vec!["token".to_string()];
        artifact.control_plane.secret_bindings = vec![
            ServiceArtifactSecretBinding { secret_ref: "token".to_string(), target_environment_variable: "TOKEN_A".to_string(), version: None },
            ServiceArtifactSecretBinding { secret_ref: "token".to_string(), target_environment_variable: "TOKEN_B".to_string(), version: None },
        ];
        assert!(matches!(
            normalize_artifact("project-a", artifact),
            Err(ServiceArtifactRegistryError::Invalid(_))
        ));
    }

    #[test]
    fn two_secret_refs_targeting_the_same_environment_variable_is_rejected() {
        let mut artifact = valid_artifact();
        artifact.control_plane.secret_refs = vec!["token_a".to_string(), "token_b".to_string()];
        artifact.control_plane.secret_bindings = vec![
            ServiceArtifactSecretBinding { secret_ref: "token_a".to_string(), target_environment_variable: "SECRET".to_string(), version: None },
            ServiceArtifactSecretBinding { secret_ref: "token_b".to_string(), target_environment_variable: "SECRET".to_string(), version: None },
        ];
        assert!(matches!(
            normalize_artifact("project-a", artifact),
            Err(ServiceArtifactRegistryError::Invalid(_))
        ));
    }

    #[test]
    fn distinct_secret_bindings_are_accepted() {
        let mut artifact = valid_artifact();
        artifact.control_plane.secret_refs = vec!["token_a".to_string(), "token_b".to_string()];
        artifact.control_plane.secret_bindings = vec![
            ServiceArtifactSecretBinding { secret_ref: "token_a".to_string(), target_environment_variable: "SECRET_A".to_string(), version: None },
            ServiceArtifactSecretBinding { secret_ref: "token_b".to_string(), target_environment_variable: "SECRET_B".to_string(), version: None },
        ];
        assert!(normalize_artifact("project-a", artifact).is_ok());
    }

    #[test]
    fn validate_registry_state_rejects_zero_generation_on_a_current_envelope() {
        let mut artifact = valid_artifact();
        artifact.control_plane.project_id = "project-a".to_string();
        artifact.control_plane.execution_generation = 0;
        let state = ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![artifact])].into_iter().collect(),
            ..Default::default()
        };
        assert!(matches!(validate_registry_state(&state), Err(ServiceArtifactRegistryError::Invalid(_))));
    }

    #[test]
    fn validate_registry_state_rejects_generation_above_its_own_high_water() {
        let mut artifact = valid_artifact();
        artifact.control_plane.project_id = "project-a".to_string();
        artifact.control_plane.execution_generation = 5;
        let state = ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![artifact])].into_iter().collect(),
            execution_generation_high_water: [(
                "project-a".to_string(),
                [("sma".to_string(), 3u64)].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        assert!(matches!(validate_registry_state(&state), Err(ServiceArtifactRegistryError::Invalid(_))));
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

    fn temp_db_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("datum-artifact-registry-{label}-{}.db", Uuid::new_v4()))
    }

    fn cleanup(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    // ---- Stage 107A: generation / digest / migration tests (K.1-K.7, J) --- //

    #[test]
    fn k1_first_artifact_generation_is_one() {
        let created = empty_registry().create("project-a", valid_artifact()).unwrap();
        assert_eq!(created.control_plane.execution_generation, 1);
        assert!(!created.control_plane.execution_projection_digest.is_empty());
    }

    #[test]
    fn k2_content_changing_put_increments_generation_exactly_once() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(created.control_plane.execution_generation, 1);

        let mut changed = created.clone();
        changed.runtime.container.as_mut().unwrap().command = vec!["--verbose".to_string()];
        let updated = registry.update("project-a", "sma", changed).unwrap();
        assert_eq!(updated.control_plane.execution_generation, 2);
        assert_ne!(
            updated.control_plane.execution_projection_digest,
            created.control_plane.execution_projection_digest
        );
    }

    #[test]
    fn k3_semantic_no_op_put_does_not_increment_generation() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();

        // version is evidence-only, excluded from the projection (ADR-0011 §1
        // design-review correction): a version-only PUT must not bump the
        // generation.
        let mut same_content = created.clone();
        same_content.control_plane.version = "v2-label-only".to_string();
        let updated = registry.update("project-a", "sma", same_content).unwrap();
        assert_eq!(updated.control_plane.execution_generation, 1);
        assert_eq!(
            updated.control_plane.execution_projection_digest,
            created.control_plane.execution_projection_digest
        );
    }

    #[test]
    fn k4_delete_recreate_generation_is_monotonic() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(created.control_plane.execution_generation, 1);

        registry.delete("project-a", "sma").unwrap();
        let recreated = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(
            recreated.control_plane.execution_generation, 2,
            "delete/recreate must never resurrect an earlier generation"
        );

        registry.delete("project-a", "sma").unwrap();
        let recreated_again = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(recreated_again.control_plane.execution_generation, 3);
    }

    #[test]
    fn k6_project_scopes_do_not_collide_on_generation() {
        let registry = empty_registry();
        let a = registry.create("project-a", valid_artifact()).unwrap();
        let mut other_project_artifact = valid_artifact();
        other_project_artifact.control_plane.project_id = "project-b".to_string();
        let b = registry.create("project-b", other_project_artifact).unwrap();
        assert_eq!(a.control_plane.execution_generation, 1);
        assert_eq!(
            b.control_plane.execution_generation, 1,
            "an unrelated project's artifact_id must start its own generation at 1"
        );
    }

    #[test]
    fn k7_content_a_then_b_then_a_again_never_resurrects_earlier_generation() {
        let registry = empty_registry();
        let a = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(a.control_plane.execution_generation, 1);

        let original_image = a.runtime.container.as_ref().unwrap().image.clone();
        let mut content_b = a.clone();
        content_b.runtime.container.as_mut().unwrap().image = format!("{original_image}-changed");
        let b = registry.update("project-a", "sma", content_b).unwrap();
        assert_eq!(b.control_plane.execution_generation, 2);
        let digest_a = a.control_plane.execution_projection_digest.clone();
        let digest_b = b.control_plane.execution_projection_digest.clone();
        assert_ne!(digest_a, digest_b);

        // Revert to content identical to `a`'s projection. Even though the
        // content digest now matches generation 1's digest again, the
        // generation itself must advance. New authorizations bind generation 3;
        // an already-issued generation-1 snapshot retains its finite lease,
        // and an expired lease cannot revive when the original bytes return.
        let mut content_a_again = b.clone();
        content_a_again.runtime.container.as_mut().unwrap().image = original_image;
        let a_again = registry.update("project-a", "sma", content_a_again).unwrap();
        assert_eq!(a_again.control_plane.execution_generation, 3);
        assert_eq!(
            a_again.control_plane.execution_projection_digest, digest_a,
            "content reverted to the original, so the digest matches generation 1's digest again"
        );
        assert_ne!(
            a_again.control_plane.execution_generation, 1,
            "new authorizations must bind the current generation, not resurrect generation 1"
        );
    }

    #[test]
    fn k5_restart_preserves_generation_and_high_water() {
        let path = temp_db_path("k5-restart");
        let registry = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        registry.create("project-a", valid_artifact()).unwrap();
        registry.delete("project-a", "sma").unwrap();
        registry.create("project-a", valid_artifact()).unwrap();
        drop(registry);

        let restored = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = restored.get("project-a", "sma").unwrap();
        assert_eq!(
            artifact.control_plane.execution_generation, 2,
            "restart must preserve the generation reached before the process exited"
        );

        // A further delete/recreate after restart must still advance past 2,
        // proving the durable high-water mark itself survived the restart,
        // not merely the last artifact's in-place generation value.
        restored.delete("project-a", "sma").unwrap();
        let recreated = restored.create("project-a", valid_artifact()).unwrap();
        assert_eq!(recreated.control_plane.execution_generation, 3);

        cleanup(&path);
    }

    #[test]
    fn empty_database_initializes_directly_at_current_schema_version() {
        let path = temp_db_path("empty-init");
        let registry = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        assert_eq!(registry.list("project-a").unwrap().count, 0);
        // A fresh create still starts generation at 1, proving no phantom
        // migration state leaked in.
        let created = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(created.control_plane.execution_generation, 1);
        cleanup(&path);
    }

    #[test]
    fn legacy_registry_migrates_to_current_schema_on_load_and_is_idempotent() {
        let path = temp_db_path("legacy-migrate");
        let mut legacy_artifact = valid_artifact();
        legacy_artifact.control_plane.project_id = "project-a".to_string();
        // Simulate a pre-Stage-107A record: no generation/digest ever set.
        legacy_artifact.control_plane.execution_generation = 0;
        legacy_artifact.control_plane.execution_projection_digest = String::new();
        let legacy_state = ServiceArtifactRegistryState {
            schema_version: LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![legacy_artifact])]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &legacy_state).unwrap();

        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "sma").unwrap();
        assert_eq!(
            artifact.control_plane.execution_generation, 1,
            "migration must deterministically back-fill generation 1 for untouched legacy data"
        );
        assert!(crate::core::jcs::is_canonical_digest_format(&artifact.control_plane.execution_projection_digest));
        drop(migrated);

        // Idempotency: loading the now-migrated database again must not
        // re-bump the generation.
        let reloaded = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact_again = reloaded.get("project-a", "sma").unwrap();
        assert_eq!(artifact_again.control_plane.execution_generation, 1);

        cleanup(&path);
    }

    #[test]
    fn migration_persists_immediately_before_further_use() {
        // Regression for the corrected design (finding 5): the migrated
        // state must be durably persisted by `load_from_path` itself, not
        // deferred to the next `mutate()`. A second, independent
        // `load_from_path` against the same file (simulating a process that
        // never writes) must already observe the migrated envelope version
        // and generation, with no write in between.
        let path = temp_db_path("migrate-persist-immediately");
        let mut legacy_artifact = valid_artifact();
        legacy_artifact.control_plane.project_id = "project-a".to_string();
        legacy_artifact.control_plane.execution_generation = 0;
        legacy_artifact.control_plane.execution_projection_digest = String::new();
        let legacy_state = ServiceArtifactRegistryState {
            schema_version: LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![legacy_artifact])]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &legacy_state).unwrap();

        let _first_loader = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        drop(_first_loader);

        let raw: ServiceArtifactRegistryState =
            load_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN)
                .unwrap()
                .expect("row must exist");
        assert_eq!(raw.schema_version, SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION);
        assert_eq!(
            raw.artifacts_by_project["project-a"][0].control_plane.execution_generation,
            1
        );

        cleanup(&path);
    }

    #[test]
    fn validate_registry_state_rejects_a_non_current_envelope_version() {
        // This exercises the same strict-equality mechanism that (unmodified,
        // no code change on that side) makes a pre-Stage-107A binary refuse a
        // 107A-migrated ("0.2.0") registry rather than silently loading and
        // re-serializing it without the generation / high-water data. Old
        // binary code cannot be executed from this repository, so this test
        // demonstrates the mechanism the downgrade-safety guarantee rests on:
        // any schema_version other than the current one is rejected outright.
        let legacy_shaped_state = ServiceArtifactRegistryState {
            schema_version: LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            ..Default::default()
        };
        assert!(matches!(
            validate_registry_state(&legacy_shaped_state),
            Err(ServiceArtifactRegistryError::Invalid(_))
        ));
    }

    // ---- P0-1 (v4 review): `with_stable_view` actually excludes writers -- //

    /// Proves the mechanism the P0-1 fix in
    /// `authorize_operational_reconciliation` relies on: while a
    /// `with_stable_view` closure is still running (holding the registry's
    /// read lock), a concurrent write (`set_pinning_mode`, and by the same
    /// `mutate()` path `create`/`update`/`delete`) cannot complete. This is
    /// what closes the P0-1 TOCTOU window -- the authorization handler
    /// resolves artifacts and commits the authorization inside one
    /// `with_stable_view` call, so nothing can mutate the registry in
    /// between.
    #[test]
    fn with_stable_view_blocks_a_concurrent_mutate_until_the_view_completes() {
        let registry = ServiceArtifactRegistry::from_state(ServiceArtifactRegistryState::default());
        let (view_entered_tx, view_entered_rx) = std::sync::mpsc::channel::<()>();
        let (release_view_tx, release_view_rx) = std::sync::mpsc::channel::<()>();

        let reader = registry.clone();
        let reader_handle = std::thread::spawn(move || {
            reader
                .with_stable_view(|_state| {
                    view_entered_tx.send(()).unwrap();
                    // Hold the read lock open until the test explicitly
                    // releases it, so a concurrent writer has a real chance
                    // to (incorrectly) race in underneath it.
                    release_view_rx.recv().unwrap();
                })
                .unwrap();
        });

        // Don't race the writer thread's spawn against the reader actually
        // entering the closure.
        view_entered_rx.recv().unwrap();

        let writer = registry.clone();
        let (writer_done_tx, writer_done_rx) = std::sync::mpsc::channel::<()>();
        let writer_handle = std::thread::spawn(move || {
            writer
                .set_pinning_mode(
                    "project-a",
                    EXECUTION_PINNING_MODE_LEGACY_PARTIAL,
                    "operator@example.test",
                )
                .unwrap();
            writer_done_tx.send(()).unwrap();
        });

        // The writer must NOT be able to complete while the stable view is
        // still held -- give it a generous bounded window to (incorrectly)
        // finish anyway, which it must fail to do.
        let writer_finished_early = writer_done_rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_ok();
        assert!(
            !writer_finished_early,
            "set_pinning_mode completed while a with_stable_view read lock was still held -- the P0-1 TOCTOU window is open"
        );

        // Release the view; the writer must now proceed and finish promptly.
        release_view_tx.send(()).unwrap();
        reader_handle.join().unwrap();
        writer_done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("writer should complete promptly once the stable view is released");
        writer_handle.join().unwrap();

        assert_eq!(
            registry.pinning_mode("project-a").unwrap(),
            EXECUTION_PINNING_MODE_LEGACY_PARTIAL
        );
    }

    #[test]
    fn backend_path_is_none_for_an_in_memory_registry() {
        let registry = ServiceArtifactRegistry::from_state(ServiceArtifactRegistryState::default());
        assert!(registry.backend_path().is_none());
    }

    #[test]
    fn secret_target_cannot_collide_with_non_secret_binding() {
        let mut artifact = valid_artifact();
        artifact.control_plane.secret_refs = vec!["token".to_string()];
        artifact.control_plane.secret_bindings = vec![ServiceArtifactSecretBinding {
            secret_ref: "token".to_string(), target_environment_variable: "TOKEN".to_string(), version: None,
        }];
        artifact.control_plane.bindings.insert("TOKEN".to_string(), "public-value".to_string());
        assert!(normalize_artifact("project-a", artifact.clone()).unwrap_err().message().contains("collides"));
        artifact.control_plane.bindings.clear();
        artifact.control_plane.bindings.insert("MODE".to_string(), "prod".to_string());
        assert!(normalize_artifact("project-a", artifact).is_ok());
    }

    #[test]
    fn invalid_migration_candidate_never_overwrites_legacy_state() {
        let path = temp_db_path("invalid-migration");
        let mut artifact = valid_artifact();
        artifact.control_plane.project_id = "wrong-project".to_string();
        let legacy = ServiceArtifactRegistryState {
            schema_version: LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![artifact])].into_iter().collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &legacy).unwrap();
        let before = serde_json::to_value(&legacy).unwrap();
        assert!(ServiceArtifactRegistry::load_from_path(&path).is_err());
        let after: serde_json::Value = load_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN).unwrap().unwrap();
        assert_eq!(after, before);
        cleanup(&path);
    }
    #[test]
    fn persisted_projection_digest_rejects_bare_or_nonmatching_canonical_values() {
        let registry = empty_registry();
        registry.create("project-a", valid_artifact()).unwrap();
        let mut state = registry.state.read().unwrap().clone();
        assert!(validate_registry_state(&state).is_ok());
        let artifact = &mut state.artifacts_by_project.get_mut("project-a").unwrap()[0];
        artifact.control_plane.execution_projection_digest = artifact.control_plane.execution_projection_digest[7..].to_string();
        assert!(validate_registry_state(&state).unwrap_err().message().contains("execution_projection_digest"));
        state.artifacts_by_project.get_mut("project-a").unwrap()[0].control_plane.execution_projection_digest = format!("sha256:{}", "0".repeat(64));
        assert!(validate_registry_state(&state).unwrap_err().message().contains("execution_projection_digest"));
    }

}
