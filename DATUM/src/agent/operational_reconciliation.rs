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
use crate::agent::execution_projection;
use crate::agent::resource_discovery::SmartSentinelResourceDiscovery;
use crate::agent::stage107a_enforcement::{
    check_generation_and_projection, effective_pinning_mode, required_completeness_gaps,
    resolve_clock_skew_seconds, resolve_network_aliases, resolve_ports,
    resolve_secret_target_environment_variable, LeaseGuard, BLOCKED_REQUIRED_INCOMPLETE,
    EXECUTION_PINNING_MODE_REQUIRED,
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
}

/// One M1-M8 governed-host-mutation boundary attempt (review finding P1-2,
/// v4). `boundary` is the stable M1-M8 identifier (`"M1"` docker run,
/// `"M2"` docker start, `"M3"` attachment-drift recreate `rm -f`, `"M4"`
/// stopped+secrets recreate `rm -f`, `"M5"` docker pull, `"M6"` lifecycle
/// remove_container, `"M7"` docker network create, `"M8"` rollback `rm -f`).
/// `result` is `"passed"` or `"blocked"`; `block_reason` is the full
/// `stage107a_enforcement` block-reason string (present only when
/// `result == "blocked"`). Never carries a secret value.
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
) -> bool {
    stage107a_service_block.is_none() && config_content_block.is_none()
}

