use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use reqwest::blocking::Client;
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const SERVICE_ARTIFACT_SCHEMA_VERSION: &str = "0.1.0";
pub const SERVICE_ASSESSMENT_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceArtifact {
    pub schema_version: String,
    pub artifact_id: String,
    pub service_id: String,
    pub runtime: ServiceRuntimeSpec,
    pub probes: ServiceProbeSet,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceRuntimeSpec {
    pub kind: String,
    #[serde(default)]
    pub container: Option<ContainerDeploymentSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContainerDeploymentSpec {
    pub container_name: String,
    pub image: String,
    #[serde(default)]
    pub ports: Vec<ContainerPortMapping>,
    #[serde(default)]
    pub network: Option<String>,
    #[serde(default)]
    pub volumes: Vec<ContainerVolumeMount>,
    #[serde(default)]
    pub configuration_mounts: Vec<ContainerConfigurationMount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerVolumeMount {
    pub name: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerConfigurationMount {
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerPortMapping {
    pub container_port: u16,
    #[serde(default)]
    pub host_port: Option<u16>,
    #[serde(default = "default_tcp_protocol")]
    pub protocol: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceProbeSet {
    pub execution: ServiceProbeSpec,
    #[serde(default)]
    pub health: Option<ServiceProbeSpec>,
    #[serde(default)]
    pub availability: Option<ServiceProbeSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceProbeSpec {
    pub kind: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub vantage_point: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Passed,
    Failed,
    NotVerifiable,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceProbeResult {
    pub dimension: String,
    pub kind: String,
    pub outcome: ProbeOutcome,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vantage_point: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceOperationalAssessment {
    pub schema_version: String,
    pub observed_at_utc: String,
    pub artifact_id: String,
    pub service_id: String,
    pub runtime_kind: String,
    pub execution: ServiceProbeResult,
    pub health: ServiceProbeResult,
    pub availability: ServiceProbeResult,
    pub operational_state: String,
    pub assessment_basis: String,
}

pub fn load_service_artifact(path: &Path) -> Result<ServiceArtifact> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("read service artifact {}", path.display()))?;
    let artifact: ServiceArtifact = serde_json::from_str(&raw)
        .with_context(|| format!("parse service artifact {}", path.display()))?;
    validate_service_artifact(&artifact)?;
    Ok(artifact)
}

pub fn validate_service_artifact(artifact: &ServiceArtifact) -> Result<()> {
    if artifact.schema_version.trim() != SERVICE_ARTIFACT_SCHEMA_VERSION {
        bail!(
            "service artifact schema_version must be {}",
            SERVICE_ARTIFACT_SCHEMA_VERSION
        );
    }
    required("artifact_id", &artifact.artifact_id)?;
    required("service_id", &artifact.service_id)?;
    required("runtime.kind", &artifact.runtime.kind)?;

    if artifact.runtime.kind.trim() != "container" {
        bail!(
            "service artifact '{}' uses unsupported runtime kind '{}'; step 52 supports container artifacts",
            artifact.artifact_id,
            artifact.runtime.kind
        );
    }

    let container = artifact
        .runtime
        .container
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("container runtime requires runtime.container"))?;
    required("runtime.container.container_name", &container.container_name)?;
    required("runtime.container.image", &container.image)?;
    for mapping in &container.ports {
        if mapping.container_port == 0 {
            bail!("container_port must be greater than zero");
        }
        if mapping.host_port == Some(0) {
            bail!("host_port must be greater than zero when provided");
        }
        if !matches!(mapping.protocol.trim(), "tcp" | "udp") {
            bail!("container port protocol must be tcp or udp");
        }
    }

    validate_container_attachments(container)?;
    validate_probe("execution", &artifact.probes.execution)?;
    if let Some(probe) = &artifact.probes.health {
        validate_probe("health", probe)?;
    }
    if let Some(probe) = &artifact.probes.availability {
        validate_probe("availability", probe)?;
    }
    Ok(())
}

pub fn assess_service_artifact(artifact: &ServiceArtifact) -> Result<ServiceOperationalAssessment> {
    validate_service_artifact(artifact)?;

    let execution = run_probe(artifact, "execution", &artifact.probes.execution);
    let health = artifact
        .probes
        .health
        .as_ref()
        .map(|probe| run_probe(artifact, "health", probe))
        .unwrap_or_else(|| not_configured("health"));
    let availability = artifact
        .probes
        .availability
        .as_ref()
        .map(|probe| run_probe(artifact, "availability", probe))
        .unwrap_or_else(|| not_configured("availability"));

    let assessment_basis = match (
        artifact.probes.health.is_some(),
        artifact.probes.availability.is_some(),
    ) {
        (true, true) => "execution_health_availability",
        (true, false) => "execution_health",
        (false, true) => "execution_availability",
        (false, false) => "execution_only",
    }
    .to_string();

    let operational_state = derive_operational_state(&execution, &health, &availability);

    Ok(ServiceOperationalAssessment {
        schema_version: SERVICE_ASSESSMENT_SCHEMA_VERSION.to_string(),
        observed_at_utc: Utc::now().to_rfc3339(),
        artifact_id: artifact.artifact_id.clone(),
        service_id: artifact.service_id.clone(),
        runtime_kind: artifact.runtime.kind.clone(),
        execution,
        health,
        availability,
        operational_state,
        assessment_basis,
    })
}

fn derive_operational_state(
    execution: &ServiceProbeResult,
    health: &ServiceProbeResult,
    availability: &ServiceProbeResult,
) -> String {
    match execution.outcome {
        ProbeOutcome::Failed => return "not_running".to_string(),
        ProbeOutcome::Error => return "unknown".to_string(),
        ProbeOutcome::NotVerifiable => return "execution_not_verifiable".to_string(),
        ProbeOutcome::Passed => {}
    }

    match health.outcome {
        ProbeOutcome::Failed => return "unhealthy".to_string(),
        ProbeOutcome::Error => return "unknown".to_string(),
        _ => {}
    }

    match availability.outcome {
        ProbeOutcome::Failed => "unavailable".to_string(),
        ProbeOutcome::Error => "unknown".to_string(),
        ProbeOutcome::Passed => "available".to_string(),
        ProbeOutcome::NotVerifiable => {
            if health.outcome == ProbeOutcome::Passed {
                "healthy_availability_not_verifiable".to_string()
            } else {
                "running_not_fully_verifiable".to_string()
            }
        }
    }
}

fn validate_container_attachments(container: &ContainerDeploymentSpec) -> Result<()> {
    if let Some(network) = container.network.as_deref() {
        if !is_safe_docker_name(network) {
            bail!("runtime.container.network must be a non-empty Docker network name");
        }
    }
    for volume in &container.volumes {
        if !is_safe_docker_name(&volume.name) {
            bail!("runtime.container.volumes name must be a non-empty Docker volume name");
        }
        if !volume.target.starts_with('/')
            || volume.target.chars().any(char::is_whitespace)
            || volume.target.contains(',')
        {
            bail!("runtime.container.volumes target must be an absolute path without whitespace or commas");
        }
    }
    for mount in &container.configuration_mounts {
        if !is_safe_configuration_source(&mount.source) {
            bail!("runtime.container.configuration_mounts source must be a relative path without traversal");
        }
        if !mount.target.starts_with('/')
            || mount.target.chars().any(char::is_whitespace)
            || mount.target.contains(',')
        {
            bail!("runtime.container.configuration_mounts target must be an absolute path without whitespace or commas");
        }
        if !mount.read_only {
            bail!("runtime.container.configuration_mounts must be read-only");
        }
    }
    Ok(())
}

fn is_safe_configuration_source(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 512
        && !value.starts_with('/')
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '.'))
        && value.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn is_safe_docker_name(value: &str) {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-'))
        && value.chars().next().is_some_and(|character| character.is_ascii_alphanumeric())
}

fn validate_probe(dimension: &str, probe: &ServiceProbeSpec) -> Result<()> {
    let kind = probe.kind.trim();
    required(&format!("probes.{dimension}.kind"), kind)?;

    let allowed = match dimension {
        "execution" => matches!(
            kind,
            "docker_container_running" | "process_running" | "systemd_active"
        ),
        "health" => matches!(kind, "docker_health" | "tcp_connect" | "http" | "mqtt_connect"),
        "availability" => matches!(kind, "tcp_connect" | "http" | "mqtt_connect"),
        _ => false,
    };
    if !allowed {
        bail!("unsupported {dimension} probe kind '{kind}'");
    }

    match kind {
        "process_running" => {
            required_param_string(probe, "process_name")?;
        }
        "systemd_active" => {
            required_param_string(probe, "unit")?;
        }
        "tcp_connect" | "mqtt_connect" => {
            required_param_string(probe, "host")?;
            required_param_u16(probe, "port")?;
        }
        "http" => {
            required_param_string(probe, "url")?;
        }
        _ => {}
    }
    Ok(())
}

fn run_probe(artifact: &ServiceArtifact, dimension: &str, probe: &ServiceProbeSpec) -> ServiceProbeResult {
    let started = Instant::now();
    let result = match probe.kind.trim() {
        "docker_container_running" => probe_docker_running(artifact),
        "docker_health" => probe_docker_health(artifact),
        "process_running" => probe_process_running(probe),
        "systemd_active" => probe_systemd_active(probe),
        "tcp_connect" => probe_tcp_connect(probe),
        "http" => probe_http(probe),
        "mqtt_connect" => probe_mqtt_connect(probe),
        other => Err(anyhow::anyhow!("unsupported probe plugin '{other}'")),
    };

    match result {
        Ok(mut result) => {
            result.dimension = dimension.to_string();
            result.kind = probe.kind.clone();
            result.latency_ms = Some(started.elapsed().as_millis());
            result.vantage_point = probe.vantage_point.clone();
            result
        }
        Err(error) => ServiceProbeResult {
            dimension: dimension.to_string(),
            kind: probe.kind.clone(),
            outcome: ProbeOutcome::Error,
            detail: error.to_string(),
            latency_ms: Some(started.elapsed().as_millis()),
            vantage_point: probe.vantage_point.clone(),
            observed: None,
        },
    }
}

fn probe_docker_running(artifact: &ServiceArtifact) -> Result<ServiceProbeResult> {
    let container_name = container_spec(artifact)?.container_name.trim();
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", container_name])
        .output()
        .with_context(|| format!("execute docker running probe for '{container_name}'"))?;
    if !output.status.success() {
        let detail = combined_output(&output);
        return Ok(failed(format!(
            "docker could not confirm container '{container_name}' running: {detail}"
        )));
    }
    let observed = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if observed == "true" {
        Ok(passed(
            format!("container '{container_name}' is running"),
            Some(json!({"running":true,"container_name":container_name})),
        ))
    } else {
        Ok(failed(format!(
            "container '{container_name}' is not running (observed={observed})"
        )))
    }
}

fn probe_docker_health(artifact: &ServiceArtifact) -> Result<ServiceProbeResult> {
    let container_name = container_spec(artifact)?.container_name.trim();
    let output = Command::new("docker")
        .args(["inspect", "-f", "{{json .State.Health}}", container_name])
        .output()
        .with_context(|| format!("execute docker health probe for '{container_name}'"))?;
    if !output.status.success() {
        return Ok(failed(format!(
            "docker health inspection failed for '{container_name}': {}",
            combined_output(&output)
        )));
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() || raw == "null" || raw == "<no value>" {
        return Ok(not_verifiable(format!(
            "container '{container_name}' has no Docker HEALTHCHECK state"
        )));
    }
    let value: Value = serde_json::from_str(&raw).context("decode Docker health state")?;
    let status = value
        .get("Status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match status {
        "healthy" => Ok(passed(
            format!("container '{container_name}' Docker health status is healthy"),
            Some(value),
        )),
        "unhealthy" => Ok(ServiceProbeResult {
            dimension: String::new(),
            kind: String::new(),
            outcome: ProbeOutcome::Failed,
            detail: format!("container '{container_name}' Docker health status is unhealthy"),
            latency_ms: None,
            vantage_point: None,
            observed: Some(value),
        }),
        other => Ok(ServiceProbeResult {
            dimension: String::new(),
            kind: String::new(),
            outcome: ProbeOutcome::NotVerifiable,
            detail: format!("container '{container_name}' Docker health status is '{other}'"),
            latency_ms: None,
            vantage_point: None,
            observed: Some(value),
        }),
    }
}

fn probe_process_running(probe: &ServiceProbeSpec) -> Result<ServiceProbeResult> {
    let name = param_string(probe, "process_name")?;
    let output = Command::new("pgrep")
        .args(["-x", &name])
        .output()
        .with_context(|| format!("execute pgrep for '{name}'"))?;
    if output.status.success() {
        Ok(passed(
            format!("process '{name}' is running"),
            Some(json!({"pids":String::from_utf8_lossy(&output.stdout).trim()})),
        ))
    } else {
        Ok(failed(format!("process '{name}' is not running")))
    }
}

fn probe_systemd_active(probe: &ServiceProbeSpec) -> Result<ServiceProbeResult> {
    let unit = param_string(probe, "unit")?;
    let output = Command::new("systemctl")
        .args(["is-active", &unit])
        .output()
        .with_context(|| format!("execute systemd active probe for '{unit}'"))?;
    let state = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && state == "active" {
        Ok(passed(
            format!("systemd unit '{unit}' is active"),
            Some(json!({"state":state})),
        ))
    } else {
        Ok(failed(format!(
            "systemd unit '{unit}' is not active (state={state})"
        )))
    }
}

fn probe_tcp_connect(probe: &ServiceProbeSpec) -> Result<ServiceProbeResult> {
    let host = param_string(probe, "host")?;
    let port = param_u16(probe, "port")?;
    let timeout = Duration::from_millis(param_u64_default(probe, "timeout_ms", 2_000)?);
    match connect_tcp(&host, port, timeout) {
        Ok(_) => Ok(passed(
            format!("TCP connection to {host}:{port} succeeded"),
            Some(json!({"host":host,"port":port})),
        )),
        Err(error) => Ok(failed(format!(
            "TCP connection to {host}:{port} failed: {error}"
        ))),
    }
}

fn probe_http(probe: &ServiceProbeSpec) -> Result<ServiceProbeResult> {
    let url = param_string(probe, "url")?;
    let timeout_ms = param_u64_default(probe, "timeout_ms", 2_000)?;
    let expected_status = param_u64_default(probe, "expected_status", 200)? as u16;
    let method = probe
        .params
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("GET")
        .to_ascii_uppercase();
    let method = Method::from_bytes(method.as_bytes()).context("parse HTTP probe method")?;
    let client = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .context("build HTTP service probe client")?;
    match client.request(method, &url).send() {
        Ok(response) => {
            let status = response.status().as_u16();
            if status == expected_status {
                Ok(passed(
                    format!("HTTP probe {url} returned expected status {status}"),
                    Some(json!({"status":status,"url":url})),
                ))
            } else {
                Ok(failed(format!(
                    "HTTP probe {url} returned status {status}; expected {expected_status}"
                )))
            }
        }
        Err(error) => Ok(failed(format!("HTTP probe {url} failed: {error}"))),
    }
}

fn probe_mqtt_connect(probe: &ServiceProbeSpec) -> Result<ServiceProbeResult> {
    let host = param_string(probe, "host")?;
    let port = param_u16(probe, "port")?;
    let timeout = Duration::from_millis(param_u64_default(probe, "timeout_ms", 2_000)?);
    let mut stream = match connect_tcp(&host, port, timeout) {
        Ok(stream) => stream,
        Err(error) => {
            return Ok(failed(format!(
                "MQTT TCP connection to {host}:{port} failed: {error}"
            )))
        }
    };
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();

    let client_id = format!("smartsentinel-probe-{}", std::process::id());
    let packet = mqtt_connect_packet(&client_id)?;
    stream
        .write_all(&packet)
        .context("write MQTT CONNECT probe packet")?;
    let mut connack = [0u8; 4];
    stream
        .read_exact(&mut connack)
        .context("read MQTT CONNACK probe packet")?;
    if connack[0] != 0x20 || connack[1] != 0x02 {
        return Ok(failed(format!(
            "MQTT endpoint {host}:{port} returned invalid CONNACK header {:02x?}",
            connack
        )));
    }
    if connack[3] != 0 {
        return Ok(failed(format!(
            "MQTT endpoint {host}:{port} rejected CONNECT with return code {}",
            connack[3]
        )));
    }
    Ok(passed(
        format!("MQTT CONNECT/CONNACK succeeded against {host}:{port}"),
        Some(json!({"host":host,"port":port,"connack_return_code":0})),
    ))
}

fn mqtt_connect_packet(client_id: &str) -> Result<Vec<u8>> {
    let id = client_id.as_bytes();
    if id.len() > u16::MAX as usize {
        bail!("MQTT probe client id is too long");
    }
    let mut variable_and_payload = vec![
        0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x0A,
    ];
    variable_and_payload.extend_from_slice(&(id.len() as u16).to_be_bytes());
    variable_and_payload.extend_from_slice(id);
    if variable_and_payload.len() >= 128 {
        bail!("MQTT probe CONNECT packet exceeds single-byte remaining length");
    }
    let mut packet = vec![0x10, variable_and_payload.len() as u8];
    packet.extend_from_slice(&variable_and_payload);
    Ok(packet)
}

fn connect_tcp(host: &str, port: u16, timeout: Duration) -> Result<TcpStream> {
    let address = resolve_address(host, port)?;
    TcpStream::connect_timeout(&address, timeout)
        .with_context(|| format!("connect TCP {host}:{port}"))
}

fn resolve_address(host: &str, port: u16) -> Result<SocketAddr> {
    (host, port)
        .to_socket_addrs()
        .with_context(|| format!("resolve service probe address {host}:{port}"))?
        .next()
        .ok_or_else(|| anyhow::anyhow!("no address resolved for {host}:{port}"))
}

fn container_spec(artifact: &ServiceArtifact) -> Result<&ContainerDeploymentSpec> {
    artifact
        .runtime
        .container
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("service artifact has no container deployment spec"))
}

fn not_configured(dimension: &str) -> ServiceProbeResult {
    ServiceProbeResult {
        dimension: dimension.to_string(),
        kind: "not_configured".to_string(),
        outcome: ProbeOutcome::NotVerifiable,
        detail: format!("{dimension} probe is not configured; no stronger claim is made"),
        latency_ms: None,
        vantage_point: None,
        observed: None,
    }
}

fn passed(detail: String, observed: Option<Value>) -> ServiceProbeResult {
    ServiceProbeResult {
        dimension: String::new(),
        kind: String::new(),
        outcome: ProbeOutcome::Passed,
        detail,
        latency_ms: None,
        vantage_point: None,
        observed,
    }
}

fn failed(detail: String) -> ServiceProbeResult {
    ServiceProbeResult {
        dimension: String::new(),
        kind: String::new(),
        outcome: ProbeOutcome::Failed,
        detail,
        latency_ms: None,
        vantage_point: None,
        observed: None,
    }
}

fn not_verifiable(detail: String) -> ServiceProbeResult {
    ServiceProbeResult {
        dimension: String::new(),
        kind: String::new(),
        outcome: ProbeOutcome::NotVerifiable,
        detail,
        latency_ms: None,
        vantage_point: None,
        observed: None,
    }
}

fn required(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("service artifact {label} must not be empty");
    }
    Ok(())
}

fn required_param_string(probe: &ServiceProbeSpec, key: &str) -> Result<()> {
    param_string(probe, key).map(|_| ())
}

fn required_param_u16(probe: &ServiceProbeSpec, key: &str) -> Result<()> {
    param_u16(probe, key).map(|_| ())
}

fn param_string(probe: &ServiceProbeSpec, key: &str) -> Result<String> {
    probe
        .params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("probe '{}' requires non-empty string param '{key}'", probe.kind))
}

fn param_u16(probe: &ServiceProbeSpec, key: &str) -> Result<u16> {
    let value = probe
        .params
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow::anyhow!("probe '{}' requires integer param '{key}'", probe.kind))?;
    u16::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow::anyhow!("probe '{}' param '{key}' must be 1..65535", probe.kind))
}

fn param_u64_default(probe: &ServiceProbeSpec, key: &str, default: u64) -> Result<u64> {
    let value = probe
        .params
        .get(key)
        .map(|value| {
            value.as_u64().ok_or_else(|| {
                anyhow::anyhow!("probe '{}' param '{key}' must be an integer", probe.kind)
            })
        })
        .transpose()?
        .unwrap_or(default);
    if value == 0 {
        bail!("probe '{}' param '{key}' must be greater than zero", probe.kind);
    }
    Ok(value)
}

fn combined_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout={stdout}; stderr={stderr}"),
        (false, true) => stdout,
        (true, false) => stderr,
        (true, true) => format!("exit={}", output.status),
    }
}

