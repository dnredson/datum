use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::{
    operational_dgraph::{
        OperationalDGraphSlice, OperationalDesiredService,
        OperationalReconciliationAuthorization, OperationalReconcilerDescriptor,
    },
    service_artifact_registry::{
        ContainerPortMapping, ServiceArtifact, ServiceArtifactLifecycle,
        ServiceArtifactSecretBinding, ServiceArtifactWasmDecision,
    },
};

pub const ARTIFACT_RECONCILIATION_CONTEXT_SCHEMA_VERSION: &str = "0.1.0";
pub const ARTIFACT_RECONCILIATION_CONTEXT_KIND: &str =
    "datum_artifact_reconciliation_context";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactReconciliationAssignment {
    pub artifact_id: String,
    /// The DESIRED D-Serv id from the operational slice this artifact was
    /// joined against -- may be a `service_aliases` alias, not necessarily
    /// equal to `artifact_service_id`. Used for D-Graph / desired-service
    /// joins and for `ArtifactBindingV1.service_id` (the authorization
    /// binding key). Never used for projection identity -- see
    /// `artifact_service_id`.
    pub service_id: String,
    /// The artifact's OWN canonical `service_id`
    /// (`ServiceArtifact.service_id`), regardless of which alias (if any)
    /// resolved the join above. `ArtifactExecutionProjectionV1.service_id`
    /// is always sourced from this field, never from `service_id`, on both
    /// DServer and the agent (review finding P0-6: using the desired/alias
    /// id for projection identity made the two sides' independently
    /// computed digests diverge for a valid, untampered, alias-joined
    /// assignment).
    #[serde(default)]
    pub artifact_service_id: String,
    pub version: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub container_name: String,
    pub image: String,
    pub target_node: String,
    #[serde(default)]
    pub network: Option<String>,
    /// Control-plane-declared Docker network aliases (Stage 107A, F2c). See
    /// `ContainerDeploymentSpec.network_aliases`.
    #[serde(default)]
    pub network_aliases: Vec<String>,
    /// Control-plane-declared port mappings (Stage 107A field-audit finding:
    /// this was previously present on `ContainerDeploymentSpec.ports` but
    /// never carried onto the wire; effective ports came entirely from
    /// node-local `DispatchContainerMapping.ports`, a Section F2c
    /// substitution). `required` mode uses this and only this.
    #[serde(default)]
    pub ports: Vec<ContainerPortMapping>,
    #[serde(default)]
    pub volumes: Vec<ArtifactVolumeMount>,
    #[serde(default)]
    pub configuration_mounts: Vec<ArtifactConfigurationMount>,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub lifecycle: ServiceArtifactLifecycle,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub wasm_decision: ServiceArtifactWasmDecision,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
    /// Control-plane-declared secret target bindings (Stage 107A, F2c). See
    /// `ServiceArtifactControlPlane.secret_bindings`.
    #[serde(default)]
    pub secret_bindings: Vec<ServiceArtifactSecretBinding>,
    /// Server-owned (Stage 107A, D-7a). The generation of this artifact's
    /// content at the moment this context was assembled.
    #[serde(default)]
    pub execution_generation: u64,
    /// Server-owned (Stage 107A). SHA-256 (hex) of the RFC 8785 JCS
    /// serialization of this artifact's `ArtifactExecutionProjectionV1`, as
    /// computed authoritatively by DServer. The agent independently
    /// reconstructs the projection from this same assignment and recomputes
    /// the digest to verify it was not tampered with in transit.
    #[serde(default)]
    pub execution_projection_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactVolumeMount {
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactConfigurationMount {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
    /// See `ContainerConfigurationMount.declared_content_sha256` (Stage 107A,
    /// Section G).
    #[serde(default)]
    pub declared_content_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactReconciliationExecutionPolicy {
    pub mode: String,
    pub explicit_invocation_required: bool,
    pub host_imports_allowed: bool,
    pub side_effects_allowed: bool,
    pub safe_to_auto_execute: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactReconciliationContext {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    pub desired_services: Vec<OperationalDesiredService>,
    pub artifacts: Vec<ArtifactReconciliationAssignment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<OperationalReconciliationAuthorization>,
    pub reconciler: OperationalReconcilerDescriptor,
    pub execution_policy: ArtifactReconciliationExecutionPolicy,
    pub side_effects_performed: bool,
}

/// Resolve `assignments` from the CURRENT (mutable) artifact registry, then
/// build the envelope. This is the MUTABLE-RESOLUTION path: use it only when
/// there is no authorized immutable snapshot to serve instead (review
/// finding P0-1) -- at authorization-issuance time (where resolving current
/// content is exactly what's needed, to capture the new snapshot), and as
/// the fallback for a context fetch with no attached authorization
/// (unauthenticated/proposal-preview use, matching pre-Stage-107A
/// behaviour). A context fetch WITH a valid, snapshot-bearing authorization
/// must call `build_context_from_assignments` with the authorization's
/// stored `snapshot` instead of this function -- see
/// `dserver/src/api/artifact_reconciliation.rs`.
pub fn build_artifact_reconciliation_context(
    slice: &OperationalDGraphSlice,
    artifacts: &[ServiceArtifact],
) -> Result<ArtifactReconciliationContext, String> {
    let assignments = resolve_assignments_from_registry(slice, artifacts)?;
    build_context_from_assignments(slice, assignments)
}

/// The resolution half of `build_artifact_reconciliation_context`: join
/// `slice.desired_services` against the CURRENT, mutable `artifacts` list.
pub fn resolve_assignments_from_registry(
    slice: &OperationalDGraphSlice,
    artifacts: &[ServiceArtifact],
) -> Result<Vec<ArtifactReconciliationAssignment>, String> {
    let mut assignments = Vec::with_capacity(slice.desired_services.len());

    for service in &slice.desired_services {
        let matches = artifacts.iter().filter(|artifact| {
            let direct_match = artifact.artifact_id == service.service_id
                || artifact.service_id == service.service_id;
            let alias_match = artifact.metadata.as_ref()
                .and_then(|metadata| metadata.get("service_aliases"))
                .and_then(|aliases| aliases.as_array())
                .is_some_and(|aliases| aliases.iter().any(|alias| {
                    alias.as_str() == Some(service.service_id.as_str())
                }));
            (direct_match || alias_match)
                && artifact.control_plane.target_nodes.iter()
                    .any(|node| node == &slice.node_id)
        }).collect::<Vec<_>>();

        let artifact = match matches.as_slice() {
            [artifact] => *artifact,
            [] => {
                return Err(format!(
                    "no service artifact for desired service '{}' on node '{}'",
                    service.service_id, slice.node_id
                ));
            }
            _ => {
                return Err(format!(
                    "multiple service artifacts resolve desired service '{}' on node '{}'",
                    service.service_id, slice.node_id
                ));
            }
        };

        let container = artifact.runtime.container.as_ref().ok_or_else(|| {
            format!(
                "service artifact '{}' has no container runtime definition",
                artifact.artifact_id
            )
        })?;

        assignments.push(ArtifactReconciliationAssignment {
            artifact_id: artifact.artifact_id.clone(),
            service_id: service.service_id.clone(),
            artifact_service_id: artifact.service_id.clone(),
            version: artifact.control_plane.version.clone(),
            artifact_kind: artifact.control_plane.artifact_kind.clone(),
            runtime_kind: artifact.runtime.kind.clone(),
            container_name: container.container_name.clone(),
            image: container.image.clone(),
            command: container.command.clone(),
            target_node: slice.node_id.clone(),
            network: container.network.clone(),
            network_aliases: container.network_aliases.clone(),
            ports: container.ports.clone(),
            volumes: container
                .volumes
                .iter()
                .map(|volume| ArtifactVolumeMount {
                    name: volume.name.clone(),
                    target: volume.target.clone(),
                    read_only: volume.read_only,
                })
                .collect(),
            configuration_mounts: container
                .configuration_mounts
                .iter()
                .map(|mount| ArtifactConfigurationMount {
                    source: mount.source.clone(),
                    target: mount.target.clone(),
                    read_only: mount.read_only,
                    declared_content_sha256: mount.declared_content_sha256.clone(),
                })
                .collect(),
            lifecycle: artifact.control_plane.lifecycle.clone(),
            dependencies: artifact.control_plane.dependencies.clone(),
            wasm_decision: artifact.control_plane.wasm_decision.clone(),
            bindings: artifact.control_plane.bindings.clone(),
            secret_refs: artifact.control_plane.secret_refs.clone(),
            secret_bindings: artifact.control_plane.secret_bindings.clone(),
            execution_generation: artifact.control_plane.execution_generation,
            execution_projection_digest: artifact.control_plane.execution_projection_digest.clone(),
        });
    }

    Ok(assignments)
}

/// Build the context envelope from an ALREADY-RESOLVED `assignments` list --
/// either freshly resolved from the mutable registry
/// (`resolve_assignments_from_registry`) or, when a valid authorization with
/// a stored snapshot is attached, the authorization's own immutable
/// `snapshot` (review finding P0-1). Either way, `slice.desired_services`
/// and `assignments` must describe the same set of services;
/// `validate_artifact_reconciliation_context` (called by every real
/// endpoint) enforces that.
/// Section F/G (ADR-0011 N-2) -- `required`-mode external-input
/// completeness. Returns a stably-prefixed gap description per unpinned
/// input; empty means `assignment` is `required`-complete. Evaluated
/// identically here (DServer, at authorization issuance -- see
/// `authorize_operational_reconciliation`) and by the agent
/// (`DATUM/src/agent/stage107a_enforcement::required_completeness_gaps`,
/// defense in depth before mutation). This is NOT catalog pinning (Stage
/// 107B) -- it only detects whether today's catalog happens to already be
/// pinned enough to satisfy `required`, which for the unpinned VS1 catalog
/// it will not be (expected, ADR-0011 N-3 / spec section N: `required`
/// authorization on today's catalog is refused, not silently downgraded).
///
/// - Image: 107A has no mechanism to declare a verifiable image digest at
///   all (`ImageIdentityV1.declared_digest` exists only as the interface
///   Stage 107B fills in), so every image is unconditionally unpinned here.
/// - Configuration mounts: ALWAYS unpinned (review finding P1-3, TOCTOU):
///   an ordinary mutable bind-mounted file is never required-complete in
///   107A even with a declared, verified `declared_content_sha256` -- see
///   the function body for the full rationale.
/// - Secrets: every env-var-backed secret is unpinned, including declarations
///   with a version string. No actual secret identity verification exists in 107A.
pub fn required_completeness_gaps(assignment: &ArtifactReconciliationAssignment) -> Vec<String> {
    let mut gaps = Vec::new();
    if !assignment.image.trim().is_empty() {
        gaps.push(format!("unpinned_image_reference:{}", assignment.image));
    }
    // Review finding P1-3 (TOCTOU / "actual bytes used" gap), Option B: an
    // ordinary mutable bind-mounted configuration file is NEVER
    // required-complete in Stage 107A, REGARDLESS of whether
    // `declared_content_sha256` is present. Verifying the hash immediately
    // before mounting (which the agent still does -- see
    // `verify_declared_configuration_mount_content` / Section G) narrows the
    // window but does not close it: the host file can still change between
    // that verification and the mutation, and a read-only bind mount only
    // stops the CONTAINER from writing, not the host. A declared digest is
    // still valuable, verified, best-effort protection under
    // `legacy_partial` -- it is just not sufficient to call the mount
    // "required-complete" without a real content-addressed immutable
    // snapshot mechanism (Stage 107B / Option A, not implemented here).
    for mount in &assignment.configuration_mounts {
        gaps.push(format!("unpinned_configuration_mount:{}", mount.source));
    }
    // P0-3 (v4 review): a `secret_bindings` entry's `version` string does NOT
    // prove the resolved secret's actual identity -- under today's
    // infrastructure the value always comes from a local environment
    // variable on the node (`stage107a_enforcement::resolve_secret_bindings`
    // in the agent), which the `version` string does not pin, verify, or
    // even see. Treating a versioned binding as "not a gap" implied a
    // content-pinning guarantee that does not exist yet, matching the image
    // reference before this same fix. `version` is still meaningful as a
    // declared safety/documentation contract and remains in the projection,
    // but it can never satisfy `required` completeness by itself -- every
    // `secret_ref` is unconditionally unpinned, exactly like image and
    // configuration-mount content, until a real verified secret-identity
    // mechanism exists (Stage 107B or later).
    for secret_ref in &assignment.secret_refs {
        gaps.push(format!("unpinned_secret_reference:{secret_ref}"));
    }
    gaps
}

pub fn build_context_from_assignments(
    slice: &OperationalDGraphSlice,
    assignments: Vec<ArtifactReconciliationAssignment>,
) -> Result<ArtifactReconciliationContext, String> {
    validate_assignments(&assignments)?;

    Ok(ArtifactReconciliationContext {
        schema_version: ARTIFACT_RECONCILIATION_CONTEXT_SCHEMA_VERSION.to_string(),
        contract_kind: ARTIFACT_RECONCILIATION_CONTEXT_KIND.to_string(),
        project_id: slice.project_id.clone(),
        dgraph_id: slice.dgraph_id.clone(),
        dgraph_revision: slice.dgraph_revision,
        node_id: slice.node_id.clone(),
        desired_services: slice.desired_services.clone(),
        artifacts: assignments,
        authorization: slice.authorization.clone(),
        reconciler: slice.reconciler.clone(),
        execution_policy: ArtifactReconciliationExecutionPolicy {
            mode: "proposal_only".to_string(),
            explicit_invocation_required: true,
            host_imports_allowed: false,
            side_effects_allowed: false,
            safe_to_auto_execute: false,
        },
        side_effects_performed: false,
    })
}

pub fn validate_artifact_reconciliation_context(
    context: &ArtifactReconciliationContext,
) -> Result<(), String> {
    if context.schema_version != ARTIFACT_RECONCILIATION_CONTEXT_SCHEMA_VERSION {
        return Err("unsupported artifact reconciliation context schema_version".to_string());
    }
    if context.contract_kind != ARTIFACT_RECONCILIATION_CONTEXT_KIND {
        return Err("unsupported artifact reconciliation context contract_kind".to_string());
    }
    if context.project_id.trim().is_empty()
        || context.dgraph_id.trim().is_empty()
        || context.node_id.trim().is_empty()
        || context.dgraph_revision == 0
    {
        return Err("artifact reconciliation context identity is incomplete".to_string());
    }
    if context.execution_policy.mode != "proposal_only"
        || !context.execution_policy.explicit_invocation_required
        || context.execution_policy.host_imports_allowed
        || context.execution_policy.side_effects_allowed
        || context.execution_policy.safe_to_auto_execute
        || context.side_effects_performed
    {
        return Err("artifact reconciliation context execution policy is unsafe".to_string());
    }
    if context.reconciler.host_imports_allowed {
        return Err("artifact reconciliation context reconciler cannot allow host imports".to_string());
    }
    if context.desired_services.len() != context.artifacts.len() {
        return Err("artifact reconciliation context service/artifact counts differ".to_string());
    }

    validate_assignments(&context.artifacts)?;

    for service in &context.desired_services {
        if !context.artifacts.iter().any(|artifact| {
            artifact.service_id == service.service_id
                && artifact.target_node == context.node_id
        }) {
            return Err(format!(
                "desired service '{}' has no matching artifact assignment",
                service.service_id
            ));
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

fn validate_assignments(
    assignments: &[ArtifactReconciliationAssignment],
) -> Result<(), String> {
    for assignment in assignments {
        if assignment.command.len() > 64
            || assignment.command.iter().any(|arg| arg.len() > 4096 || arg.contains('\0'))
        {
            return Err("invalid container command arguments".to_string());
        }
        if assignment.artifact_id.trim().is_empty()
            || assignment.service_id.trim().is_empty()
            || assignment.version.trim().is_empty()
            || assignment.runtime_kind.trim().is_empty()
            || assignment.container_name.trim().is_empty()
            || assignment.image.trim().is_empty()
            || assignment.target_node.trim().is_empty()
        {
            return Err("artifact reconciliation assignment has an empty required field".to_string());
        }
        if assignment.runtime_kind != "container" {
            return Err(format!(
                "artifact '{}' has unsupported runtime kind '{}'",
                assignment.artifact_id, assignment.runtime_kind
            ));
        }
        if let Some(network) = assignment.network.as_deref() {
            if !is_safe_docker_name(network) {
                return Err(format!(
                    "artifact '{}' has an invalid Docker network name",
                    assignment.artifact_id
                ));
            }
        }
        for volume in &assignment.volumes {
            if !is_safe_docker_name(&volume.name)
                || !volume.target.starts_with('/')
                || volume.target.chars().any(char::is_whitespace)
                || volume.target.contains(',')
            {
                return Err(format!(
                    "artifact '{}' has an invalid Docker volume mount",
                    assignment.artifact_id
                ));
            }
        }
        for mount in &assignment.configuration_mounts {
            if !is_safe_configuration_source(&mount.source)
                || !mount.target.starts_with('/')
                || mount.target.chars().any(char::is_whitespace)
                || mount.target.contains(',')
                || !mount.read_only
            {
                return Err(format!(
                    "artifact '{}' has an invalid governed configuration mount",
                    assignment.artifact_id
                ));
            }
        }

        let wasm = &assignment.wasm_decision;
        if !wasm.explicit_invocation_required
            || wasm.host_imports_allowed
            || wasm.side_effects_allowed
            || wasm.safe_to_auto_execute
            || wasm.module_id.trim().is_empty()
            || wasm.function_name.trim().is_empty()
        {
            return Err(format!(
                "artifact '{}' has an unsafe or incomplete WASM decision policy",
                assignment.artifact_id
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operational_dgraph::OperationalDGraphSlice;
    use crate::core::service_artifact_registry::{
        ContainerDeploymentSpec, ServiceArtifactAcquisition, ServiceArtifactControlPlane,
        ServiceArtifactLifecycle, ServiceArtifactWasmDecision, ServiceProbeSet, ServiceProbeSpec, ServiceRuntimeSpec,
    };

    fn artifact(node_id: &str) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: "0.1.0".to_string(),
            artifact_id: "mqtt".to_string(),
            service_id: "mqtt".to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-mqtt".to_string(),
                    image: "eclipse-mosquitto:2".to_string(),
                    ports: Vec::new(),
                    network: Some("datum-magistrala-net".to_string()),
                    network_aliases: Vec::new(),
                    volumes: vec![crate::core::service_artifact_registry::ContainerVolumeMount {
                        name: "datum-mqtt-volume".to_string(),
                        target: "/var/lib/mqtt".to_string(),
                        read_only: false,
                    }],
                    configuration_mounts: Vec::new(),
                    command: Vec::new(),
                }),
            },
            probes: ServiceProbeSet { execution: ServiceProbeSpec { kind: "docker_container_running".to_string(), params: BTreeMap::new(), vantage_point: Some("local_host".to_string()) }, health: None, availability: None },
            metadata: None,
            control_plane: ServiceArtifactControlPlane {
                project_id: "project:test".to_string(),
                name: "MQTT".to_string(),
                version: "2".to_string(),
                artifact_kind: "container".to_string(),
                target_nodes: vec![node_id.to_string()],
                target_stages: vec!["fog".to_string()],
                dependencies: Vec::new(),
                acquisition: ServiceArtifactAcquisition {
                    kind: "image".to_string(),
                    ..Default::default()
                },
                lifecycle: ServiceArtifactLifecycle {
                    install: vec![crate::core::service_artifact_registry::ServiceArtifactCommand {
                        program: "start_container".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                wasm_decision: ServiceArtifactWasmDecision {
                    module_id: "reconcile_service".to_string(),
                    function_name: "decide".to_string(),
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }

    fn slice() -> OperationalDGraphSlice {
        OperationalDGraphSlice {
            schema_version: "0.1.0".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            dgraph_revision: 1,
            project_id: "project:test".to_string(),
            node_id: "fog-01".to_string(),
            desired_services: vec![OperationalDesiredService {
                service_id: "mqtt".to_string(),
                runtime: "container".to_string(),
                desired_state: "running".to_string(),
            }],
            authorization: None,
            reconciler: OperationalReconcilerDescriptor {
                module_id: "smartsentinel-reconciler-v1".to_string(),
                function_name: "reconcile_service".to_string(),
                abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
                host_imports_allowed: false,
            },
        }
    }

    #[test]
    fn command_survives_context_and_invalid_command_is_rejected() {
        let mut item = artifact("fog-01");
        item.runtime.container.as_mut().unwrap().command = vec!["-c".into(), "/etc/chirpstack".into()];
        let mut context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        assert_eq!(context.artifacts[0].command, vec!["-c", "/etc/chirpstack"]);
        context.artifacts[0].command = vec!["bad\0arg".into()];
        assert!(validate_artifact_reconciliation_context(&context).is_err());
    }

    #[test]
    fn desired_service_resolves_to_node_scoped_artifact() {
        let context = build_artifact_reconciliation_context(&slice(), &[artifact("fog-01")])
            .expect("artifact context should build");
        assert_eq!(context.artifacts.len(), 1);
        assert_eq!(context.artifacts[0].image, "eclipse-mosquitto:2");
        assert_eq!(
            context.artifacts[0].network.as_deref(),
            Some("datum-magistrala-net")
        );
        assert_eq!(context.artifacts[0].volumes[0].target, "/var/lib/mqtt");
        assert_eq!(context.execution_policy.mode, "proposal_only");
        assert_eq!(
            context.artifacts[0].artifact_service_id, "mqtt",
            "artifact_service_id must be the artifact's own canonical service_id"
        );
        validate_artifact_reconciliation_context(&context)
            .expect("built context should validate");
    }

    #[test]
    fn alias_joined_service_carries_canonical_artifact_service_id_separately() {
        // P0-6: the desired service_id ("broker") is an alias match against
        // artifact.service_id ("mqtt"). The assignment's `service_id` field
        // reflects the alias (used for D-Graph joins / authorization
        // binding), but `artifact_service_id` must always be the artifact's
        // OWN canonical id, so DServer's stored digest (computed from
        // artifact.service_id) and the agent's independently recomputed
        // digest (computed from artifact_service_id) can agree regardless of
        // which alias resolved the join.
        let mut aliased_artifact = artifact("fog-01");
        aliased_artifact.metadata = Some(serde_json::json!({"service_aliases": ["broker"]}));
        let mut aliased_slice = slice();
        aliased_slice.desired_services[0].service_id = "broker".to_string();

        let context = build_artifact_reconciliation_context(&aliased_slice, &[aliased_artifact])
            .expect("alias join should resolve");
        assert_eq!(context.artifacts[0].service_id, "broker");
        assert_eq!(context.artifacts[0].artifact_service_id, "mqtt");
    }

    // ---- P0-1: authorization snapshot is actually immutable at issuance --- //

    mod snapshot_immutability {
        use super::*;
        use crate::core::operational_dgraph::{
            build_operational_slice, build_reconciliation_authorization, ArtifactBindingV1,
            OperationalDGraph, OperationalDesiredService, OperationalNodeDesiredState,
            OperationalReconciliationAuthorization, OperationalReconciliationAuthorizationRequest,
            AUTHORIZATION_SCOPE_DURABLE,
        };
        use crate::core::service_artifact_registry::ServiceArtifactRegistry;
        use chrono::{TimeZone, Utc};

        fn graph() -> OperationalDGraph {
            OperationalDGraph {
                schema_version: "0.1.0".to_string(),
                dgraph_id: "dgraph:test".to_string(),
                revision: 1,
                project_id: "project-a".to_string(),
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

        fn auth_request() -> OperationalReconciliationAuthorizationRequest {
            OperationalReconciliationAuthorizationRequest {
                dgraph_id: "dgraph:test".to_string(),
                dgraph_revision: Some(1),
                authorized_by: "operator:test".to_string(),
                allow_host_mutation: true,
                operation: "reconcile".to_string(),
                persistence_scope: Some(AUTHORIZATION_SCOPE_DURABLE.to_string()),
                valid_for_seconds: Some(60),
            }
        }

        /// Mirrors exactly what `authorize_operational_reconciliation`
        /// (dserver/src/api/operational_dgraph.rs) does: resolve the current
        /// registry content, bind it, and capture it as the snapshot.
        fn issue_authorization(registry: &ServiceArtifactRegistry) -> OperationalReconciliationAuthorization {
            let graph = graph();
            let slice = build_operational_slice(&graph, "fog-01", None).unwrap();
            let now = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            // Mirrors the real handler exactly (P0-1, v4 review): resolve
            // the artifact snapshot AND read the project's pinning mode from
            // the SAME `with_stable_view` call, under one held registry read
            // lock, exactly like `authorize_operational_reconciliation` now
            // does -- not two separately-timed reads (`list` then
            // `pinning_mode`) that a concurrent write could interleave
            // between.
            registry
                .with_stable_view(|state| {
                    let artifacts = state
                        .artifacts_by_project
                        .get("project-a")
                        .cloned()
                        .unwrap_or_default();
                    let resolved = build_artifact_reconciliation_context(&slice, &artifacts).unwrap();
                    let bindings = resolved
                        .artifacts
                        .iter()
                        .map(|assignment| ArtifactBindingV1 {
                            service_id: assignment.service_id.clone(),
                            artifact_id: assignment.artifact_id.clone(),
                            execution_generation: assignment.execution_generation,
                            execution_projection_digest: assignment.execution_projection_digest.clone(),
                        })
                        .collect();
                    let pinning_mode =
                        crate::core::service_artifact_registry::pinning_mode_from_state(state, "project-a");
                    build_reconciliation_authorization(
                        &graph,
                        "fog-01",
                        "binding:test",
                        "resource:test",
                        &auth_request(),
                        now,
                        bindings,
                        &pinning_mode,
                        resolved.artifacts,
                    )
                    .unwrap()
                })
                .unwrap()
        }

        #[test]
        fn snapshot_survives_an_artifact_update_within_the_lease() {
            let registry = ServiceArtifactRegistry::from_state(Default::default());
            let created = registry.create("project-a", artifact("fog-01")).unwrap();
            assert_eq!(created.control_plane.execution_generation, 1);

            let authorization = issue_authorization(&registry);
            assert_eq!(authorization.snapshot.len(), 1);
            assert_eq!(authorization.snapshot[0].execution_generation, 1);
            assert_eq!(authorization.snapshot[0].image, "eclipse-mosquitto:2");

            // The artifact changes to generation 2 (content-changing update)
            // WHILE the authorization above is still valid.
            let mut updated = created.clone();
            updated.runtime.container.as_mut().unwrap().image = "eclipse-mosquitto:2.1".to_string();
            let updated = registry.update("project-a", "mqtt", updated).unwrap();
            assert_eq!(updated.control_plane.execution_generation, 2);

            // Fetching the context THROUGH THE STILL-VALID AUTHORIZATION must
            // yield the ORIGINAL (gen 1) snapshot -- the whole point of P0-1 --
            // not a re-resolution of the now-updated registry.
            let graph = graph();
            let mut slice = build_operational_slice(&graph, "fog-01", None).unwrap();
            slice.authorization = Some(authorization.clone());
            let served_context = build_context_from_assignments(&slice, authorization.snapshot.clone()).unwrap();
            assert_eq!(served_context.artifacts[0].execution_generation, 1);
            assert_eq!(served_context.artifacts[0].image, "eclipse-mosquitto:2");

            // Contrast: re-resolving from the CURRENT (mutable) registry --
            // what a NEW authorization issued now would see -- correctly
            // shows the updated content. This is not a bug in the mutable
            // path; it is what proves the snapshot path above is doing
            // something different and correct.
            let current_artifacts = registry.list("project-a").unwrap();
            let fresh_context = build_artifact_reconciliation_context(&slice, &current_artifacts.artifacts).unwrap();
            assert_eq!(fresh_context.artifacts[0].execution_generation, 2);
            assert_eq!(fresh_context.artifacts[0].image, "eclipse-mosquitto:2.1");
        }

        #[test]
        fn todays_bare_tag_catalog_is_never_required_complete_through_the_real_resolution_path() {
            // P0-3, end to end through the EXACT join
            // `authorize_operational_reconciliation` uses: resolve, then
            // check completeness on the resolved snapshot itself.
            let registry = ServiceArtifactRegistry::from_state(Default::default());
            registry.create("project-a", artifact("fog-01")).unwrap();
            let authorization = issue_authorization(&registry);
            let gaps = required_completeness_gaps(&authorization.snapshot[0]);
            assert!(!gaps.is_empty(), "today's catalog must not be reported required-complete");
            assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")));
        }

        #[test]
        fn pinning_mode_change_is_prospective_not_retroactive() {
            // P1-7: an already-issued authorization keeps the mode it was
            // bound with at issuance, even after the project's mode changes.
            let registry = ServiceArtifactRegistry::from_state(Default::default());
            registry.create("project-a", artifact("fog-01")).unwrap();
            registry.set_pinning_mode("project-a", "legacy_partial", "operator:test").unwrap();

            let old_authorization = issue_authorization(&registry);
            assert_eq!(old_authorization.execution_pinning_mode, "legacy_partial");

            registry.set_pinning_mode("project-a", "required", "operator:test").unwrap();

            // The OLD authorization's own bound mode is untouched.
            assert_eq!(old_authorization.execution_pinning_mode, "legacy_partial");

            // A NEW authorization, issued after the change, sees the new mode.
            let new_authorization = issue_authorization(&registry);
            assert_eq!(new_authorization.execution_pinning_mode, "required");
        }

        #[test]
        fn a_new_authorization_issued_after_the_update_sees_the_new_generation() {
            let registry = ServiceArtifactRegistry::from_state(Default::default());
            registry.create("project-a", artifact("fog-01")).unwrap();
            let old_authorization = issue_authorization(&registry);
            assert_eq!(old_authorization.snapshot[0].execution_generation, 1);

            let created = registry.get("project-a", "mqtt").unwrap();
            let mut updated = created.clone();
            updated.runtime.container.as_mut().unwrap().image = "eclipse-mosquitto:2.1".to_string();
            registry.update("project-a", "mqtt", updated).unwrap();

            let new_authorization = issue_authorization(&registry);
            assert_eq!(new_authorization.snapshot[0].execution_generation, 2);
            assert_eq!(new_authorization.snapshot[0].image, "eclipse-mosquitto:2.1");
            // The old authorization's OWN snapshot is completely unaffected
            // by issuing a new one.
            assert_eq!(old_authorization.snapshot[0].execution_generation, 1);
        }

        #[test]
        fn snapshot_survives_delete_and_recreate_while_the_old_lease_is_alive() {
            let registry = ServiceArtifactRegistry::from_state(Default::default());
            registry.create("project-a", artifact("fog-01")).unwrap();
            let authorization = issue_authorization(&registry);
            assert_eq!(authorization.snapshot[0].execution_generation, 1);
            let original_image = authorization.snapshot[0].image.clone();

            // Delete, then recreate with DIFFERENT content, entirely while
            // the old authorization/lease is still conceptually alive.
            registry.delete("project-a", "mqtt").unwrap();
            let mut recreated = artifact("fog-01");
            recreated.runtime.container.as_mut().unwrap().image = "eclipse-mosquitto:3".to_string();
            let recreated = registry.create("project-a", recreated).unwrap();
            assert_eq!(
                recreated.control_plane.execution_generation, 2,
                "delete/recreate must still be monotonic (D-7a) even while an old lease survives"
            );

            // The OLD authorization's stored snapshot is a plain owned Vec,
            // never touched by delete/recreate -- it still serves the
            // ORIGINAL content unaffected.
            let graph = graph();
            let mut slice = build_operational_slice(&graph, "fog-01", None).unwrap();
            slice.authorization = Some(authorization.clone());
            let served_context = build_context_from_assignments(&slice, authorization.snapshot.clone()).unwrap();
            assert_eq!(served_context.artifacts[0].execution_generation, 1);
            assert_eq!(served_context.artifacts[0].image, original_image);
        }
    }

    // ---- P0-3: required-mode completeness -------------------------------- //

    #[test]
    fn bare_image_tag_is_always_an_unpinned_gap() {
        let context = build_artifact_reconciliation_context(&slice(), &[artifact("fog-01")]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")), "{gaps:?}");
    }

    #[test]
    fn config_mount_without_declared_digest_is_a_gap() {
        let mut item = artifact("fog-01");
        item.runtime.container.as_mut().unwrap().configuration_mounts =
            vec![crate::core::service_artifact_registry::ContainerConfigurationMount {
                source: "mosquitto/mosquitto.conf".to_string(),
                target: "/mosquitto/config/mosquitto.conf".to_string(),
                read_only: true,
                declared_content_sha256: None,
            }];
        let context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert!(gaps.iter().any(|gap| gap == "unpinned_configuration_mount:mosquitto/mosquitto.conf"), "{gaps:?}");
    }

    #[test]
    fn config_mount_with_declared_digest_is_still_a_gap_toctou() {
        // P1-3: even a declared, verified content digest does not make an
        // ordinary mutable bind-mounted file required-complete -- Option B
        // (TOCTOU / "actual bytes used" gap).
        let mut item = artifact("fog-01");
        item.runtime.container.as_mut().unwrap().configuration_mounts =
            vec![crate::core::service_artifact_registry::ContainerConfigurationMount {
                source: "mosquitto/mosquitto.conf".to_string(),
                target: "/mosquitto/config/mosquitto.conf".to_string(),
                read_only: true,
                declared_content_sha256: Some("a".repeat(64)),
            }];
        let context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert!(gaps.iter().any(|gap| gap == "unpinned_configuration_mount:mosquitto/mosquitto.conf"), "{gaps:?}");
    }

    #[test]
    fn secret_ref_without_versioned_binding_is_a_gap() {
        let mut item = artifact("fog-01");
        item.control_plane.secret_refs = vec!["token".to_string()];
        let context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert!(gaps.iter().any(|gap| gap == "unpinned_secret_reference:token"), "{gaps:?}");
    }

    #[test]
    fn secret_ref_with_versioned_binding_is_still_a_gap_p0_3() {
        // P0-3 (v4 review): a `version` string does not prove the resolved
        // secret's actual identity -- the value always comes from a local
        // environment variable the binding does not see or pin. A versioned
        // binding therefore does NOT close the gap, matching the image
        // reference and the configuration-mount content, both of which are
        // unconditional gaps in `required` mode for the same underlying
        // reason (no verified content-identity mechanism exists yet).
        let mut item = artifact("fog-01");
        item.control_plane.secret_refs = vec!["token".to_string()];
        item.control_plane.secret_bindings = vec![crate::core::service_artifact_registry::ServiceArtifactSecretBinding {
            secret_ref: "token".to_string(),
            target_environment_variable: "TOKEN".to_string(),
            version: Some("v3".to_string()),
        }];
        let context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert!(gaps.iter().any(|gap| gap == "unpinned_secret_reference:token"), "{gaps:?}");
    }

    #[test]
    fn secret_ref_with_unversioned_binding_is_still_a_gap() {
        // A binding entry existing is not enough either; without a `version`
        // there is nothing to call "declared identity/version" at all, and
        // (per P0-3) a `version` would not have been sufficient anyway.
        let mut item = artifact("fog-01");
        item.control_plane.secret_refs = vec!["token".to_string()];
        item.control_plane.secret_bindings = vec![crate::core::service_artifact_registry::ServiceArtifactSecretBinding {
            secret_ref: "token".to_string(),
            target_environment_variable: "TOKEN".to_string(),
            version: None,
        }];
        let context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert!(gaps.iter().any(|gap| gap == "unpinned_secret_reference:token"), "{gaps:?}");
    }

    #[test]
    fn fully_declared_assignment_still_has_image_config_and_secret_gaps() {
        // P0-3 (v4 review): nothing in today's catalog can close ANY of the
        // three gaps -- image, configuration-mount content, and secret
        // identity are all unconditional in `required` mode (Stage 107B or
        // later closes them with a real content-addressed / verified-secret
        // mechanism). A fully-declared assignment (versioned secret binding,
        // declared config digest) still reports all three.
        let mut item = artifact("fog-01");
        item.runtime.container.as_mut().unwrap().configuration_mounts =
            vec![crate::core::service_artifact_registry::ContainerConfigurationMount {
                source: "mosquitto/mosquitto.conf".to_string(),
                target: "/mosquitto/config/mosquitto.conf".to_string(),
                read_only: true,
                declared_content_sha256: Some("a".repeat(64)),
            }];
        item.control_plane.secret_refs = vec!["token".to_string()];
        item.control_plane.secret_bindings = vec![crate::core::service_artifact_registry::ServiceArtifactSecretBinding {
            secret_ref: "token".to_string(),
            target_environment_variable: "TOKEN".to_string(),
            version: Some("v3".to_string()),
        }];
        let context = build_artifact_reconciliation_context(&slice(), &[item]).unwrap();
        let gaps = required_completeness_gaps(&context.artifacts[0]);
        assert_eq!(gaps.len(), 3, "{gaps:?}");
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_image_reference:")));
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_configuration_mount:")));
        assert!(gaps.iter().any(|gap| gap.starts_with("unpinned_secret_reference:")));
    }

    #[test]
    fn empty_image_reference_produces_no_gap_for_that_specific_check() {
        // Defensive: an empty image string is caught elsewhere (validation
        // rejects it outright before this point ever matters), not
        // double-reported here.
        let gaps = required_completeness_gaps(&ArtifactReconciliationAssignment {
            artifact_id: "x".to_string(),
            service_id: "x".to_string(),
            artifact_service_id: "x".to_string(),
            version: "1".to_string(),
            artifact_kind: "container".to_string(),
            runtime_kind: "container".to_string(),
            container_name: "x".to_string(),
            image: String::new(),
            target_node: "n".to_string(),
            network: None,
            network_aliases: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            configuration_mounts: Vec::new(),
            command: Vec::new(),
            lifecycle: Default::default(),
            dependencies: Vec::new(),
            wasm_decision: ServiceArtifactWasmDecision {
                module_id: "m".to_string(),
                function_name: "f".to_string(),
                ..Default::default()
            },
            bindings: BTreeMap::new(),
            secret_refs: Vec::new(),
            secret_bindings: Vec::new(),
            execution_generation: 1,
            execution_projection_digest: String::new(),
        });
        assert!(gaps.iter().all(|gap| !gap.starts_with("unpinned_image_reference:")));
    }

    #[test]
    fn artifact_for_another_node_is_not_resolved() {
        let error = build_artifact_reconciliation_context(&slice(), &[artifact("cloud-01")])
            .expect_err("wrong-node artifact should fail closed");
        assert!(error.contains("no service artifact"));
    }

    #[test]
    fn unsafe_artifact_wasm_policy_is_rejected() {
        let mut item = artifact("fog-01");
        item.control_plane.wasm_decision.side_effects_allowed = true;
        let error = build_artifact_reconciliation_context(&slice(), &[item])
            .expect_err("unsafe policy should fail closed");
        assert!(error.contains("unsafe"));
    }
}
