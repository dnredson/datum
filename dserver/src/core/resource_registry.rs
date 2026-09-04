use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const RESOURCE_REGISTRY_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceDiscoveryRequest {
    pub schema_version: String,
    pub observed_at_utc: String,
    pub instance_id: String,
    pub observation_host_id: String,
    #[serde(default)]
    pub stage_hint: Option<String>,
    pub identity: ResourceIdentityEvidence,
    pub platform: Value,
    pub resources: Value,
    #[serde(default)]
    pub capabilities: Vec<Value>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceIdentityEvidence {
    pub hostname: String,
    pub sentinel_version: String,
    #[serde(default)]
    pub hardware_fingerprint_id: Option<String>,
    #[serde(default)]
    pub hardware_identity_basis: Vec<String>,
    pub hardware_identity_confidence: String,
    pub hardware_identity_attested: bool,
    #[serde(default)]
    pub os_installation_id: Option<String>,
    pub os_identity_basis: String,
    #[serde(default)]
    pub role_binding_id: Option<String>,
    #[serde(default)]
    pub predecessor_binding_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceIdentityScope {
    CanonicalHardware,
    ProvisionalOs,
    EphemeralInstance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceRegistryRecord {
    pub registry_schema_version: String,
    pub project_id: String,
    pub resource_id: String,
    pub identity_scope: ResourceIdentityScope,
    pub planning_eligible: bool,
    pub state: String,
    #[serde(default)]
    pub assigned_node_id: Option<String>,
    pub first_seen_utc: String,
    pub last_seen_utc: String,
    pub observation_count: u64,
    pub latest_instance_id: String,
    pub discovery: ResourceDiscoveryRequest,
}

pub fn build_resource_registry_record(
    project_id: &str,
    request: ResourceDiscoveryRequest,
    existing: Option<&ResourceRegistryRecord>,
    now: DateTime<Utc>,
) -> Result<ResourceRegistryRecord, String> {
    validate_resource_discovery_request(&request)?;
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err("resource registry project_id must not be empty".to_string());
    }

    let (resource_id, identity_scope, planning_eligible) = derive_resource_identity(&request)?;
    if let Some(existing) = existing {
        if existing.project_id != project_id {
            return Err("resource registry existing project_id mismatch".to_string());
        }
        if existing.resource_id != resource_id {
            return Err("resource registry existing resource_id mismatch".to_string());
        }
    }

    Ok(ResourceRegistryRecord {
        registry_schema_version: RESOURCE_REGISTRY_SCHEMA_VERSION.to_string(),
        project_id: project_id.to_string(),
        resource_id,
        identity_scope,
        planning_eligible,
        state: existing
            .map(|record| record.state.clone())
            .unwrap_or_else(|| "available_unbound".to_string()),
        assigned_node_id: existing.and_then(|record| record.assigned_node_id.clone()),
        first_seen_utc: existing
            .map(|record| record.first_seen_utc.clone())
            .unwrap_or_else(|| now.to_rfc3339()),
        last_seen_utc: now.to_rfc3339(),
        observation_count: existing
            .map(|record| record.observation_count.saturating_add(1))
            .unwrap_or(1),
        latest_instance_id: request.instance_id.clone(),
        discovery: request,
    })
}

pub fn validate_resource_discovery_request(request: &ResourceDiscoveryRequest) -> Result<(), String> {
    required("schema_version", &request.schema_version)?;
    required("instance_id", &request.instance_id)?;
    required("observation_host_id", &request.observation_host_id)?;
    required("identity.hostname", &request.identity.hostname)?;
    required("identity.sentinel_version", &request.identity.sentinel_version)?;
    required("identity.hardware_identity_confidence", &request.identity.hardware_identity_confidence)?;
    required("identity.os_identity_basis", &request.identity.os_identity_basis)?;

    DateTime::parse_from_rfc3339(request.observed_at_utc.trim())
        .map_err(|error| format!("resource observed_at_utc must be RFC3339: {error}"))?;

    if request.safe_to_auto_execute {
        return Err("resource discovery must not authorize automatic execution".to_string());
    }
    if request.side_effects_performed {
        return Err("resource discovery must remain observation-only".to_string());
    }
    if request.identity.role_binding_id.is_some() {
        return Err("unbound resource discovery must not claim role_binding_id".to_string());
    }
    if request.identity.predecessor_binding_id.is_some() {
        return Err("unbound resource discovery must not claim predecessor binding".to_string());
    }
    if request.identity.hardware_identity_attested {
        return Err("O2 resource discovery must not claim hardware attestation".to_string());
    }

    if let Some(hardware) = request.identity.hardware_fingerprint_id.as_deref() {
        validate_prefixed_digest("hardware_fingerprint_id", hardware, "hw:")?;
        if request.identity.hardware_identity_basis.is_empty() {
            return Err("hardware fingerprint requires identity basis evidence".to_string());
        }
        if request.identity.hardware_identity_confidence == "unknown" {
            return Err("hardware fingerprint cannot have unknown confidence".to_string());
        }
    } else if !request.identity.hardware_identity_basis.is_empty()
        || request.identity.hardware_identity_confidence != "unknown"
    {
        return Err(
            "missing hardware fingerprint requires empty basis and unknown confidence".to_string(),
        );
    }

    if let Some(os_id) = request.identity.os_installation_id.as_deref() {
        validate_prefixed_digest("os_installation_id", os_id, "osinst:")?;
    }

    let capacity = request
        .resources
        .get("capacity_profile_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    validate_prefixed_digest("resources.capacity_profile_id", capacity, "capacity:")?;

    required_value_string(&request.platform, "os_family", "platform.os_family")?;
    required_value_string(&request.platform, "arch", "platform.arch")?;

    Ok(())
}

pub fn derive_resource_identity(
    request: &ResourceDiscoveryRequest,
) -> Result<(String, ResourceIdentityScope, bool), String> {
    if let Some(hardware) = request.identity.hardware_fingerprint_id.as_deref() {
        let digest = prefixed_digest(hardware, "hw:")?;
        return Ok((
            format!("resource:hw:{digest}"),
            ResourceIdentityScope::CanonicalHardware,
            true,
        ));
    }

    if let Some(os_id) = request.identity.os_installation_id.as_deref() {
        let digest = prefixed_digest(os_id, "osinst:")?;
        return Ok((
            format!("resource:os:{digest}"),
            ResourceIdentityScope::ProvisionalOs,
            true,
        ));
    }

    Ok((
        format!("resource:instance:{}", request.instance_id.trim()),
        ResourceIdentityScope::EphemeralInstance,
        false,
    ))
}

fn required(label: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("resource {label} must not be empty"))
    } else {
        Ok(())
    }
}

fn required_value_string(value: &Value, key: &str, label: &str) -> Result<(), String> {
    let found = value.get(key).and_then(Value::as_str).unwrap_or_default();
    required(label, found)
}

fn validate_prefixed_digest(label: &str, value: &str, prefix: &str) -> Result<(), String> {
    let digest = prefixed_digest(value, prefix)?;
    if digest.len() != 64 || !digest.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(format!("resource {label} must contain a 64-character hexadecimal digest"));
    }
    Ok(())
}

