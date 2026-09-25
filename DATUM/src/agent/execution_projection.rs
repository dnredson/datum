//! `ArtifactExecutionProjectionV1` (Stage 107A, ADR-0011 §1) -- SmartSentinel
//! side.
//!
//! Independently reconstructs the same logical projection DServer computes
//! authoritatively (`dserver/src/core/execution_projection.rs`), from the
//! `OperationalArtifactAssignment` this agent actually receives over the
//! wire, and recomputes its RFC 8785 JCS + SHA-256 digest. The reconstruction
//! is "independent" in that this crate never imports DServer's types or
//! functions (separate Cargo workspaces, by design -- ADR-0006/ADR-0011);
//! what proves the two sides agree is field-shape parity (asserted by the
//! `dserver_shape_field_names` test below, which enumerates the exact set)
//! plus the shared JCS conformance vectors in `jcs.rs`.
//!
//! Excluded from the projection, per ADR-0011 §1 and the Stage 107A
//! design-review correction: `version` (evidence only); `probes`;
//! `health_checks`; `acquisition`; `wasm_decision.module_id`,
//! `.function_name`, `.max_fuel`, `.timeout_ms` (not consumed for behaviour
//! selection by `run_embedded_reconciler`, and the latter two are not even
//! present on `ArtifactWasmDecisionPolicy`). `service_aliases` is likewise
//! excluded on both sides -- see `dserver/src/core/execution_projection.rs`'s
//! module note.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::jcs;
use crate::agent::operational_reconciliation::{
    ArtifactWasmDecisionPolicy, OperationalArtifactAssignment,
};

