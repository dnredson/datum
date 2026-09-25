use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::core::artifact_reconciliation_context::ArtifactReconciliationContext;

pub const GOVERNED_PREFLIGHT_SCHEMA_VERSION: &str = "0.1.0";
pub const GOVERNED_PREFLIGHT_CONTRACT_KIND: &str = "datum_governed_deployment_preflight";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GovernedDeploymentPreflight {
    pub schema_version: String,
    pub contract_kind: String,
    pub project_id: String,
    pub dgraph_id: String,
    pub dgraph_revision: u64,
    pub node_id: String,
    pub planner: PreflightPlanner,
    pub services: Vec<PreflightService>,
    pub dependency_order: Vec<String>,
    pub blockers: Vec<String>,
    pub execution_policy: PreflightExecutionPolicy,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightPlanner {
    pub kind: String,
    pub execution_performed: bool,
    pub dependency_ordering_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightService {
    pub service_id: String,
    pub artifact_id: String,
    pub version: String,
    pub dependencies: Vec<String>,
    pub dependency_status: String,
    pub secret_refs: Vec<String>,
    pub secret_status: String,
    pub readiness: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreflightExecutionPolicy {
    pub mode: String,
    pub execution_allowed: bool,
    pub explicit_invocation_required: bool,
    pub side_effects_allowed: bool,
}

pub fn build_governed_deployment_preflight(
    context: &ArtifactReconciliationContext,
) -> Result<GovernedDeploymentPreflight, String> {
    crate::core::artifact_reconciliation_context::validate_artifact_reconciliation_context(context)?;

    let mut by_service = BTreeMap::new();
    for artifact in &context.artifacts {
        by_service.insert(artifact.service_id.to_ascii_lowercase(), artifact);
    }

    let mut blockers = Vec::new();
    let mut services = Vec::with_capacity(context.artifacts.len());

    for artifact in &context.artifacts {
        let missing = artifact
            .dependencies
            .iter()
            .filter(|dependency| !by_service.contains_key(&dependency.to_ascii_lowercase()))
            .cloned()
            .collect::<Vec<_>>();

        let dependency_status = if missing.is_empty() {
            "satisfied"
        } else {
            blockers.extend(missing.iter().map(|dependency| {
                format!(
                    "service '{}' depends on '{}' which is not assigned to node '{}'",
                    artifact.service_id, dependency, context.node_id
                )
            }));
            "missing"
        };

        let secret_status = if artifact.secret_refs.is_empty() {
            "not_required"
        } else {
            blockers.push(format!(
                "service '{}' declares {} secret reference(s); no secret material is available in this preflight contract",
                artifact.service_id,
                artifact.secret_refs.len()
            ));
            "unresolved"
        };

        let readiness = if dependency_status == "satisfied" && secret_status != "unresolved" {
            "ready_for_runtime"
        } else {
            "blocked"
        };

        services.push(PreflightService {
            service_id: artifact.service_id.clone(),
            artifact_id: artifact.artifact_id.clone(),
            version: artifact.version.clone(),
            dependencies: artifact.dependencies.clone(),
            dependency_status: dependency_status.to_string(),
            secret_refs: artifact.secret_refs.clone(),
            secret_status: secret_status.to_string(),
            readiness: readiness.to_string(),
        });
    }

    let dependency_order = topological_order(context, &by_service, &mut blockers)?;

    Ok(GovernedDeploymentPreflight {
        schema_version: GOVERNED_PREFLIGHT_SCHEMA_VERSION.to_string(),
        contract_kind: GOVERNED_PREFLIGHT_CONTRACT_KIND.to_string(),
        project_id: context.project_id.clone(),
        dgraph_id: context.dgraph_id.clone(),
        dgraph_revision: context.dgraph_revision,
        node_id: context.node_id.clone(),
        planner: PreflightPlanner {
            kind: "dependency_ordered_governed_preflight".to_string(),
            execution_performed: false,
            dependency_ordering_performed: true,
        },
        services,
        dependency_order,
        blockers: blockers.clone(),
        execution_policy: PreflightExecutionPolicy {
            mode: "proposal_only".to_string(),
            execution_allowed: blockers.is_empty(),
            explicit_invocation_required: true,
            side_effects_allowed: false,
        },
        side_effects_performed: false,
    })
}

fn topological_order(
    context: &ArtifactReconciliationContext,
    by_service: &BTreeMap<
        String,
        &crate::core::artifact_reconciliation_context::ArtifactReconciliationAssignment,
    >,
    blockers: &mut Vec<String>,
) -> Result<Vec<String>, String> {
    let mut remaining = context
        .artifacts
        .iter()
        .map(|artifact| artifact.service_id.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(remaining.len());

    while !remaining.is_empty() {
        let ready = remaining
            .iter()
            .filter(|service| {
                by_service
                    .get(*service)
                    .map(|artifact| {
                        artifact.dependencies.iter().all(|dependency| {
                            let dep = dependency.to_ascii_lowercase();
                            !remaining.contains(&dep) && by_service.contains_key(&dep)
                        })
                    })
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        if ready.is_empty() {
            let cycle = remaining.iter().cloned().collect::<Vec<_>>().join(", ");
            blockers.push(format!("dependency cycle detected among: {cycle}"));
            return Err(
                "cannot produce dependency order because the artifact graph contains a cycle"
                    .to_string(),
            );
        }

        for service in ready {
            remaining.remove(&service);
            order.push(
                by_service
                    .get(&service)
                    .expect("service was present in assignment map")
                    .service_id
                    .clone(),
            );
        }
    }

    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::artifact_reconciliation_context::{
        ArtifactReconciliationAssignment, ArtifactReconciliationExecutionPolicy,
    };
    use crate::core::operational_dgraph::{
        OperationalDesiredService, OperationalReconcilerDescriptor,
    };
    use crate::core::service_artifact_registry::ServiceArtifactWasmDecision;

    fn context() -> ArtifactReconciliationContext {
        let policy = ServiceArtifactWasmDecision {
            explicit_invocation_required: true,
            module_id: "reconcile_service".to_string(),
            function_name: "decide".to_string(),
            host_imports_allowed: false,
            side_effects_allowed: false,
            safe_to_auto_execute: false,
            max_fuel: Some(1_000_000),
            timeout_ms: Some(1_000),
        };
        ArtifactReconciliationContext {
            schema_version: "0.1.0".to_string(),
            contract_kind: "datum_artifact_reconciliation_context".to_string(),
            project_id: "monitor-f49c8".to_string(),
            dgraph_id: "dgraph:test".to_string(),
            dgraph_revision: 6,
            node_id: "cloud-01".to_string(),
            desired_services: vec![
                OperationalDesiredService { service_id: "db".to_string(), runtime: "container".to_string(), desired_state: "running".to_string(), application_id: String::new(), dmap_ref: None },
                OperationalDesiredService { service_id: "api".to_string(), runtime: "container".to_string(), desired_state: "running".to_string(), application_id: String::new(), dmap_ref: None },
            ],
            artifacts: vec![
                ArtifactReconciliationAssignment {
                    artifact_id: "db".to_string(), service_id: "db".to_string(), artifact_service_id: "db".to_string(), version: "1".to_string(),
                    artifact_kind: "container".to_string(), runtime_kind: "container".to_string(),
                    container_name: "db".to_string(), image: "postgres:16".to_string(),
                    image_source_kind: crate::core::service_artifact_registry::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
                    resolved_image_identity: None,
                    acquisition: None,
                    native_process: None,
                    target_node: "cloud-01".to_string(),
                    network: None, network_aliases: vec![], ports: vec![], volumes: vec![], configuration_mounts: vec![], command: vec![],
                    lifecycle: Default::default(),
                    dependencies: vec![], wasm_decision: policy.clone(), bindings: BTreeMap::new(), secret_refs: vec![],
                    secret_bindings: vec![], execution_generation: 1, execution_projection_digest: String::new(),
                    application_id: String::new(), dmap_ref: None,
                },
                ArtifactReconciliationAssignment {
                    artifact_id: "api".to_string(), service_id: "api".to_string(), artifact_service_id: "api".to_string(), version: "1".to_string(),
                    artifact_kind: "container".to_string(), runtime_kind: "container".to_string(),
                    container_name: "api".to_string(), image: "api:1".to_string(),
                    image_source_kind: crate::core::service_artifact_registry::IMAGE_SOURCE_KIND_REGISTRY.to_string(),
                    resolved_image_identity: None,
                    acquisition: None,
                    native_process: None,
                    target_node: "cloud-01".to_string(),
                    network: None, network_aliases: vec![], ports: vec![], volumes: vec![], configuration_mounts: vec![], command: vec![],
                    lifecycle: Default::default(),
                    dependencies: vec!["db".to_string()], wasm_decision: policy, bindings: BTreeMap::new(), secret_refs: vec!["api_secret".to_string()],
                    secret_bindings: vec![], execution_generation: 1, execution_projection_digest: String::new(),
                    application_id: String::new(), dmap_ref: None,
                },
            ],
            authorization: None,
            reconciler: OperationalReconcilerDescriptor {
                module_id: "smartsentinel-reconciler-v1".to_string(),
                function_name: "reconcile_service".to_string(),
                abi: "(desired_running:i32, observed_running:i32)->action_code:i32".to_string(),
                host_imports_allowed: false,
            },
            execution_policy: ArtifactReconciliationExecutionPolicy {
                mode: "proposal_only".to_string(), explicit_invocation_required: true,
                host_imports_allowed: false, side_effects_allowed: false, safe_to_auto_execute: false,
            },
            side_effects_performed: false,
        }
    }

    #[test]
    fn orders_dependencies_and_blocks_unresolved_secrets() {
        let result = build_governed_deployment_preflight(&context()).unwrap();
        assert_eq!(result.dependency_order, vec!["db", "api"]);
        assert!(!result.execution_policy.execution_allowed);
        assert_eq!(result.services[1].secret_status, "unresolved");
        assert!(!result.side_effects_performed);
    }

    #[test]
    fn rejects_dependency_cycles() {
        let mut item = context();
        item.artifacts[0].dependencies = vec!["api".to_string()];
        assert!(build_governed_deployment_preflight(&item).is_err());
    }
}
