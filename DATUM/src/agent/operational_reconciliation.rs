use std::{collections::{BTreeMap, HashSet}, env};
use std::process::Command;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use wasmtime::{Config as WasmConfig, Engine, Instance, Module, Store};

use crate::agent::bootstrap::{
    run_bootstrap, run_bootstrap_with_resource_discovery, BootstrapState,
    SmartSentinelBootstrapReport,
};
use crate::agent::resource_discovery::SmartSentinelResourceDiscovery;
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
    pub authorized_by: String,
    pub authorized_at_utc: String,
    pub allow_host_mutation: bool,
    #[serde(default = "default_authorization_operation")]
    pub operation: String,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationalArtifactAssignment {
    pub artifact_id: String,
    pub service_id: String,
    pub version: String,
    pub artifact_kind: String,
    pub runtime_kind: String,
    pub container_name: String,
    pub image: String,
    pub target_node: String,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub volumes: Vec<OperationalVolumeMount>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub wasm_decision: ArtifactWasmDecisionPolicy,
    #[serde(default)]
    pub bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
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

    let mut side_effects_performed = false;
    let batch_secret_resolution_error = if execute_requested && control_plane_authorized {
        context.artifacts.iter().find_map(|artifact| {
            if artifact.secret_refs.is_empty() {
                None
            } else {
                resolve_secret_bindings(cfg, &artifact.secret_refs)
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
                    Some(artifact) => match resolve_secret_bindings(cfg, &artifact.secret_refs) {
                        Ok(environment) => (environment, None, "resolved_locally".to_string()),
                        Err(error) => (
                            Vec::new(),
                            Some(error),
                            "blocked_missing_secret".to_string(),
                        ),
                    },
                    None => (Vec::new(), None, "not_required".to_string()),
                }
            };
        let attachment_mismatch = match (mapping, assignment, observed_before) {
            (Some(mapping), Some(artifact), ContainerState::Running | ContainerState::Stopped) => {
                !container_attachments_match(
                    &mapping.container_name,
                    artifact.network.as_deref(),
                    &artifact.volumes,
                )
                .unwrap_or(false)
            }
            _ => false,
        };
        let wasm = run_embedded_reconciler(
            true,
            (observed_before_running || completion_succeeded) && !attachment_mismatch,
        )?;

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
                    match mapping {
                        Some(mapping) => match rollback_trusted_container_intent(mapping, observed_before) {
                            Ok(result) => result,
                            Err(error) => (
                                "failed".to_string(),
                                format!("governed trusted container rollback failed: {error}"),
                                false,
                            ),
                        },
                        None => (
                            "blocked_local_mapping".to_string(),
                            "artifact assignment has no matching local mapping".to_string(),
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
            } else if wasm.action_code == 0 {
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
            match mapping {
                Some(mapping) => match execute_trusted_container_intent(
                    mapping,
                    observed_before,
                    assignment.and_then(|artifact| artifact.network.as_deref()),
                    assignment.map(|artifact| artifact.volumes.as_slice()).unwrap_or(&[]),
                    assignment.map(|artifact| &artifact.bindings),
                    &secret_environment,
                ) {
                    Ok(result) => result,
                    Err(error) => (
                        "failed".to_string(),
                        format!("governed trusted container execution failed: {error}"),
                        false,
                    ),
                },
                None => (
                    "blocked_local_mapping".to_string(),
                    "artifact assignment has no matching local mapping".to_string(),
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

fn container_attachments_match(
    container_name: &str,
    network: Option<&str>,
    volumes: &[OperationalVolumeMount],
) -> Result<bool> {
    if let Some(network) = network {
        let output = Command::new("docker")
            .args(["inspect", "-f", "{{json .NetworkSettings.Networks}}", container_name])
            .output()
            .with_context(|| format!("inspect Docker networks for '{container_name}'"))?;
        if !output.status.success() {
            return Ok(false);
        }
        let networks: serde_json::Value = serde_json::from_slice(&output.stdout)
            .with_context(|| format!("decode Docker networks for '{container_name}'"))?;
        if networks.get(network.trim()).is_none() {
            return Ok(false);
        }
    }

    if volumes.is_empty() {
        return Ok(true);
    }

    let output = Command::new("docker")
        .args(["inspect", "-f", "{{json .Mounts}}", container_name])
        .output()
        .with_context(|| format!("inspect Docker mounts for '{container_name}'"))?;
    if !output.status.success() {
        return Ok(false);
    }
    let mounts: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("decode Docker mounts for '{container_name}'"))?;
    let Some(mounts) = mounts.as_array() else {
        return Ok(false);
    };

    Ok(volumes.iter().all(|volume| {
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
    }))
}

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

fn resolve_secret_bindings(
    cfg: &Config,
    secret_refs: &[String],
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
        let binding = dispatch
            .secret_bindings
            .iter()
            .find(|binding| binding.secret_ref.trim() == secret_ref)
            .ok_or_else(|| format!("secret reference '{}' has no local environment binding", secret_ref))?;
        let variable = binding.environment_variable.trim();
        if variable.is_empty() {
            return Err(format!("secret reference '{}' has an empty local environment binding", secret_ref));
        }
        let value = env::var(variable).map_err(|_| format!(
            "local environment variable for secret reference '{}' is not available",
            secret_ref
        ))?;
        if value.is_empty() {
            return Err(format!("local environment variable for secret reference '{}' is empty", secret_ref));
        }
        resolved.push((variable.to_string(), value));
    }
    Ok(resolved)
}

fn execute_trusted_container_intent(
    mapping: &DispatchContainerMapping,
    observed_before: ContainerState,
    network: Option<&str>,
    volumes: &[OperationalVolumeMount],
    bindings: Option<&BTreeMap<String, String>>,
    secret_environment: &[(String, String)],
) -> Result<(String, String, bool)> {
    let container_name = mapping.container_name.trim();
    let image = mapping
        .artifact
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .expect("trusted mapping requires non-empty artifact image");

    let attachments_mismatch = match observed_before {
        ContainerState::Running | ContainerState::Stopped => {
            !container_attachments_match(container_name, network, volumes)?
        }
        ContainerState::Missing => false,
    };
    if attachments_mismatch {
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
            bindings,
            secret_environment,
        );
    }

    if observed_before == ContainerState::Stopped && !secret_environment.is_empty() {
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
            bindings,
            secret_environment,
        );
    }

    match observed_before {
        ContainerState::Running => Ok((
            "already_running".to_string(),
            format!("trusted container '{}' is already running", container_name),
            false,
        )),
        ContainerState::Stopped => {
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
                ensure_docker_network(network)?;
                args.push("--network".to_string());
                args.push(network.trim().to_string());
                if let Some(service_name) = mapping
                    .service_name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| is_safe_docker_name(name))
                {
                    args.push("--network-alias".to_string());
                    args.push(service_name.to_string());
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
            for port in &mapping.ports {
                args.push("-p".to_string());
                args.push(format!(
                    "{}:{}{}",
                    port.host_port,
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
            args.push(image.to_string());
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

fn ensure_docker_network(network: &str) -> Result<()> {
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

fn rollback_trusted_container_intent(
    mapping: &DispatchContainerMapping,
    observed_before: ContainerState,
) -> Result<(String, String, bool)> {
    let container_name = mapping.container_name.trim();
    match observed_before {
        ContainerState::Missing => Ok((
            "rollback_already_absent".to_string(),
            format!("trusted container '{}' is already absent; no rollback mutation was needed", container_name),
            false,
        )),
        ContainerState::Running | ContainerState::Stopped => {
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
    fn docker_missing_container_error_is_case_insensitive() {
        assert!(docker_inspect_reports_missing("Error: No such object: datum-mqtt"));
        assert!(docker_inspect_reports_missing("error: no such object: datum-mqtt"));
        assert!(docker_inspect_reports_missing("Error response from daemon: No such container: datum-mqtt"));
        assert!(docker_inspect_reports_missing("network datum-magistrala-net not found"));
        assert!(!docker_inspect_reports_missing("permission denied while trying to connect to the docker API"));
    }
}