fn default_tcp_protocol() -> String {
    "tcp".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mqtt_artifact() -> ServiceArtifact {
        ServiceArtifact {
            schema_version: SERVICE_ARTIFACT_SCHEMA_VERSION.to_string(),
            artifact_id: "service:mqtt:mosquitto-2".to_string(),
            service_id: "mqtt".to_string(),
            runtime: ServiceRuntimeSpec {
                kind: "container".to_string(),
                container: Some(ContainerDeploymentSpec {
                    container_name: "datum-mqtt".to_string(),
                    image: "eclipse-mosquitto:2".to_string(),
                    ports: vec![ContainerPortMapping {
                        container_port: 1883,
                        host_port: Some(1883),
                        protocol: "tcp".to_string(),
                    }],
                    network: None,
                    volumes: Vec::new(),
                    configuration_mounts: Vec::new(),
                }),
            },
            probes: ServiceProbeSet {
                execution: ServiceProbeSpec {
                    kind: "docker_container_running".to_string(),
                    params: BTreeMap::new(),
                    vantage_point: Some("local_host".to_string()),
                },
                health: Some(ServiceProbeSpec {
                    kind: "docker_health".to_string(),
                    params: BTreeMap::new(),
                    vantage_point: Some("local_runtime".to_string()),
                }),
                availability: Some(ServiceProbeSpec {
                    kind: "mqtt_connect".to_string(),
                    params: BTreeMap::from([
                        ("host".to_string(), Value::String("127.0.0.1".to_string())),
                        ("port".to_string(), Value::from(1883u64)),
                    ]),
                    vantage_point: Some("local_host".to_string()),
                }),
            },
            metadata: None,
        }
    }

    #[test]
    fn mqtt_service_artifact_contract_is_valid() {
        validate_service_artifact(&mqtt_artifact()).unwrap();
    }

    #[test]
    fn service_artifact_rejects_zero_host_port() {
        let mut artifact = mqtt_artifact();
        artifact.runtime.container.as_mut().unwrap().ports[0].host_port = Some(0);
        assert!(validate_service_artifact(&artifact).is_err());
    }

    #[test]
    fn mqtt_connect_packet_uses_protocol_level_4_and_clean_session() {
        let packet = mqtt_connect_packet("probe").unwrap();
        assert_eq!(packet[0], 0x10);
        assert_eq!(&packet[4..8], b"MQTT");
        assert_eq!(packet[8], 0x04);
        assert_eq!(packet[9], 0x02);
    }

    #[test]
    fn execution_passed_with_failed_availability_is_unavailable() {
        let execution = passed("running".to_string(), None);
        let health = not_verifiable("no health".to_string());
        let availability = failed("cannot consume".to_string());
        assert_eq!(
            derive_operational_state(&execution, &health, &availability),
            "unavailable"
        );
    }

    #[test]
    fn missing_availability_does_not_claim_available() {
        let execution = passed("running".to_string(), None);
        let health = not_verifiable("no health".to_string());
        let availability = not_configured("availability");
        assert_eq!(
            derive_operational_state(&execution, &health, &availability),
            "running_not_fully_verifiable"
        );
    }
}
