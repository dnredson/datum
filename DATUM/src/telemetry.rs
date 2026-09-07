use std::{
    fs,
    net::{TcpStream, ToSocketAddrs},
    path::Path,
    time::Instant,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{config::PeerTargetConfig, core::model::Snapshot};

pub const SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmartSentinelTelemetryEnvelope {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub node_id: String,
    pub cycle_id: String,
    pub sentinel_instance_id: String,
    pub sequence: u64,
    pub observed_at_utc: DateTime<Utc>,
    pub host: HostTelemetry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<RuntimeTelemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkTelemetry>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HostTelemetry {
    pub hostname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime_seconds: Option<u64>,
    pub cpu: CpuTelemetry,
    pub memory: MemoryTelemetry,
    pub swap: SwapTelemetry,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filesystems: Vec<FilesystemTelemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_count: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CpuTelemetry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_cores: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_1: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_5: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_15: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct MemoryTelemetry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SwapTelemetry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FilesystemTelemetry {
    pub mount: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RuntimeTelemetry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_runtime_available: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_runtime_error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<ContainerTelemetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub processes: Vec<ProcessTelemetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<ServiceTelemetry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ContainerTelemetry {
    pub entity_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProcessTelemetry {
    pub entity_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_percent_total: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_used_kb_total: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ServiceTelemetry {
    pub entity_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desired_state: Option<String>,
    pub observed_state: String,
    pub bound: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<ServiceCheckTelemetry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ServiceCheckTelemetry {
    pub kind: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NetworkTelemetry {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<NetworkInterfaceTelemetry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_plane: Option<ControlPlaneTelemetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probes: Vec<NetworkProbeTelemetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<PeerTelemetry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NetworkInterfaceTelemetry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rx_errors: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_errors: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ControlPlaneTelemetry {
    pub target: String,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NetworkProbeTelemetry {
    pub target: String,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loss_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_min_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_avg_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_max_ms: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PeerTelemetry {
    pub node_id: String,
    pub target: String,
    pub reachable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<f64>,
    pub consecutive_failures: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at_utc: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SmartSentinelTelemetryEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        required("schema_version", &self.schema_version)?;
        if self.schema_version != SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION {
            return Err(format!(
                "unsupported SmartSentinel telemetry schema_version '{}'; expected '{}'",
                self.schema_version, SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION
            ));
        }
        required("node_id", &self.node_id)?;
        required("sentinel_instance_id", &self.sentinel_instance_id)?;
        if self.sequence == 0 {
            return Err("SmartSentinel telemetry sequence must be greater than zero".to_string());
        }
        required("host.hostname", &self.host.hostname)?;
        if let Some(project_id) = &self.project_id {
            required("project_id", project_id)?;
        }
        if let Some(usage) = self.host.cpu.usage_percent {
            if !usage.is_finite() || !(0.0..=100.0).contains(&usage) {
                return Err(format!(
                    "SmartSentinel telemetry CPU usage_percent must be between 0 and 100, received {usage}"
                ));
            }
        }
        validate_capacity(
            "memory",
            self.host.memory.total_bytes,
            self.host.memory.used_bytes,
            self.host.memory.available_bytes,
        )?;
        validate_capacity(
            "swap",
            self.host.swap.total_bytes,
            self.host.swap.used_bytes,
            None,
        )?;
        if let Some(runtime) = &self.runtime {
            for process in &runtime.processes {
                if let Some(cpu) = process.cpu_percent_total {
                    if !cpu.is_finite() || cpu < 0.0 {
                        return Err(format!(
                            "runtime process CPU must be finite and non-negative for '{}'",
                            process.name
                        ));
                    }
                }
            }
        }
        if let Some(network) = &self.network {
            if let Some(control_plane) = &network.control_plane {
                if let Some(rtt) = control_plane.rtt_ms {
                    if !rtt.is_finite() || rtt < 0.0 {
                        return Err("control-plane RTT must be finite and non-negative".to_string());
                    }
                }
            }
            for probe in &network.probes {
                if let Some(loss) = probe.loss_percent {
                    if !loss.is_finite() || !(0.0..=100.0).contains(&loss) {
                        return Err(format!(
                            "network probe loss_percent must be between 0 and 100 for '{}'",
                            probe.target
                        ));
                    }
                }
            }
            for peer in &network.peers {
                required("network.peer.node_id", &peer.node_id)?;
                required("network.peer.target", &peer.target)?;
                if let Some(rtt) = peer.rtt_ms {
                    if !rtt.is_finite() || rtt < 0.0 {
                        return Err(format!(
                            "network peer RTT must be finite and non-negative for '{}'",
                            peer.node_id
                        ));
                    }
                }
            }
        }
        for filesystem in &self.host.filesystems {
            required("filesystem.mount", &filesystem.mount)?;
            validate_capacity(
                "filesystem",
                filesystem.total_bytes,
                filesystem.used_bytes,
                filesystem.available_bytes,
            )?;
        }
        if self.safe_to_auto_execute || self.side_effects_performed {
            return Err(
                "observational SmartSentinel telemetry must disable auto-execution and side effects"
                    .to_string(),
            );
        }
        Ok(())
    }
}

pub fn attach_host_telemetry(
    snapshot: &mut Snapshot,
    project_id: Option<&str>,
    sequence: u64,
) -> Result<SmartSentinelTelemetryEnvelope, String> {
    attach_host_telemetry_with_control_plane(snapshot, project_id, sequence, None)
}

pub fn attach_host_telemetry_with_control_plane(
    snapshot: &mut Snapshot,
    project_id: Option<&str>,
    sequence: u64,
    control_plane_url: Option<&str>,
) -> Result<SmartSentinelTelemetryEnvelope, String> {
    attach_host_telemetry_with_peers(snapshot, project_id, sequence, control_plane_url, &[])
}

pub fn attach_host_telemetry_with_peers(
    snapshot: &mut Snapshot,
    project_id: Option<&str>,
    sequence: u64,
    control_plane_url: Option<&str>,
    peer_targets: &[PeerTargetConfig],
) -> Result<SmartSentinelTelemetryEnvelope, String> {
    let mut telemetry = build_host_telemetry(snapshot, project_id, sequence)?;
    if let Some(network) = telemetry.network.as_mut() {
        network.control_plane = control_plane_url.map(probe_control_plane);
        network.peers = probe_peer_targets(&snapshot.meta.node_id, peer_targets);
    }
    telemetry.validate()?;
    let value = serde_json::to_value(&telemetry)
        .map_err(|error| format!("serialize SmartSentinel telemetry: {error}"))?;

    let host_id = snapshot.meta.host_id.clone();
    let host = snapshot
        .entities
        .iter_mut()
        .find(|entity| entity.id == host_id)
        .ok_or_else(|| "SmartSentinel telemetry requires a host entity".to_string())?;

    if !host.attrs.is_object() {
        host.attrs = json!({});
    }
    host.attrs
        .as_object_mut()
        .expect("host attrs converted to object")
        .insert("telemetry".to_string(), value);

    Ok(telemetry)
}

pub fn build_host_telemetry(
    snapshot: &Snapshot,
    project_id: Option<&str>,
    sequence: u64,
) -> Result<SmartSentinelTelemetryEnvelope, String> {
    let host = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == snapshot.meta.host_id)
        .ok_or_else(|| "SmartSentinel telemetry requires a host entity".to_string())?;

    let system = host.attrs.get("system").filter(|value| value.is_object());
    let hostname = host
        .attrs
        .get("hostname")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();

    let telemetry = SmartSentinelTelemetryEnvelope {
        schema_version: SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION.to_string(),
        project_id: project_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string),
        node_id: snapshot.meta.node_id.clone(),
        cycle_id: snapshot.meta.cycle_id.clone(),
        sentinel_instance_id: format!("{}-sentinel", snapshot.meta.node_id),
        sequence,
        observed_at_utc: snapshot.meta.observed_at_utc,
        host: HostTelemetry {
            hostname,
            os: system
                .and_then(|value| value.get("os_name").or_else(|| value.get("os_caption")))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            kernel: system
                .and_then(|value| value.get("kernel"))
                .and_then(Value::as_str)
                .map(ToString::to_string),
            uptime_seconds: system.and_then(|value| value.get("uptime_seconds")).and_then(as_u64),
            cpu: CpuTelemetry {
                logical_cores: system
                    .and_then(|value| value.get("cpu_logical"))
                    .and_then(as_u64),
                usage_percent: system
                    .and_then(|value| value.get("cpu_usage_pct"))
                    .and_then(Value::as_f64),
                load_1: system
                    .and_then(|value| value.get("load_1"))
                    .and_then(Value::as_f64),
                load_5: system
                    .and_then(|value| value.get("load_5"))
                    .and_then(Value::as_f64),
                load_15: system
                    .and_then(|value| value.get("load_15"))
                    .and_then(Value::as_f64),
            },
            memory: MemoryTelemetry {
                total_bytes: system
                    .and_then(|value| value.get("mem_total_bytes"))
                    .and_then(as_u64),
                used_bytes: system
                    .and_then(|value| value.get("mem_used_bytes"))
                    .and_then(as_u64),
                available_bytes: system
                    .and_then(|value| value.get("mem_available_bytes"))
                    .and_then(as_u64),
            },
            swap: SwapTelemetry {
                total_bytes: system
                    .and_then(|value| value.get("swap_total_bytes"))
                    .and_then(as_u64),
                used_bytes: system
                    .and_then(|value| value.get("swap_used_bytes"))
                    .and_then(as_u64),
            },
            filesystems: system
                .and_then(|value| value.get("filesystems"))
                .and_then(Value::as_array)
                .map(|filesystems| {
                    filesystems
                        .iter()
                        .filter_map(|filesystem| {
                            Some(FilesystemTelemetry {
                                mount: filesystem
                                    .get("mount")
                                    .and_then(Value::as_str)?
                                    .trim()
                                    .to_string(),
                                total_bytes: filesystem
                                    .get("total_bytes")
                                    .and_then(as_u64),
                                used_bytes: filesystem
                                    .get("used_bytes")
                                    .and_then(as_u64),
                                available_bytes: filesystem
                                    .get("available_bytes")
                                    .and_then(as_u64),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            process_count: system
                .and_then(|value| value.get("process_count"))
                .and_then(as_u64),
        },
        runtime: Some(build_runtime_telemetry(snapshot)),
        network: Some(build_network_telemetry(snapshot, None)),
        safe_to_auto_execute: false,
        side_effects_performed: false,
    };

    telemetry.validate()?;
    Ok(telemetry)
}

pub fn write_telemetry_atomic(
    path: &Path,
    telemetry: &SmartSentinelTelemetryEnvelope,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(telemetry)
        .map_err(|error| format!("serialize SmartSentinel telemetry: {error}"))?;
    let temporary = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("json")
    ));
    fs::write(&temporary, format!("{content}\n"))
        .map_err(|error| format!("write {}: {error}", temporary.display()))?;
    if path.exists() {
        fs::remove_file(path)
            .map_err(|error| format!("remove {}: {error}", path.display()))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| format!("rename {} to {}: {error}", temporary.display(), path.display()))
}

fn build_runtime_telemetry(snapshot: &Snapshot) -> RuntimeTelemetry {
    let containers = snapshot
        .entities
        .iter()
        .filter(|entity| entity.kind == "container")
        .map(|entity| ContainerTelemetry {
            entity_id: entity.id.clone(),
            name: entity
                .attrs
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            image: entity
                .attrs
                .get("image")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            state: entity
                .attrs
                .get("state")
                .and_then(Value::as_str)
                .map(ToString::to_string),
            status: entity
                .attrs
                .get("status")
                .and_then(Value::as_str)
                .map(ToString::to_string),
        })
        .collect::<Vec<_>>();

    let processes = snapshot
        .entities
        .iter()
        .filter(|entity| entity.kind == "process")
        .map(|entity| ProcessTelemetry {
            entity_id: entity.id.clone(),
            name: entity
                .attrs
                .get("name")
                .or_else(|| entity.attrs.get("image_name"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            instance_count: entity
                .attrs
                .get("instance_count")
                .and_then(as_u64),
            cpu_percent_total: entity
                .attrs
                .get("cpu_pct_total")
                .or_else(|| entity.attrs.get("cpu_percent_total"))
                .and_then(Value::as_f64),
            memory_used_kb_total: entity
                .attrs
                .get("mem_used_kb_total")
                .or_else(|| entity.attrs.get("memory_used_kb_total"))
                .and_then(as_u64),
        })
        .collect::<Vec<_>>();

    let services = snapshot
        .entities
        .iter()
        .filter(|entity| entity.kind == "service")
        .map(|entity| {
            let checks = entity
                .attrs
                .get("checks_result")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|check| ServiceCheckTelemetry {
                            kind: check
                                .get("kind")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string(),
                            outcome: check
                                .get("outcome")
                                .or_else(|| check.get("status"))
                                .and_then(Value::as_str)
                                .unwrap_or("unknown")
                                .to_string(),
                            detail: check
                                .get("detail")
                                .or_else(|| check.get("message"))
                                .and_then(Value::as_str)
                                .map(ToString::to_string),
                        })
                        .collect()
                })
                .unwrap_or_default();

            ServiceTelemetry {
                entity_id: entity.id.clone(),
                name: entity
                    .attrs
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                runtime: entity
                    .attrs
                    .get("runtime")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                desired_state: entity
                    .attrs
                    .get("desired_state")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                observed_state: entity
                    .attrs
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                bound: entity
                    .attrs
                    .get("binding")
                    .and_then(|value| value.get("bound"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                checks,
            }
        })
        .collect::<Vec<_>>();

    let host_system = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == snapshot.meta.host_id)
        .and_then(|entity| entity.attrs.get("system"));

    RuntimeTelemetry {
        container_runtime: host_system
            .and_then(|system| system.get("container_runtime"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        container_runtime_available: host_system
            .and_then(|system| system.get("container_runtime_available"))
            .and_then(Value::as_bool),
        container_count: host_system
            .and_then(|system| system.get("container_count"))
            .and_then(as_u64)
            .or_else(|| (!containers.is_empty()).then_some(containers.len() as u64)),
        container_runtime_error: host_system
            .and_then(|system| system.get("container_runtime_error"))
            .and_then(Value::as_str)
            .map(ToString::to_string),
        containers,
        processes,
        services,
    }
}

fn build_network_telemetry(
    snapshot: &Snapshot,
    control_plane_url: Option<&str>,
) -> NetworkTelemetry {
    let host_system = snapshot
        .entities
        .iter()
        .find(|entity| entity.id == snapshot.meta.host_id)
        .and_then(|entity| entity.attrs.get("system"));

    let interfaces = host_system
        .and_then(|system| system.get("interfaces"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(NetworkInterfaceTelemetry {
                        name: item.get("name").and_then(Value::as_str)?.to_string(),
                        up: item.get("up").and_then(Value::as_bool),
                        addresses: item
                            .get("addresses")
                            .and_then(Value::as_array)
                            .map(|values| {
                                values
                                    .iter()
                                    .filter_map(Value::as_str)
                                    .map(ToString::to_string)
                                    .collect()
                            })
                            .unwrap_or_default(),
                        rx_bytes: item.get("rx_bytes").and_then(as_u64),
                        tx_bytes: item.get("tx_bytes").and_then(as_u64),
                        rx_errors: item.get("rx_errors").and_then(as_u64),
                        tx_errors: item.get("tx_errors").and_then(as_u64),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let probes = snapshot
        .entities
        .iter()
        .filter(|entity| entity.kind == "connection")
        .filter_map(|entity| {
            Some(NetworkProbeTelemetry {
                target: entity
                    .attrs
                    .get("to")
                    .or_else(|| entity.attrs.get("target"))
                    .and_then(Value::as_str)?
                    .to_string(),
                reachable: entity
                    .attrs
                    .get("available")
                    .or_else(|| entity.attrs.get("reachable"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                loss_percent: entity
                    .attrs
                    .get("loss_pct")
                    .or_else(|| entity.attrs.get("loss_percent"))
                    .and_then(Value::as_f64)
                    .or_else(|| entity.attrs.get("loss_pct").and_then(as_u64).map(|v| v as f64)),
                rtt_min_ms: entity
                    .attrs
                    .get("rtt_min_ms")
                    .and_then(Value::as_f64),
                rtt_avg_ms: entity
                    .attrs
                    .get("rtt_avg_ms")
                    .and_then(Value::as_f64),
                rtt_max_ms: entity
                    .attrs
                    .get("rtt_max_ms")
                    .and_then(Value::as_f64),
            })
        })
        .collect();

    let mut network = NetworkTelemetry {
        interfaces,
        control_plane: None,
        probes,
        peers: Vec::new(),
    };
    if let Some(url) = control_plane_url {
        network.control_plane = Some(probe_control_plane(url));
    }
    network
}

fn probe_control_plane(url: &str) -> ControlPlaneTelemetry {
    let target = match reqwest::Url::parse(url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("").to_string();
            let port = parsed
                .port_or_known_default()
                .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
            format!("{host}:{port}")
        }
        Err(error) => {
            return ControlPlaneTelemetry {
                target: url.to_string(),
                reachable: false,
                rtt_ms: None,
                error: Some(format!("invalid control-plane URL: {error}")),
            };
        }
    };

    let (host, port) = target
        .rsplit_once(':')
        .map(|(host, port)| (host, port.parse::<u16>().unwrap_or(80)))
        .unwrap_or((target.as_str(), 80));
    let started = Instant::now();
    let result = (host, port)
        .to_socket_addrs()
        .ok()
        .and_then(|addresses| {
            addresses
                .filter_map(|address| TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2)).ok())
                .next()
        });

    match result {
        Some(_) => ControlPlaneTelemetry {
            target,
            reachable: true,
            rtt_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            error: None,
        },
        None => ControlPlaneTelemetry {
            target,
            reachable: false,
            rtt_ms: None,
            error: Some("TCP control-plane connection failed".to_string()),
        },
    }
}

fn probe_peer_targets(current_node_id: &str, targets: &[PeerTargetConfig]) -> Vec<PeerTelemetry> {
    targets
        .iter()
        .filter_map(|peer| {
            let node_id = peer.node_id.trim();
            let configured_target = peer.target.trim();
            if node_id.is_empty() || configured_target.is_empty() || node_id == current_node_id {
                return None;
            }

            let (host, port) = split_probe_target(configured_target, peer.port.unwrap_or(80));
            let started = Instant::now();
            let result = (host.as_str(), port)
                .to_socket_addrs()
                .ok()
                .and_then(|addresses| {
                    addresses
                        .filter_map(|address| {
                            TcpStream::connect_timeout(
                                &address,
                                std::time::Duration::from_secs(
                                    u64::from(peer.timeout_s.unwrap_or(2)),
                                ),
                            )
                            .ok()
                        })
                        .next()
                });
            let reachable = result.is_some();
            PeerTelemetry {
                node_id: node_id.to_string(),
                target: configured_target.to_string(),
                reachable,
                rtt_ms: reachable.then(|| started.elapsed().as_secs_f64() * 1000.0),
                consecutive_failures: u64::from(!reachable),
                last_success_at_utc: reachable.then_some(Utc::now()),
                error: (!reachable).then(|| "TCP peer probe failed".to_string()),
            }
        })
        .collect()
}

fn split_probe_target(target: &str, default_port: u16) -> (String, u16) {
    if let Some((host, port)) = target.rsplit_once(':') {
        if let Ok(port) = port.parse::<u16>() {
            return (host.to_string(), port);
        }
    }
    (target.to_string(), default_port)
}

fn as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_f64().filter(|value| value.is_finite() && *value >= 0.0).map(|value| value.round() as u64))
}

fn validate_capacity(
    label: &str,
    total: Option<u64>,
    used: Option<u64>,
    available: Option<u64>,
) -> Result<(), String> {
    if let (Some(total), Some(used)) = (total, used) {
        if used > total {
            return Err(format!("{label} used_bytes cannot exceed total_bytes"));
        }
    }
    if let (Some(total), Some(available)) = (total, available) {
        if available > total {
            return Err(format!("{label} available_bytes cannot exceed total_bytes"));
        }
    }
    Ok(())
}

fn required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("SmartSentinel telemetry {label} must not be empty"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Entity, Meta};

    fn snapshot() -> Snapshot {
        Snapshot {
            meta: Meta {
                schema_version: "0.0.1".to_string(),
                agent_version: "test".to_string(),
                cycle_id: "cycle:test".to_string(),
                observed_at_utc: "2026-09-07T22:00:00Z".parse().unwrap(),
                node_id: "fog-01".to_string(),
                host_id: "host:fog-01".to_string(),
            },
            entities: vec![Entity {
                id: "host:fog-01".to_string(),
                kind: "host".to_string(),
                attrs: json!({
                    "hostname":"xubuntu",
                    "system": {
                        "os_name":"Ubuntu",
                        "kernel":"6.17.0",
                        "uptime_seconds":1200,
                        "cpu_logical":4,
                        "cpu_usage_pct":37.5,
                        "mem_total_bytes":8589934592u64,
                        "mem_used_bytes":3221225472u64,
                        "mem_available_bytes":5368709120u64,
                        "swap_total_bytes":2147483648u64,
                        "swap_used_bytes":0u64,
                        "process_count":132u64,
                        "filesystems":[{"mount":"/","total_bytes":1000u64,"used_bytes":400u64,"available_bytes":600u64}]
                    }
                }),
                observed_at_utc: "2026-09-07T22:00:00Z".parse().unwrap(),
                ttl_s: 60,
            }],
            edges: vec![],
            events: vec![],
            actions: vec![],
        }
    }

    #[test]
    fn typed_host_telemetry_is_built_and_validated() {
        let telemetry = build_host_telemetry(&snapshot(), Some("project-a"), 1).unwrap();
        assert_eq!(telemetry.schema_version, SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION);
        assert_eq!(telemetry.host.cpu.logical_cores, Some(4));
        assert_eq!(telemetry.host.memory.available_bytes, Some(5368709120));
        assert_eq!(telemetry.host.filesystems.len(), 1);
        assert!(!telemetry.safe_to_auto_execute);
        assert!(!telemetry.side_effects_performed);
    }

    #[test]
    fn invalid_cpu_usage_is_rejected() {
        let mut value = snapshot();
        value.entities[0].attrs["system"]["cpu_usage_pct"] = json!(101.0);
        assert!(build_host_telemetry(&value, Some("project-a"), 1).is_err());
    }

    #[test]
    fn missing_system_metrics_remain_absent() {
        let mut value = snapshot();
        value.entities[0].attrs["system"] = json!({});
        let telemetry = build_host_telemetry(&value, Some("project-a"), 1).unwrap();
        assert!(telemetry.host.memory.total_bytes.is_none());
        assert!(telemetry.host.swap.total_bytes.is_none());
        assert!(telemetry.host.filesystems.is_empty());
    }

    #[test]
    fn host_telemetry_is_attached_under_a_namespaced_snapshot_field() {
        let mut value = snapshot();
        let telemetry = attach_host_telemetry(&mut value, Some("project-a"), 3).unwrap();
        assert_eq!(
            value.entities[0].attrs["telemetry"]["sequence"],
            json!(3)
        );
        assert_eq!(value.entities[0].attrs["telemetry"], serde_json::to_value(telemetry).unwrap());
    }

    #[test]
    fn zero_sequence_is_rejected() {
        assert!(build_host_telemetry(&snapshot(), Some("project-a"), 0).is_err());
    }
}
