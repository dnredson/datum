//! `ArtifactExecutionProjectionV1` (Stage 107A, ADR-0011 §1).
//!
//! A node-independent projection of a `ServiceArtifact` covering **only**
//! fields that can alter executed behaviour or a safety decision in the v1
//! operational reconciliation path, per the Stage 107A field audit
//! (`execution-projection-field-audit.txt`) and the design-review correction
//! that followed it. DServer computes this projection as the **authoritative**
//! side (this module); SmartSentinel independently reconstructs the same
//! logical shape from the wire assignment it receives
//! (`DATUM/src/agent/execution_projection.rs`) and recomputes the digest.
//! Equality of the two crates' digests over equivalent content is asserted by
//! shared JCS test vectors and by the cross-crate fixture in
//! `scripts/fixtures/datum-contracts/vs1/`.
//!
//! Explicitly **excluded** (evidence-only or class N; see ADR-0011 §1 and the
//! Stage 107A design-review corrections): `control_plane.version`; `probes`;
//! `health_checks`; `acquisition`; `wasm_decision.module_id`,
//! `.function_name`, `.max_fuel`, `.timeout_ms`; `target_stages`; non-alias
//! metadata; timestamps.
//!
//! **ADR-0006 conformance (v5 review, P0-1).** ADR-0006 (Accepted) governs
//! this type's digest and applies two rules this module must honour
//! exactly: (1) an absent optional projection member serializes as JSON
//! `null`, never omitted -- every `Option` field below is
//! `#[serde(default)]` WITHOUT `skip_serializing_if`, deliberately, because
//! omission and `null` are different JCS inputs and therefore different
//! digests; (2) the digest itself is written as `sha256:<64 lowercase hex>`
//! (`projection_digest` returns this canonical form via
//! `jcs::canonical_digest_sha256_hex`, never the bare-hex
//! `jcs::digest_sha256_hex`). This is distinct from an exact-byte CONTENT
//! hash field on the SAME struct (`ConfigurationMountV1.declared_content_sha256`),
//! which keeps its own field contract's bare-hex representation -- see that
//! field's own doc comment and the v5 review's P2 finding.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::jcs;
use crate::core::service_artifact_registry::ServiceArtifact;

pub const ARTIFACT_EXECUTION_PROJECTION_KIND: &str = "datum.artifact-execution-projection/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactExecutionProjectionV1 {
    pub projection: String,
    pub artifact_id: String,
    /// The artifact's OWN canonical `service_id` (`ServiceArtifact.service_id`)
    /// -- NEVER the desired D-Serv id a service-alias join resolved through
    /// (review finding P0-6). A desired-service alias only decides WHICH
    /// artifact is selected server-side; once selected, both DServer and the
    /// agent must project the SAME identity value regardless of which alias
    /// (if any) was used to reach it, or their digests diverge for a
    /// perfectly valid, untampered assignment. See `compute_projection` and
    /// `ArtifactReconciliationAssignment.artifact_service_id` /
    /// `OperationalArtifactAssignment.artifact_service_id`.
    pub service_id: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub container_name: String,
    pub image_identity: ImageIdentityV1,
    pub command: Vec<String>,
    /// ADR-0006 point 2 (v5 review, P0-1): an absent optional value in a
    /// digest-critical canonical contract must serialize as JSON `null`,
    /// NEVER be omitted -- omission and `null` are different JCS inputs and
    /// therefore different digests, silently, for any consumer who did not
    /// audit this exact serde attribute. `#[serde(default)]` is kept (an
    /// old wire payload missing this field still deserializes as `None`);
    /// `skip_serializing_if` is deliberately NOT used here, unlike an
    /// ordinary API-response optional field.
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub network_aliases: Vec<String>,
    #[serde(default)]
    pub ports: Vec<PortMappingV1>,
    #[serde(default)]
    pub volumes: Vec<VolumeMountV1>,
    #[serde(default)]
    pub configuration_mounts: Vec<ConfigurationMountV1>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub lifecycle: LifecycleV1,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    /// Which secrets are resolved and injected (`resolve_secret_bindings`
    /// iterates exactly this vector). Execution-relevant on its own --
    /// changing this list changes which secrets the agent must resolve, even
    /// when `secret_bindings` (the target-variable-name declarations) is
    /// unaffected (review finding P0-5). Declaration order is preserved, not
    /// sorted: the runtime iterates this vector in order, and proving
    /// iteration order has no behavioural effect was not attempted, so it is
    /// treated as potentially significant per the review's instruction.
    #[serde(default)]
    pub secret_refs: Vec<String>,
    #[serde(default)]
    pub secret_bindings: Vec<SecretBindingV1>,
    pub wasm_safety: WasmSafetyFlagsV1,
}

