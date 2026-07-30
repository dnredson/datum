use std::collections::BTreeMap;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::{
    dgraph::{
        build_basic_dgraph_relationships, summarize_dgraph, validate_dgraph, DGraph,
        DGraphArtifact, DGraphCapabilityProfile, DGraphDeploymentIntent, DGraphFlow,
        DGraphHealthReport, DGraphModule, DGraphNode, DGraphPlacementDecision,
        DGraphReconfigurationDecision, DGraphRelationship, DGraphRuntime, DGraphSummary,
        DGraphWasmModuleRef, DGraphWasmRuntimeReportRef,
    },
    local_dgraph_snapshot::{local_snapshot_to_dgraph_fragment, LocalDGraphSnapshot},
};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ProjectDGraphMaterialization {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub source_snapshot_count: u64,
    #[serde(default)]
    pub graph: DGraph,
    #[serde(default)]
    pub summary: DGraphSummary,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    #[serde(default)]
    pub side_effects_performed: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphMaterializationSummary {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub source_snapshot_count: u64,
    #[serde(default)]
    pub summary: DGraphSummary,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    #[serde(default)]
    pub side_effects_performed: bool,
}

pub fn materialize_project_dgraph_from_snapshots(
    project_id: &str,
    snapshots: &[LocalDGraphSnapshot],
) -> DGraph {
    let mut graph = DGraph {
        project_id: project_id.to_string(),
        generated_at_utc: latest_generated_at(snapshots),
        ..DGraph::default()
    };

    let mut nodes = BTreeMap::<String, DGraphNode>::new();
    let mut runtimes = BTreeMap::<String, DGraphRuntime>::new();
    let mut modules = BTreeMap::<String, DGraphModule>::new();
    let mut artifacts = BTreeMap::<String, DGraphArtifact>::new();
    let mut flows = BTreeMap::<String, DGraphFlow>::new();
    let mut capabilities = BTreeMap::<String, DGraphCapabilityProfile>::new();
    let mut health_reports = BTreeMap::<String, DGraphHealthReport>::new();
    let mut deployment_intents = BTreeMap::<String, DGraphDeploymentIntent>::new();
    let mut placements = BTreeMap::<String, DGraphPlacementDecision>::new();
    let mut reconfigurations = BTreeMap::<String, DGraphReconfigurationDecision>::new();
    let mut wasm_modules = BTreeMap::<String, DGraphWasmModuleRef>::new();
    let mut wasm_reports = BTreeMap::<String, DGraphWasmRuntimeReportRef>::new();
    let mut relationships = BTreeMap::<String, DGraphRelationship>::new();

    for snapshot in snapshots {
        let fragment = local_snapshot_to_dgraph_fragment(snapshot);
        let canonical_runtime_ids = fragment
            .runtimes
            .iter()
            .map(|runtime| {
                (
                    runtime.runtime_id.clone(),
                    canonical_runtime_id(project_id, &runtime.node_id, &runtime.runtime_id),
                )
            })
            .collect::<BTreeMap<_, _>>();

        for node in fragment.nodes {
            merge_node(&mut nodes, node);
        }
        for runtime in fragment.runtimes {
            let runtime = node_scope_runtime(project_id, runtime);
            runtimes
                .entry(runtime.runtime_id.clone())
                .or_insert(runtime);
        }
        for module in fragment.modules {
            modules.entry(module.module_id.clone()).or_insert(module);
        }
        for artifact in fragment.artifacts {
            artifacts
                .entry(artifact.artifact_id.clone())
                .or_insert(artifact);
        }
        for flow in fragment.flows {
            merge_flow(&mut flows, flow);
        }
        for capability in fragment.capabilities {
            capabilities
                .entry(capability.capability_id.clone())
                .or_insert(capability);
        }
        for mut report in fragment.health_reports {
            if report.target_kind.eq_ignore_ascii_case("runtime") {
                if let Some(canonical_id) = canonical_runtime_ids.get(&report.target_id) {
                    report.target_id = canonical_id.clone();
                }
            }
            health_reports
                .entry(report.report_id.clone())
                .or_insert(report);
        }
        for intent in fragment.deployment_intents {
            deployment_intents
                .entry(intent.intent_id.clone())
                .or_insert(intent);
        }
        for placement in fragment.placements {
            placements
                .entry(placement.placement_id.clone())
                .or_insert(placement);
        }
        for reconfiguration in fragment.reconfigurations {
            reconfigurations
                .entry(reconfiguration.reconfiguration_id.clone())
                .or_insert(reconfiguration);
        }
        for wasm_module in fragment.wasm_modules {
            wasm_modules
                .entry(wasm_module_ref_key(&wasm_module))
                .or_insert(wasm_module);
        }
        for wasm_report in fragment.wasm_reports {
            wasm_reports
                .entry(wasm_report.report_id.clone())
                .or_insert(wasm_report);
        }
        for mut relationship in fragment.relationships {
            canonicalize_runtime_relationship(&mut relationship, &canonical_runtime_ids);
            relationships
                .entry(relationship_key(&relationship))
                .or_insert(relationship);
        }
    }

    graph.nodes = nodes.into_values().collect();
    graph.runtimes = runtimes.into_values().collect();
    graph.modules = modules.into_values().collect();
    graph.artifacts = artifacts.into_values().collect();
    graph.flows = flows.into_values().collect();
    graph.capabilities = capabilities.into_values().collect();
    graph.health_reports = health_reports.into_values().collect();
    graph.deployment_intents = deployment_intents.into_values().collect();
    graph.placements = placements.into_values().collect();
    graph.reconfigurations = reconfigurations.into_values().collect();
    graph.wasm_modules = wasm_modules.into_values().collect();
    graph.wasm_reports = wasm_reports.into_values().collect();

    let rebuilt_relationships = build_basic_dgraph_relationships(&graph);
    for relationship in rebuilt_relationships {
        relationships
            .entry(relationship_key(&relationship))
            .or_insert(relationship);
    }
    graph.relationships = relationships.into_values().collect();
    graph.metadata = Some(json!({
        "materialization_kind": "project_local_dgraph_snapshots",
        "source_snapshot_count": snapshots.len(),
        "source_snapshots": snapshots
            .iter()
            .map(|snapshot| {
                json!({
                    "project_id": snapshot.project_id,
                    "node_id": snapshot.node_id,
                    "snapshot_id": snapshot.snapshot_id,
                    "reported_at_utc": snapshot.reported_at_utc,
                    "generated_at_utc": snapshot.generated_at_utc,
                    "source": snapshot.source,
                    "side_effects_performed": snapshot.side_effects_performed,
                    "safe_to_auto_execute": snapshot.safe_to_auto_execute,
                })
            })
            .collect::<Vec<_>>(),
    }));

    graph
}

pub fn canonical_runtime_id(project_id: &str, node_id: &str, local_runtime_id: &str) -> String {
    format!("project:{project_id}:node:{node_id}:runtime:{local_runtime_id}")
}

fn node_scope_runtime(project_id: &str, mut runtime: DGraphRuntime) -> DGraphRuntime {
    let local_runtime_id = runtime.runtime_id.clone();
    runtime.runtime_id = canonical_runtime_id(project_id, &runtime.node_id, &local_runtime_id);
    runtime.metadata = Some(match runtime.metadata {
        Some(Value::Object(mut metadata)) => {
            metadata.insert("local_runtime_id".to_string(), json!(local_runtime_id));
            Value::Object(metadata)
        }
        Some(metadata) => json!({
            "local_runtime_id": local_runtime_id,
            "runtime_metadata": metadata,
        }),
        None => json!({ "local_runtime_id": local_runtime_id }),
    });
    runtime
}

fn canonicalize_runtime_relationship(
    relationship: &mut DGraphRelationship,
    canonical_runtime_ids: &BTreeMap<String, String>,
) {
    if relationship.from_kind.eq_ignore_ascii_case("runtime") {
        if let Some(canonical_id) = canonical_runtime_ids.get(&relationship.from_id) {
            relationship.from_id = canonical_id.clone();
        }
    }
    if relationship.to_kind.eq_ignore_ascii_case("runtime") {
        if let Some(canonical_id) = canonical_runtime_ids.get(&relationship.to_id) {
            relationship.to_id = canonical_id.clone();
        }
    }

    relationship.relationship_id = format!(
        "rel:{}:{}:{}:{}:{}",
        relationship.from_kind.to_lowercase(),
        relationship.from_id,
        relationship.relation.to_lowercase(),
        relationship.to_kind.to_lowercase(),
        relationship.to_id,
    );
}

pub fn build_project_dgraph_materialization(
    project_id: &str,
    snapshots: &[LocalDGraphSnapshot],
) -> ProjectDGraphMaterialization {
    let graph = materialize_project_dgraph_from_snapshots(project_id, snapshots);
    let validation_errors = validate_dgraph(&graph).err().unwrap_or_default();
    let summary = summarize_dgraph(&graph);

    ProjectDGraphMaterialization {
        project_id: project_id.to_string(),
        generated_at_utc: Some(Utc::now().to_rfc3339()),
        source_snapshot_count: snapshots.len() as u64,
        graph,
        summary,
        validation_errors,
        side_effects_performed: false,
        metadata: Some(json!({
            "materialization_kind": "project_local_dgraph_snapshots",
            "latest_snapshot_reported_at_utc": latest_reported_at(snapshots),
        })),
    }
}

impl ProjectDGraphMaterialization {
    pub fn as_summary(&self) -> ProjectDGraphMaterializationSummary {
        ProjectDGraphMaterializationSummary {
            project_id: self.project_id.clone(),
            source_snapshot_count: self.source_snapshot_count,
            summary: self.summary.clone(),
            validation_errors: self.validation_errors.clone(),
            side_effects_performed: self.side_effects_performed,
        }
    }
}

fn latest_generated_at(snapshots: &[LocalDGraphSnapshot]) -> Option<String> {
    snapshots
        .iter()
        .filter_map(|snapshot| {
            snapshot
                .generated_at_utc
                .as_ref()
                .or(Some(&snapshot.reported_at_utc))
        })
        .max()
        .cloned()
}

fn latest_reported_at(snapshots: &[LocalDGraphSnapshot]) -> Option<String> {
    snapshots
        .iter()
        .map(|snapshot| &snapshot.reported_at_utc)
        .max()
        .cloned()
}

fn relationship_key(relationship: &DGraphRelationship) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        relationship.from_kind,
        relationship.from_id,
        relationship.relation,
        relationship.to_kind,
        relationship.to_id
    )
}

