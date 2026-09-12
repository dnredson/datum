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

/// Per-artifact record shape version. Stage 107B-A (ADR-0013) bumps this from
/// `"0.1.0"` to `"0.2.0"`: `ContainerDeploymentSpec` gains
/// `image_source_kind` / `resolved_image_identity`, and adding that sibling
/// key to `ImageIdentityV1` changes the canonical JCS bytes -- and therefore
/// `execution_projection_digest` -- of EVERY existing artifact, regardless of
/// the new field's own null-reservation (object-key presence perturbs JCS
/// serialization independent of value). That is a genuine, one-time,
/// schema-caused content change under D-7a, not a cosmetic one, so the
/// per-artifact schema version must version too: `validate_artifact`'s strict
/// equality check is what makes a pre-Stage-107B-A binary refuse a migrated
/// artifact rather than silently re-serializing it without the new fields.
/// See `migrate_legacy_artifact_schema_versions`.
pub const SERVICE_ARTIFACT_SCHEMA_VERSION: &str = "0.2.0";
/// The pre-Stage-107B-A per-artifact schema version, recognised on load so
/// the one-time per-artifact migration can run. Distinct from
/// `LEGACY_REGISTRY_STATE_SCHEMA_VERSION` below, which is the ENVELOPE's
/// legacy value -- the two migrate independently (the common case in
/// practice: the envelope is already current from Stage 107A, but every
/// artifact's own `schema_version` is still this value, since Stage 107A
/// never touched that field).
const LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION: &str = "0.1.0";
pub const SERVICE_ARTIFACT_REGISTRY_DOMAIN: &str = "service_artifact_registry";

/// Registry-declared image source kind (Stage 107B-A, ADR-0013). `"registry"`
/// is resolvable against a real OCI registry via `oci_registry_resolver`.
/// `"local_build"` has no upstream registry at all (a locally built image,
/// e.g. `datum/sma:dev`) and is DELIBERATELY never auto-trusted via a
/// `docker image inspect`-based shortcut -- see `pin_image_identity`, which
/// refuses to resolve a `local_build` artifact rather than inventing local
/// trust. `local_build` artifacts remain permanently `required`-ineligible
/// until a real governed-build-attestation path exists (deferred, not part
/// of Stage 107B-A).
pub const IMAGE_SOURCE_KIND_REGISTRY: &str = "registry";
pub const IMAGE_SOURCE_KIND_LOCAL_BUILD: &str = "local_build";
/// v5 review P0-5: an artifact record whose `image_source_kind` was never
/// explicitly classified -- either a genuinely old (pre-Stage-107B-A)
/// record migrating with no such field at all, or a new-schema record a
/// careless client failed to classify. NEVER silently treated as
/// `registry`: that would let a previously-`local_build` image (e.g.
/// `datum/sma:dev`) become falsely trustable merely because an old schema
/// didn't know the field. Permanently `required`-ineligible;
/// `pin_image_identity` refuses to resolve it. Only an explicit operator
/// PUT that sets `image_source_kind` to `registry` or `local_build` can
/// move an artifact out of this state -- never an inferred guess from the
/// image name.
pub const IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED: &str = "legacy_unclassified";

fn default_image_source_kind() -> String {
    // v5 review P0-5: the safe default for ANY artifact missing this field
    // is "unclassified", never "registry" -- see the constant's own doc
    // comment for why silently inferring trust here is unsafe.
    IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED.to_string()
}

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
    /// Stage 108 (ADR-0015). The minimal, DESIRED-STATE-ONLY declarative
    /// payload for a `runtime_kind::RuntimeKind::NativeProcess` artifact.
    /// Populated if and only if `kind == "native_process"`; `container` and
    /// `native_process` are never both `Some` nor both `None` for a valid
    /// artifact (`validate_artifact` enforces this). Deliberately carries no
    /// content hash (see `ServiceArtifactAcquisition`, the single owner of
    /// native executable content identity -- Stage 109) and no observed
    /// runtime state (PID, process start time, current state -- those belong
    /// to a future D-Node-local `ManagedProcessRuntimeState`, never here).
    #[serde(default)]
    pub native_process: Option<NativeProcessDeploymentSpecV1>,
}

/// Stage 108 (ADR-0015). See `ServiceRuntimeSpec.native_process`'s doc
/// comment for the desired/observed boundary this type sits on the DESIRED
/// side of. No process lifecycle exists yet in Stage 108 -- this type exists
/// solely so a native-process artifact can round-trip honestly through
/// validation, the execution projection, and the wire assignment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeProcessDeploymentSpecV1 {
    pub process_name: String,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
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
    /// Stage 107B-A (ADR-0013). One of `"registry"`, `"local_build"`, or
    /// `"legacy_unclassified"` -- see `IMAGE_SOURCE_KIND_REGISTRY` /
    /// `IMAGE_SOURCE_KIND_LOCAL_BUILD` / `IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED`.
    /// v7 review P2-2: the safe serde default for a record missing this
    /// field is `"legacy_unclassified"` (`default_image_source_kind`), NOT
    /// `"registry"` -- a stale, pre-v5 version of this comment claimed
    /// otherwise. Every real, checked-in catalog artifact explicitly sets
    /// this field (see `catalog_inventory_classifies_every_real_artifact_explicitly`);
    /// the default only matters for a genuinely legacy record.
    #[serde(default = "default_image_source_kind")]
    pub image_source_kind: String,
    /// Stage 107B-A (ADR-0013). SERVER-OWNED: the ordinary create/update
    /// path (`normalize_artifact`) rejects any client-submitted non-None
    /// value outright, regardless of shape -- `validate_artifact` only
    /// checks the SHAPE of an already-accepted value, it plays no role in
    /// deciding whether a client may set this field at all. The only
    /// legitimate writer is the internal, compare-and-swap
    /// `ServiceArtifactRegistry::apply_resolved_image_identity`, invoked
    /// exclusively by `pin_image_identity`. Absent means the image is
    /// unpinned; `required` mode refuses to treat a bare tag or a
    /// `local_build`/`legacy_unclassified` source as sufficient.
    #[serde(default)]
    pub resolved_image_identity: Option<ResolvedImageIdentityV1>,
}

/// Stage 107B-A (ADR-0013). A server-resolved, immutable, multi-platform OCI
/// image identity. Binding always happens on a platform's own
/// `manifest_digest`, never on `index_digest` alone: an index digest changing
/// does not imply a given node's platform-specific content changed, and vice
/// versa (Amendment 3). `resolved_at_utc` / `resolver_tool_version` /
/// `registry_host` are administrative/provenance metadata only -- they are
/// deliberately NOT mirrored into `ArtifactExecutionProjectionV1`
/// (`execution_projection::ResolvedImageIdentityProjectionV1`), so
/// re-resolving identical registry content at a different wall-clock time
/// does not perturb `execution_generation`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedImageIdentityV1 {
    /// Fully-qualified repository, e.g. `docker.io/library/eclipse-mosquitto`.
    pub repository: String,
    /// Present only when the reference resolved through an OCI index /
    /// Docker manifest list. Informational provenance -- never the binding
    /// identity by itself.
    #[serde(default)]
    pub index_digest: Option<String>,
    /// Non-empty; exactly the platforms requested at pin time.
    pub platforms: Vec<ResolvedPlatformManifestV1>,
    pub resolved_at_utc: String,
    pub resolver_tool_version: String,
    /// v5 review P1-1, corrected by v7 review P2-1: a CALLER-SUPPLIED audit
    /// label / asserted actor label for whoever/whatever triggered this pin.
    /// DServer has no application-layer authentication (see ADR-0013 §6), so
    /// this is provenance -- a hint for a human reading the record later --
    /// never a verified/authenticated operator identity, and it must never
    /// be described or relied upon as one. Administrative/audit metadata
    /// only -- excluded from the projection, same as the three fields above.
    #[serde(default)]
    pub resolved_by: String,
    pub registry_host: String,
}

