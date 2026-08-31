use std::path::PathBuf;
use std::time::Duration;

use chrono::Utc;
use datum::agent::service_loop::{annotate_heartbeat, instance_lock_path, ServiceLoopOptions};
use datum::core::model::{Entity, Meta, Snapshot};
use serde_json::json;

#[test]
fn service_loop_defaults_keep_stable_cross_platform_lifecycle_controls() {
    let options = ServiceLoopOptions::default();
    assert_eq!(options.stop_file, PathBuf::from(".smartsentinel-stop"));
    assert_eq!(options.heartbeat_s, 1);
    assert!(options.max_cycles.is_none());
}

#[test]
fn instance_lock_path_is_node_scoped_and_sanitized() {
    let fog = instance_lock_path("fog-01");
    let cloud = instance_lock_path("cloud-01");
    let unsafe_id = instance_lock_path("fog/01:test");

    assert_ne!(fog, cloud);
    assert_eq!(
        fog.file_name().and_then(|name| name.to_str()),
        Some("smartsentinel-fog-01.lock")
    );
    assert_eq!(
        unsafe_id.file_name().and_then(|name| name.to_str()),
        Some("smartsentinel-fog_01_test.lock")
    );
}

#[test]
fn heartbeat_annotation_is_explicitly_observation_only() {
    let now = Utc::now();
    let host_id = "host:test".to_string();
    let mut snapshot = Snapshot {
        meta: Meta {
            schema_version: "test".to_string(),
            agent_version: "test".to_string(),
            cycle_id: "cycle-test".to_string(),
            observed_at_utc: now,
            node_id: "local-01".to_string(),
            host_id: host_id.clone(),
        },
        entities: vec![Entity {
            id: host_id,
            kind: "host".to_string(),
            attrs: json!({"hostname": "test"}),
            observed_at_utc: now,
            ttl_s: 60,
        }],
        edges: vec![],
        events: vec![],
        actions: vec![],
    };

    annotate_heartbeat(&mut snapshot, 7, now, Duration::from_secs(3));

    let heartbeat = &snapshot.entities[0].attrs["smartsentinel"];
    assert_eq!(heartbeat["mode"], "continuous");
    assert_eq!(heartbeat["heartbeat_sequence"], 7);
    assert_eq!(heartbeat["service_uptime_ms"], 3000);
    assert_eq!(heartbeat["safe_to_auto_execute"], false);
    assert_eq!(heartbeat["side_effects_performed"], false);
}
