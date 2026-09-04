use chrono::{TimeZone, Utc};
use serde_json::json;

use super::resource_registry::{
    build_resource_registry_record, ResourceDiscoveryRequest, ResourceIdentityEvidence,
};

fn request(os_installation_id: &str, instance_id: &str) -> ResourceDiscoveryRequest {
    ResourceDiscoveryRequest {
        schema_version: "0.4.0".to_string(),
        observed_at_utc: "2026-09-04T20:00:00Z".to_string(),
        instance_id: instance_id.to_string(),
        observation_host_id: "host:probe".to_string(),
        stage_hint: Some("fog".to_string()),
        identity: ResourceIdentityEvidence {
            hostname: "Xubuntu".to_string(),
            sentinel_version: "0.1.0".to_string(),
            hardware_fingerprint_id: None,
            hardware_identity_basis: Vec::new(),
            hardware_identity_confidence: "unknown".to_string(),
            hardware_identity_attested: false,
            os_installation_id: Some(os_installation_id.to_string()),
            os_identity_basis: "machine_id".to_string(),
            role_binding_id: None,
            predecessor_binding_id: None,
        },
        platform: json!({
            "os_family": "linux",
            "os_name": "Ubuntu",
            "os_version": "25.10",
            "arch": "x86_64"
        }),
        resources: json!({
            "capacity_profile_id": format!("capacity:{}", "c".repeat(64))
        }),
        capabilities: Vec::new(),
        safe_to_auto_execute: false,
        side_effects_performed: false,
    }
}

#[test]
fn periodic_rediscovery_preserves_active_role_assignment() {
    let os_id = format!("osinst:{}", "b".repeat(64));
    let first = build_resource_registry_record(
        "project-1",
        request(&os_id, "instance-1"),
        None,
        Utc.with_ymd_and_hms(2026, 9, 4, 20, 0, 0).unwrap(),
    )
    .unwrap();

    let mut bound = first.clone();
    bound.state = "bound".to_string();
    bound.assigned_node_id = Some("fog-01".to_string());

    let rediscovered = build_resource_registry_record(
        "project-1",
        request(&os_id, "instance-2"),
        Some(&bound),
        Utc.with_ymd_and_hms(2026, 9, 4, 20, 5, 0).unwrap(),
    )
    .unwrap();

    assert_eq!(rediscovered.state, "bound");
    assert_eq!(rediscovered.assigned_node_id.as_deref(), Some("fog-01"));
    assert_eq!(rediscovered.first_seen_utc, bound.first_seen_utc);
    assert_eq!(rediscovered.observation_count, bound.observation_count + 1);
    assert_eq!(rediscovered.latest_instance_id, "instance-2");
}
