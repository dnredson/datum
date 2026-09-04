use std::fs;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::discovery::{
    BootstrapMode, CapabilityObservation, IdentityObservation, PlatformObservation,
    ResourceObservation, SmartSentinelDiscovery, DISCOVERY_SCHEMA_VERSION,
};
use crate::agent::discovery_collectors::collect_discovery;
use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartSentinelResourceDiscovery {
    pub schema_version: String,
    pub observed_at_utc: DateTime<Utc>,
    pub instance_id: String,
    pub observation_host_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_hint: Option<String>,
    pub identity: IdentityObservation,
    pub platform: PlatformObservation,
    pub resources: ResourceObservation,
    pub capabilities: Vec<CapabilityObservation>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
}

impl SmartSentinelResourceDiscovery {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version.trim().is_empty() {
            bail!("resource discovery schema_version must not be empty");
        }
        if self.instance_id.trim().is_empty() {
            bail!("resource discovery instance_id must not be empty");
        }
        if self.observation_host_id.trim().is_empty() {
            bail!("resource discovery observation_host_id must not be empty");
        }
        if let Some(stage_hint) = &self.stage_hint {
            if stage_hint.trim().is_empty() {
                bail!("resource discovery stage_hint must be omitted or non-empty");
            }
        }
        if self.identity.role_binding_id.is_some() {
            bail!("unbound resource discovery must not claim a role binding");
        }
        if self.identity.predecessor_binding_id.is_some() {
            bail!("unbound resource discovery must not claim binding lineage");
        }
        if self.safe_to_auto_execute {
            bail!("resource discovery must not authorize automatic execution");
        }
        if self.side_effects_performed {
            bail!("resource discovery must remain observation-only");
        }
        if self.resources.capacity_profile_id.trim().is_empty() {
            bail!("resource discovery capacity_profile_id must not be empty");
        }
        Ok(())
    }
}

pub fn load_config_allow_unbound(path: &str) -> Result<Config> {
    let raw = fs::read_to_string(path).with_context(|| format!("read config file: {path}"))?;
    let mut value: toml::Value = toml::from_str(&raw).context("parse TOML resource config")?;
    let root = value
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("resource config root must be a TOML table"))?;
    let agent = root
        .get_mut("agent")
        .and_then(toml::Value::as_table_mut)
        .ok_or_else(|| anyhow::anyhow!("resource config requires an [agent] table"))?;

    let needs_probe_id = agent
        .get("node_id")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .map(str::is_empty)
        .unwrap_or(true);
    if needs_probe_id {
        agent.insert(
            "node_id".to_string(),
            toml::Value::String(format!("resource-probe-{}", uuid::Uuid::new_v4())),
        );
    }

    let normalized = toml::to_string(&value).context("normalize role-free resource config")?;
    let cfg: Config = toml::from_str(&normalized).context("decode role-free resource config")?;
    if cfg.agent.schema_version.trim().is_empty() {
        bail!("resource config agent.schema_version must not be empty");
    }
    Ok(cfg)
}

pub fn collect_unbound_resource_discovery(
    cfg: &Config,
    hostname_override: Option<&str>,
) -> Result<SmartSentinelResourceDiscovery> {
    let probe_scope = format!("resource-probe-{}", uuid::Uuid::new_v4());
    let mut discovery = collect_discovery(
        &probe_scope,
        hostname_override,
        BootstrapMode::Discovery,
        cfg,
    )?;

    discovery.identity.role_binding_id = None;
    discovery.identity.predecessor_binding_id = None;

    let report = resource_discovery_from_node_discovery(discovery);
    report.validate()?;
    Ok(report)
}

