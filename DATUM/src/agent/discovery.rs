use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const DISCOVERY_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapMode {
    KnownNode,
    Discovery,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartSentinelDiscovery {
    pub schema_version: String,
    pub observed_at_utc: DateTime<Utc>,
    pub node_id: String,
    pub host_id: String,
    pub bootstrap_mode: BootstrapMode,
    pub identity: IdentityObservation,
    pub platform: PlatformObservation,
    pub resources: ResourceObservation,
    pub capabilities: Vec<CapabilityObservation>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
}

impl SmartSentinelDiscovery {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version.trim().is_empty() {
            bail!("discovery schema_version must not be empty");
        }
        if self.node_id.trim().is_empty() {
            bail!("discovery node_id must not be empty");
        }
        if self.host_id.trim().is_empty() {
            bail!("discovery host_id must not be empty");
        }
        if self.identity.hostname.trim().is_empty() {
            bail!("discovery hostname must not be empty");
        }
        if self.platform.os_family.trim().is_empty() {
            bail!("discovery os_family must not be empty");
        }
        if self.platform.arch.trim().is_empty() {
            bail!("discovery architecture must not be empty");
        }
        if self.resources.cpu.logical_cores == 0 {
            bail!("discovery cpu.logical_cores must be greater than zero");
        }
        validate_memory(&self.resources.memory)?;
        validate_swap(&self.resources.swap)?;
        for fs in &self.resources.filesystems {
            if fs.mount.trim().is_empty() {
                bail!("discovery filesystem mount must not be empty");
            }
            if fs.used_bytes.saturating_add(fs.available_bytes) > fs.total_bytes {
                bail!("discovery filesystem usage exceeds total capacity for {}", fs.mount);
            }
        }
        for capability in &self.capabilities {
            if capability.name.trim().is_empty() {
                bail!("discovery capability name must not be empty");
            }
            if capability.usable && !capability.present {
                bail!(
                    "discovery capability {} cannot be usable when it is not present",
                    capability.name
                );
            }
        }
        if self.side_effects_performed {
            bail!("discovery must remain observation-only");
        }
        Ok(())
    }
}

fn validate_memory(memory: &MemoryObservation) -> Result<()> {
    if memory.total_bytes == 0 {
        bail!("discovery memory.total_bytes must be greater than zero");
    }
    if memory.used_bytes > memory.total_bytes {
        bail!("discovery memory.used_bytes exceeds total_bytes");
    }
    if memory.available_bytes > memory.total_bytes {
        bail!("discovery memory.available_bytes exceeds total_bytes");
    }
    Ok(())
}

fn validate_swap(swap: &SwapObservation) -> Result<()> {
    if swap.used_bytes > swap.total_bytes {
        bail!("discovery swap.used_bytes exceeds total_bytes");
    }
    if swap.available_bytes > swap.total_bytes {
        bail!("discovery swap.available_bytes exceeds total_bytes");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentityObservation {
    pub hostname: String,
    pub sentinel_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformObservation {
    pub os_family: String,
    pub os_name: String,
    pub os_version: String,
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceObservation {
    pub uptime_seconds: Option<u64>,
    pub cpu: CpuObservation,
    pub memory: MemoryObservation,
    pub swap: SwapObservation,
    pub filesystems: Vec<FilesystemObservation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CpuObservation {
    pub logical_cores: usize,
    pub usage_percent: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryObservation {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwapObservation {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemObservation {
    pub mount: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityCategory {
    LanguageRuntime,
    ContainerRuntime,
    ExecutionRuntime,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    Observed,
    Embedded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityObservation {
    pub name: String,
    pub category: CapabilityCategory,
    pub present: bool,
    pub usable: bool,
    pub version: Option<String>,
    pub source: CapabilitySource,
    pub detail: Option<String>,
}
