use anyhow::{bail, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agent::discovery::{
    CapabilityObservation, IdentityObservation, PlatformObservation, ResourceObservation,
    SmartSentinelDiscovery, DISCOVERY_SCHEMA_VERSION,
};
use crate::agent::discovery_collectors::collect_discovery;
use crate::agent::discovery::BootstrapMode;
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

    #[test]
    fn unbound_resource_rejects_role_binding() {
        let mut report = SmartSentinelResourceDiscovery {
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
                os_installation_id: Some("osinst:abc".to_string()),
                os_identity_basis: OsIdentityBasis::MachineId,
                role_binding_id: Some("binding:wrong".to_string()),
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
        };
        assert!(report.validate().is_err());
        report.identity.role_binding_id = None;
        assert!(report.validate().is_ok());

        let _ = HardwareIdentityBasis::SystemUuid;
    }
}
