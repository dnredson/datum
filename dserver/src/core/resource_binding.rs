use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::core::resource_registry::ResourceRegistryRecord;

pub const RESOURCE_BINDING_SCHEMA_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BindResourceRequest {
    pub node_id: String,
    #[serde(default = "default_initial_assignment")]
    pub transition_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceRoleBindingRecord {
    pub binding_schema_version: String,
    pub project_id: String,
    pub binding_id: String,
    pub resource_id: String,
    pub node_id: String,
    pub transition_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predecessor_binding_id: Option<String>,
    pub status: String,
    pub bound_at_utc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unbound_at_utc: Option<String>,
    pub identity_scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hardware_fingerprint_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_installation_id: Option<String>,
    pub capacity_profile_id: String,
    pub control_plane_authorized: bool,
    pub automatic_assignment_performed: bool,
}

pub fn build_initial_binding(
    project_id: &str,
    resource: &ResourceRegistryRecord,
    request: &BindResourceRequest,
    now: DateTime<Utc>,
) -> Result<ResourceRoleBindingRecord, String> {
    let project_id = project_id.trim();
    let node_id = request.node_id.trim();
    if project_id.is_empty() {
        return Err("resource binding project_id must not be empty".to_string());
    }
    if node_id.is_empty() {
        return Err("resource binding node_id must not be empty".to_string());
    }
    if node_id.len() > 128 {
        return Err("resource binding node_id must not exceed 128 characters".to_string());
    }
    if resource.project_id != project_id {
        return Err("resource binding project_id mismatch".to_string());
    }
    if !resource.planning_eligible {
        return Err("resource is not planning-eligible and cannot be bound".to_string());
    }
    if resource.state != "available_unbound" || resource.assigned_node_id.is_some() {
        return Err("resource is not available_unbound".to_string());
    }
    if request.transition_kind.trim() != "initial_assignment" {
        return Err(
            "initial resource binding only accepts transition_kind=initial_assignment".to_string(),
        );
    }

    let capacity_profile_id = resource
        .discovery
        .resources
        .get("capacity_profile_id")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "resource binding requires capacity_profile_id evidence".to_string())?
        .to_string();

    Ok(ResourceRoleBindingRecord {
        binding_schema_version: RESOURCE_BINDING_SCHEMA_VERSION.to_string(),
        project_id: project_id.to_string(),
        binding_id: format!("binding:{}", uuid::Uuid::new_v4()),
        resource_id: resource.resource_id.clone(),
        node_id: node_id.to_string(),
        transition_kind: "initial_assignment".to_string(),
        predecessor_binding_id: None,
        status: "active".to_string(),
        bound_at_utc: now.to_rfc3339(),
        unbound_at_utc: None,
        identity_scope: serde_json::to_value(resource.identity_scope)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        hardware_fingerprint_id: resource.discovery.identity.hardware_fingerprint_id.clone(),
        os_installation_id: resource.discovery.identity.os_installation_id.clone(),
        capacity_profile_id,
        control_plane_authorized: true,
        automatic_assignment_performed: false,
    })
}

fn default_initial_assignment() -> String {
    "initial_assignment".to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use crate::core::resource_registry::{
        ResourceDiscoveryRequest, ResourceIdentityEvidence, ResourceIdentityScope,
        ResourceRegistryRecord,
    };

    use super::*;

    fn resource() -> ResourceRegistryRecord {
        ResourceRegistryRecord {
            registry_schema_version: "0.1.0".to_string(),
            project_id: "project-1".to_string(),
            resource_id: "resource:os:abc".to_string(),
            identity_scope: ResourceIdentityScope::ProvisionalOs,
            planning_eligible: true,
            state: "available_unbound".to_string(),
            assigned_node_id: None,
            first_seen_utc: "2026-09-04T18:00:00Z".to_string(),
            last_seen_utc: "2026-09-04T18:00:00Z".to_string(),
            observation_count: 1,
            latest_instance_id: "instance-1".to_string(),
            discovery: ResourceDiscoveryRequest {
                schema_version: "0.4.0".to_string(),
                observed_at_utc: "2026-09-04T18:00:00Z".to_string(),
                instance_id: "instance-1".to_string(),
                observation_host_id: "host:probe".to_string(),
                stage_hint: None,
                identity: ResourceIdentityEvidence {
                    hostname: "Xubuntu".to_string(),
                    sentinel_version: "0.1.0".to_string(),
                    hardware_fingerprint_id: None,
                    hardware_identity_basis: Vec::new(),
                    hardware_identity_confidence: "unknown".to_string(),
                    hardware_identity_attested: false,
                    os_installation_id: Some("osinst:abc".to_string()),
                    os_identity_basis: "machine_id".to_string(),
                    role_binding_id: None,
                    predecessor_binding_id: None,
                },
                platform: json!({"os_family":"linux","arch":"x86_64"}),
                resources: json!({"capacity_profile_id":"capacity:abc"}),
                capabilities: Vec::new(),
                safe_to_auto_execute: false,
                side_effects_performed: false,
            },
        }
    }

    #[test]
    fn initial_binding_preserves_resource_context_without_auto_assignment() {
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 18, 10, 0).unwrap();
        let binding = build_initial_binding(
            "project-1",
            &resource(),
            &BindResourceRequest {
                node_id: "fog-01".to_string(),
                transition_kind: "initial_assignment".to_string(),
            },
            now,
        )
        .unwrap();
        assert_eq!(binding.node_id, "fog-01");
        assert_eq!(binding.resource_id, "resource:os:abc");
        assert_eq!(binding.predecessor_binding_id, None);
        assert_eq!(binding.status, "active");
        assert!(binding.control_plane_authorized);
        assert!(!binding.automatic_assignment_performed);
    }

    #[test]
    fn initial_binding_rejects_ineligible_or_noninitial_transitions() {
        let now = Utc.with_ymd_and_hms(2026, 9, 4, 18, 10, 0).unwrap();
        let mut ineligible = resource();
        ineligible.planning_eligible = false;
        assert!(build_initial_binding(
            "project-1",
            &ineligible,
            &BindResourceRequest {
                node_id: "fog-01".to_string(),
                transition_kind: "initial_assignment".to_string(),
            },
            now,
        )
        .is_err());

        assert!(build_initial_binding(
            "project-1",
            &resource(),
            &BindResourceRequest {
                node_id: "fog-01".to_string(),
                transition_kind: "hardware_replacement".to_string(),
            },
            now,
        )
        .is_err());
    }
}