fn wasm_module_ref_key(wasm_module: &DGraphWasmModuleRef) -> String {
    format!(
        "{}|{}|{}",
        wasm_module.module_id,
        wasm_module.wasm_module_id,
        wasm_module.function_name.as_deref().unwrap_or("")
    )
}

fn merge_node(nodes: &mut BTreeMap<String, DGraphNode>, node: DGraphNode) {
    if let Some(existing) = nodes.get_mut(&node.node_id) {
        if existing.display_name.is_none() {
            existing.display_name = node.display_name;
        }
        if existing.node_kind.is_none() {
            existing.node_kind = node.node_kind;
        }
        if existing.status.is_none() {
            existing.status = node.status;
        }
        existing.labels.extend(node.labels);
        existing.labels.sort();
        existing.labels.dedup();
        if existing.metadata.is_none() {
            existing.metadata = node.metadata;
        }
        return;
    }

    nodes.insert(node.node_id.clone(), node);
}

fn merge_flow(flows: &mut BTreeMap<String, DGraphFlow>, flow: DGraphFlow) {
    if let Some(existing) = flows.get_mut(&flow.flow_id) {
        if existing.flow_kind.is_none() {
            existing.flow_kind = flow.flow_kind;
        }
        if existing.status.is_none() {
            existing.status = flow.status;
        }
        if existing.metadata.is_none() {
            existing.metadata = flow.metadata;
        }

        for module_id in flow.modules {
            if !existing.modules.contains(&module_id) {
                existing.modules.push(module_id);
            }
        }
        existing.modules.sort();
        existing.modules.dedup();

        let mut edge_map = existing
            .edges
            .iter()
            .cloned()
            .map(|edge| (edge.edge_id.clone(), edge))
            .collect::<BTreeMap<_, _>>();
        for edge in flow.edges {
            edge_map.entry(edge.edge_id.clone()).or_insert(edge);
        }
        existing.edges = edge_map.into_values().collect();
        return;
    }

    flows.insert(flow.flow_id.clone(), flow);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::local_dgraph_snapshot::{
        LocalCapabilitySnapshot, LocalDGraphSnapshot, LocalFlowEdgeSnapshot, LocalFlowSnapshot,
        LocalHealthSnapshot, LocalModuleSnapshot, LocalRuntimeSnapshot,
    };

    fn snapshot(node_id: &str, snapshot_id: &str) -> LocalDGraphSnapshot {
        LocalDGraphSnapshot {
            project_id: "smart-city-campinas".to_string(),
            node_id: node_id.to_string(),
            snapshot_id: snapshot_id.to_string(),
            reported_at_utc: format!(
                "2026-05-21T12:00:0{}Z",
                if node_id == "fog-01" { 1 } else { 2 }
            ),
            source: Some("client".to_string()),
            generated_at_utc: Some("2026-05-21T11:59:00Z".to_string()),
            runtimes: vec![LocalRuntimeSnapshot {
                runtime_id: format!("runtime-{}", node_id),
                runtime_kind: "container".to_string(),
                status: "configured".to_string(),
                source: "local_config".to_string(),
                metadata: None,
            }],
            modules: vec![LocalModuleSnapshot {
                module_id: format!("collector-{}", node_id),
                module_kind: Some("service".to_string()),
                desired_state: Some("running".to_string()),
                observed_state: Some("unknown".to_string()),
                runtime: Some("container".to_string()),
                artifact_ref: Some(format!("ghcr.io/example/collector:{}", node_id)),
                local_mapping_key: Some(format!("mapping-{}", node_id)),
                local_container_name: Some(format!("container-{}", node_id)),
                source: "service_registry".to_string(),
                metadata: None,
            }],
            flows: vec![LocalFlowSnapshot {
                flow_id: format!("flow-{}", node_id),
                flow_kind: Some("service_dependency".to_string()),
                modules: vec![format!("collector-{}", node_id)],
                edges: Vec::new(),
                status: Some("configured".to_string()),
                source: "local_config".to_string(),
                metadata: None,
            }],
            health: vec![LocalHealthSnapshot {
                target_kind: "module".to_string(),
                target_id: format!("collector-{}", node_id),
                status: "unknown".to_string(),
                reason: Some("reporting only".to_string()),
                source: "local_config".to_string(),
                metadata: None,
            }],
            capabilities: vec![LocalCapabilitySnapshot {
                capability_id: format!("capability-{}", node_id),
                capability_kind: "container_runtime_configured".to_string(),
                supported: true,
                source: "local_config".to_string(),
                metadata: None,
            }],
            metadata: None,
            safe_to_auto_execute: false,
            side_effects_performed: false,
        }
    }

    #[test]
    fn materializing_empty_snapshot_list_produces_empty_graph_with_project_id() {
        let graph = materialize_project_dgraph_from_snapshots("smart-city-campinas", &[]);
        assert_eq!(graph.project_id, "smart-city-campinas");
        assert!(graph.nodes.is_empty());
        assert!(graph.relationships.is_empty());
    }

    #[test]
    fn materializing_one_snapshot_creates_expected_entities() {
        let graph = materialize_project_dgraph_from_snapshots(
            "smart-city-campinas",
            &[snapshot("fog-01", "snap-1")],
        );
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.runtimes.len(), 1);
        assert_eq!(graph.modules.len(), 1);
        assert_eq!(graph.flows.len(), 1);
    }

    #[test]
    fn materializing_two_snapshots_creates_two_nodes() {
        let graph = materialize_project_dgraph_from_snapshots(
            "smart-city-campinas",
            &[snapshot("fog-01", "snap-1"), snapshot("fog-02", "snap-2")],
        );
        assert_eq!(graph.nodes.len(), 2);
    }

    #[test]
    fn duplicate_module_ids_are_deduplicated() {
        let first = snapshot("fog-01", "snap-1");
        let mut second = snapshot("fog-01", "snap-2");
        second.modules[0].module_id = first.modules[0].module_id.clone();
        second.flows[0].flow_id = first.flows[0].flow_id.clone();
        second.flows[0].modules = vec![first.modules[0].module_id.clone()];

        let graph =
            materialize_project_dgraph_from_snapshots("smart-city-campinas", &[first, second]);
        assert_eq!(graph.modules.len(), 1);
    }

    #[test]
    fn node_scoped_ids_remain_deterministic_for_runtimes_and_artifacts() {
        let graph = materialize_project_dgraph_from_snapshots(
            "smart-city-campinas",
            &[snapshot("fog-01", "snap-1")],
        );
        assert_eq!(
            graph.runtimes[0].runtime_id,
            "project:smart-city-campinas:node:fog-01:runtime:runtime-fog-01"
        );
        assert_eq!(
            graph.runtimes[0]
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("local_runtime_id")),
            Some(&json!("runtime-fog-01"))
        );
        assert_eq!(
            graph.artifacts[0].artifact_id,
            "artifact:fog-01:collector-fog-01"
        );
    }

    #[test]
    fn merged_relationships_include_has_runtime_and_contains() {
        let graph = materialize_project_dgraph_from_snapshots(
            "smart-city-campinas",
            &[snapshot("fog-01", "snap-1")],
        );
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == "HAS_RUNTIME"
                && relationship.from_id == "fog-01"
                && relationship.to_id
                    == "project:smart-city-campinas:node:fog-01:runtime:runtime-fog-01"
        }));
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == "CONTAINS"
                && relationship.from_id == "flow-fog-01"
                && relationship.to_id == "collector-fog-01"
        }));
    }

    #[test]
    fn summary_counts_match_merged_graph() {
        let materialization = build_project_dgraph_materialization(
            "smart-city-campinas",
            &[snapshot("fog-01", "snap-1"), snapshot("fog-02", "snap-2")],
        );
        assert_eq!(materialization.summary.node_count, 2);
        assert_eq!(materialization.summary.runtime_count, 2);
        assert_eq!(materialization.summary.module_count, 2);
        assert_eq!(
            materialization.summary.relationship_count,
            materialization.graph.relationships.len() as u64
        );
    }

    #[test]
    fn same_local_runtime_id_on_two_nodes_remains_node_scoped() {
        let mut first = snapshot("fog-01", "snap-1");
        let mut second = snapshot("fog-02", "snap-2");
        first.runtimes[0].runtime_id = "runtime-container-local".to_string();
        second.runtimes[0].runtime_id = "runtime-container-local".to_string();

        let materialization =
            build_project_dgraph_materialization("smart-city-campinas", &[first, second]);
        let runtime_ids = materialization
            .graph
            .runtimes
            .iter()
            .map(|runtime| runtime.runtime_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(materialization.summary.runtime_count, 2);
        assert!(runtime_ids
            .contains(&"project:smart-city-campinas:node:fog-01:runtime:runtime-container-local"));
        assert!(runtime_ids
            .contains(&"project:smart-city-campinas:node:fog-02:runtime:runtime-container-local"));
        assert!(materialization.graph.runtimes.iter().all(|runtime| {
            runtime
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("local_runtime_id"))
                == Some(&json!("runtime-container-local"))
        }));
    }

    #[test]
    fn one_node_runtime_is_materialized_once_with_its_local_identity_traceable() {
        let mut single = snapshot("fog-01", "snap-1");
        single.runtimes[0].runtime_id = "runtime-container-local".to_string();

        let graph = materialize_project_dgraph_from_snapshots("smart-city-campinas", &[single]);

        assert_eq!(graph.runtimes.len(), 1);
        assert_eq!(
            graph.runtimes[0].runtime_id,
            "project:smart-city-campinas:node:fog-01:runtime:runtime-container-local"
        );
        assert_eq!(
            graph.runtimes[0]
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("local_runtime_id")),
            Some(&json!("runtime-container-local"))
        );
    }

    #[test]
    fn validation_errors_are_collected_and_side_effects_remain_false() {
        let mut broken = snapshot("fog-01", "snap-1");
        broken.flows[0].modules.push("missing-module".to_string());

        let materialization =
            build_project_dgraph_materialization("smart-city-campinas", &[broken]);
        assert!(!materialization.validation_errors.is_empty());
        assert!(!materialization.side_effects_performed);
    }

    #[test]
    fn materialization_does_not_run_planner_or_emit_side_effects() {
        let materialization = build_project_dgraph_materialization(
            "smart-city-campinas",
            &[snapshot("fog-01", "snap-1")],
        );
        assert!(materialization.graph.placements.is_empty());
        assert!(materialization.graph.reconfigurations.is_empty());
        assert!(!materialization.side_effects_performed);
    }

    #[test]
    fn duplicate_flow_edges_are_deduplicated_when_merging_flows() {
        let mut first = snapshot("fog-01", "snap-1");
        let mut second = snapshot("fog-01", "snap-2");
        first.modules.push(LocalModuleSnapshot {
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
        });
        first.flows[0].modules.push("broker".to_string());
        first.flows[0].edges.push(LocalFlowEdgeSnapshot {
            edge_id: "edge-1".to_string(),
            from_module_id: "collector-fog-01".to_string(),
            to_module_id: "broker".to_string(),
            protocol: Some("http".to_string()),
            status: Some("configured".to_string()),
            metadata: None,
        });
        second.modules = first.modules.clone();
        second.flows[0] = first.flows[0].clone();

        let graph =
            materialize_project_dgraph_from_snapshots("smart-city-campinas", &[first, second]);
        assert_eq!(graph.flows.len(), 1);
        assert_eq!(graph.flows[0].edges.len(), 1);
    }
}
