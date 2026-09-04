use std::process::{Command, Output};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::agent::service_artifact::{validate_service_artifact, ServiceArtifact};
use crate::agent::service_deployment_drift::{
    assess_service_deployment_drift, ServiceDeploymentDriftAssessment,
};

pub const SERVICE_DEPLOYMENT_RECONCILIATION_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServiceDeploymentCommandResult {
    pub phase: String,
    pub program: String,
    pub args: Vec<String>,
    pub success: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceDeploymentReconciliationReport {
    pub schema_version: String,
    pub reconciled_at_utc: String,
    pub artifact_id: String,
    pub service_id: String,
    pub runtime_kind: String,
    pub execute_requested: bool,
    pub desired_in_active_slice: bool,
    pub control_plane_authorized: bool,
    pub local_container_execution_allowed: bool,
    pub execution_authorized: bool,
    pub status: String,
    pub side_effects_performed: bool,
    pub drift_before: ServiceDeploymentDriftAssessment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_after: Option<ServiceDeploymentDriftAssessment>,
    #[serde(default)]
    pub command_results: Vec<ServiceDeploymentCommandResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceDeploymentReconciliationGate {
    pub allowed: bool,
    pub status: &'static str,
}

pub fn evaluate_service_deployment_reconciliation_gate(
    execute_requested: bool,
    desired_in_active_slice: bool,
    control_plane_authorized: bool,
    local_container_execution_allowed: bool,
) -> ServiceDeploymentReconciliationGate {
    if !desired_in_active_slice {
        return ServiceDeploymentReconciliationGate {
            allowed: false,
            status: "blocked_not_desired",
        };
    }
    if !execute_requested {
        return ServiceDeploymentReconciliationGate {
            allowed: false,
            status: "suggested",
        };
    }
    if !control_plane_authorized {
        return ServiceDeploymentReconciliationGate {
            allowed: false,
            status: "blocked_control_plane",
        };
    }
    if !local_container_execution_allowed {
        return ServiceDeploymentReconciliationGate {
            allowed: false,
            status: "blocked_local_policy",
        };
    }
    ServiceDeploymentReconciliationGate {
        allowed: true,
        status: "authorized",
    }
}

pub fn reconcile_service_deployment_from_artifact(
    artifact: &ServiceArtifact,
    execute_requested: bool,
    desired_in_active_slice: bool,
    control_plane_authorized: bool,
    local_container_execution_allowed: bool,
) -> Result<ServiceDeploymentReconciliationReport> {
    validate_service_artifact(artifact)?;
    if artifact.runtime.kind.trim() != "container" {
        bail!("service deployment reconciliation supports container runtime artifacts only");
    }

    let drift_before = assess_service_deployment_drift(artifact)?;
    if !drift_before.drift_detected {
        return Ok(build_report(
            artifact,
            execute_requested,
            desired_in_active_slice,
            control_plane_authorized,
            local_container_execution_allowed,
            "converged",
            false,
            drift_before.clone(),
            Some(drift_before),
            Vec::new(),
        ));
    }

    // Missing/stopped execution remains owned by the existing operational/WASM reconciler.
    // This artifact reconciler only owns configuration/deployment drift on an existing container.
    if drift_before.observed.is_none() {
        return Ok(build_report(
            artifact,
            execute_requested,
            desired_in_active_slice,
            control_plane_authorized,
            local_container_execution_allowed,
            "deferred_execution_reconciler",
            false,
            drift_before,
            None,
            Vec::new(),
        ));
    }

    let gate = evaluate_service_deployment_reconciliation_gate(
        execute_requested,
        desired_in_active_slice,
        control_plane_authorized,
        local_container_execution_allowed,
    );
    if !gate.allowed {
        return Ok(build_report(
            artifact,
            execute_requested,
            desired_in_active_slice,
            control_plane_authorized,
            local_container_execution_allowed,
            gate.status,
            false,
            drift_before,
            None,
            Vec::new(),
        ));
    }

    let container = artifact
        .runtime
        .container
        .as_ref()
        .expect("validated container artifact must carry runtime.container");

    let mut command_results = Vec::new();
    let remove_args = vec![
        "rm".to_string(),
        "-f".to_string(),
        container.container_name.trim().to_string(),
    ];
    let remove_output = run_docker(&remove_args).with_context(|| {
        format!(
            "remove drifted container '{}' before trusted artifact redeployment",
            container.container_name
        )
    })?;
    let remove_result = command_result("remove_existing", &remove_args, &remove_output);
    if !remove_result.success {
        command_results.push(remove_result);
        return Ok(build_report(
            artifact,
            execute_requested,
            desired_in_active_slice,
            control_plane_authorized,
            local_container_execution_allowed,
            "failed_remove_existing",
            false,
            drift_before,
            None,
            command_results,
        ));
    }
    command_results.push(remove_result);

    let run_args = build_docker_run_args(artifact)?;
    let run_output = run_docker(&run_args).with_context(|| {
        format!(
            "redeploy service '{}' from trusted artifact '{}'",
            artifact.service_id, artifact.artifact_id
        )
    })?;
    let run_result = command_result("deploy_expected", &run_args, &run_output);
    let run_success = run_result.success;
    command_results.push(run_result);

    let drift_after = assess_service_deployment_drift(artifact).ok();
    let status = if !run_success {
        "failed_deploy_expected"
    } else if drift_after
        .as_ref()
        .is_some_and(|assessment| !assessment.drift_detected)
    {
        "reconciled"
    } else {
        "reconcile_incomplete"
    };

    Ok(build_report(
        artifact,
        execute_requested,
        desired_in_active_slice,
        control_plane_authorized,
        local_container_execution_allowed,
        status,
        true,
        drift_before,
        drift_after,
        command_results,
    ))
}

pub fn build_docker_run_args(artifact: &ServiceArtifact) -> Result<Vec<String>> {
    validate_service_artifact(artifact)?;
    let container = artifact
        .runtime
        .container
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("container runtime requires runtime.container"))?;

    let mut args = vec![
        "run".to_string(),
        "-d".to_string(),
        "--name".to_string(),
        container.container_name.trim().to_string(),
    ];

    for mapping in &container.ports {
        let Some(host_port) = mapping.host_port else {
            continue;
        };
        args.push("-p".to_string());
        args.push(format!(
            "{}:{}/{}",
            host_port,
            mapping.container_port,
            mapping.protocol.trim().to_ascii_lowercase()
        ));
    }

    args.push(container.image.trim().to_string());
    Ok(args)
}

#[allow(clippy::too_many_arguments)]
fn build_report(
    artifact: &ServiceArtifact,
    execute_requested: bool,
    desired_in_active_slice: bool,
    control_plane_authorized: bool,
    local_container_execution_allowed: bool,
    status: &str,
    side_effects_performed: bool,
    drift_before: ServiceDeploymentDriftAssessment,
    drift_after: Option<ServiceDeploymentDriftAssessment>,
    command_results: Vec<ServiceDeploymentCommandResult>,
) -> ServiceDeploymentReconciliationReport {
    ServiceDeploymentReconciliationReport {
        schema_version: SERVICE_DEPLOYMENT_RECONCILIATION_SCHEMA_VERSION.to_string(),
        reconciled_at_utc: Utc::now().to_rfc3339(),
        artifact_id: artifact.artifact_id.clone(),
        service_id: artifact.service_id.clone(),
        runtime_kind: artifact.runtime.kind.clone(),
        execute_requested,
        desired_in_active_slice,
        control_plane_authorized,
        local_container_execution_allowed,
        execution_authorized: execute_requested
            && desired_in_active_slice
            && control_plane_authorized
            && local_container_execution_allowed,
        status: status.to_string(),
        side_effects_performed,
        drift_before,
        drift_after,
        command_results,
    }
}

fn run_docker(args: &[String]) -> Result<Output> {
    Command::new("docker")
        .args(args)
        .output()
        .with_context(|| format!("execute docker {}", args.join(" ")))
}

fn command_result(phase: &str, args: &[String], output: &Output) -> ServiceDeploymentCommandResult {
    ServiceDeploymentCommandResult {
        phase: phase.to_string(),
        program: "docker".to_string(),
        args: args.to_vec(),
        success: output.status.success(),
        detail: combined_output(output),
    }
}

fn combined_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout={stdout}; stderr={stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => format!("exit_status={}", output.status),
    }
}