pub fn derive_resource_lookup_id(discovery: &SmartSentinelResourceDiscovery) -> String {
    if let Some(hardware) = discovery.identity.hardware_fingerprint_id.as_deref() {
        if let Some(digest) = hardware.trim().strip_prefix("hw:") {
            if !digest.is_empty() {
                return format!("resource:hw:{digest}");
            }
        }
    }
    if let Some(os_id) = discovery.identity.os_installation_id.as_deref() {
        if let Some(digest) = os_id.trim().strip_prefix("osinst:") {
            if !digest.is_empty() {
                return format!("resource:os:{digest}");
            }
        }
    }
    format!("resource:instance:{}", discovery.instance_id.trim())
}

fn resource_discovery_from_node_discovery(
    discovery: SmartSentinelDiscovery,
) -> SmartSentinelResourceDiscovery {
    SmartSentinelResourceDiscovery {
        schema_version: DISCOVERY_SCHEMA_VERSION.to_string(),
        observed_at_utc: discovery.observed_at_utc,
        instance_id: discovery.instance_id,
        observation_host_id: discovery.host_id,
        stage_hint: discovery.stage,
        identity: discovery.identity,
        platform: discovery.platform,
        resources: discovery.resources,
        capabilities: discovery.capabilities,
        safe_to_auto_execute: false,
        side_effects_performed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::discovery::{
        CpuObservation, HardwareIdentityBasis, IdentityConfidence, MemoryObservation,
        OsIdentityBasis, SwapObservation,
    };

    fn base_report() -> SmartSentinelResourceDiscovery {
        SmartSentinelResourceDiscovery {
            schema_version: DISCOVERY_SCHEMA_VERSION.to_string(),
            observed_at_utc: Utc::now(),
            instance_id: "instance-1".to_string(),
            observation_host_id: "host:probe".to_string(),
            stage_hint: None,
            identity: IdentityObservation {
                hostname: "host-a".to_string(),
                sentinel_version: "0.1.0".to_string(),
                hardware_fingerprint_id: None,
                hardware_identity_basis: Vec::new(),
                hardware_identity_confidence: IdentityConfidence::Unknown,
                hardware_identity_attested: false,
                os_installation_id: Some(format!("osinst:{}", "a".repeat(64))),
                os_identity_basis: OsIdentityBasis::MachineId,
                role_binding_id: None,
                predecessor_binding_id: None,
            },
            platform: PlatformObservation {
                os_family: "linux".to_string(),
                os_name: "Ubuntu".to_string(),
                os_version: "25.10".to_string(),
                arch: "x86_64".to_string(),
            },
            resources: ResourceObservation {
                capacity_profile_id: "capacity:test".to_string(),
                uptime_seconds: Some(1),
                cpu: CpuObservation {
                    logical_cores: 4,
                    usage_percent: Some(1.0),
                },
                memory: MemoryObservation {
                    total_bytes: 1024,
                    used_bytes: 1,
                    available_bytes: 1023,
                },
                swap: SwapObservation {
                    total_bytes: 0,
                    used_bytes: 0,
                    available_bytes: 0,
                },
                filesystems: Vec::new(),
            },
            capabilities: Vec::new(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn unbound_resource_rejects_role_binding() {
        let mut report = base_report();
        report.identity.role_binding_id = Some("binding:wrong".to_string());
        assert!(report.validate().is_err());
        report.identity.role_binding_id = None;
        assert!(report.validate().is_ok());

        let _ = HardwareIdentityBasis::SystemUuid;
    }

    #[test]
    fn resource_lookup_prefers_hardware_then_os_then_instance() {
        let mut report = base_report();
        assert_eq!(
            derive_resource_lookup_id(&report),
            format!("resource:os:{}", "a".repeat(64))
        );

        report.identity.hardware_fingerprint_id = Some(format!("hw:{}", "b".repeat(64)));
        assert_eq!(
            derive_resource_lookup_id(&report),
            format!("resource:hw:{}", "b".repeat(64))
        );

        report.identity.hardware_fingerprint_id = None;
        report.identity.os_installation_id = None;
        assert_eq!(derive_resource_lookup_id(&report), "resource:instance:instance-1");
    }
}