/// v5 review P0-2: the artifact state `oci_image_pinning::pin_image_identity`
/// observed BEFORE starting the (slow) network resolution. Passed to
/// `ServiceArtifactRegistry::apply_resolved_image_identity`, which re-reads
/// the CURRENT artifact inside its write-lock critical section and refuses
/// to persist (returns a `Conflict`) if any of these fields no longer match
/// -- the compare-and-swap boundary that closes the TOCTOU/lost-update race
/// a slow resolution would otherwise create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageResolutionExpectedState {
    pub execution_generation: u64,
    pub execution_projection_digest: String,
    pub image: String,
    pub image_source_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedPlatformManifestV1 {
    pub os: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
    /// The binding identity for this platform -- OCI-digest domain
    /// (`oci_registry_resolver::is_oci_digest_format`), never the ADR-0006
    /// canonical DATUM contract digest domain.
    pub manifest_digest: String,
    pub config_digest: String,
    /// `"{repository}@{manifest_digest}"` -- NEVER `"{repository}@{index_digest}"`
    /// (Amendment 3: index digest alone never identifies a concrete
    /// platform's content).
    pub execution_reference: String,
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
            Some(state) => {
                // v7 review P1-2: the envelope migration (Stage 107A,
                // ADR-0011 §3) and the per-artifact schema migration (Stage
                // 107B-A, ADR-0013) are two independent axes that can BOTH be
                // legacy at once for a genuinely old, never-yet-loaded-by-
                // any-post-107A-binary state (envelope "0.1.0", every
                // artifact's own schema_version also "0.1.0"). The two
                // migrations must be composed IN MEMORY, on one candidate,
                // with exactly one `validate_registry_state` call and one
                // persist at the end -- never validating or persisting the
                // intermediate, envelope-only-migrated candidate, because
                // `validate_registry_state` calls `validate_artifact`, which
                // requires `artifact.schema_version ==
                // SERVICE_ARTIFACT_SCHEMA_VERSION` for EVERY artifact
                // unconditionally. An envelope-only-migrated candidate whose
                // artifacts are still per-artifact-legacy would fail that
                // check and could never reach the per-artifact migration
                // stage at all -- exactly the bug this composition fixes.
                //
                // Both stages remain independently idempotent (each is
                // gated on its own axis's legacy marker), so this composed
                // flow is a strict superset of running either stage alone:
                // envelope-only-legacy, artifact-only-legacy, and
                // both-legacy states are all migrated correctly in one call,
                // and an already-current state on both axes takes neither
                // migration path below.
                let envelope_needs_migration =
                    state.schema_version == LEGACY_REGISTRY_STATE_SCHEMA_VERSION;
                let artifact_needs_migration = state
                    .artifacts_by_project
                    .values()
                    .flatten()
                    .any(|artifact| artifact.schema_version == LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION);
                if envelope_needs_migration || artifact_needs_migration {
                    let mut candidate = state;
                    if envelope_needs_migration {
                        // Stage 107A migration: normalise every artifact's
                        // execution_generation / execution_projection_digest
                        // deterministically and seed the durable high-water
                        // map, bumping the envelope to the current schema
                        // version. Runs first so the per-artifact migration
                        // below (if also needed) bumps generation a SECOND,
                        // independently-justified time from the high-water
                        // baseline this stage just established, rather than
                        // from a possibly-stale in-place value.
                        candidate = migrate_legacy_registry_state(candidate)?;
                    }
                    if artifact_needs_migration {
                        // Stage 107B-A migration: bump schema_version and
                        // recompute the projection digest / generation for
                        // any artifact whose OWN schema_version is still
                        // legacy, regardless of the envelope's version.
                        candidate = migrate_legacy_artifact_schema_versions(candidate)?;
                    }
                    // Validate the ONE fully-migrated candidate -- never an
                    // intermediate, partially-migrated one -- then persist
                    // it immediately, before this function returns and
                    // before the server can serve any endpoint that could
                    // read or authorize against it. If the fully-migrated
                    // candidate is itself invalid, nothing is persisted: the
                    // on-disk row is left exactly as it was found.
                    validate_registry_state(&candidate).map_err(|error| error.message().to_string())?;
                    persist_domain_json(path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &candidate)?;
                    candidate
                } else {
                    state
                }
            }
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
        // ignored, never trusted. Stage 108 (ADR-0015): runtime-dispatched so
        // a native_process artifact gets its own projection shape, never the
        // container-only one.
        let digest = crate::core::execution_projection::compute_execution_projection_digest(&artifact)
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
        // `normalize_artifact` already rejects any client-submitted non-None
        // `resolved_image_identity` outright (v5 P0-1); the candidate
        // reaching here always has that field `None`. Whether the FINAL
        // stored artifact keeps the previously-pinned identity or loses it
        // is decided below, inside the write-lock critical section, once we
        // can compare against the artifact actually stored today.
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
            let existing_digest = items[index].control_plane.execution_projection_digest.clone();
            let existing_generation = items[index].control_plane.execution_generation;

            let mut updated = artifact.clone();
            updated.control_plane.created_at_utc = created_at;

            // v7 review P1-1: `resolved_image_identity` is server-owned and
            // MUST survive a PUT that does not change the very thing it
            // pinned, or Stage 107A's frozen invariant ("a control_plane.
            // version-only change is a semantic no-op") breaks the moment an
            // artifact has ever been pinned -- an ordinary client can never
            // submit a non-None value here (rejected above), so wholesale
            // replacement would silently null out an existing pin on every
            // PUT, not just ones that actually change the image. Preserve
            // the existing pin exactly when BOTH `image` and
            // `image_source_kind` are unchanged; otherwise leave it cleared
            // (already `None` from normalization above) -- a changed image
            // or a changed source kind both invalidate whatever was
            // previously pinned, since the pin describes content that no
            // longer matches what is now declared.
            if let (Some(existing_container), Some(updated_container)) = (
                items[index].runtime.container.as_ref(),
                updated.runtime.container.as_mut(),
            ) {
                if existing_container.image == updated_container.image
                    && existing_container.image_source_kind == updated_container.image_source_kind
                {
                    updated_container.resolved_image_identity =
                        existing_container.resolved_image_identity.clone();
                }
            }

            // Server-owned (D-7a, DC-5): compute the new content's
            // projection digest -- now that any pin-preservation/clearing
            // decision above has already been folded into `updated` -- and
            // decide increment vs. no-op from whether IT changed relative
            // to the stored artifact. Stage 108 (ADR-0015): runtime-dispatched.
            let new_digest = crate::core::execution_projection::compute_execution_projection_digest(&updated)
                .map_err(ServiceArtifactRegistryError::Invalid)?;

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

    /// v5 review P0-1/P0-2: the ONLY path that may set
    /// `resolved_image_identity` -- never reachable through the public
    /// create/update API (`normalize_artifact` rejects any client-submitted
    /// non-None value outright). Also the compare-and-swap boundary for
    /// P0-2: `expected` is the state observed BEFORE the caller's (slow)
    /// network resolution ran; this re-reads the CURRENT artifact inside
    /// ONE write-lock critical section and refuses to persist anything if
    /// it no longer matches `expected` -- returning
    /// `Conflict("artifact_changed_during_image_resolution")` -- rather
    /// than overwriting newer state (a concurrent image/source-kind change,
    /// an unrelated projection-changing update, or a delete/recreate) with
    /// a stale resolution result.
    pub(crate) fn apply_resolved_image_identity(
        &self,
        project_id: &str,
        artifact_id: &str,
        expected: &ImageResolutionExpectedState,
        resolved: ResolvedImageIdentityV1,
    ) -> Result<ServiceArtifact, ServiceArtifactRegistryError> {
        self.mutate(|state| {
            let items = state.artifacts_by_project.get_mut(project_id).ok_or_else(|| {
                ServiceArtifactRegistryError::NotFound(format!(
                    "service artifact '{artifact_id}' not found for project '{project_id}'"
                ))
            })?;
            let index = items.iter().position(|item| item.artifact_id == artifact_id).ok_or_else(|| {
                ServiceArtifactRegistryError::NotFound(format!(
                    "service artifact '{artifact_id}' not found for project '{project_id}'"
                ))
            })?;

            let current_generation = items[index].control_plane.execution_generation;
            let current_digest = items[index].control_plane.execution_projection_digest.clone();
            let (current_image, current_image_source_kind) = {
                let container = items[index].runtime.container.as_ref().ok_or_else(|| {
                    ServiceArtifactRegistryError::Invalid(format!(
                        "service artifact '{artifact_id}' has no container runtime definition"
                    ))
                })?;
                (container.image.clone(), container.image_source_kind.clone())
            };

            if current_generation != expected.execution_generation
                || current_digest != expected.execution_projection_digest
                || current_image != expected.image
                || current_image_source_kind != expected.image_source_kind
            {
                return Err(ServiceArtifactRegistryError::Conflict(
                    "artifact_changed_during_image_resolution".to_string(),
                ));
            }

            // v6 review P1-2: defensive validation immediately before
            // persistence, even though `pin_image_identity` already
            // validates -- so a fabricated or internally-inconsistent
            // candidate can never reach storage through this internal
            // method, even if a future caller misused it directly.
            crate::core::oci_registry_resolver::validate_resolved_image_identity(&current_image, &resolved)
                .map_err(ServiceArtifactRegistryError::Invalid)?;

            let mut updated = items[index].clone();
            updated.runtime.container.as_mut().expect("checked non-None above").resolved_image_identity = Some(resolved);

            let (_, new_digest) = crate::core::execution_projection::compute_projection_and_digest(&updated)
                .map_err(ServiceArtifactRegistryError::Invalid)?;

            if new_digest == current_digest && current_generation > 0 {
                // Should not normally occur (adding a resolved identity
                // always changes the projection), but handled for
                // completeness -- mirrors update()'s own no-op branch.
                updated.control_plane.execution_generation = current_generation;
                updated.control_plane.execution_projection_digest = current_digest;
            } else {
                let project_high_water = state.execution_generation_high_water.entry(project_id.to_string()).or_default();
                let baseline = project_high_water.get(artifact_id).copied().unwrap_or(current_generation);
                let generation = checked_next_generation(baseline.max(current_generation))?;
                updated.control_plane.execution_generation = generation;
                updated.control_plane.execution_projection_digest = new_digest;
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
    // v5 review P0-1: `resolved_image_identity` is SERVER-OWNED state. The
    // public create/update path (the only caller of `normalize_artifact`)
    // must reject any client-submitted non-None value outright, rather than
    // silently trust or silently overwrite it -- unlike `execution_generation`/
    // `execution_projection_digest` (which the server recomputes from
    // content regardless of what the client sent), a fabricated
    // `resolved_image_identity` cannot be safely "recomputed away": its
    // only legitimate source is `apply_resolved_image_identity`, invoked
    // exclusively by the explicit `pin_image_identity` operation. Rejecting
    // outright also means an ordinary PUT that changes `image` can never
    // preserve a stale pin: the client must explicitly submit `None` (i.e.
    // omit or null the field) for ANY create/update to succeed, so a
    // previously-pinned identity can never silently survive an image change
    // through this path.
    if artifact
        .runtime
        .container
        .as_ref()
        .is_some_and(|container| container.resolved_image_identity.is_some())
    {
        return Err(ServiceArtifactRegistryError::Invalid(
            "runtime.container.resolved_image_identity is server-owned and must not be set in a create/update request body; it can only be set via the explicit image-identity pin operation".to_string(),
        ));
    }
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
            let digest =
                crate::core::execution_projection::compute_execution_projection_digest(artifact)?;
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

/// Stage 107B-A (ADR-0013) per-artifact migration, independent of (and run
/// strictly AFTER) the envelope migration above. Triggered whenever ANY
/// artifact's OWN `schema_version` is not current, regardless of the
/// envelope's version -- the common real-world case: the envelope is
/// already current from Stage 107A, but every artifact's own
/// `schema_version` is still `LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION`, since
/// Stage 107A never touched that field.
///
/// This is a deliberate, one-time, SCHEMA-CAUSED generation bump, not a
/// semantic content change: adding `ContainerDeploymentSpec.resolved_image_identity`
/// as a sibling key changes the JCS bytes -- and therefore the digest -- of
/// EVERY existing artifact's projection even though no field the agent
/// actually reads changed value. D-7a's generation model treats any digest
/// change as content-changing by definition, so this migration goes through
/// the exact same `checked_next_generation` high-water mechanism `update()`
/// uses -- it does not special-case "just a schema bump" as generation-inert,
/// because doing so would require a second, parallel notion of "does this
/// change matter" that the projection model does not otherwise have.
fn migrate_legacy_artifact_schema_versions(
    mut state: ServiceArtifactRegistryState,
) -> Result<ServiceArtifactRegistryState, String> {
    let ServiceArtifactRegistryState { artifacts_by_project, execution_generation_high_water, .. } = &mut state;

    for (project_id, artifacts) in artifacts_by_project.iter_mut() {
        for artifact in artifacts.iter_mut() {
            if artifact.schema_version == LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION {
                artifact.schema_version = SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
                let digest = crate::core::execution_projection::compute_execution_projection_digest(artifact)?;
                let project_high_water = execution_generation_high_water.entry(project_id.clone()).or_default();
                let generation = checked_next_generation(
                    project_high_water
                        .get(&artifact.artifact_id)
                        .copied()
                        .unwrap_or(artifact.control_plane.execution_generation),
                )
                .map_err(|error| error.message().to_string())?;
                artifact.control_plane.execution_generation = generation;
                artifact.control_plane.execution_projection_digest = digest;
                project_high_water.insert(artifact.artifact_id.clone(), generation);
            }
            // Already current: no-op (idempotency). Any OTHER value is left
            // for validate_artifact/validate_registry_state to reject
            // explicitly, rather than silently migrated here.
        }
    }
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
            let recomputed_digest = crate::core::execution_projection::compute_execution_projection_digest(artifact)
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

    // Stage 108 (ADR-0015, P0-4 correction): runtime-discriminated
    // validation. Container-specific requirements apply ONLY under
    // RuntimeKind::Container; a native_process artifact must never be
    // required to satisfy them, and must never be validated as "a container
    // with an empty image" (P0-4 point E).
    match crate::core::runtime_kind::RuntimeKind::parse(&artifact.runtime.kind) {
        Ok(crate::core::runtime_kind::RuntimeKind::Container) => {
            if artifact.runtime.native_process.is_some() {
                return invalid("runtime.kind='container' must not populate runtime.native_process");
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
            if !matches!(
                container.image_source_kind.as_str(),
                IMAGE_SOURCE_KIND_REGISTRY | IMAGE_SOURCE_KIND_LOCAL_BUILD | IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED
            ) {
                return invalid(format!(
                    "runtime.container.image_source_kind must be '{IMAGE_SOURCE_KIND_REGISTRY}', '{IMAGE_SOURCE_KIND_LOCAL_BUILD}', or '{IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED}'"
                ));
            }
            if let Some(resolved) = &container.resolved_image_identity {
                if resolved.repository.trim().is_empty() {
                    return invalid("runtime.container.resolved_image_identity.repository must not be empty");
                }
                if resolved.resolved_at_utc.trim().is_empty()
                    || resolved.resolver_tool_version.trim().is_empty()
                    || resolved.registry_host.trim().is_empty()
                {
                    return invalid(
                        "runtime.container.resolved_image_identity requires resolved_at_utc, resolver_tool_version, and registry_host",
                    );
                }
                if let Some(index_digest) = &resolved.index_digest {
                    if !crate::core::oci_registry_resolver::is_oci_digest_format(index_digest) {
                        return invalid("runtime.container.resolved_image_identity.index_digest must be a valid OCI digest");
                    }
                }
                if resolved.platforms.is_empty() {
                    return invalid("runtime.container.resolved_image_identity.platforms must not be empty");
                }
                for platform in &resolved.platforms {
                    if platform.os.trim().is_empty() || platform.architecture.trim().is_empty() {
                        return invalid("runtime.container.resolved_image_identity.platforms entries require os and architecture");
                    }
                    if !crate::core::oci_registry_resolver::is_oci_digest_format(&platform.manifest_digest)
                        || !crate::core::oci_registry_resolver::is_oci_digest_format(&platform.config_digest)
                    {
                        return invalid(
                            "runtime.container.resolved_image_identity.platforms entries require valid OCI manifest_digest and config_digest",
                        );
                    }
                    let expected_reference = format!("{}@{}", resolved.repository, platform.manifest_digest);
                    if platform.execution_reference != expected_reference {
                        // Amendment 3: the execution_reference must bind on the
                        // platform-specific manifest digest, never on index_digest
                        // alone -- reject any candidate that doesn't match exactly.
                        return invalid(
                            "runtime.container.resolved_image_identity.platforms entries require execution_reference == '{repository}@{manifest_digest}'",
                        );
                    }
                }
            }
        }
        Ok(crate::core::runtime_kind::RuntimeKind::NativeProcess) => {
            if artifact.runtime.container.is_some() {
                return invalid("runtime.kind='native_process' must not populate runtime.container");
            }
            let native = artifact
                .runtime
                .native_process
                .as_ref()
                .ok_or_else(|| ServiceArtifactRegistryError::Invalid(
                    "native_process runtime requires runtime.native_process".to_string(),
                ))?;
            validate_native_process_deployment_spec(native)?;
        }
        Err(_) => {
            return invalid(format!(
                "runtime.kind must be '{}' or '{}'",
                crate::core::runtime_kind::RUNTIME_KIND_CONTAINER,
                crate::core::runtime_kind::RUNTIME_KIND_NATIVE_PROCESS
            ));
        }
    }
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

/// Stage 108 (ADR-0015). Bounded, non-overbuilt validation for the minimal
/// desired-state-only native-process payload -- no filesystem access, no
/// path-security modelling occurs in Stage 108 (that is Stage 109 scope,
/// once real acquisition/spawn exists). Limits mirror this file's existing
/// `validate_command`/`validate_assignments`-style bounds (NUL-free,
/// bounded length/count), not invented fresh.
///
/// Stage 108 v2 (independent review, P1-2): `pub(crate)` so
/// `artifact_reconciliation_context::validate_assignments` (the DOWNSTREAM,
/// wire-assignment validator, evaluated after this registry-level
/// validator already ran once at create/update time) can enforce the EXACT
/// same bounds on the wire assignment, instead of independently
/// reimplementing (and risking silently drifting from) a subset of them.
/// This is intra-crate reuse only -- the agent side
/// (`DATUM/src/agent/operational_reconciliation.rs`) still has its own,
/// independently hand-written mirror of these same bounds, per this
/// project's no-shared-crate wire-parity discipline.
pub(crate) fn validate_native_process_deployment_spec(
    spec: &NativeProcessDeploymentSpecV1,
) -> Result<(), ServiceArtifactRegistryError> {
    if spec.process_name.trim().is_empty() || spec.process_name.len() > 256 || spec.process_name.contains('\0') {
        return invalid("runtime.native_process.process_name must be non-empty, NUL-free, and at most 256 characters");
    }
    if spec.entrypoint.trim().is_empty() || spec.entrypoint.len() > 4096 || spec.entrypoint.contains('\0') {
        return invalid("runtime.native_process.entrypoint must be non-empty, NUL-free, and at most 4096 characters");
    }
    if spec.args.len() > 64 || spec.args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0')) {
        return invalid("runtime.native_process.args must have at most 64 entries, each NUL-free and at most 4096 characters");
    }
    if let Some(dir) = &spec.working_directory {
        if dir.trim().is_empty() || dir.len() > 4096 || dir.contains('\0') {
            return invalid(
                "runtime.native_process.working_directory, when present, must be non-empty after trim, NUL-free, and at most 4096 characters",
            );
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
                native_process: None,
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-sma".to_string(),
                    image: "datum/sma:dev".to_string(),
                    ports: Vec::new(),
                    network: None,
                    network_aliases: Vec::new(),
                    volumes: Vec::new(),
                    configuration_mounts: Vec::new(),
                    command: Vec::new(),
                    image_source_kind: IMAGE_SOURCE_KIND_REGISTRY.to_string(),
                    resolved_image_identity: None,
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

    // Stage 108 (ADR-0015) test fixtures/tests.

    fn valid_native_process_artifact() -> ServiceArtifact {
        let mut artifact = valid_artifact();
        artifact.artifact_id = "native-svc".to_string();
        artifact.service_id = "native-svc".to_string();
        artifact.runtime = ServiceRuntimeSpec {
            kind: crate::core::runtime_kind::RUNTIME_KIND_NATIVE_PROCESS.to_string(),
            container: None,
            native_process: Some(NativeProcessDeploymentSpecV1 {
                process_name: "datum-native-svc".to_string(),
                entrypoint: "/opt/datum/native-svc/bin/run".to_string(),
                args: vec!["--config".to_string(), "/opt/datum/native-svc/config.toml".to_string()],
                working_directory: Some("/opt/datum/native-svc".to_string()),
            }),
        };
        artifact.control_plane.artifact_kind = "command_bundle".to_string();
        artifact
    }

    #[test]
    fn validate_artifact_accepts_a_well_formed_native_process_artifact() {
        validate_artifact(&valid_native_process_artifact()).unwrap();
    }

    #[test]
    fn validate_artifact_rejects_native_process_kind_with_container_payload() {
        let mut artifact = valid_native_process_artifact();
        artifact.runtime.container = valid_artifact().runtime.container;
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_native_process_kind_with_no_native_payload() {
        let mut artifact = valid_native_process_artifact();
        artifact.runtime.native_process = None;
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_container_kind_with_native_payload() {
        let mut artifact = valid_artifact();
        artifact.runtime.native_process = Some(NativeProcessDeploymentSpecV1 {
            process_name: "x".to_string(),
            entrypoint: "/bin/x".to_string(),
            args: Vec::new(),
            working_directory: None,
        });
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_unknown_runtime_kind() {
        let mut artifact = valid_artifact();
        artifact.runtime.kind = "vm".to_string();
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_empty_native_process_name() {
        let mut artifact = valid_native_process_artifact();
        artifact.runtime.native_process.as_mut().unwrap().process_name = String::new();
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_empty_native_entrypoint() {
        let mut artifact = valid_native_process_artifact();
        artifact.runtime.native_process.as_mut().unwrap().entrypoint = String::new();
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_native_arg_containing_nul() {
        let mut artifact = valid_native_process_artifact();
        artifact.runtime.native_process.as_mut().unwrap().args = vec!["bad\0arg".to_string()];
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn validate_artifact_rejects_blank_native_working_directory() {
        let mut artifact = valid_native_process_artifact();
        artifact.runtime.native_process.as_mut().unwrap().working_directory = Some("   ".to_string());
        assert!(validate_artifact(&artifact).is_err());
    }

    #[test]
    fn create_computes_a_deterministic_native_process_execution_projection_digest() {
        let registry = empty_registry();
        let mut artifact = valid_native_process_artifact();
        artifact.control_plane.project_id = "project-native".to_string();
        artifact.control_plane.target_nodes = vec!["fog-01".to_string()];
        let created = registry.create("project-native", artifact).unwrap();
        assert!(crate::core::jcs::is_canonical_digest_format(&created.control_plane.execution_projection_digest));
        assert_eq!(created.control_plane.execution_generation, 1);
    }

    #[test]
    fn native_process_execution_projection_digest_changes_when_entrypoint_changes() {
        let registry = empty_registry();
        let mut artifact = valid_native_process_artifact();
        artifact.control_plane.project_id = "project-native".to_string();
        artifact.control_plane.target_nodes = vec!["fog-01".to_string()];
        let created = registry.create("project-native", artifact.clone()).unwrap();

        let mut changed = artifact;
        changed.runtime.native_process.as_mut().unwrap().entrypoint = "/opt/datum/native-svc/bin/run-v2".to_string();
        let updated = registry.update("project-native", "native-svc", changed).unwrap();

        assert_ne!(
            updated.control_plane.execution_projection_digest,
            created.control_plane.execution_projection_digest
        );
        assert_eq!(updated.control_plane.execution_generation, created.control_plane.execution_generation + 1);
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

    // ---- v5 review P0-1: resolved_image_identity is truly server-owned - //

    /// A syntactically-valid, INTERNALLY-CONSISTENT-with-"datum/sma:dev"
    /// resolved identity that a client should still never be able to inject
    /// via create/update (P0-1's point). Internally consistent on purpose,
    /// matching `valid_artifact()`'s image, so these tests exercise the
    /// P0-1 write-path boundary specifically -- not
    /// `apply_resolved_image_identity`'s separate (P1-2) defensive
    /// structural-validation gate, which is proven independently elsewhere.
    fn fabricated_resolved_identity() -> ResolvedImageIdentityV1 {
        ResolvedImageIdentityV1 {
            repository: "docker.io/datum/sma".to_string(),
            index_digest: None,
            platforms: vec![ResolvedPlatformManifestV1 {
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
                variant: None,
                manifest_digest: format!("sha256:{}", "f".repeat(64)),
                config_digest: format!("sha256:{}", "e".repeat(64)),
                execution_reference: format!("docker.io/datum/sma@sha256:{}", "f".repeat(64)),
            }],
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "attacker/fabricated".to_string(),
            resolved_by: "attacker".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        }
    }

    #[test]
    fn p0_1_test_1_post_with_client_authored_resolved_image_identity_is_rejected() {
        let mut artifact = valid_artifact();
        artifact.runtime.container.as_mut().unwrap().resolved_image_identity = Some(fabricated_resolved_identity());
        let error = empty_registry().create("project-a", artifact).unwrap_err();
        assert!(
            matches!(&error, ServiceArtifactRegistryError::Invalid(message) if message.contains("server-owned")),
            "{error:?}"
        );
    }

    #[test]
    fn p0_1_test_2_ordinary_put_with_client_authored_resolved_image_identity_is_rejected() {
        let registry = empty_registry();
        registry.create("project-a", valid_artifact()).unwrap();
        let mut updated = valid_artifact();
        updated.runtime.container.as_mut().unwrap().resolved_image_identity = Some(fabricated_resolved_identity());
        let error = registry.update("project-a", "sma", updated).unwrap_err();
        assert!(
            matches!(&error, ServiceArtifactRegistryError::Invalid(message) if message.contains("server-owned")),
            "{error:?}"
        );
    }

    #[test]
    fn p0_1_test_3_changing_image_via_ordinary_update_clears_any_previous_pin() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();
        let expected = ImageResolutionExpectedState {
            execution_generation: created.control_plane.execution_generation,
            execution_projection_digest: created.control_plane.execution_projection_digest.clone(),
            image: created.runtime.container.as_ref().unwrap().image.clone(),
            image_source_kind: created.runtime.container.as_ref().unwrap().image_source_kind.clone(),
        };
        registry
            .apply_resolved_image_identity("project-a", "sma", &expected, fabricated_resolved_identity())
            .unwrap();
        let pinned = registry.get("project-a", "sma").unwrap();
        assert!(pinned.runtime.container.as_ref().unwrap().resolved_image_identity.is_some());

        // An ordinary PUT changing the image, correctly omitting
        // resolved_image_identity (the client's only legal submission --
        // see test 2), must NOT preserve the stale pin.
        let mut changed = valid_artifact();
        changed.runtime.container.as_mut().unwrap().image = "datum/sma:2026-01".to_string();
        let updated = registry.update("project-a", "sma", changed).unwrap();
        assert!(
            updated.runtime.container.as_ref().unwrap().resolved_image_identity.is_none(),
            "the old pin must never survive an ordinary image-changing update"
        );
    }

    #[test]
    fn p0_1_test_4_only_the_explicit_pin_operation_can_create_a_resolved_identity() {
        let registry = empty_registry();
        registry.create("project-a", valid_artifact()).unwrap();

        // The public API surface (create/update) can never set it, no
        // matter what the client submits.
        let mut via_update = valid_artifact();
        via_update.runtime.container.as_mut().unwrap().resolved_image_identity = Some(fabricated_resolved_identity());
        assert!(registry.update("project-a", "sma", via_update).is_err());

        // Only the internal apply_resolved_image_identity (the same method
        // oci_image_pinning::pin_image_identity calls) can.
        let current = registry.get("project-a", "sma").unwrap();
        let expected = ImageResolutionExpectedState {
            execution_generation: current.control_plane.execution_generation,
            execution_projection_digest: current.control_plane.execution_projection_digest.clone(),
            image: current.runtime.container.as_ref().unwrap().image.clone(),
            image_source_kind: current.runtime.container.as_ref().unwrap().image_source_kind.clone(),
        };
        let pinned = registry
            .apply_resolved_image_identity("project-a", "sma", &expected, fabricated_resolved_identity())
            .unwrap();
        assert!(pinned.runtime.container.as_ref().unwrap().resolved_image_identity.is_some());
    }

    #[test]
    fn p0_1_test_5_required_authorization_cannot_be_obtained_from_fabricated_client_data() {
        // A client cannot even get a fabricated identity INTO the registry
        // in the first place (test 1) -- so no artifact carrying one is
        // ever created, and no authorization can ever be issued over it.
        let registry = empty_registry();
        let mut artifact = valid_artifact();
        artifact.runtime.container.as_mut().unwrap().resolved_image_identity = Some(fabricated_resolved_identity());
        assert!(registry.create("project-a", artifact).is_err());
        // The SAME registry never received a persisted artifact at all.
        assert!(matches!(registry.get("project-a", "sma"), Err(ServiceArtifactRegistryError::NotFound(_))));
    }

    // ---- v6 review P1-2: apply_resolved_image_identity is defensive too - //
    #[test]
    fn apply_resolved_image_identity_rejects_a_structurally_inconsistent_candidate() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();
        let expected = ImageResolutionExpectedState {
            execution_generation: created.control_plane.execution_generation,
            execution_projection_digest: created.control_plane.execution_projection_digest.clone(),
            image: created.runtime.container.as_ref().unwrap().image.clone(),
            image_source_kind: created.runtime.container.as_ref().unwrap().image_source_kind.clone(),
        };
        // Deliberately internally inconsistent: repository disagrees with
        // "datum/sma:dev"'s actual repository ("docker.io/datum/sma").
        let mut inconsistent = fabricated_resolved_identity();
        inconsistent.repository = "ghcr.io/absmach/atom".to_string();

        let error = registry
            .apply_resolved_image_identity("project-a", "sma", &expected, inconsistent)
            .unwrap_err();
        assert!(matches!(error, ServiceArtifactRegistryError::Invalid(_)), "{error:?}");
        // Nothing was persisted -- the artifact is exactly as it was.
        let unchanged = registry.get("project-a", "sma").unwrap();
        assert_eq!(unchanged, created);
    }

    // ---- v7 review P1-1: an ordinary PUT must not silently unpin -------- //

    /// Creates and pins `valid_artifact()` (via the same internal path
    /// `pin_image_identity` uses), returning the resulting pinned artifact.
    fn pin_valid_artifact(registry: &ServiceArtifactRegistry) -> ServiceArtifact {
        let created = registry.create("project-a", valid_artifact()).unwrap();
        let expected = ImageResolutionExpectedState {
            execution_generation: created.control_plane.execution_generation,
            execution_projection_digest: created.control_plane.execution_projection_digest.clone(),
            image: created.runtime.container.as_ref().unwrap().image.clone(),
            image_source_kind: created.runtime.container.as_ref().unwrap().image_source_kind.clone(),
        };
        registry
            .apply_resolved_image_identity("project-a", "sma", &expected, fabricated_resolved_identity())
            .unwrap()
    }

    #[test]
    fn p1_1_test_a_version_only_put_on_a_pinned_artifact_preserves_the_pin_and_is_still_a_semantic_no_op() {
        let registry = empty_registry();
        let pinned = pin_valid_artifact(&registry);

        let mut version_only = valid_artifact();
        version_only.control_plane.version = "v2-otherwise-identical".to_string();
        let updated = registry.update("project-a", "sma", version_only).unwrap();

        assert!(
            updated.runtime.container.as_ref().unwrap().resolved_image_identity.is_some(),
            "a control_plane.version-only PUT must not silently unpin a previously-pinned artifact"
        );
        assert_eq!(
            updated.control_plane.execution_projection_digest,
            pinned.control_plane.execution_projection_digest,
            "version is excluded from the projection; the frozen Stage 107A no-op invariant must still hold once pinned"
        );
        assert_eq!(
            updated.control_plane.execution_generation, pinned.control_plane.execution_generation,
            "a true semantic no-op must not bump generation"
        );
    }

    #[test]
    fn p1_1_test_b_unrelated_execution_relevant_change_on_a_pinned_artifact_preserves_the_pin_and_bumps_generation_once() {
        let registry = empty_registry();
        let pinned = pin_valid_artifact(&registry);

        let mut command_changed = valid_artifact();
        command_changed.runtime.container.as_mut().unwrap().command = vec!["--verbose".to_string()];
        let updated = registry.update("project-a", "sma", command_changed).unwrap();

        assert!(
            updated.runtime.container.as_ref().unwrap().resolved_image_identity.is_some(),
            "a command change (unrelated to image/image_source_kind) must preserve the existing pin"
        );
        assert_ne!(
            updated.control_plane.execution_projection_digest,
            pinned.control_plane.execution_projection_digest,
            "command IS execution-relevant; the projection must actually change"
        );
        assert_eq!(
            updated.control_plane.execution_generation,
            pinned.control_plane.execution_generation + 1,
            "exactly one generation bump for the one real content change"
        );
    }

    #[test]
    fn p1_1_test_c_image_change_on_a_pinned_artifact_clears_the_pin_and_bumps_generation() {
        let registry = empty_registry();
        let pinned = pin_valid_artifact(&registry);

        let mut image_changed = valid_artifact();
        image_changed.runtime.container.as_mut().unwrap().image = "datum/sma:2026-02".to_string();
        let updated = registry.update("project-a", "sma", image_changed).unwrap();

        assert!(
            updated.runtime.container.as_ref().unwrap().resolved_image_identity.is_none(),
            "an image change invalidates whatever was pinned against the OLD image; the pin must be cleared, not carried over"
        );
        assert_eq!(updated.control_plane.execution_generation, pinned.control_plane.execution_generation + 1);
    }

    #[test]
    fn p1_1_test_d_source_kind_change_on_a_pinned_artifact_clears_the_pin_and_bumps_generation() {
        let registry = empty_registry();
        let pinned = pin_valid_artifact(&registry);

        let mut kind_changed = valid_artifact();
        kind_changed.runtime.container.as_mut().unwrap().image_source_kind = IMAGE_SOURCE_KIND_LOCAL_BUILD.to_string();
        let updated = registry.update("project-a", "sma", kind_changed).unwrap();

        assert!(
            updated.runtime.container.as_ref().unwrap().resolved_image_identity.is_none(),
            "a source-kind change invalidates a pin obtained under the OLD classification; it must be cleared, not carried over"
        );
        assert_eq!(updated.control_plane.execution_generation, pinned.control_plane.execution_generation + 1);
    }

    #[test]
    fn p1_1_test_e_client_submitted_resolved_identity_is_still_rejected_on_an_already_pinned_artifact() {
        let registry = empty_registry();
        pin_valid_artifact(&registry);

        let mut fabricated_on_top = valid_artifact();
        fabricated_on_top.runtime.container.as_mut().unwrap().resolved_image_identity = Some(fabricated_resolved_identity());
        let error = registry.update("project-a", "sma", fabricated_on_top).unwrap_err();
        assert!(
            matches!(&error, ServiceArtifactRegistryError::Invalid(message) if message.contains("server-owned")),
            "a client can never inject a resolved_image_identity, whether or not the artifact already carries a real one: {error:?}"
        );
        // And nothing was mutated by the rejected attempt.
        let still_pinned = registry.get("project-a", "sma").unwrap();
        assert!(still_pinned.runtime.container.as_ref().unwrap().resolved_image_identity.is_some());
    }

    // ---- v5 review P0-6: image_source_kind participates in the digest -- //

    #[test]
    fn p0_6_test_registry_to_local_build_changes_digest_and_generation() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();
        assert_eq!(created.runtime.container.as_ref().unwrap().image_source_kind, IMAGE_SOURCE_KIND_REGISTRY);

        let mut changed = created.clone();
        changed.runtime.container.as_mut().unwrap().image_source_kind = IMAGE_SOURCE_KIND_LOCAL_BUILD.to_string();
        let updated = registry.update("project-a", "sma", changed).unwrap();
        assert_eq!(
            updated.control_plane.execution_generation,
            created.control_plane.execution_generation + 1,
            "changing source_kind alone (same reference/resolved values) is a safety-relevant change and must bump generation"
        );
        assert_ne!(updated.control_plane.execution_projection_digest, created.control_plane.execution_projection_digest);
    }

    #[test]
    fn p0_6_test_registry_to_legacy_unclassified_changes_digest_and_generation() {
        let registry = empty_registry();
        let created = registry.create("project-a", valid_artifact()).unwrap();

        let mut changed = created.clone();
        changed.runtime.container.as_mut().unwrap().image_source_kind = IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED.to_string();
        let updated = registry.update("project-a", "sma", changed).unwrap();
        assert_eq!(updated.control_plane.execution_generation, created.control_plane.execution_generation + 1);
        assert_ne!(updated.control_plane.execution_projection_digest, created.control_plane.execution_projection_digest);
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

    // ---- Stage 107B-A (ADR-0013): per-artifact schema migration -------- //

    #[test]
    fn legacy_service_artifact_schema_migrates_on_load_and_is_idempotent() {
        let path = temp_db_path("legacy-artifact-schema-migrate");
        let mut legacy_artifact = valid_artifact();
        legacy_artifact.control_plane.project_id = "project-a".to_string();
        legacy_artifact.schema_version = LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
        // Simulate a Stage-107A-migrated artifact (envelope already current,
        // generation already established) whose per-artifact schema axis is
        // still legacy -- the common real-world case. The stored digest
        // below stands in for "whatever an old, pre-Stage-107B-A process
        // computed"; migration must recompute and overwrite it regardless
        // of its prior value.
        legacy_artifact.control_plane.execution_generation = 1;
        legacy_artifact.control_plane.execution_projection_digest = format!("sha256:{}", "0".repeat(64));
        let state = ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![legacy_artifact])].into_iter().collect(),
            execution_generation_high_water: [(
                "project-a".to_string(),
                [("sma".to_string(), 1u64)].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &state).unwrap();

        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "sma").unwrap();
        assert_eq!(artifact.schema_version, SERVICE_ARTIFACT_SCHEMA_VERSION);
        assert_eq!(
            artifact.control_plane.execution_generation, 2,
            "the per-artifact schema migration must bump generation by exactly one -- a genuine, schema-caused digest change, not a no-op"
        );
        assert!(crate::core::jcs::is_canonical_digest_format(&artifact.control_plane.execution_projection_digest));
        drop(migrated);

        // Idempotency: reloading the now-migrated database must not re-bump.
        let reloaded = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact_again = reloaded.get("project-a", "sma").unwrap();
        assert_eq!(artifact_again.schema_version, SERVICE_ARTIFACT_SCHEMA_VERSION);
        assert_eq!(artifact_again.control_plane.execution_generation, 2);

        cleanup(&path);
    }

    // ---- v7 review P1-2: envelope + per-artifact migration composition - //

    #[test]
    fn p1_2_test_envelope_and_artifact_both_legacy_migrate_correctly_in_one_load() {
        // A genuinely old state, legacy on BOTH axes at once: the envelope
        // was never migrated by any Stage-107A-era binary, AND this
        // artifact's own schema_version predates Stage 107B-A. Before the
        // P1-2 fix, `load_from_path` validated (and would have persisted)
        // the envelope-only-migrated candidate FIRST -- and
        // `validate_registry_state` calls `validate_artifact`, which
        // rejects ANY artifact whose own `schema_version` isn't current --
        // so this exact state could never load at all.
        let path = temp_db_path("p1-2-both-legacy");
        let mut legacy_artifact = valid_artifact();
        legacy_artifact.control_plane.project_id = "project-a".to_string();
        legacy_artifact.schema_version = LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
        legacy_artifact.control_plane.execution_generation = 0;
        legacy_artifact.control_plane.execution_projection_digest = String::new();
        let legacy_state = ServiceArtifactRegistryState {
            schema_version: LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![legacy_artifact])].into_iter().collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &legacy_state).unwrap();

        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "sma").unwrap();
        assert_eq!(artifact.schema_version, SERVICE_ARTIFACT_SCHEMA_VERSION, "per-artifact schema must be current after one load");
        assert_eq!(
            artifact.control_plane.execution_generation, 2,
            "two independently-justified bumps: one from the envelope migration (Stage 107A), one from the per-artifact schema migration (Stage 107B-A) -- composed correctly, not lost or double-counted"
        );
        assert!(crate::core::jcs::is_canonical_digest_format(&artifact.control_plane.execution_projection_digest));

        // Persisted immediately, on both axes, in one write.
        let raw: ServiceArtifactRegistryState =
            load_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN).unwrap().expect("row must exist");
        assert_eq!(raw.schema_version, SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION, "envelope must be persisted as current");
        assert_eq!(raw.artifacts_by_project["project-a"][0].schema_version, SERVICE_ARTIFACT_SCHEMA_VERSION);
        assert_eq!(raw.artifacts_by_project["project-a"][0].control_plane.execution_generation, 2);
        let high_water = raw
            .execution_generation_high_water
            .get("project-a")
            .and_then(|per_artifact| per_artifact.get("sma"))
            .copied()
            .unwrap_or(0);
        assert!(high_water >= 2, "durable high-water mark must be at least the final generation");
        drop(migrated);

        // Idempotency: reloading the now-fully-migrated database must not
        // re-bump either axis again.
        let reloaded = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact_again = reloaded.get("project-a", "sma").unwrap();
        assert_eq!(artifact_again.schema_version, SERVICE_ARTIFACT_SCHEMA_VERSION);
        assert_eq!(artifact_again.control_plane.execution_generation, 2);

        cleanup(&path);
    }

    #[test]
    fn p1_2_test_invalid_fully_migrated_candidate_is_not_persisted_when_both_axes_are_legacy() {
        // Same doubly-legacy shape as the test above, but the artifact's
        // OWN `control_plane.project_id` disagrees with the project key it
        // is stored under -- a defect that migration itself never touches
        // and that only becomes visible once `validate_registry_state`
        // actually runs against the FULLY migrated candidate. Proves the
        // composed flow still validates before persisting: an invalid
        // fully-migrated candidate must leave the on-disk row completely
        // unchanged, not partially migrated.
        let path = temp_db_path("p1-2-invalid-both-legacy");
        let mut artifact = valid_artifact();
        artifact.control_plane.project_id = "wrong-project".to_string(); // mismatches the "project-a" key below
        artifact.schema_version = LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
        artifact.control_plane.execution_generation = 0;
        artifact.control_plane.execution_projection_digest = String::new();
        let legacy_state = ServiceArtifactRegistryState {
            schema_version: LEGACY_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![artifact])].into_iter().collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &legacy_state).unwrap();

        // `ServiceArtifactRegistry` does not implement `Debug`, so
        // `Result::unwrap_err` (which requires the `Ok` side to be `Debug`
        // too) cannot be used here.
        let error = match ServiceArtifactRegistry::load_from_path(&path) {
            Err(error) => error,
            Ok(_) => panic!("expected load_from_path to reject the invalid fully-migrated candidate"),
        };
        assert!(error.contains("project_id"), "{error}");

        // Nothing was persisted: the on-disk row is exactly the original,
        // still-doubly-legacy state -- never an intermediate,
        // envelope-only-migrated one.
        let raw: ServiceArtifactRegistryState =
            load_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN).unwrap().expect("row must exist");
        assert_eq!(raw.schema_version, LEGACY_REGISTRY_STATE_SCHEMA_VERSION);
        assert_eq!(raw.artifacts_by_project["project-a"][0].schema_version, LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION);

        cleanup(&path);
    }

    /// v5 review P0-5: builds a raw JSON envelope, as an old
    /// (pre-Stage-107B-A) persisted row would actually look on disk --
    /// `runtime.container` genuinely has NO `image_source_kind` key at all
    /// (not merely a Rust struct literal that happens to omit setting it),
    /// so deserialization must exercise the real serde default.
    fn raw_legacy_state_json(artifact_id: &str, image: &str) -> Value {
        serde_json::json!({
            "schema_version": SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION,
            "artifacts_by_project": {
                "project-a": [{
                    "schema_version": LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION,
                    "artifact_id": artifact_id,
                    "service_id": artifact_id,
                    "runtime": {
                        "kind": "container",
                        "container": {
                            "container_name": format!("datum-{artifact_id}"),
                            "image": image,
                            "ports": [],
                            "network": null,
                            "network_aliases": [],
                            "volumes": [],
                            "configuration_mounts": [],
                            "command": []
                            // NOTE: no "image_source_kind", no "resolved_image_identity" key at all.
                        }
                    },
                    "probes": {
                        "execution": { "kind": "docker_container_running", "params": {}, "vantage_point": null },
                        "health": null,
                        "availability": null
                    },
                    "metadata": null,
                    "control_plane": {
                        "project_id": "project-a",
                        "name": artifact_id,
                        "version": "1",
                        "artifact_kind": "container",
                        "target_nodes": ["fog-01"],
                        "target_stages": [],
                        "dependencies": [],
                        "acquisition": { "kind": "image", "source": null, "revision": null, "sha256": null },
                        "lifecycle": { "pre_install": [], "install": [], "configure": [], "start": [], "stop": [], "post_install": [], "rollback": [] },
                        "health_checks": [],
                        "wasm_decision": {
                            "explicit_invocation_required": true, "module_id": "reconcile_service", "function_name": "decide",
                            "host_imports_allowed": false, "side_effects_allowed": false, "safe_to_auto_execute": false
                        },
                        "bindings": {},
                        "secret_refs": [],
                        "secret_bindings": [],
                        "created_at_utc": "2020-01-01T00:00:00Z",
                        "updated_at_utc": "2020-01-01T00:00:00Z",
                        "execution_generation": 1,
                        "execution_projection_digest": format!("sha256:{}", "0".repeat(64))
                    }
                }]
            },
            "execution_generation_high_water": { "project-a": { artifact_id: 1u64 } },
            "project_pinning_modes": {}
        })
    }

    #[test]
    fn p0_5_test_1_arbitrary_old_registry_backed_artifact_migrates_to_legacy_unclassified() {
        let path = temp_db_path("p0-5-registry-like");
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &raw_legacy_state_json("mqtt", "eclipse-mosquitto:2")).unwrap();
        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "mqtt").unwrap();
        assert_eq!(
            artifact.runtime.container.as_ref().unwrap().image_source_kind,
            IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED,
            "a registry-shaped image name must NOT be inferred as 'registry' -- it must land in legacy_unclassified like any other migrated artifact missing the field"
        );
        cleanup(&path);
    }

    #[test]
    fn p0_5_test_2_arbitrary_old_local_looking_artifact_also_migrates_to_legacy_unclassified() {
        let path = temp_db_path("p0-5-local-like");
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &raw_legacy_state_json("sma", "datum/sma:dev")).unwrap();
        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "sma").unwrap();
        assert_eq!(
            artifact.runtime.container.as_ref().unwrap().image_source_kind,
            IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED,
            "a local-build-looking image name must ALSO NOT be inferred as 'local_build' -- migration never guesses from the image string"
        );
        cleanup(&path);
    }

    #[test]
    fn p0_5_test_3_migration_never_infers_source_kind_from_the_image_name_either_direction() {
        // Same assertion as tests 1/2, side by side, to make the "no
        // inference in either direction" property explicit in one place.
        let registry_like_path = temp_db_path("p0-5-no-inference-registry");
        let local_like_path = temp_db_path("p0-5-no-inference-local");
        persist_domain_json(&registry_like_path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &raw_legacy_state_json("a", "postgres:16")).unwrap();
        persist_domain_json(&local_like_path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &raw_legacy_state_json("a", "local/custom-build:dev")).unwrap();

        let registry_like = ServiceArtifactRegistry::load_from_path(&registry_like_path).unwrap().get("project-a", "a").unwrap();
        let local_like = ServiceArtifactRegistry::load_from_path(&local_like_path).unwrap().get("project-a", "a").unwrap();
        assert_eq!(
            registry_like.runtime.container.as_ref().unwrap().image_source_kind,
            local_like.runtime.container.as_ref().unwrap().image_source_kind,
            "the migrated classification must be identical (legacy_unclassified) regardless of what the image string looks like"
        );
        assert_eq!(registry_like.runtime.container.as_ref().unwrap().image_source_kind, IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED);

        cleanup(&registry_like_path);
        cleanup(&local_like_path);
    }

    #[test]
    fn p0_5_test_4_migrated_unclassified_artifact_cannot_be_pinned() {
        let path = temp_db_path("p0-5-cannot-pin");
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &raw_legacy_state_json("mqtt", "eclipse-mosquitto:2")).unwrap();
        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "mqtt").unwrap();
        assert_eq!(artifact.runtime.container.as_ref().unwrap().image_source_kind, IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED);
        // `pin_image_identity` (dserver::core::oci_image_pinning) refuses
        // `legacy_unclassified` before ever reaching the network resolver --
        // see oci_image_pinning::tests::legacy_unclassified_artifacts_are_refused
        // for the direct proof against this same source kind; this test
        // proves the MIGRATION path actually produces that refused kind.
        cleanup(&path);
    }

    #[test]
    fn p0_5_test_5_migrated_unclassified_artifact_remains_required_ineligible() {
        let path = temp_db_path("p0-5-required-ineligible");
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &raw_legacy_state_json("mqtt", "eclipse-mosquitto:2")).unwrap();
        let migrated = ServiceArtifactRegistry::load_from_path(&path).unwrap();
        let artifact = migrated.get("project-a", "mqtt").unwrap();
        // Mirrors the exact check required_completeness_gaps performs (see
        // artifact_reconciliation_context::required_completeness_gaps and
        // its legacy_unclassified_image_is_always_gated_with_a_distinct_message
        // test) -- proven directly against the MIGRATED artifact's own
        // classification here.
        assert_eq!(artifact.runtime.container.as_ref().unwrap().image_source_kind, IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED);
        cleanup(&path);
    }

    /// v6 review P0-1: reads the REAL, checked-in `artifacts/catalog/*.json`
    /// files directly from disk (not a Rust struct literal standing in for
    /// them) and proves every one of them explicitly classifies
    /// `image_source_kind` -- the v2 bundle's migration-notes.txt incorrectly
    /// implied this was already true; it was not, because the default-value
    /// fix (P0-5) only affects records that OMIT the field, and the 19
    /// registry-backed catalog files still omitted it at that time.
    #[test]
    fn catalog_inventory_classifies_every_real_artifact_explicitly() {
        let catalog_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("artifacts").join("catalog");
        let mut total = 0u32;
        let mut registry_count = 0u32;
        let mut local_build_count = 0u32;
        let mut entries: Vec<_> = std::fs::read_dir(&catalog_dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", catalog_dir.display()))
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect();
        entries.sort();
        for path in entries {
            // scenario.json is a topology/scenario document, not a
            // ServiceArtifact (no runtime/control_plane shape at all) --
            // skipped by its known, singular filename, never by guessing
            // from content.
            if path.file_name().and_then(|name| name.to_str()) == Some("scenario.json") {
                continue;
            }
            total += 1;
            let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            let artifact: ServiceArtifact = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("{} must deserialize as ServiceArtifact: {error}", path.display()));
            assert_eq!(
                artifact.schema_version, SERVICE_ARTIFACT_SCHEMA_VERSION,
                "{} has an unexpected schema_version", path.display()
            );
            let container = artifact
                .runtime
                .container
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no container runtime definition", path.display()));
            match container.image_source_kind.as_str() {
                IMAGE_SOURCE_KIND_REGISTRY => registry_count += 1,
                IMAGE_SOURCE_KIND_LOCAL_BUILD => local_build_count += 1,
                IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED => panic!(
                    "{} deserialized as legacy_unclassified -- every real catalog artifact must explicitly declare image_source_kind, never rely on the safe-but-unusable default",
                    path.display()
                ),
                other => panic!("{} has an unrecognized image_source_kind '{other}'", path.display()),
            }
        }
        assert_eq!(total, 22, "expected exactly 22 real catalog artifacts (excluding scenario.json)");
        assert_eq!(registry_count, 19, "expected exactly 19 explicitly registry-classified catalog artifacts");
        assert_eq!(local_build_count, 3, "expected exactly 3 explicitly local_build-classified catalog artifacts (sma, lora-device-simulator, lora-packet-forwarder)");
    }

    #[test]
    fn invalid_per_artifact_migration_candidate_never_overwrites_existing_state() {
        let path = temp_db_path("invalid-artifact-schema-migration");
        let mut artifact = valid_artifact();
        artifact.control_plane.project_id = "wrong-project".to_string(); // mismatches the "project-a" key below
        artifact.schema_version = LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
        artifact.control_plane.execution_generation = 1;
        artifact.control_plane.execution_projection_digest = format!("sha256:{}", "0".repeat(64));
        let state = ServiceArtifactRegistryState {
            schema_version: SERVICE_ARTIFACT_REGISTRY_STATE_SCHEMA_VERSION.to_string(),
            artifacts_by_project: [("project-a".to_string(), vec![artifact])].into_iter().collect(),
            execution_generation_high_water: [(
                "project-a".to_string(),
                [("sma".to_string(), 1u64)].into_iter().collect(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        persist_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN, &state).unwrap();
        let before = serde_json::to_value(&state).unwrap();

        assert!(ServiceArtifactRegistry::load_from_path(&path).is_err());

        let after: serde_json::Value = load_domain_json(&path, SERVICE_ARTIFACT_REGISTRY_DOMAIN).unwrap().unwrap();
        assert_eq!(after, before, "a per-artifact migration candidate that fails validation must never be persisted");

        cleanup(&path);
    }

    #[test]
    fn validate_artifact_rejects_a_schema_version_that_is_not_current() {
        // This exercises the same strict-equality mechanism that (unmodified,
        // no code change on that side) makes a pre-Stage-107B-A binary --
        // compiled with SERVICE_ARTIFACT_SCHEMA_VERSION = "0.1.0" -- refuse a
        // migrated ("0.2.0") artifact rather than silently re-serializing it
        // without resolved_image_identity/image_source_kind. Old binary code
        // cannot be executed from this repository, so this test demonstrates
        // the mechanism the downgrade-safety guarantee rests on: any
        // schema_version other than the validator's own current constant is
        // rejected outright, in either direction.
        let mut artifact = valid_artifact();
        artifact.control_plane.project_id = "project-a".to_string();
        artifact.schema_version = LEGACY_SERVICE_ARTIFACT_SCHEMA_VERSION.to_string();
        assert!(matches!(validate_artifact(&artifact), Err(ServiceArtifactRegistryError::Invalid(_))));
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
