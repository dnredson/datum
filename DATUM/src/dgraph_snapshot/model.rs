use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalDGraphSnapshot {
    pub node_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub reported_at_utc: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub runtimes: Vec<LocalRuntimeSnapshot>,
    #[serde(default)]
    pub modules: Vec<LocalModuleSnapshot>,
    #[serde(default)]
    pub flows: Vec<LocalFlowSnapshot>,
    #[serde(default)]
    pub health: Vec<LocalHealthSnapshot>,
    #[serde(default)]
    pub capabilities: Vec<LocalCapabilitySnapshot>,
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalDGraphSnapshotList {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub snapshots: Vec<LocalDGraphSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalRuntimeSnapshot {
    pub runtime_id: String,
    pub runtime_kind: String,
    pub status: String,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalModuleSnapshot {
    pub module_id: String,
    #[serde(default)]
    pub module_kind: Option<String>,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub observed_state: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub artifact_ref: Option<String>,
    #[serde(default)]
    pub local_mapping_key: Option<String>,
    #[serde(default)]
    pub local_container_name: Option<String>,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalFlowSnapshot {
    pub flow_id: String,
    #[serde(default)]
    pub flow_kind: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub edges: Vec<LocalFlowEdgeSnapshot>,
    #[serde(default)]
    pub status: Option<String>,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalFlowEdgeSnapshot {
    pub edge_id: String,
    pub from_module_id: String,
    pub to_module_id: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalHealthSnapshot {
    pub target_kind: String,
    pub target_id: String,
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct LocalCapabilitySnapshot {
    pub capability_id: String,
    pub capability_kind: String,
    pub supported: bool,
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

pub fn validate_local_dgraph_snapshot_publish_invariants(
    snapshot: &LocalDGraphSnapshot,
) -> anyhow::Result<()> {
    if snapshot.safe_to_auto_execute {
        anyhow::bail!("local d-graph snapshot publish requires safe_to_auto_execute=false");
    }

    if snapshot.side_effects_performed {
        anyhow::bail!("local d-graph snapshot publish requires side_effects_performed=false");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        validate_local_dgraph_snapshot_publish_invariants, LocalCapabilitySnapshot,
        LocalDGraphSnapshot, LocalDGraphSnapshotList, LocalHealthSnapshot, LocalModuleSnapshot,
        LocalRuntimeSnapshot,
    };

    #[test]
    fn publish_invariants_accept_safe_reporting_only_snapshot() {
        let snapshot = LocalDGraphSnapshot {
            node_id: "fog-01".to_string(),
            project_id: Some("smart-city-campinas".to_string()),
            snapshot_id: None,
            reported_at_utc: None,
            source: None,
            generated_at_utc: None,
            runtimes: vec![],
            modules: vec![],
            flows: vec![],
            health: vec![],
            capabilities: vec![],
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        };

        validate_local_dgraph_snapshot_publish_invariants(&snapshot)
            .expect("publish invariants should accept safe snapshot");
    }

    #[test]
    fn publish_invariants_reject_safe_to_auto_execute_true() {
        let snapshot = LocalDGraphSnapshot {
            safe_to_auto_execute: true,
            ..LocalDGraphSnapshot::default()
        };

        assert!(validate_local_dgraph_snapshot_publish_invariants(&snapshot).is_err());
    }

    #[test]
    fn publish_invariants_reject_side_effects_performed_true() {
        let snapshot = LocalDGraphSnapshot {
            side_effects_performed: true,
            ..LocalDGraphSnapshot::default()
        };

        assert!(validate_local_dgraph_snapshot_publish_invariants(&snapshot).is_err());
    }

    #[test]
    fn snapshot_list_serialization_preserves_snapshots_array() {
        let list = LocalDGraphSnapshotList {
            project_id: Some("smart-city-campinas".to_string()),
            count: Some(1),
            snapshots: vec![LocalDGraphSnapshot {
                node_id: "fog-01".to_string(),
                project_id: Some("smart-city-campinas".to_string()),
                snapshot_id: Some("snapshot-1".to_string()),
                reported_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
                source: Some("client".to_string()),
                generated_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
                runtimes: vec![LocalRuntimeSnapshot {
                    runtime_id: "runtime-container-local".to_string(),
                    runtime_kind: "container".to_string(),
                    status: "configured".to_string(),
                    source: "local_config".to_string(),
                    metadata: None,
                }],
                modules: vec![LocalModuleSnapshot {
                    module_id: "chirpstack-primary".to_string(),
                    module_kind: Some("service".to_string()),
                    desired_state: Some("running".to_string()),
                    observed_state: Some("unknown".to_string()),
                    runtime: Some("container".to_string()),
                    artifact_ref: None,
                    local_mapping_key: None,
                    local_container_name: Some("chirpstack".to_string()),
                    source: "service_registry".to_string(),
                    metadata: None,
                }],
                flows: vec![],
                health: vec![LocalHealthSnapshot {
                    target_kind: "module".to_string(),
                    target_id: "chirpstack-primary".to_string(),
                    status: "unknown".to_string(),
                    reason: None,
                    source: "local_config".to_string(),
                    metadata: None,
                }],
                capabilities: vec![LocalCapabilitySnapshot {
                    capability_id: "capability-container-runtime-configured".to_string(),
                    capability_kind: "container_runtime_configured".to_string(),
                    supported: true,
                    source: "local_config".to_string(),
                    metadata: None,
                }],
                metadata: None,
                safe_to_auto_execute: false,
                side_effects_performed: false,
            }],
        };

        let rendered = serde_json::to_string(&list).expect("list should serialize");
        assert!(rendered.contains("\"snapshots\""));
        assert!(rendered.contains("\"snapshot_id\":\"snapshot-1\""));
    }
}