/// Per-cycle authorization identity/lease context threaded alongside
/// `LeaseGuard` into every M1-M8 boundary purely so `MutationBoundaryRecord`s
/// (review finding P1-2, v4) can carry `authorization_id` / `expires_at_utc`
/// / `configured_clock_skew_seconds` -- the low-level Docker-mutation
/// functions (`execute_lifecycle_phase`, `execute_trusted_container_intent`,
/// `ensure_docker_network`, `rollback_trusted_container_intent`) otherwise
/// know nothing about `OperationalReconciliationAuthorization` itself, only
/// about the `LeaseGuard` derived from it.
struct MutationBoundaryContext<'a> {
    authorization_id: Option<&'a str>,
    expires_at_utc: Option<&'a str>,
    configured_clock_skew_seconds: Option<u64>,
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
fn check_boundary(
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
                !container_attachments_match(
                    &mapping.container_name,
                    &artifact.image,
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
                .unwrap_or(false)
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

        let stage107a_service_block: Option<String> = if execute_requested && control_plane_authorized {
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
        }.or(attachment_policy_block);

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
        let authorization_expires_at_utc = context
            .authorization
            .as_ref()
            .and_then(|authorization| authorization.expires_at_utc.clone());
        let local_policy_digest = compute_local_policy_digest(cfg);
        // Review finding P1-1/P1-2: the clock-skew seconds actually enforced
        // this cycle, and the per-M1-M8-boundary evidence context/collector
        // derived from it. `mutation_boundaries` accumulates whether the
        // service ultimately proceeds or blocks (mutated in place through
        // `&mut`, so an early `?` inside an M1-M8 executor never discards
        // what was already recorded).
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
                } else if let Some(reason) = stage107a_service_block.as_deref() {
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
        if !service.runtime.eq_ignore_ascii_case("container") {
            bail!("unsupported operational runtime '{}': expected container", service.runtime);
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
            || artifact.container_name.trim().is_empty()
            || artifact.image.trim().is_empty()
            || artifact.target_node != context.node_id
            || artifact.runtime_kind != "container"
            || artifact.command.len() > 64
            || artifact.command.iter().any(|arg| arg.len() > 4096 || arg.contains('\0'))
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
            || !policy.explicit_invocation_required
            || policy.host_imports_allowed
            || policy.side_effects_allowed
            || policy.safe_to_auto_execute
            || policy.module_id.trim().is_empty()
            || policy.function_name.trim().is_empty()
        {
            bail!("artifact reconciliation context contains an unsafe or incomplete assignment");
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

/// `phase_boundaries` names the M1-M8 mutation boundary each mutating
/// command in this phase corresponds to, so the lease is re-checked
/// immediately before that specific operation (ADR-0012 point 9): `pull_image`'s
/// actual `docker pull` is M5 (the preceding `docker image inspect` is
/// read-only and not gated); `remove_container` is M6.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn execute_lifecycle_phase(
    phase: &str,
    commands: &[OperationalLifecycleCommand],
    container_name: &str,
    image: &str,
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
) -> Result<(String, String, bool)> {
    let container_name = mapping.container_name.trim();
    let image = mapping
        .artifact
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("trusted mapping requires non-empty artifact image");
    let resolved_configuration_mounts =
        resolve_configuration_mounts(configuration_mounts, configuration_root)?;

    execute_lifecycle_phase("pre_install", &lifecycle.pre_install, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;
    execute_lifecycle_phase("install", &lifecycle.install, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;
    execute_lifecycle_phase("configure", &lifecycle.configure, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;
    execute_lifecycle_phase("start", &lifecycle.start, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;

    let attachments_mismatch = match observed_before {
        ContainerState::Running | ContainerState::Stopped => {
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
            )?
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
        );
    }

    match observed_before {
        ContainerState::Running => {
            execute_lifecycle_phase("post_install", &lifecycle.post_install, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;
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
            execute_lifecycle_phase("post_install", &lifecycle.post_install, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;
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
            let mut args = vec![
                "run".to_string(),
                "-d".to_string(),
                "--name".to_string(),
                container_name.to_string(),
            ];
            if let Some(network) = network {
                ensure_docker_network(network, lease, boundary_context, records)?;
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
            for mount in &resolved_configuration_mounts {
                args.push("--mount".to_string());
                args.push(format!(
                    "type=bind,source={},target={},readonly",
                    mount.0, mount.1
                ));
            }
            for port in effective_ports {
                let host_port = port.host_port.unwrap_or(port.container_port);
                args.push("-p".to_string());
                args.push(format!(
                    "{}:{}{}",
                    host_port,
                    port.container_port,
                    if port.protocol.trim().is_empty() {
                        String::new()
                    } else {
                        format!("/{}", port.protocol.trim())
                    }
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
            // Review finding P1-3: label the container with the exact
            // artifact identity it was created from, so convergence can
            // later detect drift a field-by-field inspect comparison alone
            // cannot (including a value that was REMOVED, not merely one
            // that mismatches).
            args.extend(execution_identity.as_docker_run_args());
            args.push(image.to_string());
            args.extend(container_command.iter().cloned());
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
            execute_lifecycle_phase("post_install", &lifecycle.post_install, container_name, image, configuration_mounts, configuration_root, lease, boundary_context, records)?;
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
        assert!(!action_code_zero_may_report_converged(Some(&reason), None));
    }

    #[test]
    fn action_code_zero_may_not_report_converged_when_resource_identity_is_wrong() {
        let reason = format!(
            "{}: authorization resource_id does not match this node's resolved resource_id",
            crate::agent::stage107a_enforcement::BLOCKED_WRONG_RESOURCE_IDENTITY
        );
        assert!(!action_code_zero_may_report_converged(Some(&reason), None));
    }

    #[test]
    fn action_code_zero_may_not_report_converged_when_configuration_content_mismatches() {
        let reason = format!(
            "{}: declared configuration mount content digest does not match the resolved bytes",
            crate::agent::stage107a_enforcement::BLOCKED_CONFIG_CONTENT_MISMATCH
        );
        assert!(!action_code_zero_may_report_converged(None, Some(&reason)));
    }

    #[test]
    fn action_code_zero_may_not_report_converged_when_required_mode_is_incomplete() {
        let reason = format!(
            "{}: service is not required-complete",
            crate::agent::stage107a_enforcement::BLOCKED_REQUIRED_INCOMPLETE
        );
        assert!(!action_code_zero_may_report_converged(Some(&reason), None));
    }

    #[test]
    fn action_code_zero_may_report_converged_when_no_stage107a_block_is_present() {
        assert!(action_code_zero_may_report_converged(None, None));
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
        assert!(action_code_zero_may_report_converged(None, None));
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

}
