#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::core::dgraph::{
    build_basic_dgraph_relationships, validate_dgraph, DGraph, DGraphArtifact,
    DGraphCapabilityProfile, DGraphFlow, DGraphFlowEdge, DGraphHealthReport, DGraphModule,
    DGraphNode, DGraphRuntime,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalDGraphSnapshot {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub snapshot_id: String,
    #[serde(default)]
    pub reported_at_utc: String,
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

/// Request payload accepted from a DATUM node. Identity and ingestion time are
/// intentionally optional here because dserver assigns their canonical values.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct IncomingDGraphSnapshot {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub node_id: Option<String>,
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

impl From<IncomingDGraphSnapshot> for LocalDGraphSnapshot {
    fn from(input: IncomingDGraphSnapshot) -> Self {
        Self {
            project_id: input.project_id.unwrap_or_default(),
            node_id: input.node_id.unwrap_or_default(),
            snapshot_id: input.snapshot_id.unwrap_or_default(),
            reported_at_utc: input.reported_at_utc.unwrap_or_default(),
            source: input.source,
            generated_at_utc: input.generated_at_utc,
            runtimes: input.runtimes,
            modules: input.modules,
            flows: input.flows,
            health: input.health,
            capabilities: input.capabilities,
            metadata: input.metadata,
            safe_to_auto_execute: input.safe_to_auto_execute,
            side_effects_performed: input.side_effects_performed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalRuntimeSnapshot {
    #[serde(default)]
    pub runtime_id: String,
    #[serde(default)]
    pub runtime_kind: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalModuleSnapshot {
    #[serde(default)]
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
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalFlowSnapshot {
    #[serde(default)]
    pub flow_id: String,
    #[serde(default)]
    pub flow_kind: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub edges: Vec<LocalFlowEdgeSnapshot>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalFlowEdgeSnapshot {
    #[serde(default)]
    pub edge_id: String,
    #[serde(default)]
    pub from_module_id: String,
    #[serde(default)]
    pub to_module_id: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalHealthSnapshot {
    #[serde(default)]
    pub target_kind: String,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalCapabilitySnapshot {
    #[serde(default)]
    pub capability_id: String,
    #[serde(default)]
    pub capability_kind: String,
    #[serde(default)]
    pub supported: bool,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalDGraphSnapshotList {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub snapshots: Vec<LocalDGraphSnapshot>,
}

pub fn normalize_local_dgraph_snapshot(
    project_id: &str,
    node_id: &str,
    input: IncomingDGraphSnapshot,
) -> LocalDGraphSnapshot {
    let mut input = LocalDGraphSnapshot::from(input);
    input.project_id = project_id.to_string();
    input.node_id = node_id.to_string();
    input.snapshot_id = format!(
        "project:{}:node:{}:snapshot:{}",
        project_id,
        node_id,
        Uuid::new_v4()
    );
    input.reported_at_utc = Utc::now().to_rfc3339();

    input.source = Some("client".to_string());

    input
}

pub fn validate_local_dgraph_snapshot(snapshot: &LocalDGraphSnapshot) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if snapshot.project_id.trim().is_empty() {
        errors.push("project_id must not be empty".to_string());
    }
    if snapshot.node_id.trim().is_empty() {
        errors.push("node_id must not be empty".to_string());
    }
    if snapshot.snapshot_id.trim().is_empty() {
        errors.push("snapshot_id must not be empty".to_string());
    }
    if snapshot.reported_at_utc.trim().is_empty() {
        errors.push("reported_at_utc must not be empty".to_string());
    }
    if snapshot.safe_to_auto_execute {
        errors.push("safe_to_auto_execute must be false".to_string());
    }
    if snapshot.side_effects_performed {
        errors.push("side_effects_performed must be false".to_string());
    }

    let module_ids = snapshot
        .modules
        .iter()
        .map(|module| module.module_id.trim())
        .filter(|module_id| !module_id.is_empty())
        .collect::<BTreeSet<_>>();

    for (idx, runtime) in snapshot.runtimes.iter().enumerate() {
        if runtime.runtime_id.trim().is_empty() {
            errors.push(format!("runtimes[{idx}].runtime_id must not be empty"));
        }
        if runtime.runtime_kind.trim().is_empty() {
            errors.push(format!("runtimes[{idx}].runtime_kind must not be empty"));
        }
        if runtime.status.trim().is_empty() {
            errors.push(format!("runtimes[{idx}].status must not be empty"));
        }
    }

    for (idx, module) in snapshot.modules.iter().enumerate() {
        if module.module_id.trim().is_empty() {
            errors.push(format!("modules[{idx}].module_id must not be empty"));
        }
    }

    for (flow_idx, flow) in snapshot.flows.iter().enumerate() {
        if flow.flow_id.trim().is_empty() {
            errors.push(format!("flows[{flow_idx}].flow_id must not be empty"));
        }

        for (module_idx, module_id) in flow.modules.iter().enumerate() {
            if module_id.trim().is_empty() {
                errors.push(format!(
                    "flows[{flow_idx}].modules[{module_idx}] must not be empty"
                ));
            } else if !module_ids.contains(module_id.trim()) {
                errors.push(format!(
                    "flows[{flow_idx}] references missing module '{}' in flow.modules",
                    module_id
                ));
            }
        }

        for (edge_idx, edge) in flow.edges.iter().enumerate() {
            if edge.edge_id.trim().is_empty() {
                errors.push(format!(
                    "flows[{flow_idx}].edges[{edge_idx}].edge_id must not be empty"
                ));
            }
            if edge.from_module_id.trim().is_empty() {
                errors.push(format!(
                    "flows[{flow_idx}].edges[{edge_idx}].from_module_id must not be empty"
                ));
            } else if !module_ids.contains(edge.from_module_id.trim()) {
                errors.push(format!(
                    "flows[{flow_idx}].edges[{edge_idx}] references missing from_module_id '{}'",
                    edge.from_module_id
                ));
            }
            if edge.to_module_id.trim().is_empty() {
                errors.push(format!(
                    "flows[{flow_idx}].edges[{edge_idx}].to_module_id must not be empty"
                ));
            } else if !module_ids.contains(edge.to_module_id.trim()) {
                errors.push(format!(
                    "flows[{flow_idx}].edges[{edge_idx}] references missing to_module_id '{}'",
                    edge.to_module_id
                ));
            }
        }
    }

    for (idx, health) in snapshot.health.iter().enumerate() {
        if health.target_kind.trim().is_empty() {
            errors.push(format!("health[{idx}].target_kind must not be empty"));
        }
        if health.target_id.trim().is_empty() {
            errors.push(format!("health[{idx}].target_id must not be empty"));
        }
        if health.status.trim().is_empty() {
            errors.push(format!("health[{idx}].status must not be empty"));
        }
    }

    for (idx, capability) in snapshot.capabilities.iter().enumerate() {
        if capability.capability_id.trim().is_empty() {
            errors.push(format!(
                "capabilities[{idx}].capability_id must not be empty"
            ));
        }
        if capability.capability_kind.trim().is_empty() {
            errors.push(format!(
                "capabilities[{idx}].capability_kind must not be empty"
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn local_snapshot_to_dgraph_fragment(snapshot: &LocalDGraphSnapshot) -> DGraph {
    let mut artifact_ids = BTreeMap::new();
    let artifacts = snapshot
        .modules
        .iter()
        .filter_map(|module| {
            let artifact_ref = module.artifact_ref.as_deref()?.trim();
            if artifact_ref.is_empty() {
                return None;
            }

            let artifact_id = format!("artifact:{}:{}", snapshot.node_id, module.module_id);
            artifact_ids.insert(module.module_id.clone(), artifact_id.clone());

            Some(DGraphArtifact {
                artifact_id,
                artifact_kind: "opaque_ref".to_string(),
                artifact_ref: Some(artifact_ref.to_string()),
                version: None,
                metadata: Some(json!({
                    "source": module.source,
                    "local_mapping_key": module.local_mapping_key,
                    "local_container_name": module.local_container_name,
                })),
            })
        })
        .collect::<Vec<_>>();

    let runtimes = snapshot
        .runtimes
        .iter()
        .map(|runtime| DGraphRuntime {
            runtime_id: runtime.runtime_id.clone(),
            node_id: snapshot.node_id.clone(),
            runtime_kind: runtime.runtime_kind.clone(),
            status: Some(runtime.status.clone()),
            metadata: merge_source_metadata(runtime.source.as_str(), runtime.metadata.clone()),
        })
        .collect::<Vec<_>>();

    let modules = snapshot
        .modules
        .iter()
        .map(|module| DGraphModule {
            module_id: module.module_id.clone(),
            module_kind: module
                .module_kind
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            desired_state: module.desired_state.clone(),
            observed_state: module.observed_state.clone(),
            runtime: module.runtime.clone(),
            artifact_id: artifact_ids.get(&module.module_id).cloned(),
            metadata: merge_source_metadata(
                module.source.as_str(),
                Some(json!({
                    "local_mapping_key": module.local_mapping_key,
                    "local_container_name": module.local_container_name,
                    "module_metadata": module.metadata,
                })),
            ),
        })
        .collect::<Vec<_>>();

    let flows = snapshot
        .flows
        .iter()
        .map(|flow| DGraphFlow {
            flow_id: flow.flow_id.clone(),
            flow_kind: flow.flow_kind.clone(),
            modules: flow.modules.clone(),
            edges: flow
                .edges
                .iter()
                .map(|edge| DGraphFlowEdge {
                    edge_id: edge.edge_id.clone(),
                    from_module_id: edge.from_module_id.clone(),
                    to_module_id: edge.to_module_id.clone(),
                    protocol: edge.protocol.clone(),
                    status: edge.status.clone(),
                    metadata: edge.metadata.clone(),
                })
                .collect(),
            status: flow.status.clone(),
            metadata: merge_source_metadata(flow.source.as_str(), flow.metadata.clone()),
        })
        .collect::<Vec<_>>();

    let health_reports = snapshot
        .health
        .iter()
        .enumerate()
        .map(|(idx, health)| DGraphHealthReport {
            report_id: format!(
                "health:{}:{}:{}:{}",
                snapshot.node_id, health.target_kind, health.target_id, idx
            ),
            target_kind: health.target_kind.clone(),
            target_id: health.target_id.clone(),
            status: health.status.clone(),
            reason: health.reason.clone(),
            metadata: merge_source_metadata(health.source.as_str(), health.metadata.clone()),
        })
        .collect::<Vec<_>>();

    let capabilities = snapshot
        .capabilities
        .iter()
        .map(|capability| DGraphCapabilityProfile {
            capability_id: capability.capability_id.clone(),
            node_id: Some(snapshot.node_id.clone()),
            module_id: None,
            capability_kind: capability.capability_kind.clone(),
            supported: capability.supported,
            metadata: merge_source_metadata(
                capability.source.as_str(),
                capability.metadata.clone(),
            ),
        })
        .collect::<Vec<_>>();

    let mut graph = DGraph {
        project_id: snapshot.project_id.clone(),
        generated_at_utc: Some(snapshot.reported_at_utc.clone()),
        nodes: vec![DGraphNode {
            node_id: snapshot.node_id.clone(),
            display_name: None,
            node_kind: Some("datum_local_node".to_string()),
            status: Some("reported".to_string()),
            labels: vec!["datum".to_string(), "local".to_string()],
            metadata: Some(json!({
                "snapshot_id": snapshot.snapshot_id,
                "reported_at_utc": snapshot.reported_at_utc,
                "source": snapshot.source,
                "generated_at_utc": snapshot.generated_at_utc,
                "snapshot_metadata": snapshot.metadata,
            })),
        }],
        runtimes,
        modules,
        artifacts,
        flows,
        capabilities,
        health_reports,
        deployment_intents: Vec::new(),
        placements: Vec::new(),
        reconfigurations: Vec::new(),
        wasm_modules: Vec::new(),
        wasm_reports: Vec::new(),
        relationships: Vec::new(),
        metadata: Some(json!({
            "fragment_kind": "local_dgraph_snapshot",
            "snapshot_id": snapshot.snapshot_id,
            "safe_to_auto_execute": snapshot.safe_to_auto_execute,
            "side_effects_performed": snapshot.side_effects_performed,
        })),
    };
    graph.relationships = build_basic_dgraph_relationships(&graph);

    if validate_dgraph(&graph).is_err() {
        graph.metadata = Some(json!({
            "fragment_kind": "local_dgraph_snapshot",
            "snapshot_id": snapshot.snapshot_id,
            "safe_to_auto_execute": snapshot.safe_to_auto_execute,
            "side_effects_performed": snapshot.side_effects_performed,
            "validation": "partial_fragment",
        }));
    }

    graph
}

fn merge_source_metadata(source: &str, metadata: Option<Value>) -> Option<Value> {
    Some(json!({
        "source": source,
        "details": metadata,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dgraph::validate_dgraph_relationships;

    fn minimal_snapshot() -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            snapshot_id: "snapshot-1".to_string(),
            reported_at_utc: "2026-05-21T12:00:00Z".to_string(),
            source: Some("client".to_string()),
            generated_at_utc: Some("2026-05-21T11:59:00Z".to_string()),
            runtimes: Vec::new(),
            modules: Vec::new(),
            flows: Vec::new(),
            health: Vec::new(),
            capabilities: Vec::new(),
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    fn incoming_snapshot(snapshot: LocalDGraphSnapshot) -> IncomingDGraphSnapshot {
        IncomingDGraphSnapshot {
            project_id: Some(snapshot.project_id),
            node_id: Some(snapshot.node_id),
            snapshot_id: Some(snapshot.snapshot_id),
            reported_at_utc: Some(snapshot.reported_at_utc),
            source: snapshot.source,
            generated_at_utc: snapshot.generated_at_utc,
            runtimes: snapshot.runtimes,
            modules: snapshot.modules,
            flows: snapshot.flows,
            health: snapshot.health,
            capabilities: snapshot.capabilities,
            metadata: snapshot.metadata,
            safe_to_auto_execute: snapshot.safe_to_auto_execute,
            side_effects_performed: snapshot.side_effects_performed,
        }
    }

    fn populated_snapshot() -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: "runtime-container-local".to_string(),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "local_config".to_string(),
                metadata: None,
            }],
            modules: vec![
                LocalModuleSnapshot {
                    module_id: "collector".to_string(),
                    module_kind: Some("service".to_string()),
                    desired_state: Some("running".to_string()),
                    observed_state: Some("unknown".to_string()),
                    runtime: Some("container".to_string()),
                    artifact_ref: Some("ghcr.io/example/collector:v1".to_string()),
                    local_mapping_key: Some("collector".to_string()),
                    local_container_name: Some("collector".to_string()),
                    source: "service_registry".to_string(),
                    metadata: None,
                },
                LocalModuleSnapshot {
                    module_id: "broker".to_string(),
                    module_kind: Some("service".to_string()),
                    desired_state: Some("running".to_string()),
                    observed_state: Some("unknown".to_string()),
                    runtime: Some("container".to_string()),
                    artifact_ref: None,
                    local_mapping_key: None,
                    local_container_name: Some("broker".to_string()),
                    source: "service_registry".to_string(),
                    metadata: None,
                },
            ],
            flows: vec![LocalFlowSnapshot {
                flow_id: "configured-service-flow".to_string(),
                flow_kind: Some("service_dependency".to_string()),
                modules: vec!["collector".to_string(), "broker".to_string()],
                edges: vec![LocalFlowEdgeSnapshot {
                    edge_id: "edge:collector:broker:http".to_string(),
                    from_module_id: "collector".to_string(),
                    to_module_id: "broker".to_string(),
                    protocol: Some("http".to_string()),
                    status: Some("configured".to_string()),
                    metadata: None,
                }],
                status: Some("configured".to_string()),
                source: "local_config".to_string(),
                metadata: None,
            }],
            health: vec![LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: "collector".to_string(),
                status: "unknown".to_string(),
                reason: Some("reporting only".to_string()),
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
            ..minimal_snapshot()
        }
    }

    #[test]
    fn normalization_overrides_project_id_and_node_id() {
        let normalized = normalize_local_dgraph_snapshot(
            "project-a",
            "node-b",
            incoming_snapshot(LocalDGraphSnapshot {
                project_id: "wrong".to_string(),
                node_id: "wrong".to_string(),
                ..minimal_snapshot()
            }),
        );

        assert_eq!(normalized.project_id, "project-a");
        assert_eq!(normalized.node_id, "node-b");
        assert!(!normalized.snapshot_id.is_empty());
        assert!(!normalized.reported_at_utc.is_empty());
    }

    #[test]
    fn normalization_overrides_untrusted_source_with_client() {
        let normalized = normalize_local_dgraph_snapshot(
            "project-a",
            "node-b",
            incoming_snapshot(LocalDGraphSnapshot {
                source: Some("untrusted-source".to_string()),
                ..minimal_snapshot()
            }),
        );

        assert_eq!(normalized.source.as_deref(), Some("client"));
    }

    #[test]
    fn incoming_snapshot_accepts_null_server_owned_fields_and_normalizes_them() {
        let incoming: IncomingDGraphSnapshot = serde_json::from_value(json!({
            "project_id": "untrusted-project",
            "node_id": "untrusted-node",
            "snapshot_id": null,
            "reported_at_utc": null,
            "source": null,
            "safe_to_auto_execute": false,
            "side_effects_performed": false
        }))
        .expect("incoming snapshot with null canonical fields should deserialize");

        let normalized = normalize_local_dgraph_snapshot("smart-city-campinas", "fog-01", incoming);

        assert_eq!(normalized.project_id, "smart-city-campinas");
        assert_eq!(normalized.node_id, "fog-01");
        assert!(!normalized.snapshot_id.is_empty());
        assert!(!normalized.reported_at_utc.is_empty());
        assert_eq!(normalized.source.as_deref(), Some("client"));
        assert!(!normalized.safe_to_auto_execute);
        assert!(!normalized.side_effects_performed);
    }

    #[test]
    fn incoming_snapshot_accepts_absent_server_owned_fields() {
        let incoming: IncomingDGraphSnapshot = serde_json::from_value(json!({
            "safe_to_auto_execute": false,
            "side_effects_performed": false
        }))
        .expect("incoming snapshot without canonical fields should deserialize");

        let normalized = normalize_local_dgraph_snapshot("project-a", "node-b", incoming);

        assert_eq!(normalized.project_id, "project-a");
        assert_eq!(normalized.node_id, "node-b");
        assert!(!normalized.snapshot_id.is_empty());
        assert!(!normalized.reported_at_utc.is_empty());
        assert_eq!(normalized.source.as_deref(), Some("client"));
    }

    #[test]
    fn validation_accepts_minimal_valid_empty_snapshot() {
        assert!(validate_local_dgraph_snapshot(&minimal_snapshot()).is_ok());
    }

    #[test]
    fn validation_rejects_safe_to_auto_execute_true() {
        let mut snapshot = minimal_snapshot();
        snapshot.safe_to_auto_execute = true;
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_side_effects_performed_true() {
        let mut snapshot = minimal_snapshot();
        snapshot.side_effects_performed = true;
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_empty_runtime_id() {
        let mut snapshot = minimal_snapshot();
        snapshot.runtimes.push(LocalRuntimeSnapshot {
            runtime_id: "".to_string(),
            runtime_kind: "container".to_string(),
            status: "configured".to_string(),
            source: "local_config".to_string(),
            metadata: None,
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_empty_module_id() {
        let mut snapshot = minimal_snapshot();
        snapshot.modules.push(LocalModuleSnapshot {
            module_id: "".to_string(),
            source: "service_registry".to_string(),
            ..LocalModuleSnapshot::default()
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_empty_flow_id() {
        let mut snapshot = minimal_snapshot();
        snapshot.flows.push(LocalFlowSnapshot {
            flow_id: "".to_string(),
            source: "local_config".to_string(),
            ..LocalFlowSnapshot::default()
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_flow_module_reference_missing_from_modules() {
        let mut snapshot = minimal_snapshot();
        snapshot.flows.push(LocalFlowSnapshot {
            flow_id: "flow-1".to_string(),
            modules: vec!["missing".to_string()],
            source: "local_config".to_string(),
            ..LocalFlowSnapshot::default()
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_flow_edge_reference_missing_from_modules() {
        let mut snapshot = minimal_snapshot();
        snapshot.modules.push(LocalModuleSnapshot {
            module_id: "collector".to_string(),
            source: "service_registry".to_string(),
            ..LocalModuleSnapshot::default()
        });
        snapshot.flows.push(LocalFlowSnapshot {
            flow_id: "flow-1".to_string(),
            modules: vec!["collector".to_string()],
            edges: vec![LocalFlowEdgeSnapshot {
                edge_id: "edge-1".to_string(),
                from_module_id: "collector".to_string(),
                to_module_id: "missing".to_string(),
                ..LocalFlowEdgeSnapshot::default()
            }],
            source: "local_config".to_string(),
            ..LocalFlowSnapshot::default()
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_empty_health_target() {
        let mut snapshot = minimal_snapshot();
        snapshot.health.push(LocalHealthSnapshot {
            target_kind: "".to_string(),
            target_id: "".to_string(),
            status: "unknown".to_string(),
            source: "local_config".to_string(),
            metadata: None,
            reason: None,
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn validation_rejects_empty_capability_id_or_kind() {
        let mut snapshot = minimal_snapshot();
        snapshot.capabilities.push(LocalCapabilitySnapshot {
            capability_id: "".to_string(),
            capability_kind: "".to_string(),
            supported: false,
            source: "local_config".to_string(),
            metadata: None,
        });
        assert!(validate_local_dgraph_snapshot(&snapshot).is_err());
    }

    #[test]
    fn conversion_to_dgraph_creates_node() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.nodes[0].node_id, "fog-01");
    }

    #[test]
    fn conversion_to_dgraph_creates_runtimes() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert_eq!(graph.runtimes.len(), 1);
        assert_eq!(graph.runtimes[0].runtime_kind, "container");
    }

    #[test]
    fn conversion_to_dgraph_creates_modules() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert_eq!(graph.modules.len(), 2);
        assert!(graph
            .modules
            .iter()
            .any(|module| module.module_id == "collector"));
    }

    #[test]
    fn conversion_to_dgraph_creates_flows() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert_eq!(graph.flows.len(), 1);
        assert_eq!(graph.flows[0].edges.len(), 1);
    }

    #[test]
    fn conversion_to_dgraph_creates_health_reports() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert_eq!(graph.health_reports.len(), 1);
        assert_eq!(graph.health_reports[0].target_id, "collector");
    }

    #[test]
    fn conversion_to_dgraph_creates_capabilities() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert_eq!(graph.capabilities.len(), 1);
        assert_eq!(
            graph.capabilities[0].capability_kind,
            "container_runtime_configured"
        );
    }

    #[test]
    fn converted_fragment_relationships_include_has_runtime_and_contains() {
        let graph = local_snapshot_to_dgraph_fragment(&populated_snapshot());
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == "HAS_RUNTIME"
                && relationship.from_id == "fog-01"
                && relationship.to_id == "runtime-container-local"
        }));
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == "CONTAINS"
                && relationship.from_id == "configured-service-flow"
                && relationship.to_id == "collector"
        }));
        assert!(validate_dgraph_relationships(&graph).is_ok());
    }

    #[test]
    fn validation_collects_multiple_errors() {
        let mut snapshot = minimal_snapshot();
        snapshot.safe_to_auto_execute = true;
        snapshot.side_effects_performed = true;
        snapshot.runtimes.push(LocalRuntimeSnapshot {
            runtime_id: "".to_string(),
            runtime_kind: "".to_string(),
            status: "".to_string(),
            source: "local_config".to_string(),
            metadata: None,
        });

        let errors = validate_local_dgraph_snapshot(&snapshot).expect_err("should fail");
        assert!(errors.len() >= 5);
    }
}
