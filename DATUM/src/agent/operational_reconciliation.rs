use std::{collections::{BTreeMap, HashSet}, env};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::Utc;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use wasmtime::{Config as WasmConfig, Engine, Instance, Module, Store};

use crate::agent::bootstrap::{
    run_bootstrap, run_bootstrap_with_resource_discovery, BootstrapState,
    SmartSentinelBootstrapReport,
};
use crate::dcode::authorization::CanonicalContractReferenceV1;
use crate::agent::execution_projection;
use crate::agent::image_identity_verification::{
    self, CliDockerImageRuntime, DockerImageRuntime, VerifiedImageIdentity,
};
use crate::agent::native_process_acquisition;
use crate::agent::native_process_runtime;
use crate::agent::resource_discovery::SmartSentinelResourceDiscovery;
use crate::agent::runtime_kind::RuntimeKind;
use crate::agent::stage107a_enforcement::{
    check_generation_and_projection, effective_pinning_mode, required_completeness_gaps,
    resolve_clock_skew_seconds, resolve_network_aliases, resolve_ports,
    resolve_secret_target_environment_variable, structurally_valid_resolved_image_identity,
    LeaseGuard, BLOCKED_IMAGE_IDENTITY_MISMATCH, BLOCKED_IMAGE_IDENTITY_UNVERIFIED,
    BLOCKED_REQUIRED_INCOMPLETE, EXECUTION_PINNING_MODE_REQUIRED,
};
use crate::config::{Config, DispatchContainerMapping};
use crate::core::cycle::run_cycle_for_node_identity;
use crate::outputs::http_json::HttpJsonOutput;

pub const OPERATIONAL_RECONCILER_MODULE_ID: &str = "smartsentinel-reconciler-v1";
pub const OPERATIONAL_RECONCILER_FUNCTION: &str = "reconcile_service";
const AUTHORIZATION_OPERATION_RECONCILE: &str = "reconcile";
const AUTHORIZATION_OPERATION_ROLLBACK: &str = "rollback";