fn prefixed_digest<'a>(value: &'a str, prefix: &str) -> Result<&'a str, String> {
    value
        .trim()
        .strip_prefix(prefix)
        .filter(|digest| !digest.is_empty())
        .ok_or_else(|| format!("resource identity '{}' must start with '{}'", value, prefix))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 4, 17, 30, 0).unwrap()
    }

    fn request(hardware: Option<&str>, os: Option<&str>) -> ResourceDiscoveryRequest {
        ResourceDiscoveryRequest {
            schema_version: "0.4.0".to_string(),
            observed_at_utc: "2026-09-04T17:29:00Z".to_string(),
            instance_id: "instance-1".to_string(),
            observation_host_id: "host:probe".to_string(),
            stage_hint: None,
            identity: ResourceIdentityEvidence {
                hostname: "Xubuntu".to_string(),
                sentinel_version: "0.1.0".to_string(),
                hardware_fingerprint_id: hardware.map(str::to_string),
                hardware_identity_basis: hardware
                    .map(|_| vec!["system_uuid".to_string()])
                    .unwrap_or_default(),
                hardware_identity_confidence: if hardware.is_some() {
                    "medium".to_string()
                } else {
                    "unknown".to_string()
                },
                hardware_identity_attested: false,
                os_installation_id: os.map(str::to_string),
                os_identity_basis: if os.is_some() {
                    "machine_id".to_string()
                } else {
                    "unknown".to_string()
                },
                role_binding_id: None,
                predecessor_binding_id: None,
            },
            platform: json!({"os_family":"linux","os_name":"Ubuntu","os_version":"25.10","arch":"x86_64"}),
            resources: json!({"capacity_profile_id": format!("capacity:{}", "c".repeat(64))}),
            capabilities: Vec::new(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn hardware_identity_creates_canonical_planning_resource() {
        let hardware = format!("hw:{}", "a".repeat(64));
        let record = build_resource_registry_record(
            "project-1",
            request(Some(&hardware), None),
            None,
            now(),
        )
        .unwrap();
        assert_eq!(record.resource_id, format!("resource:hw:{}", "a".repeat(64)));
        assert_eq!(record.identity_scope, ResourceIdentityScope::CanonicalHardware);
        assert!(record.planning_eligible);
        assert_eq!(record.state, "available_unbound");
        assert!(record.assigned_node_id.is_none());
    }

    #[test]
    fn os_identity_creates_provisional_planning_resource() {
        let os = format!("osinst:{}", "b".repeat(64));
        let record = build_resource_registry_record(
            "project-1",
            request(None, Some(&os)),
            None,
            now(),
        )
        .unwrap();
        assert_eq!(record.resource_id, format!("resource:os:{}", "b".repeat(64)));
        assert_eq!(record.identity_scope, ResourceIdentityScope::ProvisionalOs);
        assert!(record.planning_eligible);
    }

    #[test]
    fn missing_stable_identity_is_ephemeral_and_not_planning_eligible() {
        let record = build_resource_registry_record("project-1", request(None, None), None, now())
            .unwrap();
        assert_eq!(record.resource_id, "resource:instance:instance-1");
        assert_eq!(record.identity_scope, ResourceIdentityScope::EphemeralInstance);
        assert!(!record.planning_eligible);
    }

    #[test]
    fn rediscovery_preserves_first_seen_and_increments_count() {
        let os = format!("osinst:{}", "b".repeat(64));
        let first = build_resource_registry_record(
            "project-1",
            request(None, Some(&os)),
            None,
            now(),
        )
        .unwrap();
        let later = Utc.with_ymd_and_hms(2026, 9, 4, 17, 40, 0).unwrap();
        let second = build_resource_registry_record(
            "project-1",
            request(None, Some(&os)),
            Some(&first),
            later,
        )
        .unwrap();
        assert_eq!(second.first_seen_utc, first.first_seen_utc);
        assert_eq!(second.observation_count, 2);
        assert_ne!(second.last_seen_utc, first.last_seen_utc);
    }
}