pub const ARTIFACT_EXECUTION_PROJECTION_KIND: &str = "datum.artifact-execution-projection/1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactExecutionProjectionV1 {
    pub projection: String,
    pub artifact_id: String,
    pub service_id: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub container_name: String,
    pub image_identity: ImageIdentityV1,
    pub command: Vec<String>,
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
    /// Mirrors `dserver::core::execution_projection::ArtifactExecutionProjectionV1::secret_refs`
    /// (review finding P0-5). Order preserved, not sorted.
    #[serde(default)]
    pub secret_refs: Vec<String>,
    #[serde(default)]
    pub secret_bindings: Vec<SecretBindingV1>,
    pub wasm_safety: WasmSafetyFlagsV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImageIdentityV1 {
    /// v5 review P0-6: safety-relevant (mirrors dserver's field-for-field)
    /// -- must participate in the digest/generation.
    pub source_kind: String,
    pub reference: String,
    #[serde(default)]
    pub declared_digest: Option<String>,
    /// Stage 107B-A (ADR-0013). Mirrors `dserver`'s `ImageIdentityV1.resolved`
    /// field-for-field. Absence serializes as JSON `null`, never omitted
    /// (ADR-0006 point 2). Deliberately carries only the execution-relevant
    /// subset -- administrative/provenance fields on
    /// `OperationalResolvedImageIdentity` (`resolved_at_utc`,
    /// `resolver_tool_version`, `registry_host`) are NOT mirrored here.
    #[serde(default)]
    pub resolved: Option<ResolvedImageIdentityProjectionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedImageIdentityProjectionV1 {
    pub repository: String,
    /// v6 review P1-5: deliberately excluded from the projection -- mirrors
    /// dserver's field-for-field. See that crate's own doc comment on this
    /// field.
    pub platforms: Vec<ResolvedPlatformManifestProjectionV1>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedPlatformManifestProjectionV1 {
    pub os: String,
    pub architecture: String,
    #[serde(default)]
    pub variant: Option<String>,
    pub manifest_digest: String,
    pub config_digest: String,
    pub execution_reference: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PortMappingV1 {
    pub container_port: u16,
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
    // Mirrors dserver::core::execution_projection::LifecycleCommandV1 (review
    // finding P1-6): `args`/`working_dir`/`environment` excluded because
    // `execute_lifecycle_phase` never reads them.
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretBindingV1 {
    pub secret_ref: String,
    pub target_environment_variable: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WasmSafetyFlagsV1 {
    pub explicit_invocation_required: bool,
    pub host_imports_allowed: bool,
    pub side_effects_allowed: bool,
    pub safe_to_auto_execute: bool,
}

/// Reconstruct the projection from the assignment this agent received --
/// never from a locally re-read mutable pointer (ADR-0012 point 8: "no
/// silent re-resolution").
pub fn compute_projection(assignment: &OperationalArtifactAssignment) -> ArtifactExecutionProjectionV1 {
    let wasm: &ArtifactWasmDecisionPolicy = &assignment.wasm_decision;
    ArtifactExecutionProjectionV1 {
        projection: ARTIFACT_EXECUTION_PROJECTION_KIND.to_string(),
        artifact_id: assignment.artifact_id.clone(),
        // P0-6: the projection's identity is the artifact's OWN canonical
        // service_id, never the (possibly alias) desired service_id.
        service_id: assignment.artifact_service_id.clone(),
        artifact_kind: assignment.artifact_kind.clone(),
        runtime_kind: assignment.runtime_kind.clone(),
        container_name: assignment.container_name.clone(),
        image_identity: ImageIdentityV1 {
            source_kind: assignment.image_source_kind.clone(),
            reference: assignment.image.clone(),
            declared_digest: None,
            resolved: assignment.resolved_image_identity.as_ref().map(|resolved| {
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
        command: assignment.command.clone(),
        network: assignment.network.clone(),
        network_aliases: assignment.network_aliases.clone(),
        ports: assignment
            .ports
            .iter()
            .map(|port| PortMappingV1 {
                container_port: port.container_port,
                host_port: port.host_port,
                protocol: port.protocol.clone(),
            })
            .collect(),
        volumes: assignment
            .volumes
            .iter()
            .map(|volume| VolumeMountV1 {
                name: volume.name.clone(),
                target: volume.target.clone(),
                read_only: volume.read_only,
            })
            .collect(),
        configuration_mounts: assignment
            .configuration_mounts
            .iter()
            .map(|mount| ConfigurationMountV1 {
                source: mount.source.clone(),
                target: mount.target.clone(),
                read_only: mount.read_only,
                declared_content_sha256: mount.declared_content_sha256.clone(),
            })
            .collect(),
        dependencies: assignment.dependencies.clone(),
        lifecycle: LifecycleV1 {
            pre_install: convert_commands(&assignment.lifecycle.pre_install),
            install: convert_commands(&assignment.lifecycle.install),
            configure: convert_commands(&assignment.lifecycle.configure),
            start: convert_commands(&assignment.lifecycle.start),
            stop: convert_commands(&assignment.lifecycle.stop),
            post_install: convert_commands(&assignment.lifecycle.post_install),
            rollback: convert_commands(&assignment.lifecycle.rollback),
        },
        bindings: assignment.bindings.clone(),
        secret_refs: assignment.secret_refs.clone(),
        secret_bindings: assignment
            .secret_bindings
            .iter()
            .map(|binding| SecretBindingV1 {
                secret_ref: binding.secret_ref.clone(),
                target_environment_variable: binding.target_environment_variable.clone(),
                version: binding.version.clone(),
            })
            .collect(),
        wasm_safety: WasmSafetyFlagsV1 {
            explicit_invocation_required: wasm.explicit_invocation_required,
            host_imports_allowed: wasm.host_imports_allowed,
            side_effects_allowed: wasm.side_effects_allowed,
            safe_to_auto_execute: wasm.safe_to_auto_execute,
        },
    }
}

fn convert_commands(
    commands: &[crate::agent::operational_reconciliation::OperationalLifecycleCommand],
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
    jcs::canonical_digest_sha256_hex(&value).map_err(|error| error.to_string())
}

/// Recompute the digest directly from the delivered assignment (the common
/// call site: gate checks compare this against `assignment.execution_projection_digest`).
pub fn recompute_digest(assignment: &OperationalArtifactAssignment) -> Result<String, String> {
    projection_digest(&compute_projection(assignment))
}

// ---------------------------------------------------------------------
// Stage 108 (ADR-0015): `NativeProcessExecutionProjectionV1` -- agent side.
//
// Mirrors `dserver::core::execution_projection::NativeProcessExecutionProjectionV1`
// field-for-field. A SEPARATE projection type from `ArtifactExecutionProjectionV1`
// above, not a `native_process: null` sibling on it -- see that crate's own
// module note on why an added, always-null sibling key would perturb every
// existing container artifact's digest for no genuine content change.
// `ArtifactExecutionProjectionV1`/`compute_projection`/`projection_digest`/
// `recompute_digest` above are therefore left byte-for-byte untouched by
// Stage 108.
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
    /// Stage 109 (ADR-0016, Section C). Mirrors dserver's
    /// `acquisition_sha256` field-for-field -- the single authoritative
    /// executable content hash, execution-relevant exactly like a
    /// container's image identity. ADR-0006 point 2: absence serializes as
    /// JSON `null`, never omitted.
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

/// Reconstruct the native-process projection from the delivered assignment.
/// Mirrors `compute_projection`'s identity rules (canonical `service_id`,
/// never an alias -- P0-6 applies identically here).
pub fn compute_native_projection(assignment: &OperationalArtifactAssignment) -> Result<NativeProcessExecutionProjectionV1, String> {
    let native = assignment
        .native_process
        .as_ref()
        .ok_or_else(|| format!("assignment '{}' has no native_process payload", assignment.artifact_id))?;
    let wasm: &ArtifactWasmDecisionPolicy = &assignment.wasm_decision;

    Ok(NativeProcessExecutionProjectionV1 {
        projection: NATIVE_PROCESS_EXECUTION_PROJECTION_KIND.to_string(),
        artifact_id: assignment.artifact_id.clone(),
        service_id: assignment.artifact_service_id.clone(),
        artifact_kind: assignment.artifact_kind.clone(),
        runtime_kind: assignment.runtime_kind.clone(),
        process_name: native.process_name.clone(),
        entrypoint: native.entrypoint.clone(),
        args: native.args.clone(),
        working_directory: native.working_directory.clone(),
        acquisition_sha256: assignment.acquisition.as_ref().and_then(|a| a.sha256.clone()),
        dependencies: assignment.dependencies.clone(),
        bindings: assignment.bindings.clone(),
        secret_refs: assignment.secret_refs.clone(),
        secret_bindings: assignment
            .secret_bindings
            .iter()
            .map(|binding| SecretBindingV1 {
                secret_ref: binding.secret_ref.clone(),
                target_environment_variable: binding.target_environment_variable.clone(),
                version: binding.version.clone(),
            })
            .collect(),
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

/// Recompute the native-process projection digest directly from the
/// delivered assignment.
pub fn recompute_native_digest(assignment: &OperationalArtifactAssignment) -> Result<String, String> {
    native_projection_digest(&compute_native_projection(assignment)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::operational_reconciliation::{
        ArtifactWasmDecisionPolicy, NativeProcessDeploymentSpecV1, OperationalConfigurationMount,
        OperationalLifecycle, OperationalPortMapping, OperationalSecretBinding, OperationalVolumeMount,
    };

    // ---- Stage 108 (ADR-0015): native-process projection tests ---------- //

    fn native_assignment() -> OperationalArtifactAssignment {
        OperationalArtifactAssignment {
            artifact_id: "native-svc".to_string(),
            service_id: "native-svc".to_string(),
            artifact_service_id: "native-svc".to_string(),
            version: "1".to_string(),
            artifact_kind: "command_bundle".to_string(),
            runtime_kind: "native_process".to_string(),
            container_name: String::new(),
            image: String::new(),
            native_process: Some(NativeProcessDeploymentSpecV1 {
                process_name: "datum-native-svc".to_string(),
                entrypoint: "/opt/datum/native-svc/bin/run".to_string(),
                args: vec!["--config".to_string(), "cfg.toml".to_string()],
                working_directory: Some("/opt/datum/native-svc".to_string()),
            }),
            acquisition: Some(crate::agent::operational_reconciliation::OperationalServiceArtifactAcquisition {
                kind: "native_process_executable".to_string(),
                source: Some("/opt/datum/sources/native-svc.bin".to_string()),
                revision: None,
                sha256: Some("b".repeat(64)),
            }),
            image_source_kind: crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
            resolved_image_identity: None,
            target_node: "fog-01".to_string(),
            lifecycle: OperationalLifecycle::default(),
            network: None,
            network_aliases: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            configuration_mounts: Vec::new(),
            command: Vec::new(),
            dependencies: vec!["postgres".to_string()],
            wasm_decision: ArtifactWasmDecisionPolicy {
                explicit_invocation_required: true,
                module_id: "reconcile_service".to_string(),
                function_name: "decide".to_string(),
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            bindings: BTreeMap::new(),
            secret_refs: Vec::new(),
            secret_bindings: Vec::new(),
            execution_generation: 1,
            execution_projection_digest: String::new(),
            application_id: String::new(),
            dmap_ref: None,
        }
    }

    #[test]
    fn native_projection_round_trips_the_desired_fields() {
        let projection = compute_native_projection(&native_assignment()).unwrap();
        assert_eq!(projection.process_name, "datum-native-svc");
        assert_eq!(projection.entrypoint, "/opt/datum/native-svc/bin/run");
        assert_eq!(projection.args, vec!["--config".to_string(), "cfg.toml".to_string()]);
        assert_eq!(projection.working_directory.as_deref(), Some("/opt/datum/native-svc"));
        assert_eq!(projection.runtime_kind, "native_process");
    }

    #[test]
    fn native_projection_fails_without_a_native_process_payload() {
        let mut assignment = native_assignment();
        assignment.native_process = None;
        assert!(compute_native_projection(&assignment).is_err());
    }

    #[test]
    fn native_projection_digest_is_stable_across_recomputation() {
        let d1 = recompute_native_digest(&native_assignment()).unwrap();
        let d2 = recompute_native_digest(&native_assignment()).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn native_projection_digest_changes_when_process_name_changes() {
        let a = native_assignment();
        let mut b = native_assignment();
        b.native_process.as_mut().unwrap().process_name = "different-name".to_string();
        assert_ne!(recompute_native_digest(&a).unwrap(), recompute_native_digest(&b).unwrap());
    }

    #[test]
    fn native_projection_digest_changes_when_entrypoint_changes() {
        let a = native_assignment();
        let mut b = native_assignment();
        b.native_process.as_mut().unwrap().entrypoint = "/opt/datum/native-svc/bin/run-v2".to_string();
        assert_ne!(recompute_native_digest(&a).unwrap(), recompute_native_digest(&b).unwrap());
    }

    #[test]
    fn native_projection_digest_changes_when_args_change() {
        let a = native_assignment();
        let mut b = native_assignment();
        b.native_process.as_mut().unwrap().args = vec!["--config".to_string(), "different.toml".to_string()];
        assert_ne!(recompute_native_digest(&a).unwrap(), recompute_native_digest(&b).unwrap());
    }

    #[test]
    fn native_projection_digest_changes_when_working_directory_changes() {
        let a = native_assignment();
        let mut b = native_assignment();
        b.native_process.as_mut().unwrap().working_directory = Some("/opt/datum/native-svc-v2".to_string());
        assert_ne!(recompute_native_digest(&a).unwrap(), recompute_native_digest(&b).unwrap());
    }

    #[test]
    fn native_projection_digest_changes_when_acquisition_sha256_changes() {
        // Stage 109 (ADR-0016, Section C): a SHA-only change (bytes changed,
        // nothing else) must change the digest -- content identity is
        // execution-relevant.
        let a = native_assignment();
        let mut b = native_assignment();
        b.acquisition.as_mut().unwrap().sha256 = Some("c".repeat(64));
        assert_ne!(recompute_native_digest(&a).unwrap(), recompute_native_digest(&b).unwrap());
    }

    #[test]
    fn native_projection_binds_the_acquisition_sha256() {
        let projection = compute_native_projection(&native_assignment()).unwrap();
        assert_eq!(projection.acquisition_sha256.as_deref(), Some("b".repeat(64).as_str()));
    }

    #[test]
    fn native_projection_digest_is_prefixed_canonical_sha256() {
        let digest = recompute_native_digest(&native_assignment()).unwrap();
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), 7 + 64);
    }

    #[test]
    fn native_projection_never_serializes_observed_runtime_state_fields() {
        // Stage 108 (ADR-0015, P0-4 desired/observed correction): the
        // projection must never carry pid/proc_start_time/observed-state --
        // there is no such field on `NativeProcessExecutionProjectionV1` at
        // all, so this asserts the serialized JSON has no such key, as a
        // regression guard against ever adding one here by mistake.
        let projection = compute_native_projection(&native_assignment()).unwrap();
        let value = serde_json::to_value(&projection).unwrap();
        let object = value.as_object().unwrap();
        for forbidden in ["pid", "proc_start_time_ticks", "observed_executable", "state", "started_at_utc"] {
            assert!(!object.contains_key(forbidden), "projection must not contain '{forbidden}'");
        }
    }

    fn assignment() -> OperationalArtifactAssignment {
        OperationalArtifactAssignment {
            artifact_id: "mqtt".to_string(),
            service_id: "mqtt".to_string(),
            artifact_service_id: "mqtt".to_string(),
            version: "2".to_string(),
            artifact_kind: "container".to_string(),
            runtime_kind: "container".to_string(),
            container_name: "datum-mqtt".to_string(),
            image: "eclipse-mosquitto:2".to_string(),
            native_process: None,
            acquisition: None,
            image_source_kind: crate::agent::operational_reconciliation::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
            resolved_image_identity: None,
            target_node: "fog-01".to_string(),
            lifecycle: OperationalLifecycle::default(),
            network: Some("datum-net".to_string()),
            network_aliases: vec!["mqtt".to_string()],
            ports: vec![OperationalPortMapping {
                container_port: 1883,
                host_port: Some(1883),
                protocol: "tcp".to_string(),
            }],
            volumes: vec![OperationalVolumeMount {
                name: "mqtt-data".to_string(),
                target: "/mosquitto/data".to_string(),
                read_only: false,
            }],
            configuration_mounts: vec![OperationalConfigurationMount {
                source: "mosquitto/mosquitto.conf".to_string(),
                target: "/mosquitto/config/mosquitto.conf".to_string(),
                read_only: true,
                declared_content_sha256: Some("a".repeat(64)),
            }],
            command: vec![],
            dependencies: vec![],
            wasm_decision: ArtifactWasmDecisionPolicy {
                explicit_invocation_required: true,
                module_id: "reconcile_service".to_string(),
                function_name: "decide".to_string(),
                host_imports_allowed: false,
                side_effects_allowed: false,
                safe_to_auto_execute: false,
            },
            bindings: BTreeMap::new(),
            secret_refs: vec!["mqtt_token".to_string()],
            secret_bindings: vec![OperationalSecretBinding {
                secret_ref: "mqtt_token".to_string(),
                target_environment_variable: "MQTT_TOKEN".to_string(),
                version: Some("v1".to_string()),
            }],
            execution_generation: 1,
            execution_projection_digest: String::new(),
            application_id: String::new(),
            dmap_ref: None,
        }
    }

    #[test]
    fn projection_excludes_wasm_identity_fields() {
        let projection = compute_projection(&assignment());
        let text = serde_json::to_string(&serde_json::to_value(&projection).unwrap()).unwrap();
        assert!(!text.contains("module_id"));
        assert!(!text.contains("function_name"));
    }

    #[test]
    fn digest_is_deterministic() {
        let d1 = recompute_digest(&assignment()).unwrap();
        let d2 = recompute_digest(&assignment()).unwrap();
        assert_eq!(d1, d2);
    }

    #[test]
    fn digest_changes_with_command() {
        let a = assignment();
        let mut b = assignment();
        b.command = vec!["-v".to_string()];
        assert_ne!(recompute_digest(&a).unwrap(), recompute_digest(&b).unwrap());
    }

    #[test]
    fn p0_6_digest_changes_with_image_source_kind() {
        use crate::agent::operational_reconciliation::{IMAGE_SOURCE_KIND_LOCAL_BUILD, IMAGE_SOURCE_KIND_REGISTRY};
        let mut a = assignment();
        a.image_source_kind = IMAGE_SOURCE_KIND_REGISTRY.to_string();
        let mut b = assignment();
        b.image_source_kind = IMAGE_SOURCE_KIND_LOCAL_BUILD.to_string();
        assert_ne!(
            recompute_digest(&a).unwrap(),
            recompute_digest(&b).unwrap(),
            "image_source_kind is safety-relevant (v5 review P0-6) and must participate in the digest"
        );
    }

    #[test]
    fn alias_joined_desired_service_id_does_not_affect_digest_only_canonical_does() {
        // P0-6: a desired-service alias join (service_id = "broker", a
        // service_aliases match; artifact_service_id = "mqtt", the
        // artifact's own canonical id) must produce the SAME digest as the
        // non-aliased case, because the projection identity is sourced from
        // artifact_service_id, never service_id.
        let canonical = assignment();
        let mut aliased = assignment();
        aliased.service_id = "broker".to_string(); // desired D-Serv alias
        assert_eq!(aliased.artifact_service_id, canonical.artifact_service_id);
        assert_eq!(
            recompute_digest(&canonical).unwrap(),
            recompute_digest(&aliased).unwrap(),
            "digest must be invariant to which desired-service alias resolved the join"
        );

        // Conversely, changing the CANONICAL id must change the digest.
        let mut different_artifact = assignment();
        different_artifact.artifact_service_id = "different-canonical-id".to_string();
        assert_ne!(
            recompute_digest(&canonical).unwrap(),
            recompute_digest(&different_artifact).unwrap()
        );
    }

    #[test]
    fn version_change_alone_does_not_affect_digest() {
        // `version` is not part of OperationalArtifactAssignment's projection
        // input at all (it is carried separately for evidence), so this is
        // definitionally true -- asserted here as a guard against a future
        // change accidentally wiring it in.
        let a = assignment();
        let mut b = assignment();
        b.version = "99".to_string();
        assert_eq!(recompute_digest(&a).unwrap(), recompute_digest(&b).unwrap());
    }

    #[test]
    fn lifecycle_args_working_dir_and_environment_are_projection_no_ops() {
        // P1-6: execute_lifecycle_phase dispatches solely on `program`; it
        // never reads args/working_dir/environment, so changing only those
        // fields must not change the digest (and therefore must not
        // increment execution_generation upstream, since the registry
        // increments exactly when the projection digest changes).
        use crate::agent::operational_reconciliation::OperationalLifecycleCommand;
        let a = assignment();
        let mut b = assignment();
        b.lifecycle.install.push(OperationalLifecycleCommand {
            program: "verify_docker".to_string(),
            args: vec!["--ignored".to_string()],
            working_dir: Some("/ignored".to_string()),
            environment: [("IGNORED".to_string(), "value".to_string())].into_iter().collect(),
        });
        let mut c = assignment();
        c.lifecycle.install.push(OperationalLifecycleCommand {
            program: "verify_docker".to_string(),
            ..Default::default()
        });
        // b and c differ only in args/working_dir/environment (both add the
        // same `program`) -- their digests must be identical.
        assert_eq!(recompute_digest(&b).unwrap(), recompute_digest(&c).unwrap());
        // Adding the command AT ALL (a new `program` entry) is still
        // execution-relevant and must change the digest relative to `a`.
        assert_ne!(recompute_digest(&a).unwrap(), recompute_digest(&c).unwrap());
    }

    /// Field-shape parity with `dserver::core::execution_projection::ArtifactExecutionProjectionV1`.
    /// Both sides are hand-written, separate types (no shared crate); this
    /// test enumerates the exact JSON key set this side produces so a
    /// reviewer can diff it directly against the DServer struct's `#[derive(Serialize)]`
    /// output rather than trusting the two files stayed in sync by eye.
    #[test]
    fn dserver_shape_field_names() {
        let projection = compute_projection(&assignment());
        let value = serde_json::to_value(&projection).unwrap();
        let mut keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "artifact_id",
                "artifact_kind",
                "bindings",
                "command",
                "configuration_mounts",
                "container_name",
                "dependencies",
                "image_identity",
                "lifecycle",
                "network",
                "network_aliases",
                "ports",
                "projection",
                "runtime_kind",
                "secret_bindings",
                "secret_refs",
                "service_id",
                "volumes",
                "wasm_safety",
            ]
        );
    }

    /// Review finding P1-5 (v4 review): executes the REAL `compute_projection`
    /// against an `OperationalArtifactAssignment` from a fixture ALSO
    /// consumed by dserver's own test
    /// (`dserver::core::execution_projection::tests::cross_crate_fixture_produces_the_expected_projection_and_digest`)
    /// against its OWN real mapper over the authoritative `ServiceArtifact`
    /// for the SAME logical artifact. The fixture's assignment carries the
    /// alias case: `service_id` ("broker-alias") differs from
    /// `artifact_service_id` ("mqtt-broker") -- proving this side's digest
    /// still matches dserver's despite being computed FROM the aliased wire
    /// shape, because both real mappers key off the canonical id (P0-6).
    /// This is a real behavioural equivalence proof by execution, not a
    /// key-name-set comparison like `dserver_shape_field_names` above.
    #[test]
    fn dserver_cross_crate_fixture_produces_the_expected_projection_and_digest() {
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

        let assignment: OperationalArtifactAssignment =
            serde_json::from_value(fixture["agent_operational_artifact_assignment"].clone())
                .expect("fixture agent_operational_artifact_assignment must deserialize as OperationalArtifactAssignment");
        assert_ne!(
            assignment.service_id, assignment.artifact_service_id,
            "fixture must actually exercise the alias case (P0-6)"
        );

        let projection = compute_projection(&assignment);

        let expected_projection: ArtifactExecutionProjectionV1 =
            serde_json::from_value(fixture["expected_projection"].clone())
                .expect("fixture expected_projection must deserialize as this crate's ArtifactExecutionProjectionV1");
        assert_eq!(projection, expected_projection, "real compute_projection output must equal the fixture's expected_projection");

        let serialized = serde_json::to_value(&projection).unwrap();
        assert_eq!(serialized, fixture["expected_projection"], "serialized optional nulls must agree");
        assert_eq!(jcs::canonicalize(&serialized).unwrap(), fixture["expected_canonical_json"].as_str().unwrap());
        let expected_digest = fixture["expected_projection_digest_sha256"].as_str().unwrap();
        assert_eq!(projection_digest(&projection).unwrap(), expected_digest);
        assert_eq!(recompute_digest(&assignment).unwrap(), expected_digest);

        // The artifact's OWN canonical service_id, never the alias -- P0-6.
        assert_eq!(projection.service_id, "mqtt-broker");
    }

    /// Stage 108 (ADR-0015). The `NativeProcessExecutionProjectionV1`
    /// analogue of `dserver_cross_crate_fixture_produces_the_expected_projection_and_digest`
    /// above -- proves cross-crate parity for `RuntimeKind::NativeProcess`
    /// (Stage 108 test item 33: dserver's and this crate's independently
    /// computed native digests must be identical for equivalent content).
    #[test]
    fn dserver_native_process_cross_crate_fixture_produces_the_expected_projection_and_digest() {
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

        let assignment: OperationalArtifactAssignment =
            serde_json::from_value(fixture["agent_operational_artifact_assignment"].clone())
                .expect("fixture agent_operational_artifact_assignment must deserialize as OperationalArtifactAssignment");
        assert_ne!(
            assignment.service_id, assignment.artifact_service_id,
            "fixture must actually exercise the alias case (P0-6)"
        );

        let projection = compute_native_projection(&assignment)
            .expect("compute_native_projection must succeed on the fixture assignment");

        let expected_projection: NativeProcessExecutionProjectionV1 =
            serde_json::from_value(fixture["expected_projection"].clone())
                .expect("fixture expected_projection must deserialize as this crate's NativeProcessExecutionProjectionV1");
        assert_eq!(projection, expected_projection, "real compute_native_projection output must equal the fixture's expected_projection");

        let serialized = serde_json::to_value(&projection).unwrap();
        assert_eq!(serialized, fixture["expected_projection"], "serialized optional nulls must agree");
        assert_eq!(jcs::canonicalize(&serialized).unwrap(), fixture["expected_canonical_json"].as_str().unwrap());
        let expected_digest = fixture["expected_projection_digest_sha256"].as_str().unwrap();
        assert_eq!(native_projection_digest(&projection).unwrap(), expected_digest);
        assert_eq!(recompute_native_digest(&assignment).unwrap(), expected_digest);

        // The artifact's OWN canonical service_id, never the alias -- P0-6.
        assert_eq!(projection.service_id, "native-svc");
    }

    /// Stage 107B-A (ADR-0013), test #7 (design plan): DATUM-side half of
    /// `oci_resolved_cross_crate_fixture_produces_the_expected_projection_and_digest`
    /// (dserver). Proves a resolved multi-platform OCI identity projects
    /// identically through this crate's REAL `compute_projection` too.
    #[test]
    fn dserver_oci_resolved_cross_crate_fixture_produces_the_expected_projection_and_digest() {
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

        let assignment: OperationalArtifactAssignment =
            serde_json::from_value(fixture["agent_operational_artifact_assignment"].clone())
                .expect("fixture agent_operational_artifact_assignment must deserialize as OperationalArtifactAssignment");

        let projection = compute_projection(&assignment);
        assert!(
            projection.image_identity.resolved.is_some(),
            "this fixture must actually exercise a populated resolved_image_identity"
        );

        let expected_projection: ArtifactExecutionProjectionV1 =
            serde_json::from_value(fixture["expected_projection"].clone())
                .expect("fixture expected_projection must deserialize as this crate's ArtifactExecutionProjectionV1");
        assert_eq!(projection, expected_projection);

        let serialized = serde_json::to_value(&projection).unwrap();
        assert_eq!(serialized, fixture["expected_projection"], "serialized optional nulls must agree");
        assert_eq!(jcs::canonicalize(&serialized).unwrap(), fixture["expected_canonical_json"].as_str().unwrap());
        let expected_digest = fixture["expected_projection_digest_sha256"].as_str().unwrap();
        assert_eq!(projection_digest(&projection).unwrap(), expected_digest);
        assert_eq!(recompute_digest(&assignment).unwrap(), expected_digest);
    }

    #[test]
    fn absent_optionals_are_explicit_null_and_digest_is_canonical() {
        let mut a = assignment();
        a.network = None;
        a.ports[0].host_port = None;
        a.configuration_mounts[0].declared_content_sha256 = None;
        a.secret_bindings[0].version = None;
        let value = serde_json::to_value(compute_projection(&a)).unwrap();
        for pointer in ["/network", "/image_identity/declared_digest", "/image_identity/resolved", "/ports/0/host_port",
                        "/configuration_mounts/0/declared_content_sha256", "/secret_bindings/0/version"] {
            assert_eq!(value.pointer(pointer), Some(&Value::Null), "{pointer}");
        }
        let digest = recompute_digest(&a).unwrap();
        assert!(jcs::is_canonical_digest_format(&digest));
        assert!(!jcs::is_canonical_digest_format(&digest[7..]));
    }

}
