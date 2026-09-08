use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::core::{
    operational_dgraph::{
        OperationalDGraphSlice, OperationalDesiredService,
        OperationalReconciliationAuthorization, OperationalReconcilerDescriptor,
    },
    service_artifact_registry::{
        ServiceArtifact, ServiceArtifactWasmDecision,
    },
};

pub const ARTIFACT_RECONCILIATION_CONTEXT_SCHEMA_VERSION: &str = "0.1.0";
pub const ARTIFACT_RECONCILIATION_CONTEXT_KIND: &str =
    "datum_artifact_reconciliation_context";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactReconciliationAssignment {
    pub artifact_id: String,
    pub service_id: String,
    pub version: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub container_name: String,
    pub image: String,
    pub target_node: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub wasm_decision: ServiceArtifactWasmDecision,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
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

pub fn build_artifact_reconciliation_context(
    slice: &OperationalDGraphSlice,
    artifacts: &[ServiceArtifact],
) -> Result<ArtifactReconciliationContext, String> {
    let mut assignments = Vec::with_capacity(slice.desired_services.len());

    for service in &slice.desired_services {
        let matches = artifacts.iter().filter(|artifact| {
            (artifact.artifact_id == service.service_id
                || artifact.service_id == service.service_id)
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
            version: artifact.control_plane.version.clone(),
            artifact_kind: artifact.control_plane.artifact_kind.clone(),
            runtime_kind: artifact.runtime.kind.clone(),
            container_name: container.container_name.clone(),
            image: container.image.clone(),
            target_node: slice.node_id.clone(),
            dependencies: artifact.control_plane.dependencies.clone(),
            wasm_decision: artifact.control_plane.wasm_decision.clone(),
            bindings: artifact.control_plane.bindings.clone(),
            secret_refs: artifact.control_plane.secret_refs.clone(),
        });
    }

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

fn validate_assignments(
    assignments: &[ArtifactReconciliationAssignment],
) -> Result<(), String> {
    for assignment in assignments {
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
        ContainerDeploymentSpec, ServiceArtifactAcquisition, ServiceArtifactCommand,
        ServiceArtifactControlPlane, ServiceArtifactLifecycle, ServiceArtifactProbeSet,
        ServiceArtifactRuntimeSpec,
    };

    fn artifact(node_id: &str) -> ServiceArtifact {
        ServiceArtifact {
            schema_version: "0.1.0".to_string(),
            artifact_id: "mqtt".to_string(),
            service_id: "mqtt".to_string(),
            runtime: ServiceArtifactRuntimeSpec {
                kind: "container".to_string(),
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-mqtt".to_string(),
                    image: "eclipse-mosquitto:2".to_string(),
                    ports: Vec::new(),
                }),
            },
            probes: ServiceArtifactProbeSet::default(),
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
                    install: vec![ServiceArtifactCommand {
                        program: "start_container".to_string(),
                        ..Default::default()
                    }],
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
    fn desired_service_resolves_to_node_scoped_artifact() {
        let context = build_artifact_reconciliation_context(&slice(), &[artifact("fog-01")])
            .expect("artifact context should build");
        assert_eq!(context.artifacts.len(), 1);
        assert_eq!(context.artifacts[0].image, "eclipse-mosquitto:2");
        assert_eq!(context.execution_policy.mode, "proposal_only");
        validate_artifact_reconciliation_context(&context)
            .expect("built context should validate");
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
