use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DISCOVERY_SCHEMA_VERSION: &str = "0.4.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapMode {
    KnownNode,
    Discovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConfidence {
    Strong,
    Medium,
    Weak,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HardwareIdentityBasis {
    SystemUuid,
    BaseboardSerial,
    SystemSerial,
    BoardSerial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsIdentityBasis {
    MachineId,
    MachineGuid,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartSentinelDiscovery {
    pub schema_version: String,
    pub observed_at_utc: DateTime<Utc>,
    pub node_id: String,
    pub instance_id: String,
    pub host_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
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
        validate_identifier("node_id", &self.node_id)?;
        validate_identifier("instance_id", &self.instance_id)?;
        if self.host_id.trim().is_empty() {
            bail!("discovery host_id must not be empty");
        }
        if let Some(stage) = &self.stage {
            if stage.trim().is_empty() {
                bail!("discovery stage must be omitted or non-empty");
            }
        }
        if self.identity.hostname.trim().is_empty() {
            bail!("discovery hostname must not be empty");
        }
        if let Some(id) = &self.identity.hardware_fingerprint_id {
            validate_identifier("hardware_fingerprint_id", id)?;
            if self.identity.hardware_identity_basis.is_empty() {
                bail!("hardware fingerprint requires at least one hardware identity basis");
            }
            if self.identity.hardware_identity_confidence == IdentityConfidence::Unknown {
                bail!("hardware fingerprint cannot have unknown confidence");
            }
        } else {
            if !self.identity.hardware_identity_basis.is_empty() {
                bail!("hardware identity basis must be empty when no hardware fingerprint exists");
            }
            if self.identity.hardware_identity_confidence != IdentityConfidence::Unknown {
                bail!("missing hardware fingerprint must have unknown confidence");
            }
        }
        if let Some(id) = &self.identity.os_installation_id {
            validate_identifier("os_installation_id", id)?;
        }
        if let Some(id) = &self.identity.role_binding_id {
            validate_identifier("role_binding_id", id)?;
            if self.identity.hardware_fingerprint_id.is_none() {
                bail!("role binding candidate requires a hardware fingerprint");
            }
        }
        if self.identity.predecessor_binding_id.is_some() {
            bail!("SmartSentinel discovery must not assign predecessor binding history");
        }
        if self.identity.hardware_identity_attested {
            bail!("discovery hardware identity is observational and must not claim attestation");
        }
        if self.platform.os_family.trim().is_empty() {
            bail!("discovery os_family must not be empty");
        }
        if self.platform.arch.trim().is_empty() {
            bail!("discovery architecture must not be empty");
        }
        validate_identifier("capacity_profile_id", &self.resources.capacity_profile_id)?;
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
            if capability.available && !capability.present {
                bail!(
                    "discovery capability {} cannot be available when it is not present",
                    capability.name
                );
            }
        }
        if self.safe_to_auto_execute {
            bail!("discovery must not authorize automatic execution");
        }
        if self.side_effects_performed {
            bail!("discovery must remain observation-only");
        }
        Ok(())
    }
}

fn validate_identifier(field: &str, value: &str) -> Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        bail!("discovery {field} must not be empty");
    }
    if trimmed.len() > 128 {
        bail!("discovery {field} must not exceed 128 characters");
    }
    Ok(())
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint_id: Option<String>,
    #[serde(default)]
    pub hardware_identity_basis: Vec<HardwareIdentityBasis>,
    pub hardware_identity_confidence: IdentityConfidence,
    pub hardware_identity_attested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_installation_id: Option<String>,
    pub os_identity_basis: OsIdentityBasis,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_binding_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_binding_id: Option<String>,
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
    pub capacity_profile_id: String,
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

pub fn derive_hardware_fingerprint_id(anchors: &[(HardwareIdentityBasis, String)]) -> Option<String> {
    let mut canonical = anchors
        .iter()
        .filter_map(|(basis, value)| {
            let value = value.trim().to_ascii_lowercase();
            (!value.is_empty()).then_some(format!("{:?}={}", basis, value))
        })
        .collect::<Vec<_>>();
    if canonical.is_empty() {
        return None;
    }
    canonical.sort();
    Some(format!("hw:{}", sha256_hex(canonical.join("|").as_bytes())))
}

pub fn derive_os_installation_id(basis: OsIdentityBasis, anchor: &str) -> Option<String> {
    let anchor = anchor.trim().to_ascii_lowercase();
    if anchor.is_empty() {
        return None;
    }
    Some(format!(
        "osinst:{}",
        sha256_hex(format!("{:?}:{}", basis, anchor).as_bytes())
    ))
}

pub fn derive_role_binding_id(
    node_id: &str,
    hardware_fingerprint_id: Option<&str>,
) -> Option<String> {
    let hardware = hardware_fingerprint_id?.trim().to_ascii_lowercase();
    if hardware.is_empty() {
        return None;
    }
    Some(format!(
        "binding:{}",
        sha256_hex(
            format!(
                "node={};hardware={}",
                node_id.trim().to_ascii_lowercase(),
                hardware
            )
            .as_bytes(),
        )
    ))
}

pub fn derive_capacity_profile_id(resources: &ResourceObservation) -> String {
    let mut filesystems = resources.filesystems.clone();
    filesystems.sort_by(|a, b| a.mount.cmp(&b.mount));

    let mut canonical = format!(
        "cpu={};mem={};swap={}",
        resources.cpu.logical_cores, resources.memory.total_bytes, resources.swap.total_bytes
    );
    for fs in filesystems {
        canonical.push_str(&format!(";fs={}:{}", fs.mount, fs.total_bytes));
    }
    format!("capacity:{}", sha256_hex(canonical.as_bytes()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
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
    pub available: bool,
    pub version: Option<String>,
    pub source: CapabilitySource,
    pub detail: Option<String>,
}