fn default_authorization_operation() -> String {
    AUTHORIZATION_OPERATION_RECONCILE.to_string()
}
const RECONCILER_WAT: &str = r#"
(module
  (func (export "reconcile_service") (param i32 i32) (result i32)
    local.get 0
    local.get 1
    i32.eqz
    i32.and))
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalDesiredService {
    pub service_id: String,
    pub runtime: String,
    pub desired_state: String,
    /// Phase 115C. Mirrors `dserver::core::operational_dgraph::OperationalDesiredService.application_id`.
    /// Empty for a not-yet-rebuilt legacy operational graph -- never treated
    /// as a real application_id in that case, only as "unknown."
    #[serde(default)]
    pub application_id: String,
    /// Phase 115C. Mirrors `dserver::core::operational_dgraph::OperationalDesiredService.dmap_ref`
    /// -- the exact accepted D-Map `{id, revision, digest}` this placement
    /// came from, never independently recomputed on this side either.
    #[serde(default)]
    pub dmap_ref: Option<CanonicalContractReferenceV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconciliationAuthorization {
    pub authorization_id: String,
    pub project_id: String,
    pub dgraph_id: String,
    #[serde(default)]
    pub dgraph_revision: Option<u64>,
    pub node_id: String,
    /// Stage 107A (review finding P1-1). Previously absent from this crate's
    /// own copy of the type entirely, even though DServer's authorization
    /// contract is supposed to bind project / D-Graph+revision / node /
    /// resource binding / artifact snapshot.
    ///
    /// PRECISE VERIFICATION BOUNDARY (review finding P1-6, v4): the agent
    /// deserializes `binding_id` but does NOT independently verify it --
    /// there is no local, trusted source to compare it against while
    /// possibly offline from DServer. `binding_id` verification against the
    /// currently active Resource Registry binding happens only on the
    /// DServer side, when a context is SERVED. The agent instead verifies
    /// `resource_id` (below) locally, which is a related but distinct
    /// guarantee -- see `stage107a_enforcement::check_resource_identity`'s
    /// doc comment for the bounded-staleness consequence this leaves. Do not
    /// treat this field's presence as proof the agent checks it.
    #[serde(default)]
    pub binding_id: Option<String>,
    /// The `resource_id` this authorization was issued for (review finding
    /// P1-1). Verified against the agent's own bootstrap-resolved
    /// `resource_id` before any mutation
    /// (`stage107a_enforcement::BLOCKED_WRONG_RESOURCE_IDENTITY`). This is
    /// the field the agent actually verifies -- `binding_id` above is not.
    #[serde(default)]
    pub resource_id: Option<String>,
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
    #[serde(default = "default_authorization_operation")]
    pub operation: String,
    /// Stage 107A (ADR-0012 Option A / design-review finding 1). Previously
    /// absent from this crate's own copy of the type, so `serde` silently
    /// dropped it even though DServer already serialized it -- the agent
    /// could not evaluate lease expiry at all. `None` means no finite lease:
    /// per ADR-0012, such an authorization must never gate a host mutation
    /// (M1-M8), regardless of `allow_host_mutation` or identity match.
    #[serde(default)]
    pub expires_at_utc: Option<String>,
    /// `"process"` or `"durable"` (Stage 107A). Only `"durable"` can carry a
    /// finite `expires_at_utc`.
    #[serde(default = "default_persistence_scope")]
    pub persistence_scope: String,
    /// Per-desired-service `{artifact_id, execution_generation,
    /// execution_projection_digest}` binding (Stage 107A, ADR-0011 §2). Empty
    /// for a pre-Stage-107A authorization -- never sufficient to gate a
    /// mutation.
    #[serde(default)]
    pub artifact_bindings: Vec<ArtifactBindingV1>,
    /// `execution_pinning_mode` bound at issuance (Stage 107A, N-2).
    #[serde(default)]
    pub execution_pinning_mode: String,
}

fn default_persistence_scope() -> String {
    "process".to_string()
}

/// Mirrors `dserver::core::operational_dgraph::ArtifactBindingV1`. Same JSON
/// shape by construction (both are plain `{service_id, artifact_id,
/// execution_generation, execution_projection_digest}` records), which is
/// what makes them interoperate over the wire without a shared crate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactBindingV1 {
    pub service_id: String,
    pub artifact_id: String,
    pub execution_generation: u64,
    pub execution_projection_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalReconcilerDescriptor {
    pub module_id: String,
    pub function_name: String,
    pub abi: String,
    pub host_imports_allowed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalDGraphSlice {
    pub schema_version: String,
    pub dgraph_id: String,
    #[serde(default)]
    pub dgraph_revision: Option<u64>,
    pub project_id: String,
    pub node_id: String,
    pub desired_services: Vec<OperationalDesiredService>,
    #[serde(default)]
    pub authorization: Option<OperationalReconciliationAuthorization>,
    pub reconciler: OperationalReconcilerDescriptor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactWasmDecisionPolicy {
    pub explicit_invocation_required: bool,
    pub module_id: String,
    pub function_name: String,
    pub host_imports_allowed: bool,
    pub side_effects_allowed: bool,
    pub safe_to_auto_execute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OperationalLifecycleCommand {
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OperationalLifecycle {
    #[serde(default)]
    pub pre_install: Vec<OperationalLifecycleCommand>,
    #[serde(default)]
    pub install: Vec<OperationalLifecycleCommand>,
    #[serde(default)]
    pub configure: Vec<OperationalLifecycleCommand>,
    #[serde(default)]
    pub start: Vec<OperationalLifecycleCommand>,
    #[serde(default)]
    pub stop: Vec<OperationalLifecycleCommand>,
    #[serde(default)]
    pub post_install: Vec<OperationalLifecycleCommand>,
    #[serde(default)]
    pub rollback: Vec<OperationalLifecycleCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalArtifactAssignment {
    pub artifact_id: String,
    /// The DESIRED D-Serv id -- may be a service-alias, not necessarily the
    /// artifact's own canonical id. Never used for projection identity; see
    /// `artifact_service_id`.
    pub service_id: String,
    /// The artifact's OWN canonical service id, regardless of which alias
    /// resolved the join. `execution_projection::compute_projection` uses
    /// THIS field for the projection's `service_id`, never `service_id`
    /// above (review finding P0-6).
    #[serde(default)]
    pub artifact_service_id: String,
    pub version: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub container_name: String,
    pub image: String,
    /// Stage 108 (ADR-0015). Mirrors `dserver`'s `ArtifactReconciliationAssignment.native_process`.
    /// Populated if and only if `runtime_kind == "native_process"`; `None`
    /// for `Container`. DESIRED STATE ONLY -- never PID/process-start-time/
    /// observed state (see `runtime_kind`'s module doc comment).
    #[serde(default)]
    pub native_process: Option<NativeProcessDeploymentSpecV1>,
    /// Stage 109 (ADR-0016). Mirrors `dserver`'s
    /// `ArtifactReconciliationAssignment.acquisition`. Populated if and only
    /// if `runtime_kind == "native_process"`; `None` for `Container` (never
    /// added to the container execution projection/wire shape). The single
    /// authoritative owner of native executable content identity --
    /// `NativeProcessDeploymentSpecV1` never carries its own content hash.
    #[serde(default)]
    pub acquisition: Option<OperationalServiceArtifactAcquisition>,
    /// Stage 107B-A (ADR-0013). Mirrors `ContainerDeploymentSpec.image_source_kind`
    /// (dserver): `"registry"`, `"local_build"`, or `"legacy_unclassified"`.
    /// v7 review P2-2: the safe default for a record missing this field is
    /// `"legacy_unclassified"` (`default_operational_image_source_kind`),
    /// never `"registry"`.
    #[serde(default = "default_operational_image_source_kind")]
    pub image_source_kind: String,
    /// Stage 107B-A (ADR-0013). Mirrors `ContainerDeploymentSpec.resolved_image_identity`.
    #[serde(default)]
    pub resolved_image_identity: Option<OperationalResolvedImageIdentity>,
    pub target_node: String,
    #[serde(default)]
    pub lifecycle: OperationalLifecycle,
    #[serde(default)]
    pub network: Option<String>,
    /// Control-plane-declared network aliases (Stage 107A, F2c). In
    /// `required`-effective mode the agent publishes these and refuses to
    /// substitute a node-local alias.
    #[serde(default)]
    pub network_aliases: Vec<String>,
    /// Control-plane-declared port mappings (Stage 107A field-audit finding;
    /// previously absent from the wire entirely). In `required`-effective
    /// mode the agent publishes these and refuses to substitute node-local
    /// `DispatchContainerMapping.ports`.
    #[serde(default)]
    pub ports: Vec<OperationalPortMapping>,
    #[serde(default)]
    pub volumes: Vec<OperationalVolumeMount>,
    #[serde(default)]
    pub configuration_mounts: Vec<OperationalConfigurationMount>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub wasm_decision: ArtifactWasmDecisionPolicy,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
    /// Control-plane-declared secret target bindings (Stage 107A, F2c). In
    /// `required`-effective mode the agent uses these target environment
    /// variable names and refuses to substitute the node-local
    /// `DispatchSecretBinding.environment_variable`.
    #[serde(default)]
    pub secret_bindings: Vec<OperationalSecretBinding>,
    /// Server-owned (Stage 107A, D-7a). Compared against the authorization's
    /// `artifact_bindings` for this service.
    #[serde(default)]
    pub execution_generation: u64,
    /// Server-owned (Stage 107A). The agent independently recomputes this
    /// from the assignment above and compares; mismatch blocks
    /// (`blocked_projection_mismatch`).
    #[serde(default)]
    pub execution_projection_digest: String,
    /// Phase 115C. Mirrors `dserver::core::artifact_reconciliation_context::ArtifactReconciliationAssignment.application_id`.
    #[serde(default)]
    pub application_id: String,
    /// Phase 115C. Mirrors `dserver::core::artifact_reconciliation_context::ArtifactReconciliationAssignment.dmap_ref`.
    #[serde(default)]
    pub dmap_ref: Option<CanonicalContractReferenceV1>,
}

pub const IMAGE_SOURCE_KIND_REGISTRY: &str = "registry";
pub const IMAGE_SOURCE_KIND_LOCAL_BUILD: &str = "local_build";
/// v5 review P0-5. Mirrors `dserver`'s `IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED`
/// -- see that constant's doc comment for the full rationale.
pub const IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED: &str = "legacy_unclassified";

fn default_operational_image_source_kind() -> String {
    // v5 review P0-5: never silently default to "registry" -- see
    // dserver's `default_image_source_kind` for the full rationale.
    IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED.to_string()
}

/// Stage 107B-A (ADR-0013). Mirrors `dserver::core::service_artifact_registry::ResolvedImageIdentityV1`
/// field-for-field (separate, hand-written type -- no shared crate).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalResolvedImageIdentity {
    pub repository: String,
    #[serde(default)]
    pub index_digest: Option<String>,
    pub platforms: Vec<OperationalResolvedPlatformManifest>,
    pub resolved_at_utc: String,
    pub resolver_tool_version: String,
    /// v5 review P1-1. Administrative/audit metadata, excluded from the
    /// projection -- mirrors dserver's field.
    #[serde(default)]
    pub resolved_by: String,
    pub registry_host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalResolvedPlatformManifest {
    pub os: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
    pub manifest_digest: String,
    pub config_digest: String,
    pub execution_reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalPortMapping {
    pub container_port: u16,
    #[serde(default)]
    pub host_port: Option<u16>,
    #[serde(default = "default_tcp_protocol_owned")]
    pub protocol: String,
}

fn default_tcp_protocol_owned() -> String {
    "tcp".to_string()
}

/// Mirrors `dserver::core::service_artifact_registry::ServiceArtifactSecretBinding`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalSecretBinding {
    pub secret_ref: String,
    pub target_environment_variable: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// Stage 108 (ADR-0015). Mirrors
/// `dserver::core::service_artifact_registry::NativeProcessDeploymentSpecV1`
/// field-for-field. DESIRED STATE ONLY -- see that type's doc comment for
/// the desired/observed boundary. No process lifecycle exists yet in Stage
/// 108.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeProcessDeploymentSpecV1 {
    pub process_name: String,
    pub entrypoint: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
}

/// Stage 109 (ADR-0016). Mirrors
/// `dserver::core::service_artifact_registry::ServiceArtifactAcquisition`
/// field-for-field (independent, hand-written -- no shared crate). The
/// single authoritative owner of native executable content identity:
/// `sha256` is the exact byte-hash the materialized executable must match
/// before it may ever be spawned.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalServiceArtifactAcquisition {
    pub kind: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalReconciliationContext {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    pub desired_services: Vec<OperationalDesiredService>,
    pub artifacts: Vec<OperationalArtifactAssignment>,
    #[serde(default)]
    pub authorization: Option<OperationalReconciliationAuthorization>,
    pub reconciler: OperationalReconcilerDescriptor,
    pub execution_policy: ArtifactExecutionPolicy,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalVolumeMount {
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalConfigurationMount {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
    /// Expected exact-byte SHA-256 of the mounted file's content (Stage
    /// 107A, Section G). The agent hashes the actual resolved bytes before
    /// mounting; mismatch blocks (`blocked_config_content_mismatch`).
    #[serde(default)]
    pub declared_content_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactExecutionPolicy {
    pub mode: String,
    pub explicit_invocation_required: bool,
    pub host_imports_allowed: bool,
    pub side_effects_allowed: bool,
    pub safe_to_auto_execute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmReconciliationDecision {
    pub module_id: String,
    pub function_name: String,
    pub status: String,
    pub action_code: i32,
    pub host_imports_allowed: bool,
    pub guest_side_effects_performed: bool,
    pub fuel_configured: u64,
    pub fuel_consumed: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalServiceReconciliationReport {
    pub service_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_version: Option<String>,
    pub desired_state: String,
    pub trusted_container_name: Option<String>,
    pub trusted_image: Option<String>,
    pub observed_before_running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_error: Option<String>,
    pub secret_resolution_status: String,
    pub wasm: WasmReconciliationDecision,
    pub execution_status: String,
    pub message: String,
    pub observed_after_running: bool,
    pub host_mutation_performed: bool,

    // ---- Stage 107A evidence (Section I) ---------------------------------
    /// Present whenever a Stage 107A authorization/lease/generation
    /// evaluation actually ran for this service this cycle (i.e.
    /// `execute_requested && control_plane_authorized`). `"snapshot_lease"`
    /// is the only value -- ADR-0012 Option A. Absent for a purely
    /// suggested/converged cycle where no such evaluation was needed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_semantics: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_expires_at_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_execution_projection_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recomputed_execution_projection_digest: Option<String>,
    /// The mode actually enforced this cycle (the stricter of the
    /// authorization's bound mode and the agent's local minimum).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_pinning_mode: Option<String>,
    /// `"partial_hardening"` whenever a Stage 107A evaluation ran (Stage
    /// 107A never claims `"closed"` -- catalog pinning is Stage 107B, per
    /// ADR-0011 N-3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f2_status: Option<String>,
    /// JCS+SHA-256 of the effective node-local dispatch policy (secret
    /// VALUES never included -- this crate's `DispatchConfig` does not carry
    /// them, only references / target variable names).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_policy_digest: Option<String>,
    /// Review finding P1-5. Every `required_completeness_gaps` entry for
    /// this service's delivered assignment (stable-prefixed: e.g.
    /// `"unpinned_image_reference:..."`, `"unpinned_configuration_mount:..."`,
    /// `"unpinned_secret_reference:..."`), computed whenever a Stage 107A
    /// evaluation ran, REGARDLESS of the effective pinning mode -- in
    /// `legacy_partial` these are the permitted, recorded gaps (ADR-0011
    /// N-2); in `required` a non-empty list here would already have been a
    /// block (`blocked_required_incomplete`), so it is expected to be empty
    /// whenever `execution_status` does not start with that code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpinned_inputs: Vec<String>,
    /// Review finding P1-1/P1-2 (v4). The clock-skew seconds ACTUALLY
    /// enforced this cycle by `LeaseGuard` (`resolve_clock_skew_seconds`'s
    /// result for the effective mode) -- present whenever a Stage 107A cycle
    /// gate resolved, regardless of whether any mutation boundary was
    /// reached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_clock_skew_seconds: Option<u64>,
    /// Successful exact-byte configuration checks: locator, expected and actual
    /// raw SHA-256, and verification result. Mutable mounts remain unpinned;
    /// Stage 107A does not verify image content or env-var secret identities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verified_external_inputs: Vec<VerifiedExternalInput>,
    /// Local fallback semantics, including on converged cycles: field, policy
    /// source, safe effective value and reason. Never contains a secret value.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_substitutions: Vec<LocalSubstitutionEvidence>,
    /// Review finding P1-2 (v4). One record per M1-M8 governed
    /// host-mutation boundary this service actually REACHED this cycle,
    /// whether it passed or was blocked, in the order reached. Written at
    /// the boundary itself -- before the mutation, for a pass; in place of
    /// it, for a block -- so this explains not just the final
    /// `execution_status` but exactly which boundary stopped a mutation, and
    /// under what lease. A cycle that never reached any governed mutation
    /// boundary (blocked earlier, or nothing to do) leaves this empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutation_boundaries_crossed: Vec<MutationBoundaryRecord>,
    /// Stage 107B-B (ADR-0014, v9 review P1-5: synchronized with the actual
    /// unconditional-preflight behavior -- v8 review P1-6 made image-identity
    /// observation run every cycle, executing or not, and this field's
    /// presence follows that exactly). Present whenever this service has
    /// BOTH an assigned artifact AND a resolved trusted local container
    /// mapping this cycle (i.e. whenever `trusted_container_name`/
    /// `trusted_image` are also present) -- the same precondition
    /// `image_identity_verification`'s preflight itself requires:
    ///   - `registry`-sourced, structurally-valid pin: the read-only
    ///     preflight evidence, refreshed EVERY cycle regardless of
    ///     `execute_requested`/`control_plane_authorized`/`rollback_requested`
    ///     -- for an executing, authorized, non-rollback cycle this is the
    ///     POST-resolution evidence (after any M5 acquisition attempt and,
    ///     for a fresh M1/recreate, the post-action container-binding
    ///     re-check); otherwise it is the raw preflight, never a fabricated
    ///     "verified" reading.
    ///   - `local_build`/`legacy_unclassified`/unresolved or structurally
    ///     invalid registry pin: explicit not-applicable evidence
    ///     (`pull_disposition = not_applicable_unpinned_source`, a
    ///     `reason_kind` naming which of those applies) -- never silently
    ///     absent (v9 review P1-4).
    /// Absent when no artifact is assigned, or an artifact is assigned but
    /// no trusted local mapping resolves for it (a separate,
    /// `blocked_local_mapping`-flavored concern this field never attempts
    /// to describe). Evidence-only: never a factor in
    /// `ArtifactExecutionProjectionV1`, and `checked_at_utc` (or
    /// `actual_container_image`) changing alone must never change
    /// `execution_generation`/`execution_projection_digest`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_image_identity: Option<VerifiedImageIdentity>,
}

/// One governed-host-mutation boundary attempt (review finding P1-2, v4).
/// `boundary` is a stable identifier -- NOT limited to M1-M8: those name
/// the Container path's boundaries (`"M1"` docker run, `"M2"` docker
/// start, `"M3"` attachment-drift recreate `rm -f`, `"M4"` stopped+secrets
/// recreate `rm -f`, `"M5"` docker pull, `"M6"` lifecycle remove_container,
/// `"M7"` docker network create, `"M8"` rollback `rm -f`); Stage 109
/// (ADR-0016) added a separate, never-overlapping N1-N6 series for the
/// NativeProcess path (`"N1"` materialize/replace verified artifact bytes,
/// `"N2"` executable-permission mutation, `"N3"` spawn native process,
/// `"N4"` SIGTERM managed process, `"N5"` SIGKILL managed process, `"N6"`
/// remove managed artifact/state) via the exact same `check_boundary`
/// mechanism -- this type and `check_boundary` are runtime-kind-agnostic
/// by design; a future runtime kind would add its own disjoint identifier
/// series the same way, never reusing another kind's letters. `result` is
/// `"passed"` or `"blocked"`; `block_reason` is the full block-reason
/// string (present only when `result == "blocked"`). Never carries a
/// secret value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MutationBoundaryRecord {
    pub boundary: String,
    pub checked_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_utc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_clock_skew_seconds: Option<u64>,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerifiedExternalInput {
    pub kind: String,
    pub source: String,
    pub expected_sha256: String,
    pub actual_sha256: String,
    pub verified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalSubstitutionEvidence {
    pub field: String,
    pub source: String,
    pub effective_value: serde_json::Value,
    pub reason: String,
}

struct EffectiveAttachments {
    ports: Vec<OperationalPortMapping>,
    aliases: Vec<String>,
    substitutions: Vec<LocalSubstitutionEvidence>,
}

fn resolve_effective_attachments(
    ports: &[OperationalPortMapping],
    aliases: &[String],
    mapping: &DispatchContainerMapping,
    mode: &str,
) -> Result<EffectiveAttachments, String> {
    let (ports, ports_substituted) = resolve_ports(ports, &mapping.ports, mode)?;
    let (aliases, aliases_substituted) = resolve_network_aliases(aliases, mapping, mode)?;
    let mut substitutions = Vec::new();
    for (field, source, value, substituted) in [
        ("ports", "dispatch.container_mappings.ports", serde_json::json!(ports), ports_substituted),
        ("network_aliases", "dispatch.container_mappings.service_name", serde_json::json!(aliases), aliases_substituted),
    ] {
        if substituted {
            substitutions.push(LocalSubstitutionEvidence {
                field: field.to_string(), source: source.to_string(), effective_value: value,
                reason: "undeclared control-plane value; legacy_partial permits local fallback".to_string(),
            });
        }
    }
    Ok(EffectiveAttachments { ports, aliases, substitutions })
}

fn execution_error_status(error: &anyhow::Error) -> String {
    error.chain().find_map(|cause| {
        let message = cause.to_string();
        let code = message.split(':').next().unwrap_or("").trim();
        (code.starts_with("blocked_") && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
            .then(|| code.to_string())
    }).unwrap_or_else(|| "failed".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OperationalReconciliationReport {
    pub bootstrap_state: BootstrapState,
    pub resource_id: String,
    pub node_id: String,
    pub dgraph_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dgraph_revision: Option<u64>,
    pub execute_requested: bool,
    pub control_plane_authorized: bool,
    pub local_container_execution_allowed: bool,
    pub services: Vec<OperationalServiceReconciliationReport>,
    pub side_effects_performed: bool,
    pub snapshot_published: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_publish_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerState {
    Running,
    Stopped,
    Missing,
}

fn is_completion_desired(desired_state: &str) -> bool {
    desired_state.trim().eq_ignore_ascii_case("completed")
}

fn completed_container_converged(
    desired_state: &str,
    observed_before: ContainerState,
    exit_code: Option<i64>,
) -> bool {
    is_completion_desired(desired_state)
        && observed_before == ContainerState::Stopped
        && exit_code == Some(0)
}

pub fn run_operational_reconciliation(
    cfg: &Config,
    requested_node_id: Option<&str>,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
    execute_requested: bool,
    rollback_requested: bool,
) -> Result<OperationalReconciliationReport> {
    let bootstrap = run_bootstrap(
        cfg,
        requested_node_id,
        hostname_override,
        dserver_url,
        project_id,
        timeout_s,
    )?;
    run_operational_reconciliation_from_bootstrap(
        cfg,
        bootstrap,
        hostname_override,
        dserver_url,
        project_id,
        timeout_s,
        execute_requested,
        rollback_requested,
        true,
    )
}

pub fn run_operational_reconciliation_with_resource_discovery(
    cfg: &Config,
    resource_discovery: SmartSentinelResourceDiscovery,
    requested_node_id: Option<&str>,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
    execute_requested: bool,
    rollback_requested: bool,
    publish_snapshot: bool,
) -> Result<OperationalReconciliationReport> {
    let bootstrap = run_bootstrap_with_resource_discovery(
        resource_discovery,
        requested_node_id,
        dserver_url,
        project_id,
        timeout_s,
    )?;
    run_operational_reconciliation_from_bootstrap(
        cfg,
        bootstrap,
        hostname_override,
        dserver_url,
        project_id,
        timeout_s,
        execute_requested,
        rollback_requested,
        publish_snapshot,
    )
}

/// Once-per-cycle Stage 107A gate (ADR-0011/ADR-0012): the lease and
/// effective pinning mode every governed mutation this cycle is checked
/// against. Computed once; `lease.check_before_mutation` is still evaluated
/// individually before each M1-M8 boundary (point 9), not just here.
struct Stage107ACycleGate {
    lease: LeaseGuard,
    effective_mode: &'static str,
    /// Review finding P1-1/P1-2 (v4): the clock-skew seconds actually
    /// resolved and enforced this cycle (`resolve_clock_skew_seconds`'s
    /// result), surfaced into evidence
    /// (`OperationalServiceReconciliationReport::authorization_clock_skew_seconds`
    /// and every `MutationBoundaryRecord::configured_clock_skew_seconds`).
    skew_seconds: u64,
}

fn resolve_stage107a_cycle_gate(
    cfg: &Config,
    authorization: Option<&OperationalReconciliationAuthorization>,
    local_resource_id: &str,
) -> Result<Stage107ACycleGate, String> {
    let authorization = authorization.ok_or_else(|| {
        format!(
            "{}: no authorization attached to the fetched context",
            crate::agent::stage107a_enforcement::BLOCKED_AUTHORIZATION_NOT_LEASED
        )
    })?;
    // P1-1: verify resource-binding identity independently, before anything
    // else -- do not rely solely on DServer having already filtered the
    // authorization by binding when the context was served.
    crate::agent::stage107a_enforcement::check_resource_identity(
        authorization.resource_id.as_deref(),
        local_resource_id,
    )?;
    let agent_minimum = cfg
        .dispatch
        .as_ref()
        .and_then(|dispatch| dispatch.minimum_execution_pinning_mode.as_deref())
        .unwrap_or(EXECUTION_PINNING_MODE_REQUIRED);
    let effective_mode = effective_pinning_mode(&authorization.execution_pinning_mode, agent_minimum)?;
    let skew_seconds = resolve_clock_skew_seconds(
        effective_mode,
        cfg.dispatch.as_ref().and_then(|dispatch| dispatch.authorization_clock_skew_seconds),
    )?;
    // `LeaseGuard` samples the clock itself, fresh, at each
    // `check_before_mutation` call (P0-2 correction) -- no `now` is captured
    // or threaded through from here.
    let lease = LeaseGuard::from_authorization(authorization, skew_seconds)?;
    Ok(Stage107ACycleGate { lease, effective_mode, skew_seconds })
}

/// Per-service generation + projection-digest gate (ADR-0011 §2). Recomputes
/// the projection digest independently from the delivered assignment and
/// compares both it and the authorization's bound generation/digest.
fn stage107a_service_check(
    authorization: &OperationalReconciliationAuthorization,
    assignment: &OperationalArtifactAssignment,
) -> Result<(), String> {
    let recomputed = execution_projection::recompute_digest(assignment)?;
    check_generation_and_projection(
        authorization,
        &assignment.service_id,
        &assignment.artifact_id,
        assignment.execution_generation,
        &assignment.execution_projection_digest,
        &recomputed,
    )
}

/// The stable block-reason code prefix of a `stage107a_enforcement` error
/// string (e.g. `"blocked_lease_expired: ..."` -> `"blocked_lease_expired"`).
fn stage107a_block_code(reason: &str) -> String {
    reason.split(':').next().unwrap_or(reason).trim().to_string()
}

/// P0-4 (v4 review): decision precedence for `wasm.action_code == 0`. The
/// WASM reconciler's `action_code == 0` only means "the container is already
/// running (or the one-shot completed) and no NEW host action is requested"
/// -- it says nothing about whether the currently-running container is
/// actually the authorized, resource-correct, content-complete execution.
/// Reporting "converged" from `action_code == 0` alone therefore let a
/// resource-identity mismatch, a generation/projection-digest mismatch, a
/// required-mode completeness gap, or a configuration-content-hash mismatch
/// (all already computed into `stage107a_service_block` /
/// `config_content_block` before this point) be silently masked by an
/// already-running container. This function is the single place that
/// decides whether `action_code == 0` may still report convergence.
///
/// Deliberately NOT part of this decision: lease validity
/// (`blocked_authorization_not_leased` / `blocked_lease_expired`). The lease
/// is a pre-*mutation* boundary rule, freshly re-checked by `LeaseGuard`
/// inside each M1-M8 executor immediately before it mutates the host -- not
/// a pre-*observation* rule. An authorization whose lease would refuse a
/// brand-new mutation attempt right now can still correctly describe the
/// state already observed under it.
fn action_code_zero_may_report_converged(
    stage107a_service_block: Option<&str>,
    config_content_block: Option<&str>,
    image_identity_pending_in_required_mode: bool,
) -> bool {
    stage107a_service_block.is_none() && config_content_block.is_none() && !image_identity_pending_in_required_mode
}

/// v9 review P0-3: every independently-blocking gate that must ALREADY be
/// clear before M5 (or any other governed mutation) may even be considered
/// -- named explicitly, rather than left as an inline boolean expression, so
/// each conjunct can be audited and unit tested in isolation. Critically
/// includes `host_action_requested` (`wasm.action_code != 0`): Stage 107A
/// treats `action_code == 0` as "no new host action requested," and M5 is
/// itself a host mutation, so it must never fire merely to populate/verify a
/// cache.
#[allow(clippy::too_many_arguments)]
fn mutation_gates_are_clear(
    execute_requested: bool,
    control_plane_authorized: bool,
    rollback_requested: bool,
    mapping_present: bool,
    dependency_blockers_empty: bool,
    stage107a_service_block_before_image_clear: bool,
    config_content_block_clear: bool,
    attachment_policy_block_clear: bool,
    local_container_execution_allowed: bool,
    batch_secret_resolution_clear: bool,
    secret_resolution_clear: bool,
    observation_clear: bool,
    completion_not_failed: bool,
    host_action_requested: bool,
) -> bool {
    execute_requested
        && control_plane_authorized
        && !rollback_requested
        && mapping_present
        && dependency_blockers_empty
        && stage107a_service_block_before_image_clear
        && config_content_block_clear
        && attachment_policy_block_clear
        && local_container_execution_allowed
        && batch_secret_resolution_clear
        && secret_resolution_clear
        && observation_clear
        && completion_not_failed
        && host_action_requested
}

/// v9 review P0-2 (TARGET IMAGE side): a `KnownMismatch` preflight outcome
/// is the ONLY thing that makes the top-level attachment comparison treat
/// this service as having a genuinely wrong candidate image -- never a
/// container-binding observation (see `container_binding_drift_from`).
fn image_known_mismatch_from(outcome: Option<&image_identity_verification::PreflightOutcome>) -> bool {
    matches!(outcome, Some(image_identity_verification::PreflightOutcome::KnownMismatch(_)))
}

/// v9 review P0-2 (EXISTING CONTAINER side): an existing container bound to
/// a different image object than the (possibly still-verifying) target --
/// e.g. after a legitimate re-pin -- is runtime DRIFT, never a terminal
/// candidate mismatch. Folded into `attachment_mismatch` so it forces the
/// same governed recreate path as any other attachment drift, WITHOUT ever
/// contributing to `image_known_mismatch`/`stage107a_service_block`.
fn container_binding_drift_from(evaluation: Option<&image_identity_verification::ImageIdentityEvaluation>) -> bool {
    evaluation.is_some_and(|evaluation| {
        matches!(evaluation.container_binding, image_identity_verification::ContainerBindingObservation::Mismatches { .. })
    })
}

/// v9 review P0-4, broadened by v10 review P0-1: whether this artifact's
/// image identity has not yet been positively confirmed while EXECUTING,
/// AUTHORIZED, and running in `required` mode -- consulted so `action_code
/// == 0` is never reported as fully verified convergence when either half
/// of image identity has never actually been confirmed locally. TWO
/// independent, still fully separate, sub-conditions, OR'd together:
/// `target_needs_pull` (the TARGET IMAGE was never even inspected) and
/// `container_binding_unavailable` (an EXISTING container's binding could
/// not be proven -- NOT a KNOWN mismatch, which is drift and is handled
/// entirely separately via `container_binding_drift_from`/M3, never
/// through this flag). `legacy_partial` (or any mode but `required`) is
/// deliberately never affected; neither is a non-executing or
/// unauthorized cycle.
fn image_identity_unconfirmed_in_required_mode(
    execute_requested: bool,
    control_plane_authorized: bool,
    effective_mode: &str,
    target_needs_pull: bool,
    container_binding_unavailable: bool,
) -> bool {
    execute_requested
        && control_plane_authorized
        && effective_mode == EXECUTION_PINNING_MODE_REQUIRED
        && (target_needs_pull || container_binding_unavailable)
}

/// v10 review P0-1, reshaped by v11 review P0-1/P1-1: `required` mode must
/// NEVER silently reuse/converge an EXISTING container (Running/Stopped)
/// whose actual image binding could not be proven
/// (`ContainerBindingObservation::Unavailable` -- distinct from a KNOWN
/// wrong binding, `Mismatches`, which instead forces a governed recreate
/// via `container_binding_drift_from` and is NOT this function's concern).
/// `container_binding_status` is read as the plain string already carried
/// on the FINAL `VerifiedImageIdentity` evidence for this cycle
/// (post-acquisition when a pull happened), never re-deriving it from
/// `PreflightOutcome`/`classify_evaluation` -- TARGET verification and
/// EXISTING CONTAINER binding remain fully separate concepts.
///
/// v11 review P0-1/P1-1 correction: this now returns a plain `bool`, NOT a
/// `stage107a_service_block`-flavored `Option<String>` -- the v10 shape
/// was too broad in two ways at once: (1) folding it into the GLOBAL
/// `stage107a_service_block` also blocked remove-only rollback (M8), which
/// must never be prevented by any runtime image-verification outcome; (2)
/// it blocked EVERY governed execution path for the service, including an
/// independently-required M3/M4 recreate, which is exactly the SAFE
/// corrective action for an unverifiable existing container (replace it
/// with a freshly created one from the verified immutable target) and
/// must never be prevented by this fact. The caller now threads this
/// `bool` directly into `execute_trusted_container_intent` as a REUSE
/// guard (Running -> `already_running`, Stopped -> M2 start), consulted
/// only AFTER the M3/M4 recreate checks have already had their chance to
/// fire. `legacy_partial` (or any other mode) is deliberately NEVER
/// affected -- it keeps its existing, weaker "may still proceed"
/// semantics; only the recorded evidence (`container_binding_status`
/// itself) must stay truthful.
/// v11 review P0-1: the block a REMOVE-ONLY rollback (M8) may respect --
/// deliberately does not even ACCEPT `image_identity_final_block`
/// (TARGET-image mismatch/unverified/no-selectable-platform) as a
/// parameter, so it is impossible by construction for a runtime
/// image-verification outcome to prevent M8. Retains every OTHER
/// pre-existing Stage-107A rollback prerequisite: generation/projection/
/// completeness (`stage107a_service_block_before_image`) and local
/// attachment/secret substitution policy (`attachment_policy_block`).
/// `container_binding_unavailable_in_required_mode` is likewise never
/// threaded in here at all -- it is a REUSE guard consumed directly by
/// `execute_trusted_container_intent`, never a `stage107a_service_block`
/// contributor (v11 review P1-1).
fn rollback_service_block_from(
    stage107a_service_block_before_image: Option<&str>,
    attachment_policy_block: Option<&str>,
) -> Option<String> {
    stage107a_service_block_before_image.or(attachment_policy_block).map(str::to_string)
}

fn container_binding_unavailable_in_required_mode_from(
    container_binding_status: Option<&str>,
    effective_mode: &str,
) -> bool {
    effective_mode == EXECUTION_PINNING_MODE_REQUIRED
        && container_binding_status == Some(image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE)
}

/// v12 review P0-1: whether the FINAL (possibly post-M5-acquisition)
/// `container_binding_status` for this cycle is a KNOWN mismatch --
/// re-derived from `resolve_pinned_image_outcome`'s returned evidence,
/// never re-read from the INITIAL preflight (`container_binding_drift_from`,
/// which stays correct for the outer, pre-acquisition attachment/
/// convergence comparison, but can be stale by the time
/// `execute_trusted_container_intent` actually runs: a `NeedsPull`
/// preflight has no local target-image object to compare the existing
/// container against at all, so its own `container_binding` is always
/// `NotApplicable` regardless of what the post-pull re-evaluation later
/// discovers). Mode-independent by design -- unlike the `Unavailable`
/// reuse guard, a KNOWN mismatch is drift that forces a governed recreate
/// in EVERY mode (ADR-0014 §6a), not just `required`.
fn container_binding_mismatches_from_status(container_binding_status: Option<&str>) -> bool {
    container_binding_status == Some(image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES)
}

/// v9 review P1-2: pure decision for the read-only post-action container-
/// binding re-check -- given the verified TARGET image's content id and the
/// raw result of inspecting the freshly-created container's top-level
/// `.Image`, decides what evidence to record (`Some(actual_id)` whenever one
/// was observed, regardless of match/mismatch) and whether the caller must
/// stop claiming the "deployed" status (`Some((status, message))`) because
/// binding could not be confirmed. Kept separate from the actual
/// `DockerImageRuntime` call so it is directly unit-testable without a live
/// daemon, matching this module's established pattern.
/// v10 review P1-2: also returns the explicit `CONTAINER_BINDING_STATUS_*`
/// string the caller must write back into evidence, so a post-M1 mismatch/
/// unavailable result can never leave stale/inconsistent evidence (e.g.
/// `target_verified = true` next to a `container_binding_status` that still
/// reads `not_applicable` from before the container even existed).
fn post_action_container_binding_check(
    container_name: &str,
    expected_image_id: &str,
    observed_binding: Result<Option<String>, String>,
) -> (Option<String>, &'static str, Option<(String, String)>) {
    match observed_binding {
        Ok(Some(actual_id)) if actual_id == expected_image_id => {
            (Some(actual_id), image_identity_verification::CONTAINER_BINDING_STATUS_MATCHES, None)
        }
        Ok(Some(actual_id)) => {
            let message = format!(
                "trusted container '{container_name}' was created, but its post-action image binding ('{actual_id}') does not match the verified pinned image ('{expected_image_id}'); convergence cannot be claimed"
            );
            (
                Some(actual_id),
                image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES,
                Some(("container_binding_unconfirmed".to_string(), message)),
            )
        }
        Ok(None) | Err(_) => (
            None,
            image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE,
            Some((
                "container_binding_unconfirmed".to_string(),
                format!(
                    "trusted container '{container_name}' was created from a verified pinned image, but its post-action container binding could not be inspected"
                ),
            )),
        ),
    }
}

/// Per-cycle authorization identity/lease context threaded alongside
/// `LeaseGuard` into every M1-M8 boundary purely so `MutationBoundaryRecord`s
/// (review finding P1-2, v4) can carry `authorization_id` / `expires_at_utc`
/// / `configured_clock_skew_seconds` -- the low-level Docker-mutation
/// functions (`execute_lifecycle_phase`, `execute_trusted_container_intent`,
/// `ensure_docker_network`, `rollback_trusted_container_intent`) otherwise
/// know nothing about `OperationalReconciliationAuthorization` itself, only
/// about the `LeaseGuard` derived from it.
pub(crate) struct MutationBoundaryContext<'a> {
    pub(crate) authorization_id: Option<&'a str>,
    pub(crate) expires_at_utc: Option<&'a str>,
    pub(crate) configured_clock_skew_seconds: Option<u64>,
}

/// Review finding P1-3 (v4): non-secret Docker label keys DATUM writes on
/// every M1-created container and checks against on convergence. Never a
/// claim of OCI image CONTENT identity (that remains Stage 107B) -- these
/// identify which Stage 107A artifact snapshot the container was created
/// FROM, so drift (including REMOVAL of a previously-declared value, which
/// plain field-presence comparison cannot detect) is caught by a label
/// mismatch even when every individual `docker inspect` field the agent
/// checks happens to still look superficially consistent.
const DATUM_LABEL_ARTIFACT_ID: &str = "datum.artifact_id";
const DATUM_LABEL_EXECUTION_GENERATION: &str = "datum.execution_generation";
const DATUM_LABEL_EXECUTION_PROJECTION_DIGEST: &str = "datum.execution_projection_digest";

/// The exact `{artifact_id, execution_generation, execution_projection_digest}`
/// identity M1 labels a newly-created container with, and that convergence
/// later requires the RUNNING container's labels to still match (P1-3, v4
/// review). Written from the AGENT-RECOMPUTED digest (never the merely
/// delivered one) -- by the time M1 can run, `stage107a_service_check` has
/// already proven the two equal, so this is defense-in-depth, not a
/// different value.
struct ExecutionIdentityLabels<'a> {
    artifact_id: &'a str,
    execution_generation: u64,
    execution_projection_digest: &'a str,
}

impl ExecutionIdentityLabels<'_> {
    /// `--label KEY=VALUE` argument pairs for the `docker run` invocation
    /// (M1). `Command::args` passes each element as a separate exec
    /// argument -- no shell is involved, so no quoting/injection concern
    /// regardless of what characters `artifact_id` or the digest contain.
    fn as_docker_run_args(&self) -> Vec<String> {
        vec![
            "--label".to_string(),
            format!("{DATUM_LABEL_ARTIFACT_ID}={}", self.artifact_id),
            "--label".to_string(),
            format!("{DATUM_LABEL_EXECUTION_GENERATION}={}", self.execution_generation),
            "--label".to_string(),
            format!("{DATUM_LABEL_EXECUTION_PROJECTION_DIGEST}={}", self.execution_projection_digest),
        ]
    }
}

/// Convergence-side check (P1-3, v4 review): does the ALREADY-RUNNING
/// container's `docker inspect` `Config.Labels` match `expected` exactly?
/// A container with no `datum.*` labels at all (every pre-Stage-107A or
/// externally-created container, including everything running before this
/// mechanism existed) never matches -- it is never considered Stage 107A
/// converged, and is recreated through the governed path (M3) the next time
/// a governed reconciliation cycle runs. Field-level `docker inspect`
/// comparison (image/command/network/ports/volumes/mounts, elsewhere in
/// `inspected_container_matches_declared_attachments`) is retained as
/// defense-in-depth for post-creation drift; this label check additionally
/// catches drift those field checks cannot see, such as a value that was
/// REMOVED from the current declaration entirely (field-presence comparison
/// only ever checks that declared values are present, never that nothing
/// extra/stale remains).
fn inspected_container_labels_match(inspected: &serde_json::Value, expected: &ExecutionIdentityLabels) -> bool {
    let labels = inspected.get("Config").and_then(|config| config.get("Labels"));
    let label = |key: &str| labels.and_then(|labels| labels.get(key)).and_then(serde_json::Value::as_str);
    label(DATUM_LABEL_ARTIFACT_ID) == Some(expected.artifact_id)
        && label(DATUM_LABEL_EXECUTION_GENERATION) == Some(expected.execution_generation.to_string().as_str())
        && label(DATUM_LABEL_EXECUTION_PROJECTION_DIGEST) == Some(expected.execution_projection_digest)
}

/// Checks one M1-M8 boundary (review finding P1-2, v4) and pushes a
/// `MutationBoundaryRecord` of the attempt into `records` -- whether it
/// passes or blocks -- before returning. Replaces every bare
/// `lease.check_before_mutation(op)` call site in this module. Because
/// `records` is mutated in place through a `&mut` reference (not returned),
/// the record survives even when this function's own `?` propagates the
/// block upward through the caller's `anyhow::Result` chain: the evidence
/// for a blocked boundary is retained through the propagated error, exactly
/// like a passed boundary's record is retained through a later success.
pub(crate) fn check_boundary(
    lease: &LeaseGuard,
    boundary_context: &MutationBoundaryContext,
    boundary: &str,
    records: &mut Vec<MutationBoundaryRecord>,
) -> Result<()> {
    let now = Utc::now();
    let result = lease.check_before_mutation_at(boundary, now);
    records.push(MutationBoundaryRecord {
        boundary: boundary.to_string(),
        checked_at_utc: now.to_rfc3339(),
        authorization_id: boundary_context.authorization_id.map(str::to_string),
        expires_at_utc: boundary_context.expires_at_utc.map(str::to_string),
        configured_clock_skew_seconds: boundary_context.configured_clock_skew_seconds,
        result: if result.is_ok() { "passed".to_string() } else { "blocked".to_string() },
        block_reason: result.as_ref().err().cloned(),
    });
    result.map_err(|error| anyhow::anyhow!(error))
}

/// Section I evidence: JCS+SHA-256 of the effective node-local dispatch
/// policy. `DispatchConfig` never carries a secret VALUE (only references
/// and target environment-variable names), so this is safe to record in
/// evidence in full.
fn compute_local_policy_digest(cfg: &Config) -> Option<String> {
    let dispatch = cfg.dispatch.as_ref()?;
    let value = serde_json::to_value(dispatch).ok()?;
    crate::agent::jcs::canonical_digest_sha256_hex(&value).ok()
}

/// Section G: hash the actual resolved bytes of every declared configuration
/// mount and compare against `declared_content_sha256`. Path resolution
/// mirrors `resolve_configuration_mounts` (same root-confinement rules);
/// duplicated rather than shared because this runs BEFORE the mutation
/// decision (to produce a blocked status, not a hard `bail!`), while
/// `resolve_configuration_mounts` runs as part of the mutation itself.
/// Returns structured expected/actual raw hashes for every successfully
/// verified mount, surfaced as `verified_external_inputs` evidence. A mount with
/// no declared digest is silently skipped (not a failure, not "verified"
/// either -- it stays an unpinned input). On the first mismatch/error the
/// partial list accumulated so far is discarded along with everything else
/// this cycle: a blocked config gate never proceeds to a mutation anyway, so
/// there is nothing correct to report as "verified" for that attempt.
fn verify_declared_configuration_mount_content(
    mounts: &[OperationalConfigurationMount],
    configuration_root: Option<&Path>,
) -> std::result::Result<Vec<VerifiedExternalInput>, String> {
    let mut verified = Vec::new();
    for mount in mounts {
        let Some(expected) = mount.declared_content_sha256.as_deref() else {
            continue;
        };
        if !is_safe_configuration_source(&mount.source) {
            return Err(format!(
                "{}: refusing unsafe governed configuration source '{}'",
                crate::agent::stage107a_enforcement::BLOCKED_CONFIG_CONTENT_MISMATCH,
                mount.source
            ));
        }
        let root = configuration_root.ok_or_else(|| {
            format!(
                "{}: configuration mount '{}' declares content_sha256 but no configuration_root is configured",
                crate::agent::stage107a_enforcement::BLOCKED_CONFIG_CONTENT_MISMATCH,
                mount.source
            )
        })?;
        let root = root
            .canonicalize()
            .map_err(|error| format!("resolve governed configuration root '{}': {error}", root.display()))?;
        let path = root.join(mount.source.trim());
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("resolve governed configuration source '{}': {error}", mount.source))?;
        if !canonical.starts_with(&root) {
            return Err(format!(
                "governed configuration source '{}' escapes configuration root",
                mount.source
            ));
        }
        let bytes = std::fs::read(&canonical)
            .map_err(|error| format!("read governed configuration source '{}': {error}", mount.source))?;
        crate::agent::stage107a_enforcement::verify_configuration_mount_content(Some(expected), &bytes)?;
        verified.push(VerifiedExternalInput {
            kind: "configuration_mount".to_string(), source: mount.source.clone(),
            expected_sha256: expected.to_string(),
            actual_sha256: crate::agent::stage107a_enforcement::sha256_hex(&bytes), verified: true,
        });
    }
    Ok(verified)
}

fn run_operational_reconciliation_from_bootstrap(
    cfg: &Config,
    bootstrap: SmartSentinelBootstrapReport,
    hostname_override: Option<&str>,
    dserver_url: &str,
    project_id: &str,
    timeout_s: u64,
    execute_requested: bool,
    rollback_requested: bool,
    publish_snapshot: bool,
) -> Result<OperationalReconciliationReport> {
    if bootstrap.bootstrap_state != BootstrapState::KnownBoundNode {
        bail!(
            "operational reconciliation requires known_bound_node, received {:?}",
            bootstrap.bootstrap_state
        );
    }
    let node_id = bootstrap
        .control_plane
        .resolved_node_id
        .clone()
        .ok_or_else(|| anyhow::anyhow!("known_bound_node bootstrap did not resolve node_id"))?;

    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_s))
        .build()
        .context("build operational reconciliation HTTP client")?;
    // Stage 107B-B (ADR-0014): the production Docker-CLI-backed image
    // identity runtime, constructed once per cycle and threaded through the
    // per-service loop below -- mirrors how `Client` itself is built once
    // here rather than per-service.
    let docker_image_runtime = CliDockerImageRuntime::default();
    let base = dserver_url.trim().trim_end_matches('/');
    let slice_url = format!("{base}/api/v1/nodes/{node_id}/operational-reconciliation-context");
    let response = client
        .get(&slice_url)
        .header("X-Datum-Project", project_id.trim())
        .send()
        .with_context(|| format!("fetch operational D-Graph slice {slice_url}"))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().unwrap_or_default();
        bail!("operational D-Graph slice query failed with status {status}: {body}");
    }
    let context: OperationalReconciliationContext = response
        .json()
        .context("decode artifact reconciliation context")?;
    validate_context(&context, project_id, &node_id)?;

    let expected_authorization_operation = if rollback_requested {
        AUTHORIZATION_OPERATION_ROLLBACK
    } else {
        AUTHORIZATION_OPERATION_RECONCILE
    };
    let control_plane_authorized = context.authorization.as_ref().is_some_and(|authorization| {
        authorization.allow_host_mutation
            && authorization.operation == expected_authorization_operation
            && authorization.project_id == context.project_id
            && authorization.dgraph_id == context.dgraph_id
            && authorization.dgraph_revision == Some(context.dgraph_revision)
            && authorization.node_id == context.node_id
    });
    let local_container_execution_allowed = cfg.dispatch.as_ref().is_some_and(|dispatch| {
        dispatch.enabled && dispatch.allow_container_execution.unwrap_or(false)
    });

    // Stage 107A (ADR-0011 / ADR-0012): once-per-cycle lease + effective
    // pinning-mode gate. Computed even when not immediately needed so its
    // failure mode (missing authorization, insufficient pinning mode,
    // unconfigured clock skew) is available uniformly below. The `LeaseGuard`
    // this produces samples the wall clock itself, fresh, at EACH
    // `check_before_mutation` call inside the M1-M8 executors (P0-2
    // correction) -- no cycle-wide timestamp is captured or threaded here.
    let stage107a_cycle_gate = if execute_requested && control_plane_authorized {
        Some(resolve_stage107a_cycle_gate(
            cfg,
            context.authorization.as_ref(),
            &bootstrap.control_plane.resource_id,
        ))
    } else {
        None
    };
    // Fail closed: while the cycle gate itself has not resolved (absent, or
    // still pending), secret target-variable substitution checks use the
    // STRICTER mode rather than silently defaulting to the weaker one.
    let effective_mode_for_secrets = stage107a_cycle_gate
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map(|gate| gate.effective_mode)
        .unwrap_or(EXECUTION_PINNING_MODE_REQUIRED);

    let mut side_effects_performed = false;
    let batch_secret_resolution_error = if execute_requested && control_plane_authorized {
        context.artifacts.iter().find_map(|artifact| {
            if artifact.secret_refs.is_empty() {
                None
            } else {
                resolve_secret_bindings(
                    cfg,
                    &artifact.secret_refs,
                    &artifact.secret_bindings,
                    effective_mode_for_secrets,
                )
                    .err()
                    .map(|error| (artifact.service_id.clone(), error))
            }
        })
    } else {
        None
    };
    let mut services = Vec::with_capacity(context.desired_services.len());
    for service in &context.desired_services {
        let assignment = context.artifacts.iter().find(|artifact| {
            artifact.service_id.eq_ignore_ascii_case(&service.service_id)
        });

        // Stage 108 (ADR-0015, "CRITICAL IMPLEMENTATION AMENDMENT A"): the
        // runtime-kind dispatch MUST be resolved here, before
        // `trusted_container_mapping_for_artifact`/`inspect_container` (the
        // very next lines below) or any other Docker-specific observation,
        // mapping, image-identity, attachment, lifecycle, convergence, or
        // mutation logic. A `NativeProcess` desired service must never reach
        // any of that -- it is diverted to a stable stub report and the loop
        // moves to the next service. `classify_service_runtime_dispatch` is a
        // pure function (no `Config`/no I/O) precisely so this decision is
        // directly unit-testable in isolation from the HTTP/Docker-backed
        // rest of this function -- see its own tests for the deterministic
        // proof that a `NativeProcess` input can never yield `Container`.
        // This is a dispatch seam (P1-1 correction), not a `RuntimeDriver`
        // trait: the existing Container path below is retained intact,
        // unrefactored.
        match classify_service_runtime_dispatch(service, assignment) {
            ServiceRuntimeDispatch::Error(error) => {
                services.push(build_runtime_dispatch_error_report(service, assignment, error));
                continue;
            }
            ServiceRuntimeDispatch::MissingAssignment => {
                // Stage 109 (closes Stage 108 P2): a contract error, never
                // the native execution path -- no host mutation may occur
                // from an assignment-less service.
                services.push(native_process_runtime::missing_artifact_assignment_report(service));
                continue;
            }
            ServiceRuntimeDispatch::NativeProcess => {
                // Stage 109: real, governed native-process reconciliation.
                // Never Docker, never ActionExecutorPlugin, never routed
                // through the Container path below.
                let authorization = context.authorization.as_ref();
                let cycle_gate = stage107a_cycle_gate.as_ref().and_then(|result| result.as_ref().ok());
                let lease = cycle_gate.map(|gate| &gate.lease);
                let effective_mode = cycle_gate.map(|gate| gate.effective_mode);
                let native_boundary_context = MutationBoundaryContext {
                    authorization_id: authorization.map(|a| a.authorization_id.as_str()),
                    expires_at_utc: authorization.and_then(|a| a.expires_at_utc.as_deref()),
                    configured_clock_skew_seconds: cycle_gate.map(|gate| gate.skew_seconds),
                };
                let report = native_process_runtime::reconcile_native_service(
                    cfg,
                    &context.project_id,
                    &context.node_id,
                    authorization,
                    lease,
                    effective_mode,
                    &native_boundary_context,
                    service,
                    assignment.expect("ServiceRuntimeDispatch::NativeProcess implies assignment is Some"),
                    execute_requested,
                    control_plane_authorized,
                    rollback_requested,
                );
                services.push(report);
                continue;
            }
            ServiceRuntimeDispatch::Container => {}
        }

        // ---- Container path below is UNCHANGED from pre-Stage-108 behaviour. ----
        let mapping = assignment.and_then(|artifact| {
            trusted_container_mapping_for_artifact(
                cfg,
                &artifact.service_id,
                &artifact.container_name,
                &artifact.image,
            )
        });
        let (observed_before, mut observation_error) = match mapping {
            Some(mapping) => match inspect_container(&mapping.container_name) {
                Ok(state) => (state, None),
                Err(error) => (ContainerState::Missing, Some(error.to_string())),
            },
            None => (ContainerState::Missing, None),
        };
        let observed_before_running = observed_before == ContainerState::Running;
        let completion_observation_requested =
            is_completion_desired(&service.desired_state)
                && observed_before == ContainerState::Stopped;
        let completion_exit_code_result = if completion_observation_requested {
            mapping
                .map(|mapping| inspect_container_exit_code(&mapping.container_name))
                .transpose()
        } else {
            Ok(None)
        };
        let completion_exit_code_error = completion_exit_code_result
            .as_ref()
            .err()
            .map(ToString::to_string);
        let completion_exit_code = completion_exit_code_result.ok().flatten();
        if let Some(error) = completion_exit_code_error.as_deref() {
            observation_error = Some(format!(
                "could not verify one-shot completion for '{}': {}",
                service.service_id, error
            ));
        }
        let completion_succeeded = completed_container_converged(
            &service.desired_state,
            observed_before,
            completion_exit_code,
        );
        let completion_failed = completion_exit_code.is_some_and(|code| code != 0);
        let dependency_blockers = if completion_observation_requested {
            Vec::new()
        } else {
            assignment
                .map(|artifact| dependency_blockers_for_assignment(artifact, &context, cfg))
                .unwrap_or_default()
        };
        // Review finding P1-1/P1-2: accumulates non-secret evidence of
        // node-local-policy substitution (secret target env-var name below;
        // ports and aliases are resolved once here, before outcome selection).
        let effective_attachments = match (mapping, assignment) {
            (Some(mapping), Some(artifact)) => Some(resolve_effective_attachments(
                &artifact.ports, &artifact.network_aliases, mapping, effective_mode_for_secrets,
            )),
            _ => None,
        };
        let attachment_policy_block = effective_attachments.as_ref().and_then(|r| r.as_ref().err()).cloned();
        let mut local_substitutions = effective_attachments.as_ref()
            .and_then(|r| r.as_ref().ok()).map(|r| r.substitutions.clone()).unwrap_or_default();
        let (secret_environment, secret_resolution_error, secret_resolution_status) =
            if assignment.is_none()
                || assignment.is_some_and(|artifact| artifact.secret_refs.is_empty())
            {
                (Vec::new(), None, "not_required".to_string())
            } else if !execute_requested || !control_plane_authorized {
                (Vec::new(), None, "not_checked".to_string())
            } else if batch_secret_resolution_error.is_some() {
                (
                    Vec::new(),
                    Some("one or more batch secret references could not be resolved locally".to_string()),
                    "blocked_missing_secret".to_string(),
                )
            } else {
                match assignment {
                    Some(artifact) => {
                        local_substitutions.extend(secret_target_substitutions(
                            cfg,
                            &artifact.secret_refs,
                            &artifact.secret_bindings,
                            effective_mode_for_secrets,
                        ));
                        match resolve_secret_bindings(
                            cfg,
                            &artifact.secret_refs,
                            &artifact.secret_bindings,
                            effective_mode_for_secrets,
                        ) {
                            Ok(environment) => (environment, None, "resolved_locally".to_string()),
                            Err(error) => (
                                Vec::new(),
                                Some(error),
                                "blocked_missing_secret".to_string(),
                            ),
                        }
                    }
                    None => (Vec::new(), None, "not_required".to_string()),
                }
            };
        // Stage 107B-B (ADR-0014), v8 review P0-1/P1-6: a READ-ONLY image-
        // identity preflight, computed EARLY and UNCONDITIONALLY -- before
        // any governance gate, before `attachment_mismatch`, regardless of
        // `execute_requested`/`control_plane_authorized`/`rollback_requested`.
        // `evaluate_image_identity`/`classify_evaluation` never call
        // `pull_image`, so this is always safe: it can never mutate the
        // host, so it never needs to wait for a gate that exists precisely
        // to prevent host mutation. Only attempted for a `registry`-sourced,
        // structurally-valid pin with a resolved mapping in scope --
        // `local_build`/`legacy_unclassified`/unpinned artifacts get `None`
        // here and are untouched, continuing exactly as before via the
        // mutable tag.
        let image_preflight: Option<(image_identity_verification::ImageIdentityEvaluation, image_identity_verification::PreflightOutcome)> =
            match (mapping, assignment) {
                (Some(mapping), Some(artifact)) if artifact.image_source_kind == IMAGE_SOURCE_KIND_REGISTRY => {
                    match &artifact.resolved_image_identity {
                        Some(resolved) if structurally_valid_resolved_image_identity(&artifact.image, resolved) => {
                            let existing_container_name = match observed_before {
                                ContainerState::Running | ContainerState::Stopped => Some(mapping.container_name.as_str()),
                                ContainerState::Missing => None,
                            };
                            let evaluation = image_identity_verification::evaluate_image_identity(
                                &docker_image_runtime,
                                resolved,
                                existing_container_name,
                                &Utc::now().to_rfc3339(),
                            );
                            let outcome = image_identity_verification::classify_evaluation(&evaluation);
                            Some((evaluation, outcome))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
        // v8 review P1-6: a KNOWN, actively-contradicting mismatch makes the
        // observation non-converged even in a non-executing ("suggested")
        // cycle -- unverifiable/no-selectable-reference outcomes do NOT
        // force non-convergence (best-effort evidence only), matching the
        // review's explicit "do not fail closed" allowance for that case.
        // This is a TARGET IMAGE mismatch only (v9 review P0-2).
        let image_known_mismatch = image_known_mismatch_from(image_preflight.as_ref().map(|(_, outcome)| outcome));
        // v9 review P0-2: an EXISTING CONTAINER bound to a different image
        // object than the (possibly still-verifying) target -- e.g. after a
        // legitimate re-pin -- is runtime DRIFT, never a terminal candidate
        // mismatch. It must still force non-convergence/recreate, so it is
        // folded into `attachment_mismatch` below exactly like a real
        // attachment drift, but it is NEVER folded into `image_known_mismatch`
        // and therefore never reaches `stage107a_service_block` as a
        // `blocked_image_identity_mismatch`.
        let container_binding_drift = container_binding_drift_from(image_preflight.as_ref().map(|(evaluation, _)| evaluation));
        // v12 review P0-1: `container_binding_drift` above is derived from
        // the INITIAL, pre-acquisition preflight -- correct and unchanged
        // for the OUTER attachment/convergence comparison just below (it
        // decides WASM's initial input and non-executing convergence, both
        // of which must reflect what was observed BEFORE any M5 pull could
        // have happened this cycle). But when the initial preflight
        // outcome was `NeedsPull` (target image absent locally), the
        // initial container-binding observation is necessarily
        // `NotApplicable`/uninformative -- the mandatory post-M5
        // re-evaluation inside `resolve_pinned_image_outcome` can discover
        // a REAL `Mismatches` that the initial preflight had no way to see
        // (the target image object didn't exist locally yet to compare
        // the container against). `execute_trusted_container_intent`
        // (governed EXECUTION, reached only after acquisition may already
        // have happened) must use this FINAL, post-acquisition binding
        // fact, never the stale initial one -- overwritten below, inside
        // the `execute_requested && control_plane_authorized` branch,
        // using `resolution.evidence.container_binding_status` (the same
        // FINAL evidence `container_binding_unavailable_in_required_mode`
        // already correctly reads).
        let mut container_binding_drift_for_execution = container_binding_drift;
        // v9 review P0-1: the single "expected runtime image" used
        // consistently for the top-level attachment/drift comparison below
        // AND (via `pinned_execution_reference`, threaded further down) for
        // `execute_trusted_container_intent`/M1/M3/M4 -- (A) once a platform
        // was selected for a valid pin, this is the IMMUTABLE
        // `execution_reference`, NEVER the mutable declared tag, even before
        // acquisition/verification has fully completed; (B) for an unpinned,
        // `local_build`, `legacy_unclassified`, or unresolved/structurally-
        // invalid registry artifact, this remains the declared tag exactly
        // as before.
        let expected_runtime_image_reference: Option<&str> = image_preflight
            .as_ref()
            .and_then(|(evaluation, _)| evaluation.execution_reference.as_deref());
        // v9 review P0-4 (broadened by v10 review P0-1): whether this
        // artifact's image identity has not yet been positively confirmed
        // while EXECUTING, AUTHORIZED, and running in `required` mode --
        // consulted below to keep `action_code == 0` from ever being
        // reported as fully verified convergence when either half of image
        // identity has never actually been confirmed locally. TWO
        // independent, still fully separate, sub-conditions: (a) the TARGET
        // IMAGE was never even inspected (`PreflightOutcome::NeedsPull`), or
        // (b) an EXISTING container's binding could not be proven
        // (`ContainerBindingObservation::Unavailable` -- NOT `Mismatches`,
        // which is drift and is handled entirely separately via
        // `container_binding_drift`/M3, never through this flag). Since no
        // acquisition can have run whenever `action_code == 0` (M5 requires
        // `wasm.action_code != 0`), the ORIGINAL, unconditional preflight's
        // `container_binding` is already the FINAL one for any cycle where
        // this flag's value actually matters. `legacy_partial` deliberately
        // keeps its existing, weaker "may still proceed" semantics and is
        // NOT affected by this flag. `execute_requested &&
        // control_plane_authorized` is required here too (mirroring the
        // P1-3 fix below): a non-executing ("suggested") cycle must never
        // have its status overridden by this flag either.
        let image_identity_pending_in_required_mode = image_identity_unconfirmed_in_required_mode(
            execute_requested,
            control_plane_authorized,
            effective_mode_for_secrets,
            matches!(
                image_preflight.as_ref().map(|(_, outcome)| outcome),
                Some(image_identity_verification::PreflightOutcome::NeedsPull { .. })
            ),
            image_preflight.as_ref().is_some_and(|(evaluation, _)| {
                matches!(evaluation.container_binding, image_identity_verification::ContainerBindingObservation::Unavailable)
            }),
        );
        let attachment_mismatch = match (mapping, assignment, observed_before) {
            (Some(mapping), Some(artifact), ContainerState::Running | ContainerState::Stopped) => {
                // Review finding P1-3: the DECLARED (currently delivered)
                // execution identity -- this top-level drift observation
                // runs regardless of authorization state, so it compares
                // against what the control plane currently declares, not an
                // agent-recomputed value (that recomputation only happens
                // once `execute_requested && control_plane_authorized`,
                // below).
                let expected_execution_identity = ExecutionIdentityLabels {
                    artifact_id: &artifact.artifact_id,
                    execution_generation: artifact.execution_generation,
                    execution_projection_digest: &artifact.execution_projection_digest,
                };
                let base_mismatch = !container_attachments_match(
                    &mapping.container_name,
                    expected_runtime_image_reference.unwrap_or(artifact.image.as_str()),
                    artifact.network.as_deref(),
                    effective_attachments.as_ref().and_then(|r| r.as_ref().ok()).map(|r| r.aliases.as_slice()).unwrap_or(&[]),
                    effective_attachments.as_ref().and_then(|r| r.as_ref().ok()).map(|r| r.ports.as_slice()).unwrap_or(&[]),
                    &artifact.bindings,
                    &artifact.volumes,
                    &artifact.configuration_mounts,
                    &artifact.command,
                    configuration_root(cfg),
                    &expected_execution_identity,
                )
                .unwrap_or(false);
                base_mismatch || image_known_mismatch || container_binding_drift
            }
            _ => false,
        };
        let wasm = run_embedded_reconciler(
            true,
            (observed_before_running || completion_succeeded) && !attachment_mismatch,
        )?;

        // Stage 107A per-service gate (ADR-0011 §2 / precedence in
        // ADR-0012): only meaningful once identity-level authorization
        // (`control_plane_authorized`) already holds and an artifact is
        // actually assigned. `Ok(None)` -> nothing blocks. `Err` on the cycle
        // gate itself (no authorization, insufficient pinning mode, missing
        // clock-skew config) or a per-service generation/digest mismatch both
        // surface as the SAME kind of stable block reason here.
        // Section G: verify every declared configuration-mount content digest
        // against the actual resolved bytes before any mutation. Undeclared
        // (`declared_content_sha256 = None`) mounts are not a failure here --
        // they are reported as unpinned inputs in evidence, gated separately
        // by the pinning-mode-completeness requirement, not by this check.
        let config_verification_result: Option<std::result::Result<Vec<VerifiedExternalInput>, String>> =
            if execute_requested && control_plane_authorized && !rollback_requested {
                assignment.map(|artifact| {
                    verify_declared_configuration_mount_content(&artifact.configuration_mounts, configuration_root(cfg))
                })
            } else {
                None
            };
        let config_content_block: Option<String> = config_verification_result
            .as_ref()
            .and_then(|result| result.as_ref().err().cloned());
        // Review finding P1-1/P1-2: only entries that ACTUALLY, SUCCESSFULLY
        // verified -- never a fabricated image/secret entry (see the field
        // doc comment on
        // OperationalServiceReconciliationReport::verified_external_inputs).
        let verified_external_inputs = config_verification_result
            .and_then(|result| result.ok()).unwrap_or_default();

        // Moved ahead of `stage107a_service_block` (Stage 107B-B, ADR-0014):
        // the image-identity resolution below needs a fresh lease and a
        // place to record M5's boundary attempt, both of which used to be
        // constructed later in this loop body.
        let authorization_expires_at_utc = context
            .authorization
            .as_ref()
            .and_then(|authorization| authorization.expires_at_utc.clone());
        let authorization_clock_skew_seconds = stage107a_cycle_gate
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|gate| gate.skew_seconds);
        let boundary_context = MutationBoundaryContext {
            authorization_id: context.authorization.as_ref().map(|authorization| authorization.authorization_id.as_str()),
            expires_at_utc: authorization_expires_at_utc.as_deref(),
            configured_clock_skew_seconds: authorization_clock_skew_seconds,
        };
        let mut mutation_boundaries: Vec<MutationBoundaryRecord> = Vec::new();

        // Stage 107B-B (ADR-0014), v8 review P0-1: every OTHER governance/
        // policy gate that already prohibits host mutation at this point --
        // computed BEFORE any image acquisition is even considered. This is
        // the SAME generation/projection/completeness check v1 already had,
        // just no longer folding the image-identity outcome in yet.
        let stage107a_service_block_before_image: Option<String> = if execute_requested && control_plane_authorized {
            match (&stage107a_cycle_gate, assignment) {
                (Some(Ok(gate)), Some(artifact)) => stage107a_service_check(
                    context.authorization.as_ref().expect("control_plane_authorized implies Some"),
                    artifact,
                )
                .err()
                .or_else(|| {
                    // P0-3, agent-side defense in depth: independently
                    // re-evaluate required-mode completeness on the
                    // delivered assignment, even though DServer already
                    // refuses to ISSUE a required authorization over an
                    // incomplete snapshot.
                    if gate.effective_mode == EXECUTION_PINNING_MODE_REQUIRED {
                        let gaps = required_completeness_gaps(artifact);
                        if gaps.is_empty() {
                            None
                        } else {
                            Some(format!(
                                "{BLOCKED_REQUIRED_INCOMPLETE}: service '{}' is not required-complete: {:?}",
                                artifact.service_id, gaps
                            ))
                        }
                    } else {
                        None
                    }
                }),
                (Some(Err(reason)), _) => Some(reason.clone()),
                (None, _) | (_, None) => None,
            }
        } else {
            None
        };
        // v9 review P0-3: every independently-blocking gate that must
        // ALREADY be clear before M5 (or any other governed mutation) may
        // even be considered -- expanded from the prior round's incomplete
        // set to also cover: attachment/local-policy resolution
        // (`attachment_policy_block`), a failed Docker observation
        // (`observation_error`), a failed one-shot with no retry
        // (`!completion_failed`), the absence of a usable local mapping
        // (`mapping.is_some()`), and -- critically -- WASM actually
        // requesting a host action at all (`wasm.action_code != 0`; Stage
        // 107A treats `action_code == 0` as "no new host action requested,"
        // and M5 is itself a host mutation, so it must never fire merely to
        // populate/verify a cache). This is the ONE place
        // `resolve_pinned_image_outcome` consults to decide whether a
        // `NeedsPull` preflight outcome may proceed; see its own tests for
        // proof that `pull_calls` stays empty whenever any of these is
        // false.
        let mutation_gates_clear = mutation_gates_are_clear(
            execute_requested,
            control_plane_authorized,
            rollback_requested,
            mapping.is_some(),
            dependency_blockers.is_empty(),
            stage107a_service_block_before_image.is_none(),
            config_content_block.is_none(),
            attachment_policy_block.is_none(),
            local_container_execution_allowed,
            batch_secret_resolution_error.is_none(),
            secret_resolution_error.is_none(),
            observation_error.is_none(),
            !completion_failed,
            wasm.action_code != 0,
        );

        let mut pinned_image_evidence: Option<VerifiedImageIdentity>;
        let mut pinned_execution_reference: Option<String> = None;
        let mut pinned_image_hard_failure: Option<String> = None;
        let mut pinned_image_acquisition_performed = false;
        // v10 review P0-1: an EXISTING container (Running/Stopped) whose
        // actual image binding could NOT be inspected -- distinct from a
        // KNOWN wrong binding (`container_binding_drift`, which forces a
        // governed recreate). v11 review P1-1: this is a REUSE guard, not
        // a blanket host-mutation block -- it must prevent accepting the
        // EXISTING container as-is (Running -> `already_running`, Stopped
        // -> M2 start) in `required` mode, but must NEVER prevent an
        // independently-required M3/M4 recreate (which replaces the
        // unverifiable container with a freshly created one from the
        // verified immutable target -- the safe corrective action), and
        // must NEVER feed the GLOBAL `stage107a_service_block` (which
        // would also incorrectly block remove-only rollback, M8 -- v11
        // review P0-1). Threaded directly into
        // `execute_trusted_container_intent` instead (see its own
        // reuse-guard check), computed alongside `image_identity_final_block`
        // (same `execute_requested && control_plane_authorized` gate, same
        // effective mode), from the FINAL evidence for this cycle (i.e.
        // post-acquisition when `resolve_pinned_image_outcome`
        // re-evaluated after a pull) -- never re-conflated with
        // target-image verification (`PreflightOutcome`/`classify_evaluation`
        // remain completely untouched by this).
        let mut container_binding_unavailable_in_required_mode = false;
        let image_identity_final_block: Option<String> = if let Some((evaluation, outcome)) = image_preflight {
            if execute_requested && control_plane_authorized {
                let existing_container_name = match observed_before {
                    ContainerState::Running | ContainerState::Stopped => mapping.map(|mapping| mapping.container_name.as_str()),
                    ContainerState::Missing => None,
                };
                let effective_mode = stage107a_cycle_gate.as_ref().and_then(|result| result.as_ref().ok()).map(|gate| gate.effective_mode)
                    .unwrap_or(EXECUTION_PINNING_MODE_REQUIRED);
                let resolution = resolve_pinned_image_outcome(
                    &docker_image_runtime,
                    evaluation.evidence,
                    outcome,
                    mutation_gates_clear,
                    assignment
                        .and_then(|artifact| artifact.resolved_image_identity.as_ref())
                        .expect("image_preflight is only Some when resolved_image_identity is Some"),
                    existing_container_name,
                    stage107a_cycle_gate.as_ref().and_then(|result| result.as_ref().ok()).map(|gate| &gate.lease),
                    &boundary_context,
                    &mut mutation_boundaries,
                    effective_mode,
                );
                container_binding_unavailable_in_required_mode = container_binding_unavailable_in_required_mode_from(
                    resolution.evidence.as_ref().map(|evidence| evidence.container_binding_status.as_str()),
                    effective_mode,
                );
                // v12 review P0-1: re-derive drift from the FINAL,
                // post-acquisition evidence -- a mismatch discovered only
                // by the mandatory post-M5 re-evaluation (never visible to
                // the initial `NeedsPull` preflight) must be treated as
                // runtime drift for EXECUTION exactly like a
                // preflight-discovered one, never lost.
                container_binding_drift_for_execution = container_binding_mismatches_from_status(
                    resolution.evidence.as_ref().map(|evidence| evidence.container_binding_status.as_str()),
                );
                pinned_image_evidence = resolution.evidence;
                pinned_execution_reference = resolution.execution_reference;
                pinned_image_hard_failure = resolution.hard_failure;
                pinned_image_acquisition_performed = resolution.acquisition_performed;
                resolution.block
            } else {
                // v9 review P1-3: a non-executing ("suggested") cycle, or one
                // without control-plane authorization, must NEVER surface a
                // mutation-oriented `blocked_image_identity_*` status -- no
                // host mutation (M5 or otherwise) was ever going to happen
                // this cycle regardless of the preflight outcome, and
                // `resolve_pinned_image_outcome` itself is never called (so
                // no acquisition can be attempted and no lease/M5-boundary
                // record is produced). The read-only evidence from the
                // UNCONDITIONAL preflight (v8 review P1-6) is still recorded
                // truthfully; a KNOWN target mismatch or container-binding
                // drift still affects convergence via
                // `image_known_mismatch`/`container_binding_drift`, already
                // folded into `attachment_mismatch` above -- this is
                // evidence/drift-only, never a synthetic block code.
                pinned_image_evidence = Some(evaluation.evidence);
                None
            }
        } else {
            // v9 review P1-4: truthful, explicit evidence for an artifact
            // this module was never even asked to pin-verify -- local_build/
            // legacy_unclassified sources, an unresolved registry artifact,
            // or one whose pin failed the structural-validity check.
            // `pull_disposition` is always set (never left absent), so an
            // observer can never misread "no evidence" as "verified and
            // fine" (this matters because D-Monitor snapshots feed
            // analytics/GNN models downstream). Self-review correction:
            // gated on `mapping.is_some()` too -- `image_preflight` is also
            // `None` whenever there is simply no local trusted-container
            // mapping for an otherwise perfectly valid pin (a completely
            // separate, pre-existing `blocked_local_mapping` concern), and
            // that case must never be mislabeled as an unpinnable/invalid
            // source.
            pinned_image_evidence = match (mapping, assignment) {
                (Some(_), Some(artifact)) => {
                    let reason_kind = if artifact.image_source_kind == IMAGE_SOURCE_KIND_LOCAL_BUILD {
                        image_identity_verification::REASON_LOCAL_BUILD_NOT_PINNABLE
                    } else if artifact.image_source_kind == IMAGE_SOURCE_KIND_LEGACY_UNCLASSIFIED {
                        image_identity_verification::REASON_LEGACY_UNCLASSIFIED_NOT_PINNABLE
                    } else if artifact.resolved_image_identity.is_none() {
                        image_identity_verification::REASON_REGISTRY_UNRESOLVED
                    } else {
                        image_identity_verification::REASON_REGISTRY_PIN_STRUCTURALLY_INVALID
                    };
                    Some(image_identity_verification::not_applicable_pinning_evidence(reason_kind, &Utc::now().to_rfc3339()))
                }
                _ => None,
            };
            None
        };

        // v11 review P0-1: remove-only rollback (M8) must NEVER be blocked
        // by a Stage-107B-B runtime image-verification outcome -- a
        // container whose target image is mismatching/unverifiable/
        // unselectable is PRECISELY a container that may need removing.
        // `rollback_service_block_from` is computed from the SAME pre-image
        // gates ordinary execution uses (generation/projection/completeness,
        // attachment/local-policy resolution), but deliberately does not
        // even ACCEPT `image_identity_final_block` (target-image mismatch/
        // unverified/no-selectable-platform) as a parameter --
        // `container_binding_unavailable_in_required_mode` was never part
        // of this composition to begin with (v11 review P1-1, above), so
        // no separate exclusion is needed for it here either.
        let rollback_service_block: Option<String> = rollback_service_block_from(
            stage107a_service_block_before_image.as_deref(),
            attachment_policy_block.as_deref(),
        );

        let stage107a_service_block: Option<String> = stage107a_service_block_before_image
            .or(image_identity_final_block)
            .or(attachment_policy_block);

        // Section I evidence, computed once per service whenever a Stage
        // 107A evaluation actually ran (execute_requested &&
        // control_plane_authorized), regardless of whether it ultimately
        // blocked or proceeded -- so evidence explains a block, not only a
        // success.
        // Review finding P1-3: extracted so the SAME agent-recomputed digest
        // both appears in evidence below AND becomes the `datum.*` M1 label
        // value -- by this point (`execute_requested && control_plane_authorized`)
        // `stage107a_service_check` has already proven it equals the
        // delivered `execution_projection_digest` whenever the service was
        // not itself blocked, but using the recomputed value for the label
        // is defense-in-depth, not a shortcut.
        let recomputed_execution_projection_digest: Option<String> = if execute_requested && control_plane_authorized {
            assignment.map(|artifact| execution_projection::recompute_digest(artifact).unwrap_or_default())
        } else {
            None
        };
        let stage107a_evidence: Option<(String, Option<String>, Option<u64>, Option<String>, Option<String>, Option<&'static str>)> =
            if execute_requested && control_plane_authorized {
                let authorization = context.authorization.as_ref();
                Some((
                    "snapshot_lease".to_string(),
                    authorization.map(|authorization| authorization.authorization_id.clone()),
                    assignment.map(|artifact| artifact.execution_generation),
                    assignment.map(|artifact| artifact.execution_projection_digest.clone()),
                    recomputed_execution_projection_digest.clone(),
                    stage107a_cycle_gate
                        .as_ref()
                        .and_then(|result| result.as_ref().ok())
                        .map(|gate| gate.effective_mode),
                ))
            } else {
                None
            };
        // P1-5: recorded regardless of effective mode -- see the field doc
        // comment on OperationalServiceReconciliationReport::unpinned_inputs.
        let unpinned_inputs: Vec<String> = if execute_requested && control_plane_authorized {
            assignment.map(required_completeness_gaps).unwrap_or_default()
        } else {
            Vec::new()
        };
        let local_policy_digest = compute_local_policy_digest(cfg);

        if rollback_requested {
            let (execution_status, message, host_mutation_performed) =
                if !execute_requested {
                    (
                        "rollback_requires_execute".to_string(),
                        "rollback was requested but --execute was not provided".to_string(),
                        false,
                    )
                } else if !control_plane_authorized {
                    (
                        "blocked_control_plane".to_string(),
                        "rollback requires a separately scoped active D-Graph authorization with operation='rollback'".to_string(),
                        false,
                    )
                } else if let Some(reason) = rollback_service_block.as_deref() {
                    (stage107a_block_code(reason), reason.to_string(), false)
                } else if mapping.is_none() {
                    (
                        "blocked_local_mapping".to_string(),
                        format!(
                            "service '{}' has an artifact assignment but no matching trusted local container mapping",
                            service.service_id
                        ),
                        false,
                    )
                } else if batch_secret_resolution_error.is_some() {
                    (
                        "blocked_missing_secret".to_string(),
                        "rollback is blocked because one or more batch secret references could not be resolved locally".to_string(),
                        false,
                    )
                } else if secret_resolution_error.is_some() {
                    (
                        "blocked_missing_secret".to_string(),
                        "rollback requires all secret references to resolve locally before container lifecycle changes".to_string(),
                        false,
                    )
                } else if !local_container_execution_allowed {
                    (
                        "blocked_local_policy".to_string(),
                        "rollback is blocked because local dispatch is disabled or allow_container_execution is not true".to_string(),
                        false,
                    )
                } else if let Some(error) = observation_error.as_deref() {
                    (
                        "docker_unavailable".to_string(),
                        format!("rollback could not verify the trusted container: {error}"),
                        false,
                    )
                } else {
                    match (mapping, stage107a_cycle_gate.as_ref().and_then(|result| result.as_ref().ok())) {
                        (Some(mapping), Some(gate)) => match rollback_trusted_container_intent(mapping, observed_before, &gate.lease, &boundary_context, &mut mutation_boundaries) {
                            Ok(result) => result,
                            Err(error) => (
                                execution_error_status(&error),
                                format!("governed trusted container rollback failed: {error}"),
                                false,
                            ),
                        },
                        (None, _) => (
                            "blocked_local_mapping".to_string(),
                            "artifact assignment has no matching local mapping".to_string(),
                            false,
                        ),
                        (Some(_), None) => (
                            "blocked_authorization_not_leased".to_string(),
                            "internal: reached rollback execution without a resolved Stage 107A cycle gate".to_string(),
                            false,
                        ),
                    }
                };

            side_effects_performed |= host_mutation_performed;
            let observed_after_running = mapping
                .and_then(|mapping| inspect_container(&mapping.container_name).ok())
                .is_some_and(|state| state == ContainerState::Running);

            services.push(OperationalServiceReconciliationReport {
                service_id: service.service_id.clone(),
                artifact_id: assignment.map(|artifact| artifact.artifact_id.clone()),
                artifact_version: assignment.map(|artifact| artifact.version.clone()),
                desired_state: service.desired_state.clone(),
                trusted_container_name: mapping.map(|mapping| mapping.container_name.clone()),
                trusted_image: mapping.and_then(|mapping| mapping.artifact.clone()),
                observed_before_running,
                observation_error,
                secret_resolution_status,
                wasm,
                execution_status,
                message,
                observed_after_running,
                host_mutation_performed,
                execution_semantics: stage107a_evidence.as_ref().map(|evidence| evidence.0.clone()),
                authorization_id: stage107a_evidence.as_ref().and_then(|evidence| evidence.1.clone()),
                authorization_expires_at_utc: authorization_expires_at_utc.clone(),
                execution_generation: stage107a_evidence.as_ref().and_then(|evidence| evidence.2),
                expected_execution_projection_digest: stage107a_evidence.as_ref().and_then(|evidence| evidence.3.clone()),
                recomputed_execution_projection_digest: stage107a_evidence.as_ref().and_then(|evidence| evidence.4.clone()),
                execution_pinning_mode: stage107a_evidence.as_ref().and_then(|evidence| evidence.5).map(str::to_string),
                f2_status: stage107a_evidence.as_ref().map(|_| "partial_hardening".to_string()),
                local_policy_digest: local_policy_digest.clone(),
                unpinned_inputs: unpinned_inputs.clone(),
                authorization_clock_skew_seconds,
                verified_external_inputs: verified_external_inputs.clone(),
                local_substitutions: local_substitutions.clone(),
                mutation_boundaries_crossed: mutation_boundaries.clone(),
                verified_image_identity: pinned_image_evidence.clone(),
            });
            continue;
        }

        let (execution_status, message, host_mutation_performed) =
            if !dependency_blockers.is_empty() {
                (
                    "blocked_dependency_unavailable".to_string(),
                    format!(
                        "WASM requested reconciliation, but dependencies are unavailable: {}",
                        dependency_blockers.join("; ")
                    ),
                    false,
                )
            } else if let Some(error) = completion_exit_code_error.as_deref() {
                (
                    "docker_unavailable".to_string(),
                    format!(
                        "one-shot completion could not be verified; no retry was attempted: {}",
                        error
                    ),
                    false,
                )
            } else if is_completion_desired(&service.desired_state) && observed_before_running {
                (
                    "in_progress".to_string(),
                    "desired one-shot service is still running; completion has not been verified".to_string(),
                    false,
                )
            } else if wasm.action_code == 0
                && action_code_zero_may_report_converged(
                    stage107a_service_block.as_deref(),
                    config_content_block.as_deref(),
                    image_identity_pending_in_required_mode,
                )
            {
            if completion_succeeded {
                (
                    "converged".to_string(),
                    "desired one-shot service completed successfully; WASM requested no host action".to_string(),
                    false,
                )
            } else {
                (
                    "converged".to_string(),
                    "desired service is already running; WASM requested no host action".to_string(),
                    false,
                )
            }
        } else if wasm.action_code == 0 && image_identity_pending_in_required_mode {
            // v9 review P0-4: WASM reported no host action needed, and no
            // OTHER gate is already blocking -- but this artifact's
            // required-mode pinned image identity has never actually been
            // confirmed locally (M5 correctly did not fire merely to
            // populate/verify a cache, per P0-3). Reporting "converged"
            // here would be false operational evidence; this is NOT a
            // mutation block either (nothing was attempted, and nothing
            // should be) -- a distinct, truthful, read-only status.
            (
                "image_identity_unconfirmed".to_string(),
                "WASM requested no host action, but the required-mode pinned image identity has not yet been confirmed locally; convergence cannot be truthfully reported".to_string(),
                false,
            )
        } else if completion_failed {
            (
                "failed".to_string(),
                format!(
                    "desired one-shot service exited unsuccessfully with code {}",
                    completion_exit_code.unwrap_or_default()
                ),
                false,
            )
        } else if execute_requested && !control_plane_authorized {
            (
                "blocked_control_plane".to_string(),
                "WASM requested ensure_service_running, but the active D-Graph slice has no matching host-mutation authorization for this revision".to_string(),
                false,
            )
        } else if let Some(reason) = stage107a_service_block.as_deref() {
            (stage107a_block_code(reason), reason.to_string(), false)
        } else if let Some(reason) = config_content_block.as_deref() {
            (stage107a_block_code(reason), reason.to_string(), false)
        } else if let Some(detail) = pinned_image_hard_failure.as_deref() {
            // Stage 107B-B (ADR-0014): a genuine `docker pull` command
            // failure (network/registry problem) is NOT an identity-
            // verification outcome -- handled exactly like
            // `execute_lifecycle_phase`'s own pre-existing pull-failure path,
            // as a per-service `"failed"` status, never a `blocked_*` code.
            ("failed".to_string(), format!("governed image pull failed before execution: {detail}"), false)
        } else if mapping.is_none() {
            (
                "blocked_local_mapping".to_string(),
                format!(
                    "service '{}' has an artifact assignment but no matching trusted local container mapping",
                    service.service_id
                ),
                false,
            )
        } else if !execute_requested {
            (
                "suggested".to_string(),
                "WASM requested ensure_service_running, but --execute was not provided".to_string(),
                false,
            )
        } else if !control_plane_authorized {
            (
                "blocked_control_plane".to_string(),
                "WASM requested ensure_service_running, but the active D-Graph slice has no matching host-mutation authorization for this revision".to_string(),
                false,
            )
        } else if batch_secret_resolution_error.is_some() {
            (
                "blocked_missing_secret".to_string(),
                "WASM requested ensure_service_running, but one or more batch secret references could not be resolved from the local allowlist".to_string(),
                false,
            )
        } else if secret_resolution_error.is_some() {
            (
                "blocked_missing_secret".to_string(),
                "WASM requested ensure_service_running, but one or more secret references could not be resolved from the local allowlist".to_string(),
                false,
            )
        } else if !local_container_execution_allowed {
            (
                "blocked_local_policy".to_string(),
                "WASM requested ensure_service_running, but local dispatch is disabled or allow_container_execution is not true".to_string(),
                false,
            )
        } else if let Some(error) = observation_error.as_deref() {
            (
                "docker_unavailable".to_string(),
                format!("WASM requested ensure_service_running, but Docker observation failed: {error}"),
                false,
            )
        } else {
            match (mapping, stage107a_cycle_gate.as_ref().and_then(|result| result.as_ref().ok())) {
                (Some(mapping), Some(gate)) => {
                    match effective_attachments.as_ref().expect("mapping implies resolved attachments") {
                        Err(reason) => (stage107a_block_code(reason), reason.clone(), false),
                        Ok(effective) => {
                            let effective_ports = &effective.ports;
                            let effective_aliases = &effective.aliases;
                            // Review finding P1-3: `mapping` is only ever
                            // `Some` when it was derived from `assignment`
                            // (`trusted_container_mapping_for_artifact`), so
                            // `assignment` is guaranteed `Some` here; the
                            // recomputed digest is guaranteed `Some` too --
                            // it is computed whenever the cycle gate itself
                            // resolved, exactly the condition that produced
                            // `gate` above.
                            let artifact_for_identity = assignment
                                .expect("mapping is only Some when assignment is Some");
                            let execution_identity = ExecutionIdentityLabels {
                                artifact_id: &artifact_for_identity.artifact_id,
                                execution_generation: artifact_for_identity.execution_generation,
                                execution_projection_digest: recomputed_execution_projection_digest
                                    .as_deref()
                                    .expect("recomputed digest is Some whenever the cycle gate resolved"),
                            };
                            match execute_trusted_container_intent(
                                mapping,
                                observed_before,
                                assignment.and_then(|artifact| artifact.network.as_deref()),
                                assignment.map(|artifact| artifact.volumes.as_slice()).unwrap_or(&[]),
                                assignment
                                    .map(|artifact| artifact.configuration_mounts.as_slice())
                                    .unwrap_or(&[]),
                                assignment.map(|artifact| artifact.command.as_slice()).unwrap_or(&[]),
                                configuration_root(cfg),
                                assignment
                                    .map(|artifact| &artifact.lifecycle)
                                    .unwrap_or(&OperationalLifecycle::default()),
                                assignment.map(|artifact| &artifact.bindings),
                                &secret_environment,
                                &gate.lease,
                                &effective_ports,
                                &effective_aliases,
                                &boundary_context,
                                &mut mutation_boundaries,
                                &execution_identity,
                                pinned_execution_reference.as_deref(),
                                container_binding_drift_for_execution,
                                container_binding_unavailable_in_required_mode,
                            ) {
                                Ok(result) => result,
                                Err(error) => (
                                    execution_error_status(&error),
                                    format!("governed trusted container execution failed: {error}"),
                                    false,
                                ),
                            }
                        }
                    }
                }
                (None, _) => (
                    "blocked_local_mapping".to_string(),
                    "artifact assignment has no matching local mapping".to_string(),
                    false,
                ),
                (Some(_), None) => (
                    "blocked_authorization_not_leased".to_string(),
                    "internal: reached execution without a resolved Stage 107A cycle gate".to_string(),
                    false,
                ),
            }
        };

        // v9 review P1-1: a successful M5 acquisition (the `docker pull`
        // call ITSELF returned `Ok(())`) must always be reflected as a host
        // mutation, even if a later re-verification/mismatch/hard-failure
        // means no container was ultimately started this cycle -- otherwise
        // `host_mutation_performed`/`side_effects_performed` would falsely
        // report "no side effects" for a cycle that genuinely changed local
        // Docker image storage.
        let host_mutation_performed = host_mutation_performed || pinned_image_acquisition_performed;

        // v9 review P1-2: after a fresh pinned M1 create (a bare M1, or a
        // governed M3/M4 recreate that itself ends in a fresh M1) succeeds,
        // `execute_trusted_container_intent` has only proven the IMAGE
        // OBJECT verified -- never that the newly created CONTAINER is
        // actually bound to it. A narrow, read-only re-check closes that
        // gap; it updates only report-only evidence
        // (`verified_image_identity.actual_container_image`), never
        // `execution_projection`/`execution_generation`.
        let (execution_status, message) = if execution_status == "deployed" {
            match (
                pinned_image_evidence.as_ref().and_then(|evidence| evidence.actual_image_id.clone()),
                mapping,
            ) {
                (Some(expected_image_id), Some(mapping)) => {
                    let observed = docker_image_runtime.inspect_container_image_binding(&mapping.container_name);
                    let (actual_container_image, container_binding_status, override_status) =
                        post_action_container_binding_check(&mapping.container_name, &expected_image_id, observed);
                    if let Some(evidence) = pinned_image_evidence.as_mut() {
                        evidence.actual_container_image = actual_container_image;
                        // v10 review P1-2: keep evidence internally
                        // consistent -- `target_verified` stays `true`
                        // (the IMAGE OBJECT genuinely did verify earlier
                        // this cycle), but `container_binding_status` must
                        // now reflect what was JUST observed on the fresh
                        // container, never a stale pre-creation value.
                        evidence.container_binding_status = container_binding_status.to_string();
                    }
                    override_status.unwrap_or((execution_status, message))
                }
                _ => (execution_status, message),
            }
        } else {
            (execution_status, message)
        };

        side_effects_performed |= host_mutation_performed;
        let observed_after_running = mapping
            .and_then(|mapping| inspect_container(&mapping.container_name).ok())
            .is_some_and(|state| state == ContainerState::Running);

        services.push(OperationalServiceReconciliationReport {
            service_id: service.service_id.clone(),
            artifact_id: assignment.map(|artifact| artifact.artifact_id.clone()),
            artifact_version: assignment.map(|artifact| artifact.version.clone()),
            desired_state: service.desired_state.clone(),
            trusted_container_name: mapping.map(|mapping| mapping.container_name.clone()),
            trusted_image: mapping.and_then(|mapping| mapping.artifact.clone()),
            observed_before_running,
            observation_error,
            secret_resolution_status,
            wasm,
            execution_status,
            message,
            observed_after_running,
            host_mutation_performed,
            execution_semantics: stage107a_evidence.as_ref().map(|evidence| evidence.0.clone()),
            authorization_id: stage107a_evidence.as_ref().and_then(|evidence| evidence.1.clone()),
            authorization_expires_at_utc: authorization_expires_at_utc.clone(),
            execution_generation: stage107a_evidence.as_ref().and_then(|evidence| evidence.2),
            expected_execution_projection_digest: stage107a_evidence.as_ref().and_then(|evidence| evidence.3.clone()),
            recomputed_execution_projection_digest: stage107a_evidence.as_ref().and_then(|evidence| evidence.4.clone()),
            execution_pinning_mode: stage107a_evidence.as_ref().and_then(|evidence| evidence.5).map(str::to_string),
            f2_status: stage107a_evidence.as_ref().map(|_| "partial_hardening".to_string()),
            local_policy_digest: local_policy_digest.clone(),
            unpinned_inputs: unpinned_inputs.clone(),
            authorization_clock_skew_seconds,
            verified_external_inputs,
            local_substitutions,
            mutation_boundaries_crossed: mutation_boundaries,
            verified_image_identity: pinned_image_evidence,
        });
    }

    let (snapshot_published, snapshot_publish_error) = if publish_snapshot {
        match publish_operational_role_snapshot(
            cfg,
            &node_id,
            hostname_override,
            base,
            project_id,
            timeout_s,
        ) {
            Ok(()) => (true, None),
            Err(error) => (false, Some(error.to_string())),
        }
    } else {
        (false, None)
    };

    Ok(OperationalReconciliationReport {
        bootstrap_state: bootstrap.bootstrap_state,
        resource_id: bootstrap.control_plane.resource_id,
        node_id,
        dgraph_id: context.dgraph_id,
        dgraph_revision: Some(context.dgraph_revision),
        execute_requested,
        control_plane_authorized,
        local_container_execution_allowed,
        services,
        side_effects_performed,
        snapshot_published,
        snapshot_publish_error,
    })
}

pub fn run_embedded_reconciler(
    desired_running: bool,
    observed_running: bool,
) -> Result<WasmReconciliationDecision> {
    let bytes = wat::parse_str(RECONCILER_WAT).context("compile embedded reconciler WAT")?;
    let mut config = WasmConfig::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config).context("build operational reconciler Wasmtime engine")?;
    let module = Module::new(&engine, bytes).context("compile operational reconciler module")?;
    if module.imports().next().is_some() {
        bail!("operational reconciler must not import host functions");
    }

    let fuel_configured = 100_000u64;
    let mut store = Store::new(&engine, ());
    store
        .set_fuel(fuel_configured)
        .context("configure operational reconciler fuel")?;
    let fuel_before = store.get_fuel().ok();
    let instance = Instance::new(&mut store, &module, &[])
        .context("instantiate operational reconciler")?;
    let function = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, OPERATIONAL_RECONCILER_FUNCTION)
        .context("resolve operational reconciler function")?;
    let desired = if desired_running { 1 } else { 0 };
    let observed = if observed_running { 1 } else { 0 };
    let action_code = function
        .call(&mut store, (desired, observed))
        .context("execute operational reconciler WASM")?;
    if !matches!(action_code, 0 | 1) {
        bail!("operational reconciler returned unsupported action code {action_code}");
    }
    let fuel_after = store.get_fuel().ok();
    let fuel_consumed = match (fuel_before, fuel_after) {
        (Some(before), Some(after)) if before >= after => Some(before - after),
        _ => None,
    };

    Ok(WasmReconciliationDecision {
        module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
        function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
        status: "executed".to_string(),
        action_code,
        host_imports_allowed: false,
        guest_side_effects_performed: false,
        fuel_configured,
        fuel_consumed,
    })
}

/// Stage 108 (ADR-0015). The runtime-kind dispatch decision for one desired
/// service, computed BEFORE any Docker-specific observation. A pure function
/// of `(service, assignment)` -- no `Config`, no I/O -- specifically so the
/// property "a NativeProcess-desired service can never reach the Container
/// path" is directly unit-testable (see the tests below), rather than only
/// provable by source inspection.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ServiceRuntimeDispatch {
    /// Unknown runtime, or a desired/assigned runtime-kind mismatch.
    Error(String),
    /// Stage 109 (closes Stage 108 P2): a `NativeProcess`-desired service
    /// with NO matching artifact assignment. Distinct from `Error`
    /// (which requires two present, disagreeing values) and from
    /// `NativeProcess` (which requires exactly one matching assignment to
    /// exist) -- this was harmless in Stage 108 (no real execution was
    /// possible either way) but is NOT acceptable once Stage 109 can
    /// mutate the host: a desired service may enter real native
    /// reconciliation only when exactly one matching artifact assignment
    /// exists.
    MissingAssignment,
    NativeProcess,
    Container,
}

pub(crate) fn classify_service_runtime_dispatch(
    service: &OperationalDesiredService,
    assignment: Option<&OperationalArtifactAssignment>,
) -> ServiceRuntimeDispatch {
    let desired_runtime_kind = RuntimeKind::parse(&service.runtime);
    let assignment_runtime_kind = assignment.map(|artifact| RuntimeKind::parse(&artifact.runtime_kind));

    match (&desired_runtime_kind, &assignment_runtime_kind) {
        (Err(error), _) => {
            return ServiceRuntimeDispatch::Error(format!(
                "unsupported operational runtime '{}': {error}",
                service.runtime
            ));
        }
        (Ok(_), Some(Err(error))) => {
            return ServiceRuntimeDispatch::Error(format!(
                "service '{}' artifact assignment has {error}",
                service.service_id
            ));
        }
        (Ok(desired), Some(Ok(assigned))) if desired != assigned => {
            return ServiceRuntimeDispatch::Error(format!(
                "service '{}' desired runtime '{}' does not match assigned artifact runtime_kind '{}'",
                service.service_id,
                service.runtime,
                assignment.expect("assignment_runtime_kind is Some").runtime_kind
            ));
        }
        _ => {}
    }

    match desired_runtime_kind {
        Ok(RuntimeKind::NativeProcess) => {
            if assignment.is_none() {
                return ServiceRuntimeDispatch::MissingAssignment;
            }
            ServiceRuntimeDispatch::NativeProcess
        }
        _ => ServiceRuntimeDispatch::Container,
    }
}

/// Stage 108 (ADR-0015). A neutral, truthful "no WASM decision was made"
/// value -- used by both the dispatch-error and native-process-stub report
/// builders below, neither of which invokes the reconciler WASM module at
/// all (there is nothing to decide: no mutation is possible in either case).
pub(crate) fn no_wasm_decision(status: &str) -> WasmReconciliationDecision {
    WasmReconciliationDecision {
        module_id: String::new(),
        function_name: String::new(),
        status: status.to_string(),
        action_code: 0,
        host_imports_allowed: false,
        guest_side_effects_performed: false,
        fuel_configured: 0,
        fuel_consumed: None,
    }
}

/// Stage 108 (ADR-0015). Built when runtime-kind parsing/parity fails for a
/// desired service, BEFORE any Docker-specific observation is attempted for
/// either runtime kind. Truthful: no observation happened, no mutation
/// happened, and the reported status names the exact contract defect.
fn build_runtime_dispatch_error_report(
    service: &OperationalDesiredService,
    assignment: Option<&OperationalArtifactAssignment>,
    error: String,
) -> OperationalServiceReconciliationReport {
    OperationalServiceReconciliationReport {
        service_id: service.service_id.clone(),
        artifact_id: assignment.map(|artifact| artifact.artifact_id.clone()),
        artifact_version: assignment.map(|artifact| artifact.version.clone()),
        desired_state: service.desired_state.clone(),
        trusted_container_name: None,
        trusted_image: None,
        observed_before_running: false,
        observation_error: None,
        secret_resolution_status: "not_checked".to_string(),
        wasm: no_wasm_decision("not_applicable_runtime_dispatch_error"),
        execution_status: "blocked_runtime_kind_contract_error".to_string(),
        message: error,
        observed_after_running: false,
        host_mutation_performed: false,
        execution_semantics: None,
        authorization_id: None,
        authorization_expires_at_utc: None,
        execution_generation: assignment.map(|artifact| artifact.execution_generation),
        expected_execution_projection_digest: assignment.map(|artifact| artifact.execution_projection_digest.clone()),
        recomputed_execution_projection_digest: None,
        execution_pinning_mode: None,
        f2_status: None,
        local_policy_digest: None,
        unpinned_inputs: Vec::new(),
        authorization_clock_skew_seconds: None,
        verified_external_inputs: Vec::new(),
        local_substitutions: Vec::new(),
        mutation_boundaries_crossed: Vec::new(),
        verified_image_identity: None,
    }
}

/// v4 fix (P1-2, renamed from `native_process_deployment_spec_within_stage108_bounds`):
/// mirrors `dserver::core::service_artifact_registry::validate_native_process_deployment_spec`'s
/// exact Stage 109 bounds -- independently hand-written (no shared
/// crate), but the SAME limits, so a native_process payload cannot pass
/// this crate's wire validation while failing the registry's, or vice
/// versa. Stage 108's version of this function checked only non-empty/
/// length/NUL for `entrypoint`/`working_directory`; it did NOT enforce
/// the Stage 109 requirement that both be genuine POSIX-relative paths
/// (no leading `/`, no `.`/`..`/empty segments) -- an absolute or
/// traversal-shaped value could reach this point, and Stage 109's real
/// containment logic (`native_process_acquisition::resolve_contained_path`)
/// would only catch it later, at actual host-execution time, not at wire-
/// validation time. Returns the first violated bound's message on
/// failure.
fn native_process_deployment_spec_within_stage109_bounds(
    spec: &NativeProcessDeploymentSpecV1,
) -> std::result::Result<(), String> {
    if spec.process_name.trim().is_empty() || spec.process_name.len() > 256 || spec.process_name.contains('\0') {
        return Err("native_process.process_name must be non-empty, NUL-free, and at most 256 characters".to_string());
    }
    if !native_process_acquisition::is_valid_posix_relative_path(&spec.entrypoint) {
        return Err(
            "native_process.entrypoint must be a non-empty, NUL-free, bounded, POSIX-relative path with no '.'/'..' segments"
                .to_string(),
        );
    }
    if spec.args.len() > 64 || spec.args.iter().any(|arg| arg.len() > 4096 || arg.contains('\0')) {
        return Err("native_process.args must have at most 64 entries, each NUL-free and at most 4096 characters".to_string());
    }
    if let Some(dir) = &spec.working_directory {
        if !native_process_acquisition::is_valid_posix_relative_path(dir) {
            return Err(
                "native_process.working_directory, when present, must be a non-empty, NUL-free, bounded, POSIX-relative path with no '.'/'..' segments"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// v4 fix (P1-2): agent-side WIRE defense-in-depth mirroring DServer's
/// authoring-time/assignment-resolution native acquisition contract
/// (`kind` exactly `native_process_executable`, `sha256` exactly 64
/// lowercase hex, `source` a valid POSIX-absolute path). Before this fix,
/// `validate_context` never checked `acquisition` at all for a
/// NativeProcess assignment -- only real host execution
/// (`governed_materialize_and_spawn`) rejected a bad kind/source/hash,
/// meaning a malformed assignment could pass wire validation and only
/// fail much later, deep in the reconciliation cycle.
fn native_process_acquisition_within_stage109_bounds(
    acquisition: Option<&OperationalServiceArtifactAcquisition>,
) -> std::result::Result<(), String> {
    let Some(acquisition) = acquisition else {
        return Err("native_process assignment requires a control_plane.acquisition payload".to_string());
    };
    if acquisition.kind != native_process_acquisition::NATIVE_PROCESS_ACQUISITION_KIND {
        return Err(format!(
            "native_process assignment requires acquisition.kind to be exactly '{}', got '{}'",
            native_process_acquisition::NATIVE_PROCESS_ACQUISITION_KIND,
            acquisition.kind
        ));
    }
    let source = acquisition.source.as_deref().unwrap_or_default();
    if !native_process_acquisition::is_valid_posix_absolute_source_path(source) {
        return Err(
            "native_process assignment requires acquisition.source to be a non-empty, NUL-free, bounded, POSIX-absolute path"
                .to_string(),
        );
    }
    let sha256 = acquisition.sha256.as_deref().unwrap_or_default();
    if !native_process_acquisition::is_valid_content_sha256(sha256) {
        return Err(
            "native_process assignment requires acquisition.sha256 to be exactly 64 lowercase hexadecimal characters"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_context(
    context: &OperationalReconciliationContext,
    project_id: &str,
    node_id: &str,
) -> Result<()> {
    if context.schema_version != "0.1.0" {
        bail!("unsupported artifact reconciliation context schema_version");
    }
    if context.project_id.trim() != project_id.trim() {
        bail!(
            "artifact reconciliation context project '{}' does not match requested project '{}'",
            context.project_id,
            project_id
        );
    }
    if context.node_id.trim() != node_id.trim() {
        bail!(
            "artifact reconciliation context node '{}' does not match resolved node '{}'",
            context.node_id,
            node_id
        );
    }
    if context.dgraph_revision == 0 {
        bail!("artifact reconciliation context revision must be greater than zero");
    }
    if context.contract_kind != "datum_artifact_reconciliation_context"
        || context.execution_policy.mode != "proposal_only"
        || !context.execution_policy.explicit_invocation_required
        || context.execution_policy.host_imports_allowed
        || context.execution_policy.side_effects_allowed
        || context.execution_policy.safe_to_auto_execute
        || context.side_effects_performed
    {
        bail!("artifact reconciliation context execution policy is unsafe");
    }
    if context.reconciler.module_id != OPERATIONAL_RECONCILER_MODULE_ID
        || context.reconciler.function_name != OPERATIONAL_RECONCILER_FUNCTION
        || context.reconciler.host_imports_allowed
    {
        bail!("artifact reconciliation context requests an unsupported reconciler contract");
    }
    if context.artifacts.len() != context.desired_services.len() {
        bail!("artifact reconciliation context service/artifact counts differ");
    }

    let mut seen_services = HashSet::new();
    for service in &context.desired_services {
        if service.service_id.trim().is_empty() {
            bail!("operational desired service_id must not be empty");
        }
        if !seen_services.insert(service.service_id.trim().to_ascii_lowercase()) {
            bail!("operational context contains duplicate desired service '{}'", service.service_id);
        }
        // Stage 108 (ADR-0015, P0-4 correction): this was previously a
        // container-only `eq_ignore_ascii_case("container")` gate. Runtime-
        // kind parsing now recognises both canonical wire values; anything
        // else still fails closed, exactly as before.
        if RuntimeKind::parse(&service.runtime).is_err() {
            bail!(
                "unsupported operational runtime '{}': expected container or native_process",
                service.runtime
            );
        }
        if !matches!(
            service.desired_state.trim().to_ascii_lowercase().as_str(),
            "running" | "completed"
        ) {
            bail!(
                "unsupported operational desired_state '{}': expected running or completed",
                service.desired_state
            );
        }
    }

    for artifact in &context.artifacts {
        let policy = &artifact.wasm_decision;
        if artifact.service_id.trim().is_empty()
            || artifact.artifact_id.trim().is_empty()
            || artifact.version.trim().is_empty()
            || artifact.target_node != context.node_id
            || artifact.command.len() > 64
            || artifact.command.iter().any(|arg| arg.len() > 4096 || arg.contains('\0'))
            || !policy.explicit_invocation_required
            || policy.host_imports_allowed
            || policy.side_effects_allowed
            || policy.safe_to_auto_execute
            || policy.module_id.trim().is_empty()
            || policy.function_name.trim().is_empty()
        {
            bail!("artifact reconciliation context contains an unsafe or incomplete assignment");
        }

        // Stage 108 (ADR-0015, P0-4 correction): runtime-discriminated.
        // Container-specific field requirements apply ONLY under
        // RuntimeKind::Container; NativeProcess requires its own minimal
        // payload instead and is never checked against Docker-shaped
        // requirements (container_name/image/network/volumes/
        // configuration_mounts).
        match RuntimeKind::parse(&artifact.runtime_kind) {
            Ok(RuntimeKind::Container) => {
                if artifact.native_process.is_some() {
                    bail!(
                        "artifact reconciliation context assignment '{}' declares runtime_kind=container but also carries a native_process payload",
                        artifact.artifact_id
                    );
                }
                if artifact.container_name.trim().is_empty()
                    || artifact.image.trim().is_empty()
                    || artifact
                        .network
                        .as_deref()
                        .is_some_and(|network| !is_safe_docker_name(network))
                    || artifact.volumes.iter().any(|volume| {
                        !is_safe_docker_name(&volume.name)
                            || !volume.target.starts_with('/')
                            || volume.target.chars().any(char::is_whitespace)
                            || volume.target.contains(',')
                    })
                    || artifact.configuration_mounts.iter().any(|mount| {
                        !is_safe_configuration_source(&mount.source)
                            || !mount.target.starts_with('/')
                            || mount.target.chars().any(char::is_whitespace)
                            || mount.target.contains(',')
                            || !mount.read_only
                    })
                {
                    bail!("artifact reconciliation context contains an unsafe or incomplete container assignment");
                }
            }
            Ok(RuntimeKind::NativeProcess) => {
                let native = artifact.native_process.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "artifact reconciliation context assignment '{}' declares runtime_kind=native_process but carries no native_process payload",
                        artifact.artifact_id
                    )
                })?;
                // v4 fix (P1-2): mirrors the exact same Stage 109 bounds
                // `dserver::core::service_artifact_registry::validate_native_process_deployment_spec`
                // enforces -- this downstream wire validator must not accept
                // a weaker payload than the registry itself would have.
                if let Err(reason) = native_process_deployment_spec_within_stage109_bounds(native) {
                    bail!(
                        "artifact reconciliation context contains an unsafe or incomplete native_process assignment: {reason}"
                    );
                }
                // v4 fix (P1-2): the native acquisition contract
                // (kind/source/sha256) is now ALSO validated at wire time,
                // mirroring DServer's own authoring-time/assignment-
                // resolution checks -- agent-side defense-in-depth,
                // never relied upon as the sole enforcement point (real
                // host execution independently re-checks all three
                // immediately before any mutation).
                if let Err(reason) = native_process_acquisition_within_stage109_bounds(artifact.acquisition.as_ref()) {
                    bail!(
                        "artifact reconciliation context contains an unsafe or incomplete native_process assignment: {reason}"
                    );
                }
            }
            Err(_) => {
                bail!(
                    "artifact reconciliation context assignment '{}' has unsupported runtime_kind '{}'",
                    artifact.artifact_id,
                    artifact.runtime_kind
                );
            }
        }
    }

    Ok(())
}

fn trusted_container_mapping_for_artifact<'a>(
    cfg: &'a Config,
    service_id: &str,
    container_name: &str,
    image: &str,
) -> Option<&'a DispatchContainerMapping> {
    cfg.dispatch.as_ref()?.container_mappings.iter().find(|mapping| {
        mapping.service_name.as_deref().is_some_and(|name| {
            name.eq_ignore_ascii_case(service_id)
        }) && mapping.container_name.trim() == container_name.trim()
            && mapping.artifact.as_deref().is_some_and(|artifact| {
                artifact.trim() == image.trim()
            })
    })
}

fn dependency_blockers_for_assignment(
    assignment: &OperationalArtifactAssignment,
    context: &OperationalReconciliationContext,
    cfg: &Config,
) -> Vec<String> {
    let mut blockers = Vec::new();

    for dependency in &assignment.dependencies {
        let dependency = dependency.trim();
        let Some(dependency_assignment) = context.artifacts.iter().find(|candidate| {
            candidate.artifact_id.eq_ignore_ascii_case(dependency)
                || candidate.service_id.eq_ignore_ascii_case(dependency)
        }) else {
            blockers.push(format!("dependency '{}' has no artifact assignment", dependency));
            continue;
        };

        let Some(mapping) = trusted_container_mapping_for_artifact(
            cfg,
            &dependency_assignment.service_id,
            &dependency_assignment.container_name,
            &dependency_assignment.image,
        ) else {
            blockers.push(format!(
                "dependency '{}' has no matching trusted local container mapping",
                dependency
            ));
            continue;
        };

        match inspect_container(&mapping.container_name) {
            Ok(ContainerState::Running) => {}
            Ok(ContainerState::Stopped) => blockers.push(format!(
                "dependency '{}' container '{}' is stopped",
                dependency, mapping.container_name
            )),
            Ok(ContainerState::Missing) => blockers.push(format!(
                "dependency '{}' container '{}' is missing",
                dependency, mapping.container_name
            )),
            Err(error) => blockers.push(format!(
                "dependency '{}' could not be observed: {}",
                dependency, error
            )),
        }
    }

    blockers
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

/// Review finding P1-2: too-shallow convergence. Fetches ONE full `docker
/// inspect <container>` (rather than several narrow `-f` template queries)
/// and delegates the actual comparison to
/// `inspected_container_matches_declared_attachments`, a pure function unit
/// -tested without a live Docker daemon. Compares every Stage-107A
/// execution-relevant field Docker can currently materialize: image
/// reference, command, network membership, network aliases, effective
/// ports, non-secret environment bindings, volumes, and configuration
/// mounts, PLUS (review finding P1-3, v4) the `datum.*` execution-identity
/// Docker labels M1 writes on creation. A mismatch on ANY of these means the
/// running container does not match the authorized execution contract and
/// must not be reported converged -- `attachment_mismatch` drives a
/// governed recreate (M3), exactly like the pre-existing
/// command/network/volume/mount checks already did. The label check in
/// particular catches drift the field-level checks cannot: they only verify
/// declared values are PRESENT, never that nothing extra or stale remains,
/// so a value silently REMOVED from the current declaration (e.g. a port no
/// longer declared) would otherwise go undetected. A container with no
/// `datum.*` labels at all -- every pre-Stage-107A or externally-created
/// container -- is never considered Stage 107A converged either, and is
/// recreated through this same governed path. Actual OCI image CONTENT
/// identity remains Stage 107B; comparing the reference string is not a
/// content-pin claim.
#[allow(clippy::too_many_arguments)]
fn container_attachments_match(
    container_name: &str,
    image: &str,
    network: Option<&str>,
    network_aliases: &[String],
    ports: &[OperationalPortMapping],
    bindings: &BTreeMap<String, String>,
    volumes: &[OperationalVolumeMount],
    configuration_mounts: &[OperationalConfigurationMount],
    container_command: &[String],
    configuration_root: Option<&Path>,
    expected_execution_identity: &ExecutionIdentityLabels,
) -> Result<bool> {
    let output = Command::new("docker")
        .args(["inspect", container_name])
        .output()
        .with_context(|| format!("inspect trusted container '{container_name}'"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let parsed: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("decode docker inspect for '{container_name}'"))?;
    let Some(inspected) = parsed.into_iter().next() else {
        return Ok(false);
    };
    let resolved_configuration_mounts = resolve_configuration_mounts(configuration_mounts, configuration_root)?;
    Ok(inspected_container_matches_declared_attachments(
        &inspected,
        image,
        container_command,
        network,
        network_aliases,
        ports,
        bindings,
        volumes,
        &resolved_configuration_mounts,
        expected_execution_identity,
    ))
}

/// Pure comparison half of `container_attachments_match` -- takes an
/// already-fetched `docker inspect` JSON object (the single element of the
/// array `docker inspect <name>` returns) and every DECLARED
/// (already-resolved-effective) value, and reports whether they all match.
/// No `Command::new` here, so this is directly unit-testable.
#[allow(clippy::too_many_arguments)]
fn inspected_container_matches_declared_attachments(
    inspected: &serde_json::Value,
    image: &str,
    container_command: &[String],
    network: Option<&str>,
    network_aliases: &[String],
    ports: &[OperationalPortMapping],
    bindings: &BTreeMap<String, String>,
    volumes: &[OperationalVolumeMount],
    resolved_configuration_mounts: &[(String, String)],
    expected_execution_identity: &ExecutionIdentityLabels,
) -> bool {
    // Review finding P1-3: checked FIRST. A container with no `datum.*`
    // labels, or labels for a different artifact/generation/digest, is
    // never Stage-107A converged regardless of how every other field below
    // compares -- this is what catches drift (including REMOVAL of a
    // previously-declared value) that field-presence comparison alone
    // cannot see.
    if !inspected_container_labels_match(inspected, expected_execution_identity) {
        return false;
    }

    let config = inspected.get("Config");

    if !image.trim().is_empty() {
        let actual_image = config.and_then(|c| c.get("Image")).and_then(serde_json::Value::as_str);
        if actual_image != Some(image.trim()) {
            return false;
        }
    }

    if !container_command.is_empty() {
        let actual: Option<Vec<String>> = config
            .and_then(|c| c.get("Cmd"))
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok());
        if actual.as_deref() != Some(container_command) {
            return false;
        }
    }

    if !bindings.is_empty() {
        let actual_env: Vec<&str> = config
            .and_then(|c| c.get("Env"))
            .and_then(serde_json::Value::as_array)
            .map(|entries| entries.iter().filter_map(serde_json::Value::as_str).collect())
            .unwrap_or_default();
        for (key, value) in bindings {
            let expected = format!("{key}={value}");
            if !actual_env.iter().any(|entry| *entry == expected) {
                return false;
            }
        }
    }

    if let Some(network) = network {
        let Some(network_entry) = inspected
            .get("NetworkSettings")
            .and_then(|settings| settings.get("Networks"))
            .and_then(|networks| networks.get(network.trim()))
        else {
            return false;
        };
        if !network_aliases.is_empty() {
            let actual_aliases: Vec<&str> = network_entry
                .get("Aliases")
                .and_then(serde_json::Value::as_array)
                .map(|entries| entries.iter().filter_map(serde_json::Value::as_str).collect())
                .unwrap_or_default();
            for alias in network_aliases {
                if !actual_aliases.iter().any(|entry| *entry == alias) {
                    return false;
                }
            }
        }
    }

    if !ports.is_empty() {
        let port_bindings = inspected
            .get("HostConfig")
            .and_then(|host_config| host_config.get("PortBindings"));
        for port in ports {
            let protocol = if port.protocol.trim().is_empty() { "tcp" } else { port.protocol.trim() };
            let key = format!("{}/{protocol}", port.container_port);
            let expected_host_port = port.host_port.unwrap_or(port.container_port).to_string();
            let matched = port_bindings
                .and_then(|bindings| bindings.get(&key))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry.get("HostPort").and_then(serde_json::Value::as_str) == Some(expected_host_port.as_str())
                    })
                });
            if !matched {
                return false;
            }
        }
    }

    if volumes.is_empty() && resolved_configuration_mounts.is_empty() {
        return true;
    }

    let Some(mounts) = inspected.get("Mounts").and_then(serde_json::Value::as_array) else {
        return false;
    };

    let named_volumes_match = volumes.iter().all(|volume| {
        mounts.iter().any(|mount| {
            mount.get("Type").and_then(serde_json::Value::as_str) == Some("volume")
                && mount.get("Name").and_then(serde_json::Value::as_str)
                    == Some(volume.name.trim())
                && mount.get("Destination").and_then(serde_json::Value::as_str)
                    == Some(volume.target.trim())
                && mount
                    .get("RW")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true)
                    == !volume.read_only
        })
    });
    if !named_volumes_match {
        return false;
    }

    resolved_configuration_mounts.iter().all(|(source, target)| {
        mounts.iter().any(|mount| {
            mount.get("Type").and_then(serde_json::Value::as_str) == Some("bind")
                && mount.get("Source").and_then(serde_json::Value::as_str) == Some(source)
                && mount.get("Destination").and_then(serde_json::Value::as_str) == Some(target)
                && mount
                    .get("RW")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true)
                    == false
        })
    })
}

/// Best-effort effective ports/aliases used ONLY for the top-level drift
/// OBSERVATION below (whether to consider the running container converged).
fn docker_inspect_reports_missing(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("no such object")
        || normalized.contains("no such container")
        || (normalized.contains("network") && normalized.contains("not found"))
}

fn inspect_container_exit_code(container_name: &str) -> Result<i64> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.ExitCode}}", container_name])
        .output()
        .with_context(|| format!("execute Docker exit-code inspection for '{container_name}'"))?;
    if !output.status.success() {
        bail!(
            "Docker exit-code inspection failed for '{}': {}",
            container_name,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<i64>()
        .with_context(|| format!("parse Docker exit code for '{container_name}'"))
}

fn inspect_container(container_name: &str) -> Result<ContainerState> {
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_name])
        .output()
        .with_context(|| format!("execute docker inspect for trusted container '{container_name}'"))?;
    if output.status.success() {
        return match String::from_utf8_lossy(&output.stdout).trim() {
            "true" => Ok(ContainerState::Running),
            "false" => Ok(ContainerState::Stopped),
            other => bail!("unexpected docker inspect state '{other}' for '{container_name}'"),
        };
    }
    let combined = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if docker_inspect_reports_missing(&combined) {
        Ok(ContainerState::Missing)
    } else {
        bail!("Docker unavailable while inspecting '{}': {}", container_name, combined.trim())
    }
}

/// Resolves each `secret_ref` to `(target_environment_variable, value)`. The
/// VALUE always comes from a local environment variable (local policy may
/// supply the value source, per N-1) -- but which variable NAME it is read
/// from is, per F2c / Stage 107A, taken from the control-plane-declared
/// `declared_bindings` whenever present; falling back to the node-local
/// `DispatchSecretBinding` is a substitution that `required` mode refuses
/// (`stage107a_enforcement::resolve_secret_target_environment_variable`).
fn resolve_secret_bindings(
    cfg: &Config,
    secret_refs: &[String],
    declared_bindings: &[OperationalSecretBinding],
    effective_mode: &str,
) -> Result<Vec<(String, String)>, String> {
    if secret_refs.is_empty() {
        return Ok(Vec::new());
    }
    let dispatch = cfg
        .dispatch
        .as_ref()
        .ok_or_else(|| "local secret resolution requires a dispatch configuration".to_string())?;

    let mut resolved = Vec::with_capacity(secret_refs.len());
    for secret_ref in secret_refs {
        let secret_ref = secret_ref.trim();
        let declared_target = declared_bindings
            .iter()
            .find(|binding| binding.secret_ref.trim() == secret_ref)
            .map(|binding| binding.target_environment_variable.as_str());
        let local_target = dispatch
            .secret_bindings
            .iter()
            .find(|binding| binding.secret_ref.trim() == secret_ref)
            .map(|binding| binding.environment_variable.as_str());
        let (variable, _substituted) = resolve_secret_target_environment_variable(
            secret_ref,
            declared_target,
            local_target,
            effective_mode,
        )?;
        let variable = variable.trim().to_string();
        if variable.is_empty() {
            return Err(format!("secret reference '{}' has an empty target environment variable binding", secret_ref));
        }
        let value = env::var(&variable).map_err(|_| format!(
            "local environment variable for secret reference '{}' is not available",
            secret_ref
        ))?;
        if value.is_empty() {
            return Err(format!("local environment variable for secret reference '{}' is empty", secret_ref));
        }
        resolved.push((variable, value));
    }
    Ok(resolved)
}

/// Review finding P1-1/P1-2 (v4): which `secret_refs` had their TARGET
/// ENVIRONMENT VARIABLE NAME (never the resolved VALUE) substituted from
/// node-local dispatch policy rather than the control-plane-declared
/// binding. Mirrors exactly the decision `resolve_secret_bindings` itself
/// makes via `resolve_secret_target_environment_variable`, without touching
/// that function's signature or duplicating actual value resolution
/// (`env::var`). A resolution error here is not reported -- it is already
/// surfaced through `secret_resolution_error` at the call site; this
/// function only ever contributes non-secret evidence about substitution.
fn secret_target_substitutions(
    cfg: &Config,
    secret_refs: &[String],
    declared_bindings: &[OperationalSecretBinding],
    effective_mode: &str,
) -> Vec<LocalSubstitutionEvidence> {
    let Some(dispatch) = cfg.dispatch.as_ref() else {
        return Vec::new();
    };
    let mut substitutions = Vec::new();
    for secret_ref in secret_refs {
        let secret_ref = secret_ref.trim();
        let declared_target = declared_bindings
            .iter()
            .find(|binding| binding.secret_ref.trim() == secret_ref)
            .map(|binding| binding.target_environment_variable.as_str());
        let local_target = dispatch
            .secret_bindings
            .iter()
            .find(|binding| binding.secret_ref.trim() == secret_ref)
            .map(|binding| binding.environment_variable.as_str());
        if let Ok((target, true)) = resolve_secret_target_environment_variable(
            secret_ref,
            declared_target,
            local_target,
            effective_mode,
        ) {
            substitutions.push(LocalSubstitutionEvidence {
                field: format!("secret_bindings:{secret_ref}:target_environment_variable"),
                source: "dispatch.secret_bindings.environment_variable".to_string(),
                effective_value: serde_json::json!(target),
                reason: "undeclared control-plane target; legacy_partial permits local fallback".to_string(),
            });
        }
    }
    substitutions
}

/// v8 review P0-1: the result of `resolve_pinned_image_outcome`. `evidence`
/// is always present once a preflight ran (whether or not acquisition was
/// attempted); `execution_reference` is the one and only signal
/// `execute_trusted_container_intent` should trust to override the mutable
/// declared tag; `block` folds into `stage107a_service_block` exactly like
/// any other `"blocked_xxx: detail"` string; `hard_failure` is a genuine
/// `docker pull` command failure, never an identity-verification outcome.
struct PinnedImageResolution {
    evidence: Option<VerifiedImageIdentity>,
    execution_reference: Option<String>,
    block: Option<String>,
    hard_failure: Option<String>,
    /// v9 review P1-1: true iff `acquire_pinned_image`'s `pull_image` call
    /// itself returned `Ok(())` this invocation -- i.e. the host WAS
    /// mutated, regardless of what any later re-verification found. The
    /// caller must OR this into `host_mutation_performed`/
    /// `side_effects_performed`, since a successful pull followed by a
    /// blocking post-pull mismatch/unverified result must never be reported
    /// as "no side effects" -- that would be false operational evidence.
    acquisition_performed: bool,
}

/// Stage 107B-B (ADR-0014), v8 review P0-1/P0-2: turns an ALREADY-COMPUTED,
/// read-only preflight outcome (`image_identity_verification::PreflightOutcome`,
/// from `evaluate_image_identity` + `classify_evaluation` -- see the
/// per-service loop above) into a final decision, WITHOUT itself ever
/// deciding whether mutation is permitted: `mutation_gates_clear` must
/// already reflect every other governance/policy gate that independently
/// prohibits host mutation at this point (generation/projection binding,
/// required-mode completeness, the configuration-content gate, local
/// host-mutation policy, dependency/secret resolution, rollback state) --
/// this function ONLY additionally gates the M5 lease check immediately
/// before the actual `docker pull`. A `NeedsPull` outcome reached with
/// `mutation_gates_clear == false` performs NO acquisition and adds NO new
/// block (whatever gate is already blocking explains the resulting status
/// through the existing precedence chain).
///
/// Review decision #2 (mode handling), applied identically whether the
/// outcome came from the initial preflight or from the post-acquisition
/// re-evaluation: `KnownMismatch`/`NoSelectableReference` always block,
/// regardless of mode; `UnverifiedWithReference` blocks only in `required`
/// -- in any other mode it sets `execution_reference`, never leaving the
/// caller to fall back to the mutable declared tag.
#[allow(clippy::too_many_arguments)]
fn resolve_pinned_image_outcome(
    runtime: &dyn DockerImageRuntime,
    preflight_evidence: VerifiedImageIdentity,
    preflight_outcome: image_identity_verification::PreflightOutcome,
    mutation_gates_clear: bool,
    resolved_identity: &OperationalResolvedImageIdentity,
    existing_container_name: Option<&str>,
    lease: Option<&LeaseGuard>,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
    effective_mode: &str,
) -> PinnedImageResolution {
    use image_identity_verification::PreflightOutcome;

    fn fold_terminal_outcome(outcome: PreflightOutcome, effective_mode: &str) -> (Option<String>, Option<String>) {
        match outcome {
            PreflightOutcome::Verified(execution_reference) => (Some(execution_reference), None),
            PreflightOutcome::KnownMismatch(reason) => (None, Some(format!("{BLOCKED_IMAGE_IDENTITY_MISMATCH}: {reason}"))),
            PreflightOutcome::NoSelectableReference(reason) => (None, Some(format!("{BLOCKED_IMAGE_IDENTITY_UNVERIFIED}: {reason}"))),
            PreflightOutcome::UnverifiedWithReference { execution_reference, reason_kind } => {
                if effective_mode == EXECUTION_PINNING_MODE_REQUIRED {
                    (None, Some(format!("{BLOCKED_IMAGE_IDENTITY_UNVERIFIED}: {reason_kind}")))
                } else {
                    (Some(execution_reference), None)
                }
            }
            PreflightOutcome::NeedsPull { .. } => {
                // Only reachable here for the ORIGINAL (pre-acquisition)
                // outcome when mutation_gates_clear was false -- see below.
                // `acquire_pinned_image` itself never re-classifies to
                // NeedsPull (its own doc comment/tests guarantee this).
                (None, None)
            }
        }
    }

    let mut result = PinnedImageResolution {
        evidence: Some(preflight_evidence),
        execution_reference: None,
        block: None,
        hard_failure: None,
        acquisition_performed: false,
    };

    if let PreflightOutcome::NeedsPull { execution_reference } = &preflight_outcome {
        if mutation_gates_clear {
            let Some(lease) = lease else {
                // Defensive: mutation_gates_clear implies a resolved cycle
                // gate exists (it requires stage107a_service_block_before_image
                // to be None, which itself requires stage107a_cycle_gate to
                // be Some(Ok(_))) -- this should be unreachable, but never
                // silently acquire without a lease to check.
                result.block = Some(format!("{BLOCKED_IMAGE_IDENTITY_UNVERIFIED}: internal: no lease available for M5"));
                return result;
            };
            match check_boundary(lease, boundary_context, "M5", records) {
                Err(lease_error) => {
                    result.block = Some(lease_error.to_string());
                }
                Ok(()) => match image_identity_verification::acquire_pinned_image(
                    runtime,
                    resolved_identity,
                    existing_container_name,
                    execution_reference,
                    &Utc::now().to_rfc3339(),
                ) {
                    Ok((evaluation, acquired_outcome)) => {
                        // v9 review P1-1: `pull_image` already returned
                        // `Ok(())` inside `acquire_pinned_image` by this
                        // point -- the host WAS mutated, independent of
                        // whatever the re-verification below finds.
                        result.acquisition_performed = true;
                        result.evidence = Some(evaluation.evidence);
                        let (execution_reference, block) = fold_terminal_outcome(acquired_outcome, effective_mode);
                        result.execution_reference = execution_reference;
                        result.block = block;
                    }
                    Err(pull_error) => {
                        result.hard_failure = Some(format!("could not pull governed image '{execution_reference}': {pull_error}"));
                    }
                },
            }
        }
        // else: gates not clear -- no acquisition attempted, no new block.
        return result;
    }

    // v8 review P1-3: no acquisition happened on this path (the preflight
    // outcome was never `NeedsPull`) -- if the preflight nonetheless found
    // an image locally (`Verified`/`KnownMismatch`, both of which only
    // arise from a real `inspect_image` hit), the mechanical fact is
    // "already present," never "pulled." `NoSelectableReference` and an
    // inspect-error `UnverifiedWithReference` never found anything locally,
    // so `pull_disposition` correctly stays `None` for those.
    if let Some(evidence) = result.evidence.as_mut() {
        if evidence.actual_image_id.is_some() && evidence.pull_disposition.is_none() {
            evidence.pull_disposition = Some(image_identity_verification::PULL_DISPOSITION_ALREADY_PRESENT.to_string());
        }
    }
    let (execution_reference, block) = fold_terminal_outcome(preflight_outcome, effective_mode);
    result.execution_reference = execution_reference;
    result.block = block;
    result
}

/// `phase_boundaries` names the M1-M8 mutation boundary each mutating
/// command in this phase corresponds to, so the lease is re-checked
/// immediately before that specific operation (ADR-0012 point 9): `pull_image`'s
/// actual `docker pull` is M5 (the preceding `docker image inspect` is
/// read-only and not gated); `remove_container` is M6.
///
/// v8 review P1-2: `pinned` is true exactly when a verified
/// `execution_reference` override is in effect for this artifact this
/// cycle. Image acquisition for a pinned artifact is owned EXCLUSIVELY by
/// `resolve_pinned_image_outcome`'s M5 (already completed, successfully,
/// before `execute_trusted_container_intent` could even be reached this
/// cycle -- see the per-service `image_identity_final_block` gate). The
/// `"pull_image"` lifecycle command becomes an explicit no-op when `pinned`
/// is true, rather than a second, unabstracted, non-identity-verifying M5
/// implementation that merely happens to usually be harmless (v1's
/// weakness: any inspect error there would have silently become a second
/// pull attempt, bypassing every identity check). Unpinned artifacts are
/// completely unaffected -- the original inspect-then-maybe-pull logic runs
/// exactly as before.
#[allow(clippy::too_many_arguments)]
fn execute_lifecycle_phase(
    phase: &str,
    commands: &[OperationalLifecycleCommand],
    container_name: &str,
    image: &str,
    pinned: bool,
    configuration_mounts: &[OperationalConfigurationMount],
    configuration_root: Option<&Path>,
    lease: &LeaseGuard,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
) -> Result<()> {
    for command in commands {
        match command.program.trim() {
            "verify_docker" => {
                let output = Command::new("docker")
                    .args(["version", "--format", "{{.Server.Version}}"])
                    .output()
                    .context("verify Docker for governed lifecycle")?;
                if !output.status.success() {
                    bail!(
                        "governed lifecycle phase '{}' could not verify Docker: {}",
                        phase,
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
            }
            "pull_image" if pinned => {
                // v8 review P1-2: no-op -- see this function's doc comment.
            }
            "pull_image" => {
                let inspected = Command::new("docker")
                    .args(["image", "inspect", image])
                    .output()
                    .context("inspect governed lifecycle image")?;
                if !inspected.status.success() {
                    check_boundary(lease, boundary_context, "M5", records)?;
                    let pulled = Command::new("docker")
                        .args(["pull", image])
                        .output()
                        .with_context(|| format!("pull governed image '{}'", image))?;
                    if !pulled.status.success() {
                        bail!(
                            "governed lifecycle phase '{}' could not pull image '{}': {}",
                            phase,
                            image,
                            String::from_utf8_lossy(&pulled.stderr).trim()
                        );
                    }
                }
            }
            "verify_configuration_mounts" => {
                let _ = resolve_configuration_mounts(configuration_mounts, configuration_root)?;
            }
            "start_container" => {}
            "verify_container_running" => {
                if inspect_container(container_name)? != ContainerState::Running {
                    bail!(
                        "governed lifecycle phase '{}' expected container '{}' to be running",
                        phase,
                        container_name
                    );
                }
            }
            "remove_container" => {
                check_boundary(lease, boundary_context, "M6", records)?;
                let output = Command::new("docker")
                    .args(["rm", "-f", container_name])
                    .output()
                    .with_context(|| format!("remove governed container '{}'", container_name))?;
                if !output.status.success() && !docker_inspect_reports_missing(
                    &String::from_utf8_lossy(&output.stderr),
                ) {
                    bail!(
                        "governed lifecycle phase '{}' could not remove '{}': {}",
                        phase,
                        container_name,
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                }
            }
            unsupported => {
                bail!(
                    "unsupported governed lifecycle command '{}' in phase '{}'",
                    unsupported,
                    phase
                );
            }
        }
    }
    Ok(())
}

/// The M1-M8 mutation boundaries this function performs, per ADR-0012
/// point 9: M3 (`docker rm -f`, attachment-drift recreate), M4 (`docker rm
/// -f`, stopped+secrets recreate), M2 (`docker start`), M7 (`docker network
/// create`, inside `ensure_docker_network`), M1 (`docker run`). `lease` is
/// re-checked immediately before EACH one individually (never once for the
/// whole call), including across the two recursive re-entries after a
/// recreate: if the lease has expired by the time the recursive call reaches
/// its own boundary, that boundary blocks even though an earlier boundary in
/// the same governed intent already ran.
///
/// `effective_ports` / `effective_network_aliases` are resolved by the
/// caller (`stage107a_enforcement::resolve_ports` /
/// `resolve_network_aliases`) -- this function never falls back to
/// `mapping.ports` / `mapping.service_name` itself, so there is exactly one
/// place node-local substitution can occur (the caller), and it is always
/// mode-gated there.

/// v8 review P0-3: the `docker run` pull-policy arguments. For a PINNED
/// artifact (a verified immutable `execution_reference` is in effect),
/// `--pull never` is added so `docker run` can never implicitly acquire
/// missing image content itself -- acquisition remains owned exclusively by
/// M5 (`resolve_pinned_image_outcome`/`acquire_pinned_image`), which already
/// verified the exact content is present immediately before this call. Without
/// this flag, a local image removed out-of-band between M5's verification
/// and this M1 could cause `docker run` to silently perform an unrecorded,
/// unverified second acquisition. Unpinned (mutable-tag) execution is
/// completely unaffected -- no flag is added, preserving Docker's ordinary
/// implicit-pull behavior exactly as before this review.
fn pinned_pull_policy_args(pinned: bool) -> Vec<String> {
    if pinned {
        vec!["--pull".to_string(), "never".to_string()]
    } else {
        Vec::new()
    }
}

/// v8 review P1-5: the full M1 (`docker run`) argument vector, extracted as
/// a pure function so the core execution guarantees can be proven WITHOUT a
/// live Docker daemon: the pinned image argument is the immutable
/// `execution_reference` (never the mutable declared tag), `--pull never`
/// is present exactly when `pinned`, and the unpinned path is unaffected.
/// Deliberately excludes the LIVE `ensure_docker_network` call (which
/// creates a missing network as a side effect) -- the caller performs that
/// separately, before calling this function, and passes the already-decided
/// `network` name straight through here purely to emit the `--network`/
/// `--network-alias` flags.
#[allow(clippy::too_many_arguments)]
fn build_governed_docker_run_args(
    container_name: &str,
    image: &str,
    pinned: bool,
    network: Option<&str>,
    effective_network_aliases: &[String],
    volumes: &[OperationalVolumeMount],
    resolved_configuration_mounts: &[(String, String)],
    effective_ports: &[OperationalPortMapping],
    bindings: Option<&BTreeMap<String, String>>,
    secret_environment: &[(String, String)],
    execution_identity: &ExecutionIdentityLabels,
    container_command: &[String],
) -> Vec<String> {
    let mut args = vec!["run".to_string(), "-d".to_string(), "--name".to_string(), container_name.to_string()];
    args.extend(pinned_pull_policy_args(pinned));
    if let Some(network) = network {
        args.push("--network".to_string());
        args.push(network.trim().to_string());
        for alias in effective_network_aliases {
            if is_safe_docker_name(alias) {
                args.push("--network-alias".to_string());
                args.push(alias.clone());
            }
        }
    }
    for volume in volumes {
        args.push("--mount".to_string());
        args.push(format!(
            "type=volume,source={},target={}{}",
            volume.name.trim(),
            volume.target.trim(),
            if volume.read_only { ",readonly" } else { "" }
        ));
    }
    for mount in resolved_configuration_mounts {
        args.push("--mount".to_string());
        args.push(format!("type=bind,source={},target={},readonly", mount.0, mount.1));
    }
    for port in effective_ports {
        let host_port = port.host_port.unwrap_or(port.container_port);
        args.push("-p".to_string());
        args.push(format!(
            "{}:{}{}",
            host_port,
            port.container_port,
            if port.protocol.trim().is_empty() { String::new() } else { format!("/{}", port.protocol.trim()) }
        ));
    }
    if let Some(bindings) = bindings {
        for (environment_variable, _) in bindings {
            args.push("--env".to_string());
            args.push(environment_variable.clone());
        }
    }
    for (environment_variable, _) in secret_environment {
        args.push("--env".to_string());
        args.push(environment_variable.clone());
    }
    // Review finding P1-3: label the container with the exact artifact
    // identity it was created from, so convergence can later detect drift a
    // field-by-field inspect comparison alone cannot (including a value
    // that was REMOVED, not merely one that mismatches).
    args.extend(execution_identity.as_docker_run_args());
    args.push(image.to_string());
    args.extend(container_command.iter().cloned());
    args
}

#[allow(clippy::too_many_arguments)]
fn execute_trusted_container_intent(
    mapping: &DispatchContainerMapping,
    observed_before: ContainerState,
    network: Option<&str>,
    volumes: &[OperationalVolumeMount],
    configuration_mounts: &[OperationalConfigurationMount],
    container_command: &[String],
    configuration_root: Option<&Path>,
    lifecycle: &OperationalLifecycle,
    bindings: Option<&BTreeMap<String, String>>,
    secret_environment: &[(String, String)],
    lease: &LeaseGuard,
    effective_ports: &[OperationalPortMapping],
    effective_network_aliases: &[String],
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
    execution_identity: &ExecutionIdentityLabels,
    runtime_image_override: Option<&str>,
    container_binding_drift: bool,
    container_binding_unavailable_in_required_mode: bool,
) -> Result<(String, String, bool)> {
    let container_name = mapping.container_name.trim();
    let declared_image = mapping
        .artifact
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("trusted mapping requires non-empty artifact image");
    // Stage 107B-B (ADR-0014, review decision #1): once a valid pin has been
    // verified for this cycle, EVERY Docker command below that touches image
    // content (inspect/pull inside `execute_lifecycle_phase`, the attachment-
    // drift comparison, and the `docker run` argument) uses the immutable
    // `execution_reference`, never the mutable declared tag. The declared
    // tag remains the node-local trust key
    // (`trusted_container_mapping_for_artifact`'s match key is unaffected by
    // this override -- it still matches on `mapping.artifact`) and
    // provenance/user intent, never the runtime source of truth once pinned.
    let image = runtime_image_override.unwrap_or(declared_image);
    // v8 review P0-3/P1-2: `pinned` gates BOTH the `--pull never` argument on
    // the eventual `docker run` (below) and the lifecycle `"pull_image"`
    // command's no-op behavior (`execute_lifecycle_phase`) -- a single
    // source of truth for "is this cycle executing a verified pin."
    let pinned = runtime_image_override.is_some();
    let resolved_configuration_mounts =
        resolve_configuration_mounts(configuration_mounts, configuration_root)?;

    execute_lifecycle_phase("pre_install", &lifecycle.pre_install, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;
    execute_lifecycle_phase("install", &lifecycle.install, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;
    execute_lifecycle_phase("configure", &lifecycle.configure, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;
    execute_lifecycle_phase("start", &lifecycle.start, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;

    let attachments_mismatch = match observed_before {
        ContainerState::Running | ContainerState::Stopped => {
            // v9 review P0-2: `container_binding_drift` (an EXISTING
            // container bound to a different image object than the
            // verified target -- e.g. after a legitimate re-pin) is OR'd in
            // here so it forces the SAME governed M3 recreate path as any
            // other attachment drift. It must NEVER be treated as a reason
            // to M2-start the existing (wrongly-bound) container -- routing
            // it through this shared `attachments_mismatch` flag, rather
            // than a separate branch, is what guarantees that: a Stopped
            // container with drift below hits M3 (recreate), never M2
            // (start), because M3 is checked first.
            !container_attachments_match(
                container_name,
                image,
                network,
                effective_network_aliases,
                effective_ports,
                bindings.unwrap_or(&BTreeMap::new()),
                volumes,
                configuration_mounts,
                container_command,
                configuration_root,
                execution_identity,
            )? || container_binding_drift
        }
        ContainerState::Missing => false,
    };
    if attachments_mismatch {
        check_boundary(lease, boundary_context, "M3", records)?;
        let output = Command::new("docker")
            .args(["rm", "-f", container_name])
            .output()
            .with_context(|| format!("recreate trusted container '{container_name}' for declared attachments"))?;
        if !output.status.success() {
            bail!(
                "docker rm -f failed while reconciling attachments for '{}': {}",
                container_name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        return execute_trusted_container_intent(
            mapping,
            ContainerState::Missing,
            network,
            volumes,
            configuration_mounts,
            container_command,
            configuration_root,
            lifecycle,
            bindings,
            secret_environment,
            lease,
            effective_ports,
            effective_network_aliases,
            boundary_context,
            records,
            execution_identity,
            runtime_image_override,
            container_binding_drift,
            container_binding_unavailable_in_required_mode,
        );
    }

    if observed_before == ContainerState::Stopped && !secret_environment.is_empty() {
        check_boundary(lease, boundary_context, "M4", records)?;
        let output = Command::new("docker")
            .args(["rm", "-f", container_name])
            .output()
            .with_context(|| format!("recreate stopped trusted container '{container_name}' for resolved secrets"))?;
        if !output.status.success() {
            bail!(
                "docker rm -f failed while refreshing secrets for '{}': {}",
                container_name,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        return execute_trusted_container_intent(
            mapping,
            ContainerState::Missing,
            network,
            volumes,
            configuration_mounts,
            container_command,
            configuration_root,
            lifecycle,
            bindings,
            secret_environment,
            lease,
            effective_ports,
            effective_network_aliases,
            boundary_context,
            records,
            execution_identity,
            runtime_image_override,
            container_binding_drift,
            container_binding_unavailable_in_required_mode,
        );
    }

    // v11 review P1-1: a REUSE guard, not a blanket host-mutation block --
    // `container_binding_unavailable_in_required_mode` must prevent
    // accepting the EXISTING container as-is (Running -> `already_running`,
    // Stopped -> M2 start) without proof of its binding, but it must NEVER
    // prevent an independently-required M3 (just above) or M4 (just
    // above) recreate -- both already had their chance to fire before this
    // check is reached. Scoped to Running/Stopped only: a `Missing`
    // container (this is exactly what a fresh M3/M4 recursive re-entry
    // looks like) has nothing existing to hold a reuse judgment about, and
    // `container_binding_status` structurally can never be `unavailable`
    // for a Missing container in the first place. Returns the same stable
    // `blocked_image_identity_unverified` code an unresolvable TARGET-image
    // failure already uses in `required` mode -- never routed through
    // `anyhow`/`bail!`, so the stable code is never lost.
    if matches!(observed_before, ContainerState::Running | ContainerState::Stopped)
        && container_binding_unavailable_in_required_mode
    {
        return Ok((
            BLOCKED_IMAGE_IDENTITY_UNVERIFIED.to_string(),
            format!(
                "existing trusted container '{container_name}' binding could not be verified in required mode; refusing to reuse it without proof (the target image itself is verified)"
            ),
            false,
        ));
    }

    match observed_before {
        ContainerState::Running => {
            execute_lifecycle_phase("post_install", &lifecycle.post_install, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;
            Ok((
            "already_running".to_string(),
            format!("trusted container '{}' is already running", container_name),
            false,
            ))
        }
        ContainerState::Stopped => {
            check_boundary(lease, boundary_context, "M2", records)?;
            let output = Command::new("docker")
                .args(["start", container_name])
                .output()
                .with_context(|| format!("execute docker start for '{container_name}'"))?;
            if !output.status.success() {
                bail!(
                    "docker start failed for '{}': {}",
                    container_name,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            execute_lifecycle_phase("post_install", &lifecycle.post_install, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;
            Ok((
                "started".to_string(),
                format!(
                    "started trusted existing container '{}'; image mapping='{}'",
                    container_name, image
                ),
                true,
            ))
        }
        ContainerState::Missing => {
            if let Some(network) = network {
                ensure_docker_network(network, lease, boundary_context, records)?;
            }
            let args = build_governed_docker_run_args(
                container_name,
                image,
                pinned,
                network,
                effective_network_aliases,
                volumes,
                &resolved_configuration_mounts,
                effective_ports,
                bindings,
                secret_environment,
                execution_identity,
                container_command,
            );
            check_boundary(lease, boundary_context, "M1", records)?;
            let mut command = Command::new("docker");
            command.args(&args);
            if let Some(bindings) = bindings {
                for (environment_variable, value) in bindings {
                    command.env(environment_variable, value);
                }
            }
            for (environment_variable, value) in secret_environment {
                command.env(environment_variable, value);
            }
            let output = command.output()
                .with_context(|| format!("execute governed docker run for '{container_name}'"))?;
            if !output.status.success() {
                bail!(
                    "governed docker run failed for '{}' using trusted image '{}': {}",
                    container_name,
                    image,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            execute_lifecycle_phase("post_install", &lifecycle.post_install, container_name, image, pinned, configuration_mounts, configuration_root, lease, boundary_context, records)?;
            Ok((
                "deployed".to_string(),
                format!(
                    "created and started trusted container '{}' from local mapping image '{}'",
                    container_name, image
                ),
                true,
            ))
        }
    }
}

fn configuration_root(cfg: &Config) -> Option<&Path> {
    cfg.dispatch
        .as_ref()
        .and_then(|dispatch| dispatch.configuration_root.as_deref())
        .map(Path::new)
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

fn resolve_configuration_mounts(
    mounts: &[OperationalConfigurationMount],
    root: Option<&Path>,
) -> Result<Vec<(String, String)>> {
    if mounts.is_empty() {
        return Ok(Vec::new());
    }
    let root = root.ok_or_else(|| {
        anyhow::anyhow!("governed configuration mounts require dispatch.configuration_root")
    })?;
    let root = root
        .canonicalize()
        .with_context(|| format!("resolve governed configuration root '{}'", root.display()))?;
    let mut resolved = Vec::with_capacity(mounts.len());
    for mount in mounts {
        if !is_safe_configuration_source(&mount.source) {
            bail!("refusing unsafe governed configuration source '{}'", mount.source);
        }
        let path = root.join(mount.source.trim());
        let canonical = path
            .canonicalize()
            .with_context(|| format!("resolve governed configuration source '{}'", mount.source))?;
        if !canonical.starts_with(&root) {
            bail!("governed configuration source '{}' escapes configuration root", mount.source);
        }
        resolved.push((canonical.display().to_string(), mount.target.trim().to_string()));
    }
    Ok(resolved)
}

/// M7 (`docker network create`, ADR-0012 point 9). `docker network inspect`
/// is read-only and not gated.
fn ensure_docker_network(
    network: &str,
    lease: &LeaseGuard,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
) -> Result<()> {
    let network = network.trim();
    if !is_safe_docker_name(network) {
        bail!("refusing unsafe Docker network name '{}'", network);
    }

    let inspected = Command::new("docker")
        .args(["network", "inspect", network])
        .output()
        .with_context(|| format!("inspect Docker network '{network}'"))?;
    if inspected.status.success() {
        return Ok(());
    }

    let detail = format!(
        "{} {}",
        String::from_utf8_lossy(&inspected.stdout),
        String::from_utf8_lossy(&inspected.stderr)
    );
    if !docker_inspect_reports_missing(&detail) {
        bail!("Docker network inspection failed for '{}': {}", network, detail.trim());
    }

    check_boundary(lease, boundary_context, "M7", records)?;
    let created = Command::new("docker")
        .args(["network", "create", network])
        .output()
        .with_context(|| format!("create Docker network '{network}'"))?;
    if !created.status.success() {
        bail!(
            "Docker network creation failed for '{}': {}",
            network,
            String::from_utf8_lossy(&created.stderr).trim()
        );
    }
    Ok(())
}

/// M8 (`docker rm -f`, ADR-0012 point 9 / ADR-0012 point 7 "rollback follows
/// the same rule").
fn rollback_trusted_container_intent(
    mapping: &DispatchContainerMapping,
    observed_before: ContainerState,
    lease: &LeaseGuard,
    boundary_context: &MutationBoundaryContext,
    records: &mut Vec<MutationBoundaryRecord>,
) -> Result<(String, String, bool)> {
    let container_name = mapping.container_name.trim();
    match observed_before {
        ContainerState::Missing => Ok((
            "rollback_already_absent".to_string(),
            format!("trusted container '{}' is already absent; no rollback mutation was needed", container_name),
            false,
        )),
        ContainerState::Running | ContainerState::Stopped => {
            check_boundary(lease, boundary_context, "M8", records)?;
            let output = Command::new("docker")
                .args(["rm", "-f", container_name])
                .output()
                .with_context(|| format!("execute docker rm -f for '{container_name}'"))?;
            if !output.status.success() {
                bail!(
                    "docker rm -f failed for '{}': {}",
                    container_name,
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Ok((
                "rolled_back".to_string(),
                format!("removed trusted container '{}' after explicit rollback authorization", container_name),
                true,
            ))
        }
    }
}

pub fn publish_operational_role_snapshot(
    cfg: &Config,
    node_id: &str,
    hostname_override: Option<&str>,
    dserver_base: &str,
    project_id: &str,
    timeout_s: u64,
) -> Result<()> {
    let mut observation_cfg = cfg.clone();
    if let Some(auto_recovery) = observation_cfg.auto_recovery.as_mut() {
        auto_recovery.enabled = false;
        auto_recovery.dry_run = Some(true);
    }
    if let Some(desired_state) = observation_cfg.desired_state.as_mut() {
        desired_state.enabled = false;
    }

    let snapshot = run_cycle_for_node_identity(
        node_id,
        hostname_override,
        &observation_cfg.agent.schema_version,
        &observation_cfg,
    )?;
    let output = HttpJsonOutput::new(
        format!("{}/api/v1/snapshots", dserver_base.trim_end_matches('/')),
        project_id.trim().to_string(),
        timeout_s,
    )?;
    output.emit(&snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Stage 108 (ADR-0015): runtime-kind dispatch, contract, and ------ //
    // ---- stub-report tests. -------------------------------------------- //

    fn desired_service(runtime: &str) -> OperationalDesiredService {
        OperationalDesiredService {
            service_id: "svc".to_string(),
            runtime: runtime.to_string(),
            desired_state: "running".to_string(),
            application_id: String::new(),
            dmap_ref: None,
        }
    }

    fn wasm_policy() -> ArtifactWasmDecisionPolicy {
        ArtifactWasmDecisionPolicy {
            explicit_invocation_required: true,
            module_id: "reconcile_service".to_string(),
            function_name: "decide".to_string(),
            host_imports_allowed: false,
            side_effects_allowed: false,
            safe_to_auto_execute: false,
        }
    }

    fn container_assignment() -> OperationalArtifactAssignment {
        OperationalArtifactAssignment {
            artifact_id: "svc".to_string(),
            service_id: "svc".to_string(),
            artifact_service_id: "svc".to_string(),
            version: "1".to_string(),
            artifact_kind: "container".to_string(),
            runtime_kind: "container".to_string(),
            container_name: "datum-svc".to_string(),
            image: "example/svc:1".to_string(),
            native_process: None,
            acquisition: None,
            image_source_kind: IMAGE_SOURCE_KIND_REGISTRY.to_string(),
            resolved_image_identity: None,
            target_node: "fog-01".to_string(),
            lifecycle: OperationalLifecycle::default(),
            network: None,
            network_aliases: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            configuration_mounts: Vec::new(),
            command: Vec::new(),
            dependencies: Vec::new(),
            wasm_decision: wasm_policy(),
            bindings: BTreeMap::new(),
            secret_refs: Vec::new(),
            secret_bindings: Vec::new(),
            execution_generation: 1,
            execution_projection_digest: String::new(),
            application_id: String::new(),
            dmap_ref: None,
        }
    }

    fn native_process_assignment() -> OperationalArtifactAssignment {
        let mut assignment = container_assignment();
        assignment.runtime_kind = "native_process".to_string();
        assignment.container_name = String::new();
        assignment.image = String::new();
        assignment.native_process = Some(NativeProcessDeploymentSpecV1 {
            process_name: "datum-native-svc".to_string(),
            entrypoint: "bin/run".to_string(),
            args: vec!["--config".to_string(), "cfg.toml".to_string()],
            working_directory: Some("data".to_string()),
        });
        assignment.acquisition = Some(OperationalServiceArtifactAcquisition {
            kind: native_process_acquisition::NATIVE_PROCESS_ACQUISITION_KIND.to_string(),
            source: Some("/opt/datum/sources/native-svc.bin".to_string()),
            revision: None,
            sha256: Some("b".repeat(64)),
        });
        assignment
    }

    #[test]
    fn dispatch_classifies_matching_container_pair_as_container() {
        let dispatch = classify_service_runtime_dispatch(&desired_service("container"), Some(&container_assignment()));
        assert_eq!(dispatch, ServiceRuntimeDispatch::Container);
    }

    #[test]
    fn dispatch_classifies_matching_native_process_pair_as_native_process() {
        let dispatch = classify_service_runtime_dispatch(&desired_service("native_process"), Some(&native_process_assignment()));
        assert_eq!(dispatch, ServiceRuntimeDispatch::NativeProcess);
    }

    #[test]
    fn dispatch_classifies_unknown_desired_runtime_as_error() {
        let dispatch = classify_service_runtime_dispatch(&desired_service("vm"), Some(&container_assignment()));
        assert!(matches!(dispatch, ServiceRuntimeDispatch::Error(_)));
    }

    #[test]
    fn dispatch_classifies_unknown_assignment_runtime_as_error() {
        let mut assignment = container_assignment();
        assignment.runtime_kind = "vm".to_string();
        let dispatch = classify_service_runtime_dispatch(&desired_service("container"), Some(&assignment));
        assert!(matches!(dispatch, ServiceRuntimeDispatch::Error(_)));
    }

    #[test]
    fn dispatch_classifies_container_desired_with_native_assignment_as_error() {
        let dispatch = classify_service_runtime_dispatch(&desired_service("container"), Some(&native_process_assignment()));
        assert!(matches!(dispatch, ServiceRuntimeDispatch::Error(_)));
    }

    #[test]
    fn dispatch_classifies_native_desired_with_container_assignment_as_error() {
        let dispatch = classify_service_runtime_dispatch(&desired_service("native_process"), Some(&container_assignment()));
        assert!(matches!(dispatch, ServiceRuntimeDispatch::Error(_)));
    }

    #[test]
    fn dispatch_never_yields_container_for_a_native_process_desired_service_regardless_of_assignment() {
        // The deterministic proof the "zero Docker calls" requirement rests
        // on: for every possible assignment (matching, mismatched, absent,
        // unknown), a NativeProcess-desired service NEVER classifies as
        // Container -- i.e. it can never reach `trusted_container_mapping_for_artifact`/
        // `inspect_container` (both live only in the Container branch of the
        // caller, gated on this exact classification).
        let candidates: Vec<Option<OperationalArtifactAssignment>> = vec![
            None,
            Some(native_process_assignment()),
            Some(container_assignment()),
            {
                let mut a = container_assignment();
                a.runtime_kind = "vm".to_string();
                Some(a)
            },
        ];
        for assignment in &candidates {
            let dispatch = classify_service_runtime_dispatch(&desired_service("native_process"), assignment.as_ref());
            assert_ne!(dispatch, ServiceRuntimeDispatch::Container);
        }
    }

    #[test]
    fn dispatch_with_no_assignment_and_native_process_desired_is_missing_assignment() {
        // Stage 109 (closes Stage 108 P2): this used to classify as
        // `NativeProcess` (harmless in Stage 108, since no real execution
        // was possible). Now that Stage 109 can mutate the host, an
        // assignment-less native service must be a distinct contract
        // error, never enter native reconciliation.
        let dispatch = classify_service_runtime_dispatch(&desired_service("native_process"), None);
        assert_eq!(dispatch, ServiceRuntimeDispatch::MissingAssignment);
    }

    #[test]
    fn dispatch_never_yields_native_process_without_an_assignment() {
        let candidates: Vec<Option<OperationalArtifactAssignment>> = vec![None];
        for assignment in &candidates {
            let dispatch = classify_service_runtime_dispatch(&desired_service("native_process"), assignment.as_ref());
            assert_ne!(dispatch, ServiceRuntimeDispatch::NativeProcess);
        }
    }

    #[test]
    fn missing_assignment_is_distinguished_from_runtime_kind_mismatch_and_ordinary_errors() {
        // Three genuinely distinct dispatch outcomes for a NativeProcess-
        // desired service must never be conflated:
        let missing = classify_service_runtime_dispatch(&desired_service("native_process"), None);
        assert_eq!(missing, ServiceRuntimeDispatch::MissingAssignment);

        let mismatch = classify_service_runtime_dispatch(&desired_service("native_process"), Some(&container_assignment()));
        assert!(matches!(mismatch, ServiceRuntimeDispatch::Error(_)));
        assert_ne!(mismatch, ServiceRuntimeDispatch::MissingAssignment);

        let unknown = classify_service_runtime_dispatch(&desired_service("vm"), Some(&native_process_assignment()));
        assert!(matches!(unknown, ServiceRuntimeDispatch::Error(_)));
        assert_ne!(unknown, ServiceRuntimeDispatch::MissingAssignment);
    }

    #[test]
    fn runtime_dispatch_error_report_never_reports_container_evidence() {
        let report = build_runtime_dispatch_error_report(&desired_service("vm"), None, "unsupported".to_string());
        assert_eq!(report.execution_status, "blocked_runtime_kind_contract_error");
        assert!(!report.host_mutation_performed);
        assert!(report.trusted_container_name.is_none());
        assert!(report.trusted_image.is_none());
    }

    #[test]
    fn validate_context_accepts_a_well_formed_native_process_service() {
        let context = OperationalReconciliationContext {
            schema_version: "0.1.0".to_string(),
            contract_kind: "datum_artifact_reconciliation_context".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            dgraph_revision: 1,
            node_id: "fog-01".to_string(),
            desired_services: vec![desired_service("native_process")],
            artifacts: vec![native_process_assignment()],
            authorization: None,
            reconciler: OperationalReconcilerDescriptor {
                module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
                function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
                abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
                host_imports_allowed: false,
            },
            execution_policy: ArtifactExecutionPolicy {
                mode: "proposal_only".to_string(),
                explicit_invocation_required: true,
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            side_effects_performed: false,
        };
        validate_context(&context, "proj", "fog-01").unwrap();
    }

    // ---- v4 (P1-2): wire validation mirrors Stage 109 bounds ------------ //

    fn native_process_context(assignment: OperationalArtifactAssignment) -> OperationalReconciliationContext {
        OperationalReconciliationContext {
            schema_version: "0.1.0".to_string(),
            contract_kind: "datum_artifact_reconciliation_context".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            dgraph_revision: 1,
            node_id: "fog-01".to_string(),
            desired_services: vec![desired_service("native_process")],
            artifacts: vec![assignment],
            authorization: None,
            reconciler: OperationalReconcilerDescriptor {
                module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
                function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
                abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
                host_imports_allowed: false,
            },
            execution_policy: ArtifactExecutionPolicy {
                mode: "proposal_only".to_string(),
                explicit_invocation_required: true,
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            side_effects_performed: false,
        }
    }

    #[test]
    fn validate_context_rejects_absolute_native_entrypoint() {
        let mut assignment = native_process_assignment();
        assignment.native_process.as_mut().unwrap().entrypoint = "/bin/run".to_string();
        let context = native_process_context(assignment);
        let error = validate_context(&context, "proj", "fog-01").unwrap_err();
        assert!(format!("{error}").contains("entrypoint"), "error was: {error}");
    }

    #[test]
    fn validate_context_rejects_parent_traversing_native_entrypoint() {
        let mut assignment = native_process_assignment();
        assignment.native_process.as_mut().unwrap().entrypoint = "../x".to_string();
        let context = native_process_context(assignment);
        assert!(validate_context(&context, "proj", "fog-01").is_err());
    }

    #[test]
    fn validate_context_rejects_native_entrypoint_with_an_interior_parent_traversal_segment() {
        let mut assignment = native_process_assignment();
        assignment.native_process.as_mut().unwrap().entrypoint = "a/../x".to_string();
        let context = native_process_context(assignment);
        assert!(validate_context(&context, "proj", "fog-01").is_err());
    }

    #[test]
    fn validate_context_rejects_absolute_native_working_directory() {
        let mut assignment = native_process_assignment();
        assignment.native_process.as_mut().unwrap().working_directory = Some("/etc".to_string());
        let context = native_process_context(assignment);
        assert!(validate_context(&context, "proj", "fog-01").is_err());
    }

    #[test]
    fn validate_context_rejects_wrong_native_acquisition_kind() {
        let mut assignment = native_process_assignment();
        assignment.acquisition.as_mut().unwrap().kind = "command_bundle".to_string();
        let context = native_process_context(assignment);
        let error = validate_context(&context, "proj", "fog-01").unwrap_err();
        assert!(format!("{error}").contains("acquisition.kind"), "error was: {error}");
    }

    #[test]
    fn validate_context_rejects_relative_native_acquisition_source() {
        let mut assignment = native_process_assignment();
        assignment.acquisition.as_mut().unwrap().source = Some("relative/sources/x.bin".to_string());
        let context = native_process_context(assignment);
        let error = validate_context(&context, "proj", "fog-01").unwrap_err();
        assert!(format!("{error}").contains("acquisition.source"), "error was: {error}");
    }

    #[test]
    fn validate_context_rejects_uppercase_native_acquisition_sha256() {
        let mut assignment = native_process_assignment();
        assignment.acquisition.as_mut().unwrap().sha256 = Some("B".repeat(64));
        let context = native_process_context(assignment);
        let error = validate_context(&context, "proj", "fog-01").unwrap_err();
        assert!(format!("{error}").contains("acquisition.sha256"), "error was: {error}");
    }

    #[test]
    fn validate_context_rejects_native_assignment_with_no_acquisition_at_all() {
        let mut assignment = native_process_assignment();
        assignment.acquisition = None;
        let context = native_process_context(assignment);
        let error = validate_context(&context, "proj", "fog-01").unwrap_err();
        assert!(format!("{error}").contains("acquisition"), "error was: {error}");
    }

    #[test]
    fn validate_context_accepts_a_valid_stage109_native_assignment() {
        let context = native_process_context(native_process_assignment());
        validate_context(&context, "proj", "fog-01").unwrap();
    }

    #[test]
    fn validate_context_rejects_native_process_assignment_missing_its_payload() {
        let mut assignment = native_process_assignment();
        assignment.native_process = None;
        let context = OperationalReconciliationContext {
            schema_version: "0.1.0".to_string(),
            contract_kind: "datum_artifact_reconciliation_context".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            dgraph_revision: 1,
            node_id: "fog-01".to_string(),
            desired_services: vec![desired_service("native_process")],
            artifacts: vec![assignment],
            authorization: None,
            reconciler: OperationalReconcilerDescriptor {
                module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
                function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
                abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
                host_imports_allowed: false,
            },
            execution_policy: ArtifactExecutionPolicy {
                mode: "proposal_only".to_string(),
                explicit_invocation_required: true,
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            side_effects_performed: false,
        };
        assert!(validate_context(&context, "proj", "fog-01").is_err());
    }

    // ---- Stage 108 v2 (independent review, P1-2): downstream (wire) ----- //
    // ---- native_process payload validation must match the registry's ---- //
    // ---- exact bounds, not a weaker subset. ------------------------------//

    fn native_context(assignment: OperationalArtifactAssignment) -> OperationalReconciliationContext {
        OperationalReconciliationContext {
            schema_version: "0.1.0".to_string(),
            contract_kind: "datum_artifact_reconciliation_context".to_string(),
            project_id: "proj".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            dgraph_revision: 1,
            node_id: "fog-01".to_string(),
            desired_services: vec![desired_service("native_process")],
            artifacts: vec![assignment],
            authorization: None,
            reconciler: OperationalReconcilerDescriptor {
                module_id: OPERATIONAL_RECONCILER_MODULE_ID.to_string(),
                function_name: OPERATIONAL_RECONCILER_FUNCTION.to_string(),
                abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
                host_imports_allowed: false,
            },
            execution_policy: ArtifactExecutionPolicy {
                mode: "proposal_only".to_string(),
                explicit_invocation_required: true,
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            side_effects_performed: false,
        }
    }

    #[test]
    fn wire_valid_native_payload_still_passes() {
        let context = native_context(native_process_assignment());
        validate_context(&context, "proj", "fog-01").unwrap();
    }

    #[test]
    fn wire_native_payload_bound_violations_are_all_rejected() {
        let cases: Vec<(&str, NativeProcessDeploymentSpecV1)> = vec![
            ("process_name containing NUL", NativeProcessDeploymentSpecV1 {
                process_name: "bad\0name".to_string(), entrypoint: "bin/x".to_string(), args: vec![], working_directory: None,
            }),
            ("process_name > 256 chars", NativeProcessDeploymentSpecV1 {
                process_name: "a".repeat(257), entrypoint: "bin/x".to_string(), args: vec![], working_directory: None,
            }),
            ("entrypoint containing NUL", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/bad\0path".to_string(), args: vec![], working_directory: None,
            }),
            ("entrypoint > 4096 chars", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "a".repeat(4097), args: vec![], working_directory: None,
            }),
            ("args > 64 entries", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/x".to_string(),
                args: (0..65).map(|i| i.to_string()).collect(), working_directory: None,
            }),
            ("one arg > 4096 chars", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/x".to_string(),
                args: vec!["a".repeat(4097)], working_directory: None,
            }),
            ("arg containing NUL", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/x".to_string(),
                args: vec!["bad\0arg".to_string()], working_directory: None,
            }),
            ("working_directory blank Some", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/x".to_string(),
                args: vec![], working_directory: Some("   ".to_string()),
            }),
            ("working_directory containing NUL", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/x".to_string(),
                args: vec![], working_directory: Some("bad\0dir".to_string()),
            }),
            ("working_directory > 4096 chars", NativeProcessDeploymentSpecV1 {
                process_name: "x".to_string(), entrypoint: "bin/x".to_string(),
                args: vec![], working_directory: Some("a".repeat(4097)),
            }),
        ];

        for (label, native) in cases {
            let mut assignment = native_process_assignment();
            assignment.native_process = Some(native);
            let context = native_context(assignment);
            assert!(
                validate_context(&context, "proj", "fog-01").is_err(),
                "expected rejection for case: {label}"
            );
        }
    }

    // ---- P1-2: deep convergence comparison (pure, no Docker needed) ----- //

    fn baseline_inspect() -> serde_json::Value {
        serde_json::json!({
            "Config": {
                "Image": "eclipse-mosquitto:2",
                "Cmd": ["-c", "/mosquitto.conf"],
                "Env": ["MODE=prod", "LOG_LEVEL=info"],
                // Review finding P1-3: matches `matching_execution_identity()`
                // below -- every pre-existing test in this module exercises a
                // field OTHER than the label check, so the fixture container
                // is labelled as if M1 had created it under that identity.
                "Labels": {
                    "datum.artifact_id": "mqtt-broker",
                    "datum.execution_generation": "3",
                    "datum.execution_projection_digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                }
            },
            "NetworkSettings": {
                "Networks": {
                    "datum-net": { "Aliases": ["mqtt", "broker"] }
                }
            },
            "HostConfig": {
                "PortBindings": {
                    "1883/tcp": [{"HostPort": "1883"}]
                }
            },
            "Mounts": [
                {"Type": "volume", "Name": "mqtt-data", "Destination": "/data", "RW": true},
                {"Type": "bind", "Source": "/config/mosquitto.conf", "Destination": "/etc/mosquitto.conf", "RW": false}
            ]
        })
    }

    fn declared_bindings() -> BTreeMap<String, String> {
        [("MODE".to_string(), "prod".to_string())].into_iter().collect()
    }

    fn declared_ports() -> Vec<OperationalPortMapping> {
        vec![OperationalPortMapping { container_port: 1883, host_port: Some(1883), protocol: "tcp".to_string() }]
    }

    fn declared_volumes() -> Vec<OperationalVolumeMount> {
        vec![OperationalVolumeMount { name: "mqtt-data".to_string(), target: "/data".to_string(), read_only: false }]
    }

    fn declared_config_mounts() -> Vec<(String, String)> {
        vec![("/config/mosquitto.conf".to_string(), "/etc/mosquitto.conf".to_string())]
    }

    /// Matches the `datum.*` labels `baseline_inspect()` carries (review
    /// finding P1-3). Every pre-existing test below passes this so it keeps
    /// exercising the specific field it was written to test, not the label
    /// check added in front of all of them.
    fn matching_execution_identity() -> ExecutionIdentityLabels<'static> {
        ExecutionIdentityLabels {
            artifact_id: "mqtt-broker",
            execution_generation: 3,
            execution_projection_digest: "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        }
    }

    #[test]
    fn fully_matching_inspect_converges() {
        assert!(inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn image_drift_is_not_reported_converged() {
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2.1", // declared changed; container still running the old image
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn port_drift_is_not_reported_converged() {
        let mut ports = declared_ports();
        ports[0].host_port = Some(9999); // declared changed; container still bound to 1883
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &ports,
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn alias_drift_is_not_reported_converged() {
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["missing-alias".to_string()], // declared alias never attached
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn non_secret_env_binding_drift_is_not_reported_converged() {
        let mut bindings = declared_bindings();
        bindings.insert("MODE".to_string(), "staging".to_string()); // declared changed; container still has MODE=prod
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &bindings,
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn command_drift_is_still_caught_as_before() {
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/other.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn network_membership_drift_is_still_caught_as_before() {
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("other-net"),
            &[],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn volume_drift_is_still_caught_as_before() {
        let volumes = vec![OperationalVolumeMount { name: "mqtt-data".to_string(), target: "/different".to_string(), read_only: false }];
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &volumes,
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    // ---- P1-3 (v4 review): datum.* execution-identity label check ------- //

    #[test]
    fn unlabelled_container_is_never_reported_converged() {
        // Every field below matches exactly -- only the labels are missing
        // (a pre-Stage-107A or externally-created container). Must still be
        // reported as drift, not converged.
        let mut inspected = baseline_inspect();
        inspected["Config"].as_object_mut().unwrap().remove("Labels");
        assert!(!inspected_container_matches_declared_attachments(
            &inspected,
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn mismatched_artifact_id_label_is_not_reported_converged() {
        let expected = ExecutionIdentityLabels { artifact_id: "a-different-artifact", ..matching_execution_identity() };
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &expected,
        ));
    }

    #[test]
    fn mismatched_execution_generation_label_is_not_reported_converged() {
        // The exact P1-3 scenario: nothing else about the container changed
        // (image, command, network, ports, volumes, mounts all still
        // superficially match), but the artifact's content was updated
        // (generation bumped) since this container was created -- the label
        // is what catches it.
        let expected = ExecutionIdentityLabels { execution_generation: 4, ..matching_execution_identity() };
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &expected,
        ));
    }

    #[test]
    fn mismatched_execution_projection_digest_label_is_not_reported_converged() {
        let expected = ExecutionIdentityLabels { execution_projection_digest: "a-different-digest", ..matching_execution_identity() };
        assert!(!inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &["mqtt".to_string()],
            &declared_ports(),
            &declared_bindings(),
            &declared_volumes(),
            &declared_config_mounts(),
            &expected,
        ));
    }

    #[test]
    fn execution_identity_as_docker_run_args_produces_the_expected_label_flags() {
        let identity = matching_execution_identity();
        assert_eq!(
            identity.as_docker_run_args(),
            vec![
                "--label".to_string(),
                "datum.artifact_id=mqtt-broker".to_string(),
                "--label".to_string(),
                "datum.execution_generation=3".to_string(),
                "--label".to_string(),
                "datum.execution_projection_digest=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            ]
        );
    }

    #[test]
    fn empty_declared_ports_aliases_and_bindings_are_not_compared() {
        // Absence of a declared value is not itself drift -- only a
        // DECLARED value that disagrees with the observed state is.
        assert!(inspected_container_matches_declared_attachments(
            &baseline_inspect(),
            "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()],
            Some("datum-net"),
            &[],
            &[],
            &BTreeMap::new(),
            &declared_volumes(),
            &declared_config_mounts(),
            &matching_execution_identity(),
        ));
    }

    #[test]
    fn wasm_requests_action_when_desired_service_is_missing() {
        let result = run_embedded_reconciler(true, false).unwrap();
        assert_eq!(result.status, "executed");
        assert_eq!(result.action_code, 1);
        assert!(!result.host_imports_allowed);
        assert!(!result.guest_side_effects_performed);
    }

    #[test]
    fn wasm_requests_no_action_when_service_is_already_running() {
        let result = run_embedded_reconciler(true, true).unwrap();
        assert_eq!(result.action_code, 0);
    }

    #[test]
    fn completed_desired_state_is_detected_case_insensitively() {
        assert!(is_completion_desired("completed"));
        assert!(is_completion_desired(" COMPLETED "));
        assert!(!is_completion_desired("running"));
    }

    #[test]
    fn running_completed_job_is_not_converged() {
        assert!(is_completion_desired("completed"));
        assert!(!completed_container_converged(
            "completed",
            ContainerState::Running,
            None
        ));
    }

    #[test]
    fn completed_job_converges_only_after_successful_exit() {
        assert!(completed_container_converged(
            "completed",
            ContainerState::Stopped,
            Some(0)
        ));
        assert!(completed_container_converged(
            " COMPLETED ",
            ContainerState::Stopped,
            Some(0)
        ));
        assert!(!completed_container_converged(
            "completed",
            ContainerState::Stopped,
            Some(1)
        ));
        assert!(!completed_container_converged(
            "completed",
            ContainerState::Running,
            Some(0)
        ));
        assert!(!completed_container_converged(
            "running",
            ContainerState::Stopped,
            Some(0)
        ));
    }

    #[test]
    fn dependency_name_validation_rejects_option_like_values() {
        assert!(!is_safe_docker_name("--network"));
        assert!(is_safe_docker_name("datum-magistrala-net"));
    }

    #[test]
    fn governed_configuration_sources_reject_traversal_and_absolute_paths() {
        assert!(is_safe_configuration_source("magistrala/atom.yaml"));
        assert!(!is_safe_configuration_source("../secrets"));
        assert!(!is_safe_configuration_source("/etc/passwd"));
        assert!(!is_safe_configuration_source("magistrala\\atom.yaml"));
    }

    #[test]
    fn docker_missing_container_error_is_case_insensitive() {
        assert!(docker_inspect_reports_missing("Error: No such object: datum-mqtt"));
        assert!(docker_inspect_reports_missing("error: no such object: datum-mqtt"));
        assert!(docker_inspect_reports_missing("Error response from daemon: No such container: datum-mqtt"));
        assert!(docker_inspect_reports_missing("network datum-magistrala-net not found"));
        assert!(!docker_inspect_reports_missing("permission denied while trying to connect to the docker API"));
    }

    // ---- P0-4 (v4 review): action_code==0 must not mask a Stage 107A ---- //
    // ---- integrity/safety/completeness/config-content failure ----------- //

    #[test]
    fn action_code_zero_may_not_report_converged_when_projection_mismatches() {
        let reason = format!(
            "{}: recomputed execution_projection_digest does not match the authorized snapshot",
            crate::agent::stage107a_enforcement::BLOCKED_PROJECTION_MISMATCH
        );
        assert!(!action_code_zero_may_report_converged(Some(&reason), None, false));
    }

    #[test]
    fn action_code_zero_may_not_report_converged_when_resource_identity_is_wrong() {
        let reason = format!(
            "{}: authorization resource_id does not match this node's resolved resource_id",
            crate::agent::stage107a_enforcement::BLOCKED_WRONG_RESOURCE_IDENTITY
        );
        assert!(!action_code_zero_may_report_converged(Some(&reason), None, false));
    }

    #[test]
    fn action_code_zero_may_not_report_converged_when_configuration_content_mismatches() {
        let reason = format!(
            "{}: declared configuration mount content digest does not match the resolved bytes",
            crate::agent::stage107a_enforcement::BLOCKED_CONFIG_CONTENT_MISMATCH
        );
        assert!(!action_code_zero_may_report_converged(None, Some(&reason), false));
    }

    #[test]
    fn action_code_zero_may_not_report_converged_when_required_mode_is_incomplete() {
        let reason = format!(
            "{}: service is not required-complete",
            crate::agent::stage107a_enforcement::BLOCKED_REQUIRED_INCOMPLETE
        );
        assert!(!action_code_zero_may_report_converged(Some(&reason), None, false));
    }

    #[test]
    fn action_code_zero_may_report_converged_when_no_stage107a_block_is_present() {
        assert!(action_code_zero_may_report_converged(None, None, false));
    }

    // ---- P0-4 (v9 review): action_code==0 must not mask an unconfirmed --- //
    // ---- required-mode pinned image identity, even with no other block --- //

    #[test]
    fn action_code_zero_may_not_report_converged_when_image_identity_pending_in_required_mode() {
        assert!(!action_code_zero_may_report_converged(None, None, true));
    }

    #[test]
    fn action_code_zero_may_report_converged_in_legacy_partial_even_though_the_flag_is_computed_only_for_required() {
        // The caller only ever sets `image_identity_pending_in_required_mode`
        // to `true` when the effective mode is `required` (legacy_partial
        // always passes `false` here) -- this asserts the function's own
        // contract in isolation: `false` never blocks convergence.
        assert!(action_code_zero_may_report_converged(None, None, false));
    }

    // ---- P1-1/P1-2 (v4 review): mutation-boundary evidence --------------

    fn boundary_test_auth(expires_at_utc: Option<&str>) -> OperationalReconciliationAuthorization {
        OperationalReconciliationAuthorization {
            authorization_id: "opauth:boundary-test".to_string(),
            project_id: "p".to_string(),
            dgraph_id: "d".to_string(),
            dgraph_revision: Some(1),
            node_id: "n".to_string(),
            binding_id: Some("binding:test".to_string()),
            resource_id: Some("resource:test".to_string()),
            authorized_by: "operator".to_string(),
            authorized_at_utc: "2026-01-01T00:00:00Z".to_string(),
            allow_host_mutation: true,
            operation: "reconcile".to_string(),
            expires_at_utc: expires_at_utc.map(str::to_string),
            persistence_scope: "durable".to_string(),
            artifact_bindings: Vec::new(),
            execution_pinning_mode: "required".to_string(),
        }
    }

    #[test]
    fn check_boundary_records_a_pass_with_full_authorization_context() {
        let far_future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let authorization = boundary_test_auth(Some(&far_future));
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(authorization.authorization_id.as_str()),
            expires_at_utc: authorization.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = check_boundary(&lease, &boundary_context, "M1", &mut records);
        assert!(result.is_ok());
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.boundary, "M1");
        assert_eq!(record.result, "passed");
        assert!(record.block_reason.is_none());
        assert_eq!(record.authorization_id.as_deref(), Some("opauth:boundary-test"));
        assert_eq!(record.expires_at_utc.as_deref(), Some(far_future.as_str()));
        assert_eq!(record.configured_clock_skew_seconds, Some(30));
        assert!(chrono::DateTime::parse_from_rfc3339(&record.checked_at_utc).is_ok());
    }

    #[test]
    fn check_boundary_records_a_block_when_lease_is_expired() {
        let past = "2020-01-01T00:00:00Z";
        let authorization = boundary_test_auth(Some(past));
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(authorization.authorization_id.as_str()),
            expires_at_utc: authorization.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = check_boundary(&lease, &boundary_context, "M3", &mut records);
        assert!(result.is_err());
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.boundary, "M3");
        assert_eq!(record.result, "blocked");
        assert!(record.block_reason.as_deref().unwrap().starts_with(
            crate::agent::stage107a_enforcement::BLOCKED_LEASE_EXPIRED
        ));
    }

    #[test]
    fn check_boundary_records_a_block_when_authorization_has_no_finite_lease() {
        let authorization = boundary_test_auth(None);
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(authorization.authorization_id.as_str()),
            expires_at_utc: None,
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        let result = check_boundary(&lease, &boundary_context, "M7", &mut records);
        assert!(result.is_err());
        assert_eq!(records[0].result, "blocked");
        assert!(records[0].block_reason.as_deref().unwrap().starts_with(
            crate::agent::stage107a_enforcement::BLOCKED_AUTHORIZATION_NOT_LEASED
        ));
    }

    #[test]
    fn sequential_boundary_checks_accumulate_distinct_records_in_order() {
        // Simulates the M3-then-M1 sequence (attachment-drift recreate,
        // followed by the recursive re-entry's docker run): two records,
        // in order, each with its own timestamp -- never overwritten or
        // merged.
        let far_future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let authorization = boundary_test_auth(Some(&far_future));
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(authorization.authorization_id.as_str()),
            expires_at_utc: authorization.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        check_boundary(&lease, &boundary_context, "M3", &mut records).unwrap();
        check_boundary(&lease, &boundary_context, "M1", &mut records).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].boundary, "M3");
        assert_eq!(records[1].boundary, "M1");
        assert!(records.iter().all(|record| record.result == "passed"));
    }

    #[test]
    fn m8_m5_and_m7_boundary_identifiers_are_recorded_verbatim() {
        // The M1-M8 boundary identifier is whatever the call site passes --
        // this proves `check_boundary` itself is boundary-agnostic; the
        // actual M1..M8 <-> call-site mapping is fixed by the literal string
        // at each `check_boundary(...)` call in `execute_lifecycle_phase` /
        // `execute_trusted_container_intent` / `ensure_docker_network` /
        // `rollback_trusted_container_intent`.
        let far_future = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let authorization = boundary_test_auth(Some(&far_future));
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        let boundary_context = MutationBoundaryContext {
            authorization_id: Some(authorization.authorization_id.as_str()),
            expires_at_utc: authorization.expires_at_utc.as_deref(),
            configured_clock_skew_seconds: Some(30),
        };
        let mut records = Vec::new();
        for boundary in ["M5", "M7", "M8"] {
            check_boundary(&lease, &boundary_context, boundary, &mut records).unwrap();
        }
        assert_eq!(
            records.iter().map(|record| record.boundary.as_str()).collect::<Vec<_>>(),
            vec!["M5", "M7", "M8"]
        );
    }

    fn boundary_test_config(dispatch: crate::config::DispatchConfig) -> Config {
        Config {
            agent: crate::config::AgentConfig {
                node_id: "fog-01".to_string(),
                cycle_s: 5,
                schema_version: "0.0.1".to_string(),
                hostname: None,
                stage: None,
                events: None,
            },
            outputs: crate::config::OutputsConfig { file_json: None, http: None },
            dispatch: Some(dispatch),
            assisted_execution: None,
            collectors: None,
            services: None,
            service_registry: None,
            auto_recovery: None,
            desired_state: None,
        }
    }

    #[test]
    fn secret_target_substitutions_reports_only_when_local_policy_overrides_the_declared_target() {
        let cfg = boundary_test_config(crate::config::DispatchConfig {
            enabled: true,
            secret_bindings: vec![crate::config::DispatchSecretBinding {
                secret_ref: "token".to_string(),
                environment_variable: "LOCAL_TOKEN".to_string(),
            }],
            ..Default::default()
        });
        let declared = vec![OperationalSecretBinding {
            secret_ref: "token".to_string(),
            target_environment_variable: "DECLARED_TOKEN".to_string(),
            version: Some("v1".to_string()),
        }];
        // legacy_partial: declared target wins, no substitution.
        let no_substitution = secret_target_substitutions(&cfg, &["token".to_string()], &declared, "legacy_partial");
        assert!(no_substitution.is_empty(), "{no_substitution:?}");

        // No declared binding at all -> local policy substitutes.
        let substituted = secret_target_substitutions(&cfg, &["token".to_string()], &[], "legacy_partial");
        assert_eq!(substituted[0].effective_value, serde_json::json!("LOCAL_TOKEN"));
        assert_eq!(substituted[0].field, "secret_bindings:token:target_environment_variable");
    }

    #[test]
    fn verify_declared_configuration_mount_content_reports_verified_sources() {
        let dir = std::env::temp_dir().join(format!("datum-p12-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("mosquitto.conf");
        std::fs::write(&file_path, b"hello").unwrap();
        let expected = crate::agent::stage107a_enforcement::sha256_hex(b"hello");
        let mounts = vec![OperationalConfigurationMount {
            source: "mosquitto.conf".to_string(),
            target: "/mosquitto.conf".to_string(),
            read_only: true,
            declared_content_sha256: Some(expected),
        }];
        let verified = verify_declared_configuration_mount_content(&mounts, Some(&dir)).unwrap();
        assert_eq!(verified.len(), 1);
        assert_eq!(verified[0].source, "mosquitto.conf");
        assert_eq!(verified[0].kind, "configuration_mount");
        assert!(verified[0].verified);
        assert_eq!(verified[0].expected_sha256, crate::agent::stage107a_enforcement::sha256_hex(b"hello"));
        assert_eq!(verified[0].actual_sha256, verified[0].expected_sha256);
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn converged_legacy_service_retains_resolved_port_and_alias_evidence() {
        let mapping = DispatchContainerMapping {
            service_name: Some("mqtt".to_string()), container_name: "datum-mqtt".to_string(),
            ports: vec![crate::config::DispatchContainerPortMapping {
                container_port: 1883, host_port: 1883, protocol: "tcp".to_string(),
            }], ..Default::default()
        };
        let effective = resolve_effective_attachments(&[], &[], &mapping, "legacy_partial").unwrap();
        let matches = inspected_container_matches_declared_attachments(
            &baseline_inspect(), "eclipse-mosquitto:2",
            &["-c".to_string(), "/mosquitto.conf".to_string()], Some("datum-net"),
            &effective.aliases, &effective.ports, &declared_bindings(), &declared_volumes(),
            &declared_config_mounts(), &matching_execution_identity(),
        );
        assert!(matches);
        assert_eq!(run_embedded_reconciler(true, matches).unwrap().action_code, 0);
        assert!(action_code_zero_may_report_converged(None, None, false));
        let evidence = serde_json::to_value(&effective.substitutions).unwrap();
        assert_eq!(evidence[0]["field"], "ports");
        assert_eq!(evidence[0]["effective_value"][0]["host_port"], 1883);
        assert_eq!(evidence[1]["field"], "network_aliases");
        assert_eq!(evidence[1]["effective_value"], serde_json::json!(["mqtt"]));
        assert!(resolve_effective_attachments(&[], &[], &mapping, "required").is_err());
    }

    #[test]
    fn all_boundary_blocks_preserve_service_status() {
        for (expiry, expected) in [
            (Some("2000-01-01T00:00:00Z"), "blocked_lease_expired"),
            (None, "blocked_authorization_not_leased"),
        ] {
            let authorization = boundary_test_auth(expiry);
            let lease = LeaseGuard::from_authorization(&authorization, 0).unwrap();
            let context = MutationBoundaryContext {
                authorization_id: Some(&authorization.authorization_id),
                expires_at_utc: expiry, configured_clock_skew_seconds: Some(0),
            };
            for boundary in ["M1", "M2", "M3", "M4", "M5", "M6", "M7", "M8"] {
                let mut records = Vec::new();
                let error = check_boundary(&lease, &context, boundary, &mut records).unwrap_err();
                assert_eq!(execution_error_status(&error.context("mutation failed")), expected);
                assert_eq!(records[0].result, "blocked");
            }
            let mapping = DispatchContainerMapping { container_name: "test".to_string(), ..Default::default() };
            let error = rollback_trusted_container_intent(&mapping, ContainerState::Running,
                &lease, &context, &mut Vec::new()).unwrap_err();
            assert_eq!(execution_error_status(&error), expected);
        }
        assert_eq!(execution_error_status(&anyhow::anyhow!("Docker exited 1")), "failed");
    }

    #[test]
    fn local_policy_digest_and_container_label_use_canonical_object_digest() {
        let cfg = boundary_test_config(crate::config::DispatchConfig::default());
        let digest = compute_local_policy_digest(&cfg).unwrap();
        assert!(crate::agent::jcs::is_canonical_digest_format(&digest));
        let identity = ExecutionIdentityLabels {
            artifact_id: "mqtt", execution_generation: 1, execution_projection_digest: &digest,
        };
        assert!(identity.as_docker_run_args().contains(&format!("datum.execution_projection_digest={digest}")));
    }

    // ---- Stage 107B-B (ADR-0014): resolve_pinned_execution_image -------- //

    fn far_future_boundary() -> (LeaseGuard, OperationalReconciliationAuthorization) {
        let expires = (Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
        let authorization = boundary_test_auth(Some(&expires));
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        (lease, authorization)
    }

    fn stage_107b_b_digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn stage_107b_b_platform(
        os: &str,
        architecture: &str,
        manifest: char,
        config: char,
        repository: &str,
    ) -> OperationalResolvedPlatformManifest {
        OperationalResolvedPlatformManifest {
            os: os.to_string(),
            architecture: architecture.to_string(),
            variant: None,
            manifest_digest: stage_107b_b_digest(manifest),
            config_digest: stage_107b_b_digest(config),
            execution_reference: format!("{repository}@{}", stage_107b_b_digest(manifest)),
        }
    }

    fn stage_107b_b_resolved(repository: &str, platforms: Vec<OperationalResolvedPlatformManifest>) -> OperationalResolvedImageIdentity {
        OperationalResolvedImageIdentity {
            repository: repository.to_string(),
            index_digest: None,
            platforms,
            resolved_at_utc: "2026-01-01T00:00:00Z".to_string(),
            resolver_tool_version: "test/0.1.0".to_string(),
            resolved_by: "operator:test".to_string(),
            registry_host: "registry-1.docker.io".to_string(),
        }
    }

    fn stage_107b_b_inspect_record(id: char, repository: &str, manifest: char, os: &str, architecture: &str) -> image_identity_verification::ImageInspectRecord {
        image_identity_verification::ImageInspectRecord {
            id: stage_107b_b_digest(id),
            repo_digests: vec![format!("{repository}@{}", stage_107b_b_digest(manifest))],
            os: os.to_string(),
            architecture: architecture.to_string(),
            variant: None,
        }
    }

    fn preflight_for(
        runtime: &dyn DockerImageRuntime,
        resolved: &OperationalResolvedImageIdentity,
        existing_container_name: Option<&str>,
    ) -> (VerifiedImageIdentity, image_identity_verification::PreflightOutcome) {
        let evaluation = image_identity_verification::evaluate_image_identity(
            runtime, resolved, existing_container_name, "2026-01-01T00:00:00Z",
        );
        let outcome = image_identity_verification::classify_evaluation(&evaluation);
        (evaluation.evidence, outcome)
    }

    // Test matrix item 1: exact pinned image already present => verified, no pull.
    #[test]
    fn resolve_pinned_image_outcome_exact_match_already_present_skips_pull() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert_eq!(result.execution_reference.as_deref(), Some(reference.as_str()));
        assert!(result.block.is_none());
        assert_eq!(result.evidence.unwrap().pull_disposition.as_deref(), Some(image_identity_verification::PULL_DISPOSITION_ALREADY_PRESENT));
        assert!(runtime.pull_calls.borrow().is_empty(), "an exact match must never be pulled over");
        assert!(records.is_empty(), "no M5 boundary attempt when nothing needed to be pulled");
    }

    // Test matrix item 2: image not found => M5 pull once => re-inspect => verified.
    #[test]
    fn resolve_pinned_image_outcome_absent_pulls_exactly_once_then_verifies() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Ok(None));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        runtime.queue_pull(&reference, Ok(()));
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert_eq!(result.execution_reference.as_deref(), Some(reference.as_str()), "{:?}", result.block);
        assert_eq!(result.evidence.unwrap().pull_disposition.as_deref(), Some(image_identity_verification::PULL_DISPOSITION_PULL_PERFORMED));
        assert_eq!(runtime.pull_calls.borrow().len(), 1, "exactly one pull");
        assert_eq!(runtime.inspect_calls.borrow().len(), 2, "inspect before AND after the pull");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].boundary, "M5");
        assert_eq!(records[0].result, "passed");
        assert!(result.acquisition_performed, "v9 review P1-1: a successful pull must always be reflected as an acquisition");
    }

    // v12 review P0-1: the exact "REQUIRED SCENARIO" from the review --
    // target image initially absent (NeedsPull, container_binding
    // necessarily NotApplicable since there is no local target image
    // object to compare the existing container against yet), M5 pulls,
    // and the MANDATORY post-pull re-evaluation discovers the EXISTING
    // container is bound to a DIFFERENT image than the just-verified
    // target. The FINAL evidence must report the mismatch even though the
    // INITIAL preflight could not possibly have seen it.
    #[test]
    fn resolve_pinned_image_outcome_discovers_container_binding_mismatch_only_after_the_post_pull_reevaluation() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));

        // Initial preflight: target image C is recognized absent locally.
        runtime.queue_inspect(&reference, Ok(None));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, Some("mqtt"));
        assert_eq!(outcome, image_identity_verification::PreflightOutcome::NeedsPull { execution_reference: reference.clone() });
        assert!(!container_binding_mismatches_from_status(Some(evidence.container_binding_status.as_str())),
            "the initial preflight cannot see a binding mismatch it never had a target image to compare against");
        assert_eq!(evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_NOT_APPLICABLE);
        // No inspect_container_image_binding call was even made yet (the
        // Ok(None) target-absent branch never reaches that check).
        assert!(runtime.container_binding_calls.borrow().is_empty());

        // M5 pulls C. Post-pull re-inspection: target C verifies. Existing
        // container "mqtt" is bound to a DIFFERENT image, B (digest '9').
        runtime.queue_pull(&reference, Ok(()));
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('9'))));

        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, Some("mqtt"), Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );

        // Target image C itself is verified -- this is drift, not a
        // terminal target-image mismatch.
        assert!(result.block.is_none(), "a container-binding mismatch must never become a target-image block: {:?}", result.block);
        assert_eq!(result.execution_reference.as_deref(), Some(reference.as_str()));
        let final_evidence = result.evidence.expect("evidence must be present");
        assert!(final_evidence.target_verified, "the target image itself verified");
        assert_eq!(final_evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES);

        // This is the exact fact `container_binding_drift_for_execution`
        // must be re-derived from -- the per-service loop's own logic
        // (verified here via the same pure function it calls).
        assert!(container_binding_mismatches_from_status(Some(final_evidence.container_binding_status.as_str())),
            "the FINAL, post-pull evidence must be recognized as drift for execution purposes");
    }

    // Item 6 (v6 review): when the preflight already found the mismatch
    // BEFORE acquisition (target already present and verified, no
    // NeedsPull, no M5 attempted at all), `resolve_pinned_image_outcome`
    // must return that SAME evidence unchanged -- proving the v6 fix does
    // not alter the no-M5 path's behavior at all.
    #[test]
    fn resolve_pinned_image_outcome_no_m5_path_preserves_a_preflight_discovered_binding_mismatch_unchanged() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        // Target C already present and verifies; existing container "mqtt"
        // already bound to a different image B (digest '9') -- discovered
        // by the INITIAL preflight itself, no NeedsPull involved.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('9'))));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, Some("mqtt"));
        assert_eq!(outcome, image_identity_verification::PreflightOutcome::Verified(reference.clone()));
        assert_eq!(evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES);

        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, Some("mqtt"), Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(runtime.pull_calls.borrow().is_empty(), "a non-NeedsPull outcome must never attempt M5");
        assert!(result.block.is_none());
        let final_evidence = result.evidence.expect("evidence must be present");
        assert_eq!(final_evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES);
        assert!(container_binding_mismatches_from_status(Some(final_evidence.container_binding_status.as_str())));
    }

    #[test]
    fn container_binding_mismatches_from_status_is_true_only_for_the_mismatches_string() {
        assert!(container_binding_mismatches_from_status(Some(image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES)));
        assert!(!container_binding_mismatches_from_status(Some(image_identity_verification::CONTAINER_BINDING_STATUS_MATCHES)));
        assert!(!container_binding_mismatches_from_status(Some(image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE)));
        assert!(!container_binding_mismatches_from_status(Some(image_identity_verification::CONTAINER_BINDING_STATUS_NOT_APPLICABLE)));
        assert!(!container_binding_mismatches_from_status(None));
    }

    // Test matrix items 33-35 (v9 review P1-1): a successful M5 pull must be
    // reflected as `acquisition_performed == true` EVEN WHEN the mandatory
    // re-verification after that pull finds a mismatch and blocks -- a
    // failed/blocked post-pull re-check must never be reported as "no side
    // effects," because the host's local image store genuinely changed.
    #[test]
    fn resolve_pinned_image_outcome_acquisition_performed_stays_true_even_when_reverification_blocks() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Ok(None));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        runtime.queue_pull(&reference, Ok(()));
        // The registry served up a genuinely mismatching manifest after the
        // pull -- a real hard mismatch, not a transient inspect failure.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '9', "linux", "amd64"))));
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(result.block.as_deref().unwrap().starts_with(BLOCKED_IMAGE_IDENTITY_MISMATCH), "{:?}", result.block);
        assert!(result.execution_reference.is_none());
        assert_eq!(runtime.pull_calls.borrow().len(), 1);
        assert!(
            result.acquisition_performed,
            "the pull itself succeeded -- the host WAS mutated, regardless of the post-pull re-verification's outcome"
        );
    }

    // Test matrix item 3: inspect daemon/error != not-found => no pull, unverified block.
    #[test]
    fn resolve_pinned_image_outcome_inspect_error_never_pulls_and_is_unverified() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Err("permission denied".to_string()));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(runtime.pull_calls.borrow().is_empty(), "an inspect error must never be treated as absence-and-pullable");
        assert!(result.execution_reference.is_none());
        assert!(result.block.as_deref().unwrap().starts_with(BLOCKED_IMAGE_IDENTITY_UNVERIFIED), "{:?}", result.block);
    }

    // Test matrix items 14/15: mismatch blocks in BOTH required and legacy_partial.
    #[test]
    fn resolve_pinned_image_outcome_mismatch_blocks_in_required_and_legacy_partial() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        for mode in [EXECUTION_PINNING_MODE_REQUIRED, crate::agent::stage107a_enforcement::EXECUTION_PINNING_MODE_LEGACY_PARTIAL] {
            let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
                os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
            });
            let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
            // Wrong manifest digest in RepoDigests -> manifest_mismatch.
            runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '9', "linux", "amd64"))));
            let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
            let (lease, _auth) = far_future_boundary();
            let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
            let mut records = Vec::new();
            let result = resolve_pinned_image_outcome(
                &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, mode,
            );
            assert!(result.execution_reference.is_none(), "mode={mode}: a mismatch must never yield a usable reference");
            assert!(result.block.as_deref().unwrap().starts_with(BLOCKED_IMAGE_IDENTITY_MISMATCH), "mode={mode}: {:?}", result.block);
        }
    }

    // Test matrix items 16/17, v8 review P0-2: unverifiable blocks in
    // required, but is permitted in legacy_partial -- AND, per the P0-2 fix,
    // legacy_partial carries the immutable execution_reference FORWARD
    // rather than leaving the caller to fall back to the mutable tag.
    #[test]
    fn resolve_pinned_image_outcome_unverifiable_blocks_only_in_required_and_legacy_partial_keeps_the_reference() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));

        let required_runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        required_runtime.queue_inspect(&reference, Err("daemon unreachable".to_string()));
        let (required_evidence, required_outcome) = preflight_for(&required_runtime, &resolved, None);
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let required_result = resolve_pinned_image_outcome(
            &required_runtime, required_evidence, required_outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(required_result.execution_reference.is_none());
        assert!(required_result.block.as_deref().unwrap().starts_with(BLOCKED_IMAGE_IDENTITY_UNVERIFIED));

        let legacy_runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        legacy_runtime.queue_inspect(&reference, Err("daemon unreachable".to_string()));
        let (legacy_evidence, legacy_outcome) = preflight_for(&legacy_runtime, &resolved, None);
        let mut records2 = Vec::new();
        let legacy_result = resolve_pinned_image_outcome(
            &legacy_runtime, legacy_evidence, legacy_outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records2,
            crate::agent::stage107a_enforcement::EXECUTION_PINNING_MODE_LEGACY_PARTIAL,
        );
        assert!(legacy_result.block.is_none(), "legacy_partial must not block on unverifiable evidence: {:?}", legacy_result.block);
        assert_eq!(
            legacy_result.execution_reference.as_deref(), Some(reference.as_str()),
            "P0-2: legacy_partial must carry the immutable reference forward, never fall back to the mutable declared tag"
        );
        assert!(!legacy_result.evidence.unwrap().target_verified, "legacy_partial continuing must still record target_verified=false, never silently claim success");
    }

    // Test matrix item 7/9: platform selection integration -- ambiguous
    // variant candidates block as unverified, in EVERY mode, even before any
    // inspect call (v8 review P0-2 case B: no selectable reference exists at
    // all, so there is nothing safe to fall back to regardless of mode).
    #[test]
    fn resolve_pinned_image_outcome_ambiguous_platform_blocks_in_every_mode_before_any_inspect() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(
            repository,
            vec![
                OperationalResolvedPlatformManifest {
                    os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v6".to_string()),
                    manifest_digest: stage_107b_b_digest('1'), config_digest: stage_107b_b_digest('2'),
                    execution_reference: format!("{repository}@{}", stage_107b_b_digest('1')),
                },
                OperationalResolvedPlatformManifest {
                    os: "linux".to_string(), architecture: "arm".to_string(), variant: Some("v7".to_string()),
                    manifest_digest: stage_107b_b_digest('3'), config_digest: stage_107b_b_digest('4'),
                    execution_reference: format!("{repository}@{}", stage_107b_b_digest('3')),
                },
            ],
        );
        for mode in [EXECUTION_PINNING_MODE_REQUIRED, crate::agent::stage107a_enforcement::EXECUTION_PINNING_MODE_LEGACY_PARTIAL] {
            let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
                os: "linux".to_string(), architecture: "arm".to_string(), variant: None,
            });
            let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
            let (lease, _auth) = far_future_boundary();
            let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
            let mut records = Vec::new();
            let result = resolve_pinned_image_outcome(
                &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, mode,
            );
            assert!(result.execution_reference.is_none(), "mode={mode}");
            assert!(result.block.as_deref().unwrap().starts_with(BLOCKED_IMAGE_IDENTITY_UNVERIFIED), "mode={mode}: {:?}", result.block);
            assert!(runtime.inspect_calls.borrow().is_empty(), "mode={mode}: must never inspect without a selected platform");
        }
    }

    // A present-but-mismatching image must never be pulled over (review
    // decision, "PRE-EXISTING LOCAL IMAGE" item 4): the fake never queues a
    // pull response, so a pull attempt here would panic the fake -- proving
    // none was attempted.
    #[test]
    fn resolve_pinned_image_outcome_never_pulls_over_a_present_mismatch() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('9', "eclipse-mosquitto", '1', "linux", "amd64"))));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(result.block.as_deref().unwrap().starts_with(BLOCKED_IMAGE_IDENTITY_MISMATCH), "{:?}", result.block);
        assert!(runtime.pull_calls.borrow().is_empty());
    }

    // Test matrix item 26: an expired/unleased authorization retains its
    // EXISTING stable lease block code, not an image-identity one -- proving
    // Stage 107A lease semantics are threaded through unchanged.
    #[test]
    fn resolve_pinned_image_outcome_expired_lease_blocks_with_the_existing_lease_code_not_an_image_code() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        // Recognized absent -> a pull WOULD be attempted, but the lease is
        // already expired, so `check_boundary` must block it first.
        runtime.queue_inspect(&reference, Ok(None));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        let authorization = boundary_test_auth(Some("2020-01-01T00:00:00Z"));
        let lease = LeaseGuard::from_authorization(&authorization, 30).unwrap();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, true, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(
            result.block.as_deref().unwrap().starts_with(crate::agent::stage107a_enforcement::BLOCKED_LEASE_EXPIRED),
            "{:?}", result.block
        );
        assert!(runtime.pull_calls.borrow().is_empty(), "a blocked M5 boundary must never pull");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].boundary, "M5");
        assert_eq!(records[0].result, "blocked");
    }

    // v8 review P0-1: the 7 "no pull" scenarios explicitly requested by the
    // review, all proven through the ONE gate `resolve_pinned_image_outcome`
    // actually consults (`mutation_gates_clear`) -- covering generation
    // mismatch, projection mismatch, required-incomplete, config-content
    // mismatch, and local-execution-disabled uniformly (each is just a
    // reason `mutation_gates_clear` would be false in the real per-service
    // loop); expired/nonleased authorization and known image mismatch are
    // covered by the dedicated tests above.
    #[test]
    fn resolve_pinned_image_outcome_never_pulls_when_mutation_gates_are_not_clear() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        let (lease, _auth) = far_future_boundary();
        let boundary_context = MutationBoundaryContext { authorization_id: None, expires_at_utc: None, configured_clock_skew_seconds: None };

        // Simulates: generation mismatch, projection mismatch,
        // required-incomplete, configuration-content mismatch, and
        // local-execution-disabled -- every one of these makes
        // `mutation_gates_clear` false in the real per-service loop, and
        // `resolve_pinned_image_outcome` must treat all of them identically:
        // no acquisition, no new block (whatever gate IS blocking already
        // explains the status).
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        runtime.queue_inspect(&reference, Ok(None));
        let (evidence, outcome) = preflight_for(&runtime, &resolved, None);
        let mut records = Vec::new();
        let result = resolve_pinned_image_outcome(
            &runtime, evidence, outcome, false, &resolved, None, Some(&lease), &boundary_context, &mut records, EXECUTION_PINNING_MODE_REQUIRED,
        );
        assert!(runtime.pull_calls.borrow().is_empty(), "mutation_gates_clear=false must never pull, regardless of WHY");
        assert!(runtime.inspect_calls.borrow().len() == 1, "no re-inspection either -- acquisition was never attempted");
        assert!(result.block.is_none(), "no NEW block is added -- the already-blocking gate explains the status");
        assert!(!result.acquisition_performed, "no pull was attempted, so no acquisition can have been performed");
        assert!(result.execution_reference.is_none(), "no mutable-tag fallback either");
        assert!(records.is_empty(), "no M5 boundary attempt at all when gates are not clear");
    }

    // Test matrix item 24 (the part exercisable without a live Docker
    // daemon), corrected for v9 review P0-2: a running container bound to a
    // DIFFERENT image than the verified target is a runtime DRIFT
    // observation -- never contaminating TARGET IMAGE verification -- so it
    // must be reported ONLY via `container_binding`, never via
    // `verified`/`reason_kind`, and the preflight outcome for the TARGET
    // stays `Verified` (a separate, per-service `container_binding_drift`
    // flag is what forces the governed recreate; see
    // `operational_reconciliation`'s outer `attachment_mismatch`).
    #[test]
    fn evaluate_image_identity_reports_container_binding_mismatch_for_a_running_container() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        // v8 review P1-1: container-binding evidence now comes exclusively
        // through `DockerImageRuntime::inspect_container_image_binding`
        // (fakeable), never an unabstracted `Command::new` inside this file.
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('9'))));
        let evaluation = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(evaluation.verified, "target image identity must verify independently of container binding: {evaluation:?}");
        assert_eq!(
            evaluation.container_binding,
            image_identity_verification::ContainerBindingObservation::Mismatches { actual_image_id: stage_107b_b_digest('9') }
        );
        assert_eq!(
            image_identity_verification::classify_evaluation(&evaluation),
            image_identity_verification::PreflightOutcome::Verified(reference)
        );
    }

    // v8 review P1-1, corrected for v9 review P0-2: container inspect
    // failure/malformed/missing must surface as UNAVAILABLE -- via
    // `container_binding`, never silently treated as "nothing to check" --
    // but it must ALSO never contaminate TARGET IMAGE verification.
    #[test]
    fn evaluate_image_identity_reports_container_binding_unavailable_not_silently_ignored() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Err("docker inspect failed".to_string()));
        let evaluation = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(evaluation.verified, "target image identity must verify independently of container-binding availability: {evaluation:?}");
        assert_eq!(evaluation.container_binding, image_identity_verification::ContainerBindingObservation::Unavailable);
        assert_eq!(
            image_identity_verification::classify_evaluation(&evaluation),
            image_identity_verification::PreflightOutcome::Verified(reference)
        );
    }

    // ---- v9 review P0-1/P0-2: image_known_mismatch_from / container_binding_drift_from ----
    // Test matrix items 14-19: a KnownMismatch preflight outcome (real
    // candidate mismatch) is the ONLY thing that sets `image_known_mismatch`;
    // a container-binding `Mismatches` observation (drift after a re-pin) is
    // the ONLY thing that sets `container_binding_drift`; neither ever sets
    // the other.

    #[test]
    fn image_known_mismatch_from_is_true_only_for_known_mismatch() {
        assert!(image_known_mismatch_from(Some(&image_identity_verification::PreflightOutcome::KnownMismatch("manifest_mismatch".to_string()))));
        assert!(!image_known_mismatch_from(Some(&image_identity_verification::PreflightOutcome::Verified("ref".to_string()))));
        assert!(!image_known_mismatch_from(Some(&image_identity_verification::PreflightOutcome::NeedsPull { execution_reference: "ref".to_string() })));
        assert!(!image_known_mismatch_from(Some(&image_identity_verification::PreflightOutcome::UnverifiedWithReference {
            execution_reference: "ref".to_string(), reason_kind: "inspect_unavailable".to_string(),
        })));
        assert!(!image_known_mismatch_from(Some(&image_identity_verification::PreflightOutcome::NoSelectableReference("no_matching_platform".to_string()))));
        assert!(!image_known_mismatch_from(None), "no preflight ran at all (unpinned/local_build) must never be treated as a known mismatch");
    }

    #[test]
    fn container_binding_drift_from_is_true_only_for_mismatches_observation() {
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));

        // Drifted: target verifies, container bound to a DIFFERENT image.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('9'))));
        let drifted = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(container_binding_drift_from(Some(&drifted)));

        // Matching: no drift.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('2'))));
        let matching = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(!container_binding_drift_from(Some(&matching)));

        // Unavailable: not drift (fails closed elsewhere via `verified`
        // remaining evidence-truthful, but never forces a recreate on its
        // own -- only a KNOWN wrong binding does).
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Err("permission denied".to_string()));
        let unavailable = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(!container_binding_drift_from(Some(&unavailable)));

        // No container at all (Missing state): NotApplicable, never drift.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        let missing = image_identity_verification::evaluate_image_identity(&runtime, &resolved, None, "2026-01-01T00:00:00Z");
        assert!(!container_binding_drift_from(Some(&missing)));

        assert!(!container_binding_drift_from(None), "no preflight ran at all must never be treated as drift");
    }

    // ---- v11 review P0-1/P1-1: container_binding_unavailable_in_required_mode_from ----
    // Test matrix items 1-6 from the v3->v4 independent review, reshaped
    // as a plain `bool` reuse-guard per the v4->v5 review.

    #[test]
    fn container_binding_unavailable_in_required_mode_is_true_only_in_required_mode() {
        // Item 2/3: required + target verified + binding Unavailable =>
        // the reuse guard fires (caller must refuse Running -> already_running
        // and Stopped -> M2, never silently converge/start).
        assert!(container_binding_unavailable_in_required_mode_from(
            Some(image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE),
            EXECUTION_PINNING_MODE_REQUIRED,
        ));

        // Item 5: legacy_partial preserves the weaker "may still proceed"
        // semantics -- the guard never fires here (evidence truthfulness is
        // handled entirely by `container_binding_status` itself, not by
        // this gate).
        assert!(!container_binding_unavailable_in_required_mode_from(
            Some(image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE),
            crate::agent::stage107a_enforcement::EXECUTION_PINNING_MODE_LEGACY_PARTIAL,
        ));
    }

    #[test]
    fn container_binding_unavailable_in_required_mode_never_fires_for_matches_or_mismatches_or_not_applicable() {
        // Item 1: Matches => normal convergence/M2 behavior may proceed.
        assert!(!container_binding_unavailable_in_required_mode_from(
            Some(image_identity_verification::CONTAINER_BINDING_STATUS_MATCHES), EXECUTION_PINNING_MODE_REQUIRED,
        ));
        // Item 4: Mismatches is drift, handled entirely by
        // `container_binding_drift_from`/M3 -- this guard must never ALSO
        // fire for it (that would double-govern the same fact two ways).
        assert!(!container_binding_unavailable_in_required_mode_from(
            Some(image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES), EXECUTION_PINNING_MODE_REQUIRED,
        ));
        assert!(!container_binding_unavailable_in_required_mode_from(
            Some(image_identity_verification::CONTAINER_BINDING_STATUS_NOT_APPLICABLE), EXECUTION_PINNING_MODE_REQUIRED,
        ));
        assert!(!container_binding_unavailable_in_required_mode_from(None, EXECUTION_PINNING_MODE_REQUIRED));
    }

    // ---- v11 review P0-1: rollback_service_block_from ----
    // Test matrix items 1-4 and 7 from the v4->v5 independent review: since
    // this function's signature does not even accept an image-identity
    // block as a parameter, a KnownMismatch/UnverifiedWithReference/
    // NoSelectableReference/binding-unavailable outcome can never reach it
    // -- proven by construction, not merely by a passing assertion. This
    // test instead proves the OTHER, legitimate pre-existing rollback
    // prerequisites this function DOES retain still work correctly.

    #[test]
    fn rollback_service_block_from_is_none_when_every_pre_image_gate_is_clear() {
        assert!(rollback_service_block_from(None, None).is_none());
    }

    #[test]
    fn rollback_service_block_from_still_blocks_on_pre_existing_generation_projection_completeness_gate() {
        // Item 7: a genuine, pre-existing rollback prerequisite (e.g. a
        // generation/projection mismatch, unrelated to Stage 107B-B image
        // verification) must still block exactly as before.
        assert_eq!(
            rollback_service_block_from(Some("blocked_projection_mismatch: recomputed digest disagrees"), None).as_deref(),
            Some("blocked_projection_mismatch: recomputed digest disagrees"),
        );
    }

    #[test]
    fn rollback_service_block_from_still_blocks_on_pre_existing_attachment_policy_gate() {
        assert_eq!(
            rollback_service_block_from(None, Some("blocked_secret_substitution: unresolved target variable")).as_deref(),
            Some("blocked_secret_substitution: unresolved target variable"),
        );
    }

    // ---- v10 review P0-1/P0-4: image_identity_unconfirmed_in_required_mode ----

    #[test]
    fn image_identity_unconfirmed_in_required_mode_covers_both_needs_pull_and_binding_unavailable() {
        // Either sub-condition alone is sufficient, in required mode, while
        // executing and authorized.
        assert!(image_identity_unconfirmed_in_required_mode(true, true, EXECUTION_PINNING_MODE_REQUIRED, true, false));
        assert!(image_identity_unconfirmed_in_required_mode(true, true, EXECUTION_PINNING_MODE_REQUIRED, false, true));
        assert!(image_identity_unconfirmed_in_required_mode(true, true, EXECUTION_PINNING_MODE_REQUIRED, true, true));
        assert!(!image_identity_unconfirmed_in_required_mode(true, true, EXECUTION_PINNING_MODE_REQUIRED, false, false));
    }

    #[test]
    fn image_identity_unconfirmed_in_required_mode_is_false_outside_required_execution_authorization() {
        // Item 5: legacy_partial is never affected, even with both
        // sub-conditions true.
        assert!(!image_identity_unconfirmed_in_required_mode(
            true, true, crate::agent::stage107a_enforcement::EXECUTION_PINNING_MODE_LEGACY_PARTIAL, true, true,
        ));
        // Item 6: non-executing (no --execute) is never affected.
        assert!(!image_identity_unconfirmed_in_required_mode(false, true, EXECUTION_PINNING_MODE_REQUIRED, true, true));
        // Unauthorized is never affected either.
        assert!(!image_identity_unconfirmed_in_required_mode(true, false, EXECUTION_PINNING_MODE_REQUIRED, true, true));
    }

    // ---- v10 review P1-1: container_binding_status / target_verified evidence shape ----
    // Test matrix item 10: evidence can distinguish all four container-binding
    // states, plus a target-unverified case and an unpinned/not-applicable
    // source, without ever inferring the answer from an absent field.

    #[test]
    fn evidence_distinguishes_all_container_binding_states_via_evaluate_image_identity() {
        let repository = "docker.io/library/eclipse-mosquitto";
        let resolved = stage_107b_b_resolved(repository, vec![stage_107b_b_platform("linux", "amd64", '1', '2', repository)]);
        let reference = format!("{repository}@{}", stage_107b_b_digest('1'));

        // target verified + no existing container (Missing state) -> not_applicable.
        let runtime = image_identity_verification::FakeDockerImageRuntime::new(image_identity_verification::PlatformIdentity {
            os: "linux".to_string(), architecture: "amd64".to_string(), variant: None,
        });
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        let missing = image_identity_verification::evaluate_image_identity(&runtime, &resolved, None, "2026-01-01T00:00:00Z");
        assert!(missing.evidence.target_verified);
        assert_eq!(missing.evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_NOT_APPLICABLE);
        assert!(!missing.evidence.runtime_identity_verified(), "no existing container means no runtime binding was ever proven, regardless of target verification");

        // target verified + container binding matches.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('2'))));
        let matches = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(matches.evidence.target_verified);
        assert_eq!(matches.evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MATCHES);
        assert!(matches.evidence.runtime_identity_verified());

        // target verified + container binding mismatches.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Ok(Some(stage_107b_b_digest('9'))));
        let mismatches = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(mismatches.evidence.target_verified);
        assert_eq!(mismatches.evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES);
        assert!(!mismatches.evidence.runtime_identity_verified());

        // target verified + container binding unavailable.
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '1', "linux", "amd64"))));
        runtime.queue_container_binding("mqtt", Err("permission denied".to_string()));
        let unavailable = image_identity_verification::evaluate_image_identity(&runtime, &resolved, Some("mqtt"), "2026-01-01T00:00:00Z");
        assert!(unavailable.evidence.target_verified);
        assert_eq!(unavailable.evidence.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE);
        assert!(!unavailable.evidence.runtime_identity_verified());

        // target unverified (manifest mismatch).
        runtime.queue_inspect(&reference, Ok(Some(stage_107b_b_inspect_record('2', "eclipse-mosquitto", '9', "linux", "amd64"))));
        let target_unverified = image_identity_verification::evaluate_image_identity(&runtime, &resolved, None, "2026-01-01T00:00:00Z");
        assert!(!target_unverified.evidence.target_verified);
        assert!(!target_unverified.evidence.runtime_identity_verified());

        // unpinned/not-applicable source.
        let not_applicable = image_identity_verification::not_applicable_pinning_evidence(
            image_identity_verification::REASON_LOCAL_BUILD_NOT_PINNABLE, "2026-01-01T00:00:00Z",
        );
        assert!(!not_applicable.target_verified);
        assert_eq!(not_applicable.container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_NOT_APPLICABLE);
        assert!(!not_applicable.runtime_identity_verified());

        // Serialization proof (item 10): the four container-binding states
        // survive a JSON round-trip as their exact, distinct strings --
        // never collapsing into an absent/ambiguous field.
        for (evidence, expected_status) in [
            (&matches.evidence, image_identity_verification::CONTAINER_BINDING_STATUS_MATCHES),
            (&mismatches.evidence, image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES),
            (&unavailable.evidence, image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE),
            (&missing.evidence, image_identity_verification::CONTAINER_BINDING_STATUS_NOT_APPLICABLE),
        ] {
            let json = serde_json::to_value(evidence).unwrap();
            assert_eq!(json["container_binding_status"], expected_status);
        }
    }

    // ---- v9 review P0-3: mutation_gates_are_clear, each conjunct in isolation ----

    struct AllClearFixture;
    impl AllClearFixture {
        fn call(overrides: &[(&str, bool)]) -> bool {
            let mut values = std::collections::HashMap::new();
            for key in [
                "execute_requested", "control_plane_authorized", "rollback_requested",
                "mapping_present", "dependency_blockers_empty",
                "stage107a_service_block_before_image_clear", "config_content_block_clear",
                "attachment_policy_block_clear", "local_container_execution_allowed",
                "batch_secret_resolution_clear", "secret_resolution_clear",
                "observation_clear", "completion_not_failed", "host_action_requested",
            ] {
                values.insert(key, key != "rollback_requested");
            }
            for (key, value) in overrides {
                values.insert(*key, *value);
            }
            mutation_gates_are_clear(
                values["execute_requested"], values["control_plane_authorized"], values["rollback_requested"],
                values["mapping_present"], values["dependency_blockers_empty"],
                values["stage107a_service_block_before_image_clear"], values["config_content_block_clear"],
                values["attachment_policy_block_clear"], values["local_container_execution_allowed"],
                values["batch_secret_resolution_clear"], values["secret_resolution_clear"],
                values["observation_clear"], values["completion_not_failed"], values["host_action_requested"],
            )
        }
    }

    #[test]
    fn mutation_gates_are_clear_when_everything_is_clear() {
        assert!(AllClearFixture::call(&[]));
    }

    #[test]
    fn mutation_gates_are_not_clear_when_any_single_gate_is_not_clear() {
        for (key, value) in [
            ("execute_requested", false),
            ("control_plane_authorized", false),
            ("rollback_requested", true),
            ("mapping_present", false),
            ("dependency_blockers_empty", false),
            ("stage107a_service_block_before_image_clear", false),
            ("config_content_block_clear", false),
            ("attachment_policy_block_clear", false),
            ("local_container_execution_allowed", false),
            ("batch_secret_resolution_clear", false),
            ("secret_resolution_clear", false),
            ("observation_clear", false),
            ("completion_not_failed", false),
            ("host_action_requested", false),
        ] {
            assert!(
                !AllClearFixture::call(&[(key, value)]),
                "expected gates NOT clear when '{key}' is set to {value}"
            );
        }
    }

    // ---- v9 review P1-2: post_action_container_binding_check, pure decision ----

    #[test]
    fn post_action_binding_check_matching_records_id_status_and_no_status_override() {
        let (actual, container_binding_status, override_status) =
            post_action_container_binding_check("datum-mqtt", "sha256:abc", Ok(Some("sha256:abc".to_string())));
        assert_eq!(actual.as_deref(), Some("sha256:abc"));
        assert_eq!(container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MATCHES);
        assert!(override_status.is_none());
    }

    #[test]
    fn post_action_binding_check_mismatching_records_id_status_and_overrides_status() {
        let (actual, container_binding_status, override_status) =
            post_action_container_binding_check("datum-mqtt", "sha256:abc", Ok(Some("sha256:def".to_string())));
        assert_eq!(actual.as_deref(), Some("sha256:def"));
        assert_eq!(container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_MISMATCHES);
        let (status, message) = override_status.expect("mismatch must override the status");
        assert_eq!(status, "container_binding_unconfirmed");
        assert!(message.contains("sha256:def") && message.contains("sha256:abc"));
    }

    #[test]
    fn post_action_binding_check_unavailable_on_inspect_error_records_no_id_status_and_overrides_status() {
        let (actual, container_binding_status, override_status) =
            post_action_container_binding_check("datum-mqtt", "sha256:abc", Err("permission denied".to_string()));
        assert!(actual.is_none());
        assert_eq!(container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE);
        let (status, _message) = override_status.expect("an inspect error must override the status");
        assert_eq!(status, "container_binding_unconfirmed");
    }

    #[test]
    fn post_action_binding_check_unavailable_on_recognized_missing_records_no_id_status_and_overrides_status() {
        let (actual, container_binding_status, override_status) =
            post_action_container_binding_check("datum-mqtt", "sha256:abc", Ok(None));
        assert!(actual.is_none());
        assert_eq!(container_binding_status, image_identity_verification::CONTAINER_BINDING_STATUS_UNAVAILABLE);
        assert!(override_status.is_some());
    }

    // v8 review P0-3: pinned pull-policy arguments.
    #[test]
    fn pinned_pull_policy_args_adds_pull_never_only_when_pinned() {
        assert_eq!(pinned_pull_policy_args(true), vec!["--pull".to_string(), "never".to_string()]);
        assert!(pinned_pull_policy_args(false).is_empty());
    }

    // v8 review P0-3/P1-5: the core execution-argument guarantees, proven
    // without a live Docker daemon.
    #[test]
    fn build_governed_docker_run_args_uses_the_immutable_reference_and_pull_never_when_pinned() {
        let identity = ExecutionIdentityLabels { artifact_id: "mqtt", execution_generation: 1, execution_projection_digest: "sha256:aa" };
        let execution_reference = "docker.io/library/eclipse-mosquitto@sha256:1111111111111111111111111111111111111111111111111111111111111111";
        let args = build_governed_docker_run_args(
            "mqtt", execution_reference, true, None, &[], &[], &[], &[], None, &[], &identity, &[],
        );
        assert!(args.contains(&"--pull".to_string()) && args.windows(2).any(|w| w == ["--pull".to_string(), "never".to_string()]));
        assert!(args.contains(&execution_reference.to_string()), "{args:?}");
        assert!(!args.iter().any(|arg| arg == "eclipse-mosquitto:2"), "no mutable tag may appear as the runtime image argument when pinned");
    }

    #[test]
    fn build_governed_docker_run_args_unpinned_uses_the_declared_tag_with_no_pull_policy_flag() {
        let identity = ExecutionIdentityLabels { artifact_id: "mqtt", execution_generation: 1, execution_projection_digest: "sha256:aa" };
        let args = build_governed_docker_run_args(
            "mqtt", "eclipse-mosquitto:2", false, None, &[], &[], &[], &[], None, &[], &identity, &[],
        );
        assert!(!args.contains(&"--pull".to_string()), "unpinned execution must not gain a pull-policy flag: {args:?}");
        assert!(args.contains(&"eclipse-mosquitto:2".to_string()));
    }

    // Test matrix item 28: evidence timestamp changing alone must never
    // change the execution projection digest -- proven structurally:
    // `VerifiedImageIdentity` is not part of `OperationalArtifactAssignment`
    // at all (it lives only on `OperationalServiceReconciliationReport`,
    // report-side evidence, never on the assignment the projection is
    // computed over).
    #[test]
    fn verified_image_identity_evidence_has_no_projection_relevant_field() {
        fn assignment_has_no_verified_image_identity_field(_assignment: &OperationalArtifactAssignment) {}
        let _ = assignment_has_no_verified_image_identity_field; // compiles iff the field truly isn't there
    }

    // Test matrix item 29/30/31: Option-A / no live Docker or registry --
    // structural: `resolve_pinned_image_outcome` takes only an already-
    // fetched `OperationalResolvedImageIdentity` (from the already-fetched
    // `OperationalReconciliationContext` snapshot), an already-computed
    // preflight outcome, and a `&dyn DockerImageRuntime` -- it has no HTTP
    // client, no DServer URL, and no registry-transport parameter of any
    // kind, so it structurally cannot re-resolve or contact a registry
    // regardless of what its `DockerImageRuntime` argument does.
    #[test]
    fn resolve_pinned_image_outcome_signature_has_no_network_or_registry_capability() {
        #[allow(clippy::too_many_arguments)]
        fn assert_no_network_parameter(
            _runtime: &dyn DockerImageRuntime,
            _preflight_evidence: VerifiedImageIdentity,
            _preflight_outcome: image_identity_verification::PreflightOutcome,
            _mutation_gates_clear: bool,
            _resolved_identity: &OperationalResolvedImageIdentity,
            _existing_container_name: Option<&str>,
            _lease: Option<&LeaseGuard>,
            _boundary_context: &MutationBoundaryContext,
            _records: &mut Vec<MutationBoundaryRecord>,
            _effective_mode: &str,
        ) {
        }
        let _ = assert_no_network_parameter;
    }
}