// Note: `service_aliases` (which desired-service alias resolved to this
// artifact) is deliberately NOT part of the projection. The alias join
// happens entirely on the DServer side, before an `ArtifactReconciliationAssignment`
// exists; by the time the agent has an assignment, the join's effect is
// already captured by `artifact_id` + `service_id` (both projected identity
// fields). Re-deriving the raw alias list on the agent side is not possible
// without extra wire plumbing it adds no safety value over what `artifact_id`
// + `service_id` already provide, so it is left out rather than forcing an
// asymmetric field neither side can equally reconstruct.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageIdentityV1 {
    /// v5 review P0-6: `image_source_kind` is safety-relevant (a
    /// `registry` image may become `required`-complete once resolved;
    /// `local_build`/`legacy_unclassified` never can, regardless of
    /// `resolved`'s content) and therefore MUST participate in the
    /// canonical projection digest / `execution_generation` -- changing
    /// `registry` -> `local_build` (or -> `legacy_unclassified`) with the
    /// same `reference`/`resolved` values changes a safety decision and
    /// must change the digest.
    pub source_kind: String,
    /// The declared image reference string (e.g. `eclipse-mosquitto:2`). A
    /// bare tag alone is NOT a content pin -- see `declared_digest`.
    pub reference: String,
    /// A declared, verifiable digest for the image, when known. Absent means
    /// the image is an unpinned external input under `legacy_partial`;
    /// `required` mode refuses to treat a bare tag as sufficient. ADR-0006
    /// point 2 (v5 review, P0-1): absence serializes as JSON `null`, never
    /// omitted -- see the module-level note on
    /// `ArtifactExecutionProjectionV1.network`.
    #[serde(default)]
    pub declared_digest: Option<String>,
    /// Stage 107B-A (ADR-0013). The server-resolved, immutable, multi-platform
    /// OCI identity, when the image has been explicitly pinned (see
    /// `oci_image_pinning::pin_image_identity`). Absence means the image
    /// remains unpinned -- `required_completeness_gaps` still reports
    /// `unpinned_image_reference` for it. ADR-0006 point 2: absence
    /// serializes as JSON `null`, never omitted. Deliberately carries only
    /// the execution-relevant subset of `ResolvedImageIdentityV1` --
    /// `resolved_at_utc` / `resolver_tool_version` / `registry_host` are
    /// administrative/provenance metadata and are NOT mirrored here, so
    /// re-resolving identical registry content at a different wall-clock
    /// time does not perturb `execution_generation` (Amendment 3).
    #[serde(default)]
    pub resolved: Option<ResolvedImageIdentityProjectionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedImageIdentityProjectionV1 {
    pub repository: String,
    /// v6 review P1-5: `index_digest` is DELIBERATELY EXCLUDED from the
    /// execution projection (unlike the persisted `ResolvedImageIdentityV1`,
    /// which keeps it as provenance). ADR-0013 states binding happens on
    /// the platform-specific `manifest_digest`, never `index_digest` alone
    /// -- an index can change for a reason unrelated to this artifact's own
    /// selected platforms (e.g. a new architecture added elsewhere in the
    /// same index), while every platform actually selected here continues
    /// to reference the exact same manifest/config digests. Including
    /// `index_digest` in the projection would make `execution_generation`
    /// increment on a change that is not, by ADR-0013's own definition,
    /// execution-relevant. The persisted/evidence copy still carries it.
    pub platforms: Vec<ResolvedPlatformManifestProjectionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedPlatformManifestProjectionV1 {
    pub os: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
    /// The binding identity for this platform -- OCI-digest domain, never
    /// the ADR-0006 canonical DATUM contract digest domain (Amendment 3:
    /// bound on the platform-specific manifest digest, never `index_digest`
    /// alone).
    pub manifest_digest: String,
    pub config_digest: String,
    pub execution_reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortMappingV1 {
    pub container_port: u16,
    /// ADR-0006 point 2 (v5 review, P0-1): absence serializes as JSON
    /// `null`, never omitted.
    #[serde(default)]
    pub host_port: Option<u16>,
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VolumeMountV1 {
    pub name: String,
    pub target: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigurationMountV1 {
    pub source: String,
    pub target: String,
    pub read_only: bool,
    /// Exact-byte content hash of the mounted file, in ITS OWN field
    /// contract's representation (bare lowercase hex, no `sha256:` prefix --
    /// this is a content hash, category B in the v5 review's P2 distinction,
    /// NOT the canonical DATUM contract digest ADR-0006 governs). ADR-0006
    /// point 2 still applies to ABSENCE: when no digest is declared, this
    /// serializes as JSON `null`, never omitted.
    #[serde(default)]
    pub declared_content_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleV1 {
    #[serde(default)]
    pub pre_install: Vec<LifecycleCommandV1>,
    #[serde(default)]
    pub install: Vec<LifecycleCommandV1>,
    #[serde(default)]
    pub configure: Vec<LifecycleCommandV1>,
    #[serde(default)]
    pub start: Vec<LifecycleCommandV1>,
    #[serde(default)]
    pub stop: Vec<LifecycleCommandV1>,
    #[serde(default)]
    pub post_install: Vec<LifecycleCommandV1>,
    #[serde(default)]
    pub rollback: Vec<LifecycleCommandV1>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleCommandV1 {
    pub program: String,
    // `args`, `working_dir` and `environment` are DELIBERATELY excluded
    // (review finding P1-6): `execute_lifecycle_phase`
    // (DATUM/src/agent/operational_reconciliation.rs) dispatches solely on
    // `program` via a fixed allowlist match (`verify_docker`, `pull_image`,
    // `verify_configuration_mounts`, `start_container`,
    // `verify_container_running`, `remove_container`) and never reads any of
    // these three fields for any of those branches. Projecting fields the
    // current executor does not consume would make generation/digest
    // semantics depend on content that cannot alter executed behaviour --
    // exactly the class-N principle ADR-0011 already applies to `probes` /
    // `health_checks` / `acquisition`. If a future executor starts consuming
    // these fields, they must be added back here at that time, not before.
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretBindingV1 {
    pub secret_ref: String,
    pub target_environment_variable: String,
    /// ADR-0006 point 2 (v5 review, P0-1): absence serializes as JSON
    /// `null`, never omitted.
    #[serde(default)]
    pub version: Option<String>,
}

/// Only the four consumed / safety-gated `wasm_decision` booleans (Stage
/// 107A design-review finding 4). `module_id`, `function_name`, `max_fuel`
/// and `timeout_ms` are deliberately excluded -- see the module doc comment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmSafetyFlagsV1 {
    pub explicit_invocation_required: bool,
    pub host_imports_allowed: bool,
    pub side_effects_allowed: bool,
    pub safe_to_auto_execute: bool,
}

/// Build the authoritative projection for `artifact`. The projection's
/// `service_id` is always `artifact.service_id` (the artifact's own
/// canonical id) -- there is deliberately no way to pass a different
/// (desired-service / alias) id in here, because that was exactly review
/// finding P0-6: a desired-service alias join must select WHICH artifact is
/// used, never change what identity value gets projected for it. See the
/// module-level note on why the raw `service_aliases` list itself is not a
/// projection field either.
pub fn compute_projection(artifact: &ServiceArtifact) -> Result<ArtifactExecutionProjectionV1, String> {
    let container = artifact
        .runtime
        .container
        .as_ref()
        .ok_or_else(|| format!("artifact '{}' has no container runtime definition", artifact.artifact_id))?;

    let secret_bindings = artifact
        .control_plane
        .secret_bindings
        .iter()
        .map(|binding| SecretBindingV1 {
            secret_ref: binding.secret_ref.clone(),
            target_environment_variable: binding.target_environment_variable.clone(),
            version: binding.version.clone(),
        })
        .collect();

    let wasm = &artifact.control_plane.wasm_decision;

    Ok(ArtifactExecutionProjectionV1 {
        projection: ARTIFACT_EXECUTION_PROJECTION_KIND.to_string(),
        artifact_id: artifact.artifact_id.clone(),
        service_id: artifact.service_id.clone(),
        artifact_kind: artifact.control_plane.artifact_kind.clone(),
        runtime_kind: artifact.runtime.kind.clone(),
        container_name: container.container_name.clone(),
        image_identity: ImageIdentityV1 {
            source_kind: container.image_source_kind.clone(),
            reference: container.image.clone(),
            declared_digest: None,
            resolved: container.resolved_image_identity.as_ref().map(|resolved| {
                ResolvedImageIdentityProjectionV1 {
                    repository: resolved.repository.clone(),
                    platforms: resolved
                        .platforms
                        .iter()
                        .map(|platform| ResolvedPlatformManifestProjectionV1 {
                            os: platform.os.clone(),
                            architecture: platform.architecture.clone(),
                            variant: platform.variant.clone(),
                            manifest_digest: platform.manifest_digest.clone(),
                            config_digest: platform.config_digest.clone(),
                            execution_reference: platform.execution_reference.clone(),
                        })
                        .collect(),
                }
            }),
        },
        command: container.command.clone(),
        network: container.network.clone(),
        network_aliases: container.network_aliases.clone(),
        ports: container
            .ports
            .iter()
            .map(|port| PortMappingV1 {
                container_port: port.container_port,
                host_port: port.host_port,
                protocol: port.protocol.clone(),
            })
            .collect(),
        volumes: container
            .volumes
            .iter()
            .map(|volume| VolumeMountV1 {
                name: volume.name.clone(),
                target: volume.target.clone(),
                read_only: volume.read_only,
            })
            .collect(),
        configuration_mounts: container
            .configuration_mounts
            .iter()
            .map(|mount| ConfigurationMountV1 {
                source: mount.source.clone(),
                target: mount.target.clone(),
                read_only: mount.read_only,
                declared_content_sha256: mount.declared_content_sha256.clone(),
            })
            .collect(),
        dependencies: artifact.control_plane.dependencies.clone(),
        lifecycle: LifecycleV1 {
            pre_install: convert_commands(&artifact.control_plane.lifecycle.pre_install),
            install: convert_commands(&artifact.control_plane.lifecycle.install),
            configure: convert_commands(&artifact.control_plane.lifecycle.configure),
            start: convert_commands(&artifact.control_plane.lifecycle.start),
            stop: convert_commands(&artifact.control_plane.lifecycle.stop),
            post_install: convert_commands(&artifact.control_plane.lifecycle.post_install),
            rollback: convert_commands(&artifact.control_plane.lifecycle.rollback),
        },
        bindings: artifact.control_plane.bindings.clone(),
        secret_refs: artifact.control_plane.secret_refs.clone(),
        secret_bindings,
        wasm_safety: WasmSafetyFlagsV1 {
            explicit_invocation_required: wasm.explicit_invocation_required,
            host_imports_allowed: wasm.host_imports_allowed,
            side_effects_allowed: wasm.side_effects_allowed,
            safe_to_auto_execute: wasm.safe_to_auto_execute,
        },
    })
}

fn convert_commands(
    commands: &[crate::core::service_artifact_registry::ServiceArtifactCommand],
) -> Vec<LifecycleCommandV1> {
    commands
        .iter()
        .map(|command| LifecycleCommandV1 {
            program: command.program.clone(),
        })
        .collect()
}

/// SHA-256 (hex) of the RFC 8785 JCS serialization of `projection`.
pub fn projection_digest(projection: &ArtifactExecutionProjectionV1) -> Result<String, String> {
    let value: Value = serde_json::to_value(projection)
        .map_err(|error| format!("serialize ArtifactExecutionProjectionV1: {error}"))?;
    // ADR-0006 point 3 (v5 review, P0-1): the canonical `sha256:<64
    // lowercase hex>` form, not the bare hex `jcs::digest_sha256_hex`
    // returns.
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

/// Convenience: compute the projection for `artifact` (joined by
/// `artifact.service_id`, i.e. no alias) and its digest in one call, as used
/// by the registry's generation bookkeeping.
pub fn compute_projection_and_digest(artifact: &ServiceArtifact) -> Result<(ArtifactExecutionProjectionV1, String), String> {
    let projection = compute_projection(artifact)?;
    let digest = projection_digest(&projection)?;
    Ok((projection, digest))
}

// ---------------------------------------------------------------------
// Stage 108 (ADR-0015): `NativeProcessExecutionProjectionV1`.
//
// A SEPARATE projection type for `RuntimeKind::NativeProcess`, not a
// `native_process: null` sibling bolted onto `ArtifactExecutionProjectionV1`
// -- ADR-0006 point 2 makes an added, always-null sibling key
// digest-significant, which would perturb EVERY existing container
// artifact's `execution_projection_digest` (and therefore
// `execution_generation`) the moment Stage 108 lands, with no genuine
// content change behind it. `ArtifactExecutionProjectionV1` /
// `compute_projection` / `projection_digest` / `compute_projection_and_digest`
// above are therefore left byte-for-byte untouched by Stage 108.
//
// Carries only the native-process desired-state fields
// (`NativeProcessDeploymentSpecV1`) plus the same common execution-relevant
// fields the container projection already carries (dependencies, bindings,
// secret_refs, secret_bindings, wasm_safety) -- these apply identically
// regardless of runtime kind. Deliberately excludes any observed/runtime
// state (PID, process start time, current state): Stage 108 introduces no
// such type at all (see ADR-0015's desired-vs-observed section), so there is
// nothing to exclude by name yet, but this projection is not the place a
// future `ManagedProcessRuntimeState` would ever be added.
pub const NATIVE_PROCESS_EXECUTION_PROJECTION_KIND: &str = "datum.native-process-execution-projection/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NativeProcessExecutionProjectionV1 {
    pub projection: String,
    pub artifact_id: String,
    pub service_id: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub process_name: String,
    pub entrypoint: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    /// Stage 109 (ADR-0016, Section C). `control_plane.acquisition.sha256` --
    /// the single authoritative executable content hash. A HARD GATE: before
    /// this field existed, authorized bytes could change (a new
    /// `acquisition.sha256`) while the native projection digest stayed
    /// identical, which is forbidden -- content identity is
    /// execution-relevant for a native process exactly as image identity is
    /// for a container. ADR-0006 point 2: absence serializes as JSON `null`,
    /// never omitted (mirrors every other optional projection member in this
    /// file). Never added to `ArtifactExecutionProjectionV1` (Container) --
    /// that projection and its digest remain byte-for-byte unchanged.
    #[serde(default)]
    pub acquisition_sha256: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
    #[serde(default)]
    pub secret_bindings: Vec<SecretBindingV1>,
    pub wasm_safety: WasmSafetyFlagsV1,
}

/// Build the authoritative native-process projection for `artifact`. Mirrors
/// `compute_projection`'s identity rules exactly (canonical `service_id`,
/// never an alias -- P0-6 applies identically here).
pub fn compute_native_projection(artifact: &ServiceArtifact) -> Result<NativeProcessExecutionProjectionV1, String> {
    let native = artifact.runtime.native_process.as_ref().ok_or_else(|| {
        format!("artifact '{}' has no native_process runtime definition", artifact.artifact_id)
    })?;

    let secret_bindings = artifact
        .control_plane
        .secret_bindings
        .iter()
        .map(|binding| SecretBindingV1 {
            secret_ref: binding.secret_ref.clone(),
            target_environment_variable: binding.target_environment_variable.clone(),
            version: binding.version.clone(),
        })
        .collect();

    let wasm = &artifact.control_plane.wasm_decision;

    Ok(NativeProcessExecutionProjectionV1 {
        projection: NATIVE_PROCESS_EXECUTION_PROJECTION_KIND.to_string(),
        artifact_id: artifact.artifact_id.clone(),
        service_id: artifact.service_id.clone(),
        artifact_kind: artifact.control_plane.artifact_kind.clone(),
        runtime_kind: artifact.runtime.kind.clone(),
        process_name: native.process_name.clone(),
        entrypoint: native.entrypoint.clone(),
        args: native.args.clone(),
        working_directory: native.working_directory.clone(),
        acquisition_sha256: artifact.control_plane.acquisition.sha256.clone(),
        dependencies: artifact.control_plane.dependencies.clone(),
        bindings: artifact.control_plane.bindings.clone(),
        secret_refs: artifact.control_plane.secret_refs.clone(),
        secret_bindings,
        wasm_safety: WasmSafetyFlagsV1 {
            explicit_invocation_required: wasm.explicit_invocation_required,
            host_imports_allowed: wasm.host_imports_allowed,
            side_effects_allowed: wasm.side_effects_allowed,
            safe_to_auto_execute: wasm.safe_to_auto_execute,
        },
    })
}

/// SHA-256 (hex) of the RFC 8785 JCS serialization of `projection`.
pub fn native_projection_digest(projection: &NativeProcessExecutionProjectionV1) -> Result<String, String> {
    let value: Value = serde_json::to_value(projection)
        .map_err(|error| format!("serialize NativeProcessExecutionProjectionV1: {error}"))?;
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

pub fn compute_native_projection_and_digest(artifact: &ServiceArtifact) -> Result<(NativeProcessExecutionProjectionV1, String), String> {
    let projection = compute_native_projection(artifact)?;
    let digest = native_projection_digest(&projection)?;
    Ok((projection, digest))
}

/// Stage 108 (ADR-0015) dispatcher: the ONE entry point registry generation
/// bookkeeping should call from now on, so a `NativeProcess` artifact gets a
/// deterministic `execution_projection_digest` from the RIGHT projection
/// shape instead of failing (or worse, being coerced through the
/// container-only `compute_projection`). `Container` behaviour through this
/// dispatcher is byte-for-byte identical to calling
/// `compute_projection_and_digest` directly.
pub fn compute_execution_projection_digest(artifact: &ServiceArtifact) -> Result<String, String> {
    match crate::core::runtime_kind::RuntimeKind::parse(&artifact.runtime.kind) {
        Ok(crate::core::runtime_kind::RuntimeKind::Container) => {
            let (_, digest) = compute_projection_and_digest(artifact)?;
            Ok(digest)
        }
        Ok(crate::core::runtime_kind::RuntimeKind::NativeProcess) => {
            let (_, digest) = compute_native_projection_and_digest(artifact)?;
            Ok(digest)
        }
        Err(error) => Err(format!("artifact '{}' has {error}", artifact.artifact_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::service_artifact_registry::{
        ContainerConfigurationMount, ContainerDeploymentSpec, ContainerPortMapping,
        ContainerVolumeMount, NativeProcessDeploymentSpecV1, ServiceArtifactAcquisition, ServiceArtifactControlPlane,
        ServiceArtifactSecretBinding, ServiceArtifactWasmDecision, ServiceProbeSet,
        ServiceProbeSpec, ServiceRuntimeSpec,
    };
    use std::collections::BTreeMap as Map;

    fn artifact() -> ServiceArtifact {
        ServiceArtifact {
            schema_version: crate::core::service_artifact_registry::SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: "mqtt".to_string(),
            service_id: "mqtt".to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                native_process: None,
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-mqtt".to_string(),
                    image: "eclipse-mosquitto:2".to_string(),
                    ports: vec![ContainerPortMapping {
                        container_port: 1883,
                        host_port: Some(1883),
                        protocol: "tcp".to_string(),
                    }],
                    network: Some("datum-net".to_string()),
                    network_aliases: vec!["mqtt".to_string()],
                    volumes: vec![ContainerVolumeMount {
                        name: "mqtt-data".to_string(),
                        target: "/mosquitto/data".to_string(),
                        read_only: false,
                    }],
                    configuration_mounts: vec![ContainerConfigurationMount {
                        source: "mosquitto/mosquitto.conf".to_string(),
                        target: "/mosquitto/config/mosquitto.conf".to_string(),
                        read_only: true,
                        declared_content_sha256: Some("a".repeat(64)),
                    }],
                    command: vec![],
                    image_source_kind: crate::core::service_artifact_registry::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
                    resolved_image_identity: None,
                }),
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "docker_container_running".to_string(),
                    params: Map::new(),
                    vantage_point: None,
                },
                health: None,
                availability: None,
            },
            metadata: Some(serde_json::json!({"service_aliases": ["mq"]})),
            control_plane: ServiceArtifactControlPlane {
                project_id: "project:test".to_string(),
                name: "MQTT".to_string(),
                version: "2".to_string(),
                artifact_kind: "container".to_string(),
                target_nodes: vec!["fog-01".to_string()],
                target_stages: vec![],
                dependencies: vec![],
                acquisition: ServiceArtifactAcquisition {
                    kind: "image".to_string(),
                    ..Default::default()
                },
                secret_bindings: vec![ServiceArtifactSecretBinding {
                    secret_ref: "mqtt_token".to_string(),
                    target_environment_variable: "MQTT_TOKEN".to_string(),
                    version: Some("v1".to_string()),
                }],
                wasm_decision: ServiceArtifactWasmDecision {
                    module_id: "reconcile_service".to_string(),
                    function_name: "decide".to_string(),
                    max_fuel: Some(1000),
                    timeout_ms: Some(500),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    #[test]
    fn projection_excludes_version_probes_health_acquisition_and_wasm_identity_fields() {
        let projection = compute_projection(&artifact()).unwrap();
        let value = serde_json::to_value(&projection).unwrap();
        let text = serde_json::to_string(&value).unwrap();
        // Note: `secret_bindings[].version` is a legitimate, distinct field
        // (the secret's own version identity) that happens to share the JSON
        // key name "version" -- it is not `control_plane.version`. That
        // exclusion is asserted precisely (by digest equivalence, not
        // substring search) in `version_only_change_does_not_change_digest`.
        assert!(!text.contains("probe"), "probes must not be projected: {text}");
        assert!(!text.contains("health"), "health_checks must not be projected: {text}");
        assert!(!text.contains("acquisition"), "acquisition must not be projected: {text}");
        assert!(!text.contains("module_id"), "wasm module_id must not be projected: {text}");
        assert!(!text.contains("function_name"), "wasm function_name must not be projected: {text}");
        assert!(!text.contains("max_fuel"), "wasm max_fuel must not be projected: {text}");
        assert!(!text.contains("timeout_ms"), "wasm timeout_ms must not be projected: {text}");
    }

    #[test]
    fn projection_includes_ports_aliases_and_secret_bindings() {
        let projection = compute_projection(&artifact()).unwrap();
        assert_eq!(projection.ports.len(), 1);
        assert_eq!(projection.ports[0].container_port, 1883);
        assert_eq!(projection.network_aliases, vec!["mqtt".to_string()]);
        assert_eq!(projection.secret_bindings.len(), 1);
        assert_eq!(projection.secret_bindings[0].target_environment_variable, "MQTT_TOKEN");
        assert_eq!(
            projection.configuration_mounts[0].declared_content_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn digest_is_deterministic_and_content_sensitive() {
        let d1 = projection_digest(&compute_projection(&artifact()).unwrap()).unwrap();
        let d2 = projection_digest(&compute_projection(&artifact()).unwrap()).unwrap();
        assert_eq!(d1, d2);

        let mut changed = artifact();
        changed.runtime.container.as_mut().unwrap().command = vec!["-v".to_string()];
        let d3 = projection_digest(&compute_projection(&changed).unwrap()).unwrap();
        assert_ne!(d1, d3);
    }

    #[test]
    fn version_only_change_does_not_change_digest() {
        let a = artifact();
        let mut b = artifact();
        b.control_plane.version = "3".to_string();
        let da = projection_digest(&compute_projection(&a).unwrap()).unwrap();
        let db = projection_digest(&compute_projection(&b).unwrap()).unwrap();
        assert_eq!(da, db, "a version-only change must be a projection no-op");
    }

    #[test]
    fn wasm_module_id_and_function_name_change_does_not_change_digest() {
        let a = artifact();
        let mut b = artifact();
        b.control_plane.wasm_decision.module_id = "other-module".to_string();
        b.control_plane.wasm_decision.function_name = "other-fn".to_string();
        b.control_plane.wasm_decision.max_fuel = Some(999_999);
        b.control_plane.wasm_decision.timeout_ms = Some(1);
        let da = projection_digest(&compute_projection(&a).unwrap()).unwrap();
        let db = projection_digest(&compute_projection(&b).unwrap()).unwrap();
        assert_eq!(da, db);
    }

    #[test]
    fn wasm_safety_flag_change_does_change_digest() {
        let a = artifact();
        let mut b = artifact();
        b.control_plane.wasm_decision.host_imports_allowed = true;
        let da = projection_digest(&compute_projection(&a).unwrap()).unwrap();
        let db = projection_digest(&compute_projection(&b).unwrap()).unwrap();
        assert_ne!(da, db);
    }

    #[test]
    fn secret_refs_change_does_change_digest() {
        // P0-5: secret_refs determines which secrets the agent resolves and
        // injects (resolve_secret_bindings iterates it); it must not be a
        // projection no-op even when secret_bindings (target var names) is
        // unaffected.
        let mut a = artifact();
        a.control_plane.secret_refs = vec!["A".to_string()];
        let mut b = artifact();
        b.control_plane.secret_refs = vec!["B".to_string()];
        let projection_a = compute_projection(&a).unwrap();
        let projection_b = compute_projection(&b).unwrap();
        assert_eq!(projection_a.secret_refs, vec!["A".to_string()]);
        assert_ne!(
            projection_digest(&projection_a).unwrap(),
            projection_digest(&projection_b).unwrap()
        );
    }

    /// Review finding P1-5 (v4 review): executes the REAL
    /// `compute_projection` against a `ServiceArtifact` from a fixture ALSO
    /// consumed by DATUM's own test (`execution_projection::tests::dserver_cross_crate_fixture_produces_the_expected_projection_and_digest`)
    /// against its OWN real mapper over a wire `OperationalArtifactAssignment`
    /// for the SAME logical artifact -- including the alias case (the
    /// assignment's `service_id` differs from `artifact_service_id`). Proves
    /// digest equivalence by executing BOTH real mapper functions against
    /// independently-authored inputs, not by comparing key-name sets
    /// (`dserver_shape_field_names`-style tests on the DATUM side check
    /// shape only; this checks actual output equality).
    #[test]
    fn cross_crate_fixture_produces_the_expected_projection_and_digest() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("vs1")
            .join("execution-projection.cross-crate-mapper.mqtt-broker.json");
        let text = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        let fixture: Value = serde_json::from_str(&text).unwrap();

        let artifact: ServiceArtifact = serde_json::from_value(fixture["dserver_service_artifact"].clone())
            .expect("fixture dserver_service_artifact must deserialize as ServiceArtifact");
        let projection = compute_projection(&artifact).expect("compute_projection must succeed on the fixture artifact");

        let expected_projection: ArtifactExecutionProjectionV1 =
            serde_json::from_value(fixture["expected_projection"].clone())
                .expect("fixture expected_projection must deserialize as ArtifactExecutionProjectionV1");
        assert_eq!(projection, expected_projection, "real compute_projection output must equal the fixture's expected_projection");

        let serialized = serde_json::to_value(&projection).unwrap();
        assert_eq!(serialized, fixture["expected_projection"], "serialized optional nulls must agree");
        assert_eq!(jcs::canonicalize(&serialized).unwrap(), fixture["expected_canonical_json"].as_str().unwrap());
        let expected_digest = fixture["expected_projection_digest_sha256"].as_str().unwrap();
        assert_eq!(projection_digest(&projection).unwrap(), expected_digest);

        // The artifact's OWN canonical service_id, never the alias
        // (`x_alias_case` in the fixture) -- P0-6.
        assert_eq!(projection.service_id, "mqtt-broker");
    }

    /// Stage 109 (ADR-0016, Section C): `acquisition.sha256` must be bound
    /// into the native projection -- a hard gate, since content identity is
    /// execution-relevant exactly like a container's image identity.
    #[test]
    fn native_projection_binds_the_acquisition_sha256() {
        let mut artifact = artifact();
        artifact.runtime.kind = crate::core::runtime_kind::RUNTIME_KIND_NATIVE_PROCESS.to_string();
        artifact.runtime.container = None;
        artifact.runtime.native_process = Some(NativeProcessDeploymentSpecV1 {
            process_name: "datum-svc".to_string(),
            entrypoint: "/opt/datum/svc/bin/run".to_string(),
            args: Vec::new(),
            working_directory: None,
        });
        artifact.control_plane.acquisition.sha256 = Some("d".repeat(64));
        let projection = compute_native_projection(&artifact).unwrap();
        assert_eq!(projection.acquisition_sha256.as_deref(), Some("d".repeat(64).as_str()));
    }

    /// Stage 108 (ADR-0015). The `NativeProcessExecutionProjectionV1` analogue
    /// of `cross_crate_fixture_produces_the_expected_projection_and_digest`
    /// above -- same pattern, same fixture-sharing discipline, proving
    /// cross-crate parity for `RuntimeKind::NativeProcess`, not merely
    /// `Container`.
    #[test]
    fn native_process_cross_crate_fixture_produces_the_expected_projection_and_digest() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("vs1")
            .join("execution-projection.cross-crate-mapper.native-process-stub.json");
        let text = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        let fixture: Value = serde_json::from_str(&text).unwrap();

        let artifact: ServiceArtifact = serde_json::from_value(fixture["dserver_service_artifact"].clone())
            .expect("fixture dserver_service_artifact must deserialize as ServiceArtifact");
        let projection = compute_native_projection(&artifact)
            .expect("compute_native_projection must succeed on the fixture artifact");

        let expected_projection: NativeProcessExecutionProjectionV1 =
            serde_json::from_value(fixture["expected_projection"].clone())
                .expect("fixture expected_projection must deserialize as NativeProcessExecutionProjectionV1");
        assert_eq!(projection, expected_projection, "real compute_native_projection output must equal the fixture's expected_projection");

        let serialized = serde_json::to_value(&projection).unwrap();
        assert_eq!(serialized, fixture["expected_projection"], "serialized optional nulls must agree");
        assert_eq!(jcs::canonicalize(&serialized).unwrap(), fixture["expected_canonical_json"].as_str().unwrap());
        let expected_digest = fixture["expected_projection_digest_sha256"].as_str().unwrap();
        assert_eq!(native_projection_digest(&projection).unwrap(), expected_digest);

        // The artifact's OWN canonical service_id, never the alias -- P0-6.
        assert_eq!(projection.service_id, "native-svc");
    }

    // ---- v5 review P0-1: ADR-0006 null-encoding + canonical digest ------ //

    /// An artifact with EVERY digest-critical optional left `None` (or,
    /// where the field is a `Vec`, containing an entry whose own optional is
    /// `None`) -- the exact shape that previously vanished from the
    /// serialized projection instead of appearing as JSON `null`.
    fn artifact_with_absent_optionals() -> ServiceArtifact {
        let mut a = artifact();
        let container = a.runtime.container.as_mut().unwrap();
        container.network = None;
        container.ports[0].host_port = None;
        container.configuration_mounts[0].declared_content_sha256 = None;
        a.control_plane.secret_bindings[0].version = None;
        a
    }

    #[test]
    fn every_optional_projection_member_serializes_as_explicit_null_when_absent() {
        let projection = compute_projection(&artifact_with_absent_optionals()).unwrap();
        let value = serde_json::to_value(&projection).unwrap();
        let obj = value.as_object().unwrap();

        // Top-level `network`.
        assert!(obj.contains_key("network"), "network must be present in the serialized object");
        assert!(obj["network"].is_null(), "network must serialize as null, not be omitted: {value}");

        // `image_identity.declared_digest` -- always None today.
        let image_identity = obj["image_identity"].as_object().unwrap();
        assert!(image_identity.contains_key("declared_digest"));
        assert!(image_identity["declared_digest"].is_null(), "{value}");

        // `image_identity.resolved` (Stage 107B-A) -- None until the image
        // is explicitly pinned.
        assert!(image_identity.contains_key("resolved"));
        assert!(image_identity["resolved"].is_null(), "{value}");

        // `image_identity.source_kind` (v5 review P0-6) -- mandatory, never
        // absent or null; present as a plain string alongside the two
        // Option fields above.
        assert!(image_identity.contains_key("source_kind"));
        assert!(image_identity["source_kind"].is_string(), "{value}");

        // `ports[0].host_port`.
        let ports = obj["ports"].as_array().unwrap();
        let port0 = ports[0].as_object().unwrap();
        assert!(port0.contains_key("host_port"));
        assert!(port0["host_port"].is_null(), "{value}");

        // `configuration_mounts[0].declared_content_sha256`.
        let mounts = obj["configuration_mounts"].as_array().unwrap();
        let mount0 = mounts[0].as_object().unwrap();
        assert!(mount0.contains_key("declared_content_sha256"));
        assert!(mount0["declared_content_sha256"].is_null(), "{value}");

        // `secret_bindings[0].version`.
        let secret_bindings = obj["secret_bindings"].as_array().unwrap();
        let binding0 = secret_bindings[0].as_object().unwrap();
        assert!(binding0.contains_key("version"));
        assert!(binding0["version"].is_null(), "{value}");
    }

    #[test]
    fn two_logical_projections_with_absent_optionals_canonicalize_identically() {
        // Same logical content (every digest-critical optional absent),
        // built via two independent calls -- proves the null encoding is
        // deterministic, not merely present-by-accident on one run.
        let p1 = compute_projection(&artifact_with_absent_optionals()).unwrap();
        let p2 = compute_projection(&artifact_with_absent_optionals()).unwrap();
        let v1 = serde_json::to_value(&p1).unwrap();
        let v2 = serde_json::to_value(&p2).unwrap();
        assert_eq!(
            jcs::canonicalize(&v1).unwrap(),
            jcs::canonicalize(&v2).unwrap(),
            "two logically identical projections with absent optionals must canonicalize to byte-identical JCS text"
        );
        assert_eq!(projection_digest(&p1).unwrap(), projection_digest(&p2).unwrap());
    }

    #[test]
    fn projection_digest_has_the_canonical_sha256_prefixed_format() {
        let digest = projection_digest(&compute_projection(&artifact()).unwrap()).unwrap();
        assert!(digest.starts_with("sha256:"), "{digest}");
        assert!(jcs::is_canonical_digest_format(&digest), "{digest}");
    }

    #[test]
    fn bare_hex_projection_digest_is_never_emitted_as_a_canonical_contract_digest() {
        let projection = compute_projection(&artifact()).unwrap();
        let value = serde_json::to_value(&projection).unwrap();
        let bare = jcs::digest_sha256_hex(&value).unwrap();
        let canonical = projection_digest(&projection).unwrap();
        assert_ne!(canonical, bare, "projection_digest must never equal the unprefixed bare-hex digest");
        assert!(!jcs::is_canonical_digest_format(&bare), "a bare-hex string alone must not pass the canonical-digest-format check");
        assert_eq!(canonical, format!("sha256:{bare}"));
    }

    /// Stage 107B-A (ADR-0013), test #7 (design plan): proves a resolved
    /// multi-platform OCI identity projects identically through both crates'
    /// REAL `compute_projection`, mirroring
    /// `cross_crate_fixture_produces_the_expected_projection_and_digest`'s
    /// shape but against a fixture with a POPULATED `resolved_image_identity`.
    #[test]
    fn oci_resolved_cross_crate_fixture_produces_the_expected_projection_and_digest() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("scripts")
            .join("fixtures")
            .join("datum-contracts")
            .join("vs1")
            .join("execution-projection.cross-crate-mapper.oci-resolved.json");
        let text = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", fixture_path.display()));
        let fixture: Value = serde_json::from_str(&text).unwrap();

        let artifact: ServiceArtifact = serde_json::from_value(fixture["dserver_service_artifact"].clone())
            .expect("fixture dserver_service_artifact must deserialize as ServiceArtifact");
        let (projection, digest) = compute_projection_and_digest(&artifact).unwrap();

        let expected_projection: ArtifactExecutionProjectionV1 =
            serde_json::from_value(fixture["expected_projection"].clone())
                .expect("fixture expected_projection must deserialize as ArtifactExecutionProjectionV1");
        assert_eq!(projection, expected_projection);
        assert!(
            projection.image_identity.resolved.is_some(),
            "this fixture must actually exercise a populated resolved_image_identity"
        );

        let serialized = serde_json::to_value(&projection).unwrap();
        assert_eq!(serialized, fixture["expected_projection"], "serialized optional nulls must agree");
        assert_eq!(jcs::canonicalize(&serialized).unwrap(), fixture["expected_canonical_json"].as_str().unwrap());
        let expected_digest = fixture["expected_projection_digest_sha256"].as_str().unwrap();
        assert_eq!(digest, expected_digest);
        assert!(jcs::is_canonical_digest_format(&digest));
    }
}
