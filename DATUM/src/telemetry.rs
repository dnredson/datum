use std::{fs, path::Path};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::model::Snapshot;

pub const SMARTSENTINEL_TELEMETRY_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmartSentinelTelemetryEnvelope {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    pub node_id: String,
    pub sentinel_instance_id: String,
    pub sequence: u64,
    pub observed_at_utc: DateTime<Utc>,
    pub host: HostTelemetry,
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
    let telemetry = build_host_telemetry(snapshot, project_id, sequence)?;
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
