use std::process::Command;

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::service_artifact::{validate_service_artifact, ServiceArtifact};

pub const SERVICE_DEPLOYMENT_DRIFT_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeploymentDriftDifference {
    pub field: String,
    pub expected: Value,
    pub observed: Value,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceDeploymentDriftAssessment {
    pub schema_version: String,
    pub observed_at_utc: String,
    pub artifact_id: String,
    pub service_id: String,
    pub runtime_kind: String,
    pub status: String,
    pub drift_detected: bool,
    pub differences: Vec<DeploymentDriftDifference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Value>,
}

pub fn assess_service_deployment_drift(
    artifact: &ServiceArtifact,
) -> Result<ServiceDeploymentDriftAssessment> {
    validate_service_artifact(artifact)?;
    let container = artifact
        .runtime
        .container
        .as_ref()
        .expect("validated container artifact must carry runtime.container");

    let output = Command::new("docker")
        .args(["inspect", container.container_name.trim()])
        .output()
        .with_context(|| {
            format!(
                "execute docker inspect for deployment drift on '{}'",
                container.container_name
            )
        })?;

    let mut differences = Vec::new();
    if !output.status.success() {
        let detail = format!(
            "container '{}' is not inspectable: {}",
            container.container_name,
            combined_output(&output)
        );
        differences.push(DeploymentDriftDifference {
            field: "runtime.container.exists".to_string(),
            expected: json!(true),
            observed: json!(false),
            detail,
        });
        return Ok(build_assessment(
            artifact,
            "drift_detected",
            differences,
            None,
        ));
    }

    let inspect: Value = serde_json::from_slice(&output.stdout)
        .context("decode docker inspect JSON for deployment drift")?;
    let observed_container = inspect
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(Value::Null);

    let observed_image = observed_container
        .pointer("/Config/Image")
        .and_then(Value::as_str)
        .unwrap_or("");
    if observed_image != container.image.trim() {
        differences.push(DeploymentDriftDifference {
            field: "runtime.container.image".to_string(),
            expected: json!(container.image.trim()),
            observed: json!(observed_image),
            detail: format!(
                "container image differs: expected '{}', observed '{}'",
                container.image.trim(),
                observed_image
            ),
        });
    }

    for mapping in &container.ports {
        let Some(expected_host_port) = mapping.host_port else {
            continue;
        };
        let protocol = mapping.protocol.trim().to_ascii_lowercase();
        let port_key = format!("{}/{}", mapping.container_port, protocol);
        let port_value = observed_container
            .pointer("/NetworkSettings/Ports")
            .and_then(Value::as_object)
            .and_then(|ports| ports.get(&port_key));

        let observed_host_ports: Vec<u16> = port_value
            .and_then(Value::as_array)
            .map(|bindings| {
                bindings
                    .iter()
                    .filter_map(|binding| binding.get("HostPort"))
                    .filter_map(Value::as_str)
                    .filter_map(|value| value.parse::<u16>().ok())
                    .collect()
            })
            .unwrap_or_default();

        if !observed_host_ports.contains(&expected_host_port) {
            differences.push(DeploymentDriftDifference {
                field: format!("runtime.container.ports.{port_key}"),
                expected: json!({
                    "container_port": mapping.container_port,
                    "host_port": expected_host_port,
                    "protocol": protocol
                }),
                observed: json!({"host_ports": observed_host_ports}),
                detail: format!(
                    "required host mapping {}:{} / {} is absent",
                    expected_host_port, mapping.container_port, protocol
                ),
            });
        }
    }

    let status = if differences.is_empty() {
        "converged"
    } else {
        "drift_detected"
    };
    let observed = Some(json!({
        "container_name": container.container_name,
        "image": observed_image,
        "ports": observed_container.pointer("/NetworkSettings/Ports").cloned().unwrap_or(Value::Null)
    }));
    Ok(build_assessment(artifact, status, differences, observed))
}

fn build_assessment(
    artifact: &ServiceArtifact,
    status: &str,
    differences: Vec<DeploymentDriftDifference>,
    observed: Option<Value>,
) -> ServiceDeploymentDriftAssessment {
    ServiceDeploymentDriftAssessment {
        schema_version: SERVICE_DEPLOYMENT_DRIFT_SCHEMA_VERSION.to_string(),
        observed_at_utc: Utc::now().to_rfc3339(),
        artifact_id: artifact.artifact_id.clone(),
        service_id: artifact.service_id.clone(),
        runtime_kind: artifact.runtime.kind.clone(),
        status: status.to_string(),
        drift_detected: !differences.is_empty(),
        differences,
        observed,
    }
}

fn combined_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout={stdout}; stderr={stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => format!("exit_status={}", output.status),
    }
}
