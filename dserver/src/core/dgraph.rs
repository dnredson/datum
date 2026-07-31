#![allow(dead_code)]

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const REL_HAS_RUNTIME: &str = "HAS_RUNTIME";
const REL_HOSTS: &str = "HOSTS";
const REL_USES: &str = "USES";
const REL_CONTAINS: &str = "CONTAINS";
const REL_CONNECTS: &str = "CONNECTS";
const REL_PLACED_ON: &str = "PLACED_ON";
const REL_OBSERVES: &str = "OBSERVES";
const REL_TARGETS: &str = "TARGETS";
const REL_SELECTS: &str = "SELECTS";
const REL_REPLACES: &str = "REPLACES";
const REL_DESCRIBES: &str = "DESCRIBES";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DGraph {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub nodes: Vec<DGraphNode>,
    #[serde(default)]
    pub runtimes: Vec<DGraphRuntime>,
    #[serde(default)]
    pub modules: Vec<DGraphModule>,
    #[serde(default)]
    pub artifacts: Vec<DGraphArtifact>,
    #[serde(default)]
    pub flows: Vec<DGraphFlow>,
    #[serde(default)]
    pub capabilities: Vec<DGraphCapabilityProfile>,
    #[serde(default)]
    pub health_reports: Vec<DGraphHealthReport>,
    #[serde(default)]
    pub deployment_intents: Vec<DGraphDeploymentIntent>,
    #[serde(default)]
    pub placements: Vec<DGraphPlacementDecision>,
    #[serde(default)]
    pub reconfigurations: Vec<DGraphReconfigurationDecision>,
    #[serde(default)]
    pub wasm_modules: Vec<DGraphWasmModuleRef>,
    #[serde(default)]
    pub wasm_reports: Vec<DGraphWasmRuntimeReportRef>,
    #[serde(default)]
    pub relationships: Vec<DGraphRelationship>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphNode {
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub node_kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphRuntime {
    #[serde(default)]
    pub runtime_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub runtime_kind: String,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphModule {
    #[serde(default)]
    pub module_id: String,
    #[serde(default)]
    pub module_kind: String,
    #[serde(default)]
    pub desired_state: Option<String>,
    #[serde(default)]
    pub observed_state: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphArtifact {
    #[serde(default)]
    pub artifact_id: String,
    #[serde(default)]
    pub artifact_kind: String,
    #[serde(default)]
    pub artifact_ref: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphFlow {
    #[serde(default)]
    pub flow_id: String,
    #[serde(default)]
    pub flow_kind: Option<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub edges: Vec<DGraphFlowEdge>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphFlowEdge {
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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphCapabilityProfile {
    #[serde(default)]
    pub capability_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub capability_kind: String,
    #[serde(default)]
    pub supported: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphHealthReport {
    #[serde(default)]
    pub report_id: String,
    #[serde(default)]
    pub target_kind: String,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphDeploymentIntent {
    #[serde(default)]
    pub intent_id: String,
    #[serde(default)]
    pub target_kind: String,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub desired_state: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlacementDecision {
    #[serde(default)]
    pub placement_id: String,
    #[serde(default)]
    pub module_id: String,
    #[serde(default)]
    pub selected_node_id: String,
    #[serde(default)]
    pub selected_runtime_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphReconfigurationDecision {
    #[serde(default)]
    pub reconfiguration_id: String,
    #[serde(default)]
    pub flow_id: Option<String>,
    #[serde(default)]
    pub affected_module_id: Option<String>,
    #[serde(default)]
    pub from_node_id: Option<String>,
    #[serde(default)]
    pub to_node_id: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphWasmModuleRef {
    #[serde(default)]
    pub module_id: String,
    #[serde(default)]
    pub wasm_module_id: String,
    #[serde(default)]
    pub function_name: Option<String>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphWasmRuntimeReportRef {
    #[serde(default)]
    pub report_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub wasm_module_id: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphRelationship {
    #[serde(default)]
    pub relationship_id: String,
    #[serde(default)]
    pub from_kind: String,
    #[serde(default)]
    pub from_id: String,
    #[serde(default)]
    pub relation: String,
    #[serde(default)]
    pub to_kind: String,
    #[serde(default)]
    pub to_id: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphSummary {
    pub node_count: u64,
    pub runtime_count: u64,
    pub module_count: u64,
    pub artifact_count: u64,
    pub flow_count: u64,
    pub relationship_count: u64,
    #[serde(default)]
    pub healthy_targets: u64,
    #[serde(default)]
    pub unknown_targets: u64,
    #[serde(default)]
    pub degraded_targets: u64,
    #[serde(default)]
    pub unavailable_targets: u64,
    pub unhealthy_targets: u64,
    pub pending_deployment_intents: u64,
    pub proposed_placements: u64,
    pub proposed_reconfigurations: u64,
}

pub fn validate_dgraph(graph: &DGraph) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if graph.project_id.trim().is_empty() {
        errors.push("project_id must not be empty".to_string());
    }

    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<BTreeSet<_>>();
    let runtime_ids = graph
        .runtimes
        .iter()
        .map(|runtime| runtime.runtime_id.as_str())
        .collect::<BTreeSet<_>>();
    let module_ids = graph
        .modules
        .iter()
        .map(|module| module.module_id.as_str())
        .collect::<BTreeSet<_>>();
    let artifact_ids = graph
        .artifacts
        .iter()
        .map(|artifact| artifact.artifact_id.as_str())
        .collect::<BTreeSet<_>>();
    for node in &graph.nodes {
        if node.node_id.trim().is_empty() {
            errors.push("node_id must not be empty".to_string());
        }
    }

    for runtime in &graph.runtimes {
        if runtime.runtime_id.trim().is_empty() {
            errors.push("runtime_id must not be empty".to_string());
        }
        if runtime.node_id.trim().is_empty() {
            errors.push(format!(
                "runtime '{}' must reference a non-empty node_id",
                runtime.runtime_id
            ));
        } else if !node_ids.contains(runtime.node_id.as_str()) {
            errors.push(format!(
                "runtime '{}' references missing node '{}'",
                runtime.runtime_id, runtime.node_id
            ));
        }
    }

    for module in &graph.modules {
        if module.module_id.trim().is_empty() {
            errors.push("module_id must not be empty".to_string());
        }
        if let Some(artifact_id) = module.artifact_id.as_deref() {
            if artifact_id.trim().is_empty() {
                errors.push(format!(
                    "module '{}' has empty artifact_id reference",
                    module.module_id
                ));
            } else if !artifact_ids.contains(artifact_id) {
                errors.push(format!(
                    "module '{}' references missing artifact '{}'",
                    module.module_id, artifact_id
                ));
            }
        }
    }

    for artifact in &graph.artifacts {
        if artifact.artifact_id.trim().is_empty() {
            errors.push("artifact_id must not be empty".to_string());
        }
    }

    for flow in &graph.flows {
        if flow.flow_id.trim().is_empty() {
            errors.push("flow_id must not be empty".to_string());
        }

        for module_id in &flow.modules {
            if module_id.trim().is_empty() {
                errors.push(format!(
                    "flow '{}' contains an empty module reference",
                    flow.flow_id
                ));
            } else if !module_ids.contains(module_id.as_str()) {
                errors.push(format!(
                    "flow '{}' references missing module '{}'",
                    flow.flow_id, module_id
                ));
            }
        }

        for edge in &flow.edges {
            if edge.edge_id.trim().is_empty() {
                errors.push(format!(
                    "flow '{}' contains edge with empty edge_id",
                    flow.flow_id
                ));
            }
            if edge.from_module_id.trim().is_empty() {
                errors.push(format!(
                    "flow edge '{}' has empty from_module_id",
                    edge.edge_id
                ));
            } else if !module_ids.contains(edge.from_module_id.as_str()) {
                errors.push(format!(
                    "flow edge '{}' references missing from_module_id '{}'",
                    edge.edge_id, edge.from_module_id
                ));
            }
            if edge.to_module_id.trim().is_empty() {
                errors.push(format!(
                    "flow edge '{}' has empty to_module_id",
                    edge.edge_id
                ));
            } else if !module_ids.contains(edge.to_module_id.as_str()) {
                errors.push(format!(
                    "flow edge '{}' references missing to_module_id '{}'",
                    edge.edge_id, edge.to_module_id
                ));
            }
        }
    }

    for placement in &graph.placements {
        if placement.placement_id.trim().is_empty() {
            errors.push("placement_id must not be empty".to_string());
        }
        if placement.module_id.trim().is_empty() {
            errors.push(format!(
                "placement '{}' must reference a non-empty module_id",
                placement.placement_id
            ));
        } else if !module_ids.contains(placement.module_id.as_str()) {
            errors.push(format!(
                "placement '{}' references missing module '{}'",
                placement.placement_id, placement.module_id
            ));
        }
        if placement.selected_node_id.trim().is_empty() {
            errors.push(format!(
                "placement '{}' must reference a non-empty selected_node_id",
                placement.placement_id
            ));
        } else if !node_ids.contains(placement.selected_node_id.as_str()) {
            errors.push(format!(
                "placement '{}' references missing node '{}'",
                placement.placement_id, placement.selected_node_id
            ));
        }
        if let Some(runtime_id) = placement.selected_runtime_id.as_deref() {
            if runtime_id.trim().is_empty() {
                errors.push(format!(
                    "placement '{}' has empty selected_runtime_id reference",
                    placement.placement_id
                ));
            } else if !runtime_ids.contains(runtime_id) {
                errors.push(format!(
                    "placement '{}' references missing runtime '{}'",
                    placement.placement_id, runtime_id
                ));
            }
        }
    }

    for report in &graph.health_reports {
        if report.report_id.trim().is_empty() {
            errors.push("health report_id must not be empty".to_string());
        }
        if report.target_kind.trim().is_empty() {
            errors.push(format!(
                "health report '{}' must not have empty target_kind",
                report.report_id
            ));
        }
        if report.target_id.trim().is_empty() {
            errors.push(format!(
                "health report '{}' must not have empty target_id",
                report.report_id
            ));
        } else if !is_known_target(
            graph,
            report.target_kind.as_str(),
            report.target_id.as_str(),
        ) {
            errors.push(format!(
                "health report '{}' references unknown {} '{}'",
                report.report_id, report.target_kind, report.target_id
            ));
        }
    }

    for intent in &graph.deployment_intents {
        if intent.intent_id.trim().is_empty() {
            errors.push("deployment intent_id must not be empty".to_string());
        }
        if intent.target_kind.trim().is_empty() {
            errors.push(format!(
                "deployment intent '{}' must not have empty target_kind",
                intent.intent_id
            ));
        }
        if intent.target_id.trim().is_empty() {
            errors.push(format!(
                "deployment intent '{}' must not have empty target_id",
                intent.intent_id
            ));
        } else if !is_known_target(
            graph,
            intent.target_kind.as_str(),
            intent.target_id.as_str(),
        ) {
            errors.push(format!(
                "deployment intent '{}' references unknown {} '{}'",
                intent.intent_id, intent.target_kind, intent.target_id
            ));
        }
    }

    if let Err(mut relationship_errors) = validate_dgraph_relationships(graph) {
        errors.append(&mut relationship_errors);
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn validate_dgraph_relationships(graph: &DGraph) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    for relationship in &graph.relationships {
        if relationship.relationship_id.trim().is_empty() {
            errors.push("relationship_id must not be empty".to_string());
        }
        if relationship.from_kind.trim().is_empty() {
            errors.push(format!(
                "relationship '{}' has empty from_kind",
                relationship.relationship_id
            ));
        }
        if relationship.from_id.trim().is_empty() {
            errors.push(format!(
                "relationship '{}' has empty from_id",
                relationship.relationship_id
            ));
        } else if !is_known_target(
            graph,
            relationship.from_kind.as_str(),
            relationship.from_id.as_str(),
        ) {
            errors.push(format!(
                "relationship '{}' references unknown from target {} '{}'",
                relationship.relationship_id, relationship.from_kind, relationship.from_id
            ));
        }

        if relationship.relation.trim().is_empty() {
            errors.push(format!(
                "relationship '{}' has empty relation",
                relationship.relationship_id
            ));
        }

        if relationship.to_kind.trim().is_empty() {
            errors.push(format!(
                "relationship '{}' has empty to_kind",
                relationship.relationship_id
            ));
        }
        if relationship.to_id.trim().is_empty() {
            errors.push(format!(
                "relationship '{}' has empty to_id",
                relationship.relationship_id
            ));
        } else if !is_known_target(
            graph,
            relationship.to_kind.as_str(),
            relationship.to_id.as_str(),
        ) {
            errors.push(format!(
                "relationship '{}' references unknown to target {} '{}'",
                relationship.relationship_id, relationship.to_kind, relationship.to_id
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn build_basic_dgraph_relationships(graph: &DGraph) -> Vec<DGraphRelationship> {
    let mut relationships = Vec::new();
    let mut seen = HashSet::new();

    for runtime in &graph.runtimes {
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "Node",
                &runtime.node_id,
                REL_HAS_RUNTIME,
                "Runtime",
                &runtime.runtime_id,
            ),
        );
    }

    for module in &graph.modules {
        if let Some(artifact_id) = module.artifact_id.as_deref() {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "Module",
                    &module.module_id,
                    REL_USES,
                    "Artifact",
                    artifact_id,
                ),
            );
        }
    }

    for flow in &graph.flows {
        for module_id in &flow.modules {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship("Flow", &flow.flow_id, REL_CONTAINS, "Module", module_id),
            );
        }
        for edge in &flow.edges {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "Flow",
                    &flow.flow_id,
                    REL_CONNECTS,
                    "FlowEdge",
                    &edge.edge_id,
                ),
            );
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "Module",
                    &edge.from_module_id,
                    REL_CONNECTS,
                    "Module",
                    &edge.to_module_id,
                ),
            );
        }
    }

    for placement in &graph.placements {
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "Module",
                &placement.module_id,
                REL_PLACED_ON,
                "Node",
                &placement.selected_node_id,
            ),
        );
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "Node",
                &placement.selected_node_id,
                REL_HOSTS,
                "Module",
                &placement.module_id,
            ),
        );
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "PlacementDecision",
                &placement.placement_id,
                REL_SELECTS,
                "Node",
                &placement.selected_node_id,
            ),
        );
        if let Some(runtime_id) = placement.selected_runtime_id.as_deref() {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "PlacementDecision",
                    &placement.placement_id,
                    REL_SELECTS,
                    "Runtime",
                    runtime_id,
                ),
            );
        }
    }

    for report in &graph.health_reports {
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "HealthReport",
                &report.report_id,
                REL_OBSERVES,
                normalize_kind(&report.target_kind),
                &report.target_id,
            ),
        );
    }

    for intent in &graph.deployment_intents {
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "DeploymentIntent",
                &intent.intent_id,
                REL_TARGETS,
                normalize_kind(&intent.target_kind),
                &intent.target_id,
            ),
        );
    }

    for reconfiguration in &graph.reconfigurations {
        if let Some(flow_id) = reconfiguration.flow_id.as_deref() {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "ReconfigurationDecision",
                    &reconfiguration.reconfiguration_id,
                    REL_REPLACES,
                    "Flow",
                    flow_id,
                ),
            );
        }
        if let Some(module_id) = reconfiguration.affected_module_id.as_deref() {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "ReconfigurationDecision",
                    &reconfiguration.reconfiguration_id,
                    REL_TARGETS,
                    "Module",
                    module_id,
                ),
            );
        }
        if let Some(to_node_id) = reconfiguration.to_node_id.as_deref() {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "ReconfigurationDecision",
                    &reconfiguration.reconfiguration_id,
                    REL_SELECTS,
                    "Node",
                    to_node_id,
                ),
            );
        }
    }

    for wasm_module in &graph.wasm_modules {
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "WasmModuleDescriptor",
                &wasm_module.wasm_module_id,
                REL_DESCRIBES,
                "Module",
                &wasm_module.module_id,
            ),
        );
    }

    for wasm_report in &graph.wasm_reports {
        if let Some(wasm_module_id) = wasm_report.wasm_module_id.as_deref() {
            push_relationship(
                &mut relationships,
                &mut seen,
                relationship(
                    "WasmRuntimeReport",
                    &wasm_report.report_id,
                    REL_OBSERVES,
                    "WasmModuleDescriptor",
                    wasm_module_id,
                ),
            );
        }
        push_relationship(
            &mut relationships,
            &mut seen,
            relationship(
                "WasmRuntimeReport",
                &wasm_report.report_id,
                REL_OBSERVES,
                "Node",
                &wasm_report.node_id,
            ),
        );
    }

    relationships
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthStatusClass {
    Healthy,
    Unknown,
    Degraded,
    Unavailable,
}

fn classify_health_status(status: &str) -> HealthStatusClass {
    match status.trim().to_ascii_lowercase().as_str() {
        "healthy" | "ok" | "running" => HealthStatusClass::Healthy,
        "degraded" | "warning" => HealthStatusClass::Degraded,
        "unavailable" | "down" | "failed" | "critical" => HealthStatusClass::Unavailable,
        _ => HealthStatusClass::Unknown,
    }
}

pub fn summarize_dgraph(graph: &DGraph) -> DGraphSummary {
    let mut healthy_targets = 0;
    let mut unknown_targets = 0;
    let mut degraded_targets = 0;
    let mut unavailable_targets = 0;

    for report in &graph.health_reports {
        match classify_health_status(&report.status) {
            HealthStatusClass::Healthy => healthy_targets += 1,
            HealthStatusClass::Unknown => unknown_targets += 1,
            HealthStatusClass::Degraded => degraded_targets += 1,
            HealthStatusClass::Unavailable => unavailable_targets += 1,
        }
    }

    DGraphSummary {
        node_count: graph.nodes.len() as u64,
        runtime_count: graph.runtimes.len() as u64,
        module_count: graph.modules.len() as u64,
        artifact_count: graph.artifacts.len() as u64,
        flow_count: graph.flows.len() as u64,
        relationship_count: graph.relationships.len() as u64,
        healthy_targets,
        unknown_targets,
        degraded_targets,
        unavailable_targets,
        unhealthy_targets: degraded_targets + unavailable_targets,
        pending_deployment_intents: graph
            .deployment_intents
            .iter()
            .filter(|intent| !matches!(intent.desired_state.as_str(), "running" | "ready"))
            .count() as u64,
        proposed_placements: graph
            .placements
            .iter()
            .filter(|placement| {
                matches!(
                    placement.status.as_str(),
                    "proposed" | "planned" | "selected"
                )
            })
            .count() as u64,
        proposed_reconfigurations: graph
            .reconfigurations
            .iter()
            .filter(|reconfiguration| {
                matches!(
                    reconfiguration.status.as_str(),
                    "proposed" | "planned" | "selected"
                )
            })
            .count() as u64,
    }
}

pub fn sample_local_dgraph(project_id: &str) -> DGraph {
    let mut graph = DGraph {
        project_id: project_id.to_string(),
        generated_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
        nodes: vec![DGraphNode {
            node_id: "node-local-01".to_string(),
            display_name: Some("Local Node 01".to_string()),
            node_kind: Some("simulated".to_string()),
            status: Some("ready".to_string()),
            labels: vec!["local".to_string(), "simulated".to_string()],
            metadata: None,
        }],
        runtimes: vec![DGraphRuntime {
            runtime_id: "runtime-container-local-01".to_string(),
            node_id: "node-local-01".to_string(),
            runtime_kind: "container".to_string(),
            status: Some("available".to_string()),
            metadata: None,
        }],
        modules: vec![
            DGraphModule {
                module_id: "module-ingest".to_string(),
                module_kind: "service".to_string(),
                desired_state: Some("running".to_string()),
                observed_state: Some("running".to_string()),
                runtime: Some("container".to_string()),
                artifact_id: Some("artifact-ingest-v1".to_string()),
                metadata: None,
            },
            DGraphModule {
                module_id: "module-processor".to_string(),
                module_kind: "service".to_string(),
                desired_state: Some("running".to_string()),
                observed_state: Some("degraded".to_string()),
                runtime: Some("container".to_string()),
                artifact_id: None,
                metadata: None,
            },
        ],
        artifacts: vec![DGraphArtifact {
            artifact_id: "artifact-ingest-v1".to_string(),
            artifact_kind: "container_image".to_string(),
            artifact_ref: Some("example/ingest:v1".to_string()),
            version: Some("v1".to_string()),
            metadata: None,
        }],
        flows: vec![DGraphFlow {
            flow_id: "flow-local-pipeline".to_string(),
            flow_kind: Some("pipeline".to_string()),
            modules: vec!["module-ingest".to_string(), "module-processor".to_string()],
            edges: vec![DGraphFlowEdge {
                edge_id: "edge-ingest-to-processor".to_string(),
                from_module_id: "module-ingest".to_string(),
                to_module_id: "module-processor".to_string(),
                protocol: Some("http".to_string()),
                status: Some("planned".to_string()),
                metadata: None,
            }],
            status: Some("degraded".to_string()),
            metadata: None,
        }],
        capabilities: vec![DGraphCapabilityProfile {
            capability_id: "capability-container-node-local-01".to_string(),
            node_id: Some("node-local-01".to_string()),
            module_id: None,
            capability_kind: "container".to_string(),
            supported: true,
            metadata: None,
        }],
        health_reports: vec![DGraphHealthReport {
            report_id: "health-flow-local-pipeline".to_string(),
            target_kind: "flow".to_string(),
            target_id: "flow-local-pipeline".to_string(),
            status: "degraded".to_string(),
            reason: Some("processor latency elevated".to_string()),
            metadata: None,
        }],
        deployment_intents: vec![DGraphDeploymentIntent {
            intent_id: "intent-module-processor".to_string(),
            target_kind: "module".to_string(),
            target_id: "module-processor".to_string(),
            desired_state: "degraded".to_string(),
            priority: Some("medium".to_string()),
            metadata: None,
        }],
        placements: vec![DGraphPlacementDecision {
            placement_id: "placement-module-ingest".to_string(),
            module_id: "module-ingest".to_string(),
            selected_node_id: "node-local-01".to_string(),
            selected_runtime_id: Some("runtime-container-local-01".to_string()),
            reason: Some("single local runtime".to_string()),
            status: "proposed".to_string(),
            metadata: None,
        }],
        reconfigurations: vec![DGraphReconfigurationDecision {
            reconfiguration_id: "reconfig-flow-local-pipeline".to_string(),
            flow_id: Some("flow-local-pipeline".to_string()),
            affected_module_id: Some("module-processor".to_string()),
            from_node_id: Some("node-local-01".to_string()),
            to_node_id: Some("node-local-01".to_string()),
            reason: Some("local-only simulation".to_string()),
            status: "proposed".to_string(),
            metadata: None,
        }],
        wasm_modules: vec![DGraphWasmModuleRef {
            module_id: "module-processor".to_string(),
            wasm_module_id: "embedded-add".to_string(),
            function_name: Some("add".to_string()),
            metadata: None,
        }],
        wasm_reports: vec![DGraphWasmRuntimeReportRef {
            report_id: "wasm-report-processor-diagnostic".to_string(),
            node_id: "node-local-01".to_string(),
            wasm_module_id: Some("embedded-add".to_string()),
            status: "executed".to_string(),
            metadata: None,
        }],
        relationships: Vec::new(),
        metadata: None,
    };
    graph.relationships = build_basic_dgraph_relationships(&graph);
    graph
}

fn push_relationship(
    relationships: &mut Vec<DGraphRelationship>,
    seen: &mut HashSet<String>,
    relationship: DGraphRelationship,
) {
    let key = format!(
        "{}|{}|{}|{}|{}",
        relationship.from_kind,
        relationship.from_id,
        relationship.relation,
        relationship.to_kind,
        relationship.to_id
    );

    if seen.insert(key) {
        relationships.push(relationship);
    }
}

fn relationship(
    from_kind: &str,
    from_id: &str,
    relation: &str,
    to_kind: &str,
    to_id: &str,
) -> DGraphRelationship {
    DGraphRelationship {
        relationship_id: deterministic_relationship_id(
            from_kind, from_id, relation, to_kind, to_id,
        ),
        from_kind: from_kind.to_string(),
        from_id: from_id.to_string(),
        relation: relation.to_string(),
        to_kind: to_kind.to_string(),
        to_id: to_id.to_string(),
        metadata: None,
    }
}

fn deterministic_relationship_id(
    from_kind: &str,
    from_id: &str,
    relation: &str,
    to_kind: &str,
    to_id: &str,
) -> String {
    format!(
        "rel:{}:{}:{}:{}:{}",
        from_kind.to_lowercase(),
        from_id,
        relation.to_lowercase(),
        to_kind.to_lowercase(),
        to_id
    )
}

fn normalize_kind(kind: &str) -> &'static str {
    match kind {
        "node" | "Node" => "Node",
        "runtime" | "Runtime" => "Runtime",
        "module" | "Module" => "Module",
        "artifact" | "Artifact" => "Artifact",
        "flow" | "Flow" => "Flow",
        "flow_edge" | "FlowEdge" => "FlowEdge",
        "capability" | "CapabilityProfile" => "CapabilityProfile",
        "health_report" | "HealthReport" => "HealthReport",
        "deployment_intent" | "DeploymentIntent" => "DeploymentIntent",
        "placement" | "PlacementDecision" => "PlacementDecision",
        "reconfiguration" | "ReconfigurationDecision" => "ReconfigurationDecision",
        "wasm_module" | "WasmModuleDescriptor" => "WasmModuleDescriptor",
        "wasm_report" | "WasmRuntimeReport" => "WasmRuntimeReport",
        _ => "Unknown",
    }
}

fn is_known_target(graph: &DGraph, kind: &str, id: &str) -> bool {
    match normalize_kind(kind) {
        "Node" => graph.nodes.iter().any(|node| node.node_id == id),
        "Runtime" => graph
            .runtimes
            .iter()
            .any(|runtime| runtime.runtime_id == id),
        "Module" => graph.modules.iter().any(|module| module.module_id == id),
        "Artifact" => graph
            .artifacts
            .iter()
            .any(|artifact| artifact.artifact_id == id),
        "Flow" => graph.flows.iter().any(|flow| flow.flow_id == id),
        "FlowEdge" => graph
            .flows
            .iter()
            .flat_map(|flow| flow.edges.iter())
            .any(|edge| edge.edge_id == id),
        "CapabilityProfile" => graph
            .capabilities
            .iter()
            .any(|capability| capability.capability_id == id),
        "HealthReport" => graph
            .health_reports
            .iter()
            .any(|report| report.report_id == id),
        "DeploymentIntent" => graph
            .deployment_intents
            .iter()
            .any(|intent| intent.intent_id == id),
        "PlacementDecision" => graph
            .placements
            .iter()
            .any(|placement| placement.placement_id == id),
        "ReconfigurationDecision" => graph
            .reconfigurations
            .iter()
            .any(|reconfiguration| reconfiguration.reconfiguration_id == id),
        "WasmModuleDescriptor" => graph
            .wasm_modules
            .iter()
            .any(|wasm_module| wasm_module.wasm_module_id == id),
        "WasmRuntimeReport" => graph
            .wasm_reports
            .iter()
            .any(|report| report.report_id == id),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_sample_local_dgraph_passes_validation() {
        let graph = sample_local_dgraph("smart-city-campinas");
        assert!(validate_dgraph(&graph).is_ok());
    }

    #[test]
    fn missing_project_id_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.project_id.clear();

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors
            .iter()
            .any(|error| error.contains("project_id must not be empty")));
    }

    #[test]
    fn runtime_referencing_missing_node_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.runtimes[0].node_id = "missing-node".to_string();

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors
            .iter()
            .any(|error| error.contains("references missing node 'missing-node'")));
    }

    #[test]
    fn module_referencing_missing_artifact_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.modules[0].artifact_id = Some("missing-artifact".to_string());

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors
            .iter()
            .any(|error| error.contains("references missing artifact 'missing-artifact'")));
    }

    #[test]
    fn flow_referencing_missing_module_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.flows[0].modules.push("missing-module".to_string());

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors.iter().any(|error| error
            .contains("flow 'flow-local-pipeline' references missing module 'missing-module'")));
    }

    #[test]
    fn flow_edge_referencing_missing_module_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.flows[0].edges[0].to_module_id = "missing-module".to_string();

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors
            .iter()
            .any(|error| error.contains("references missing to_module_id 'missing-module'")));
    }

    #[test]
    fn placement_referencing_missing_node_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.placements[0].selected_node_id = "missing-node".to_string();

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors.iter().any(|error| error.contains(
            "placement 'placement-module-ingest' references missing node 'missing-node'"
        )));
    }

    #[test]
    fn placement_referencing_missing_runtime_fails_validation() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.placements[0].selected_runtime_id = Some("missing-runtime".to_string());

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors.iter().any(|error| error.contains(
            "placement 'placement-module-ingest' references missing runtime 'missing-runtime'"
        )));
    }

    #[test]
    fn relationship_builder_creates_has_runtime() {
        let graph = sample_local_dgraph("smart-city-campinas");
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == REL_HAS_RUNTIME
                && relationship.from_id == "node-local-01"
                && relationship.to_id == "runtime-container-local-01"
        }));
    }

    #[test]
    fn relationship_builder_creates_uses() {
        let graph = sample_local_dgraph("smart-city-campinas");
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == REL_USES
                && relationship.from_id == "module-ingest"
                && relationship.to_id == "artifact-ingest-v1"
        }));
    }

    #[test]
    fn relationship_builder_creates_contains() {
        let graph = sample_local_dgraph("smart-city-campinas");
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == REL_CONTAINS
                && relationship.from_id == "flow-local-pipeline"
                && relationship.to_id == "module-ingest"
        }));
    }

    #[test]
    fn relationship_builder_creates_connects() {
        let graph = sample_local_dgraph("smart-city-campinas");
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == REL_CONNECTS
                && relationship.from_id == "module-ingest"
                && relationship.to_id == "module-processor"
        }));
    }

    #[test]
    fn relationship_builder_creates_placed_on() {
        let graph = sample_local_dgraph("smart-city-campinas");
        assert!(graph.relationships.iter().any(|relationship| {
            relationship.relation == REL_PLACED_ON
                && relationship.from_id == "module-ingest"
                && relationship.to_id == "node-local-01"
        }));
    }

    #[test]
    fn relationship_ids_are_deterministic() {
        let graph = sample_local_dgraph("smart-city-campinas");
        let first = build_basic_dgraph_relationships(&graph);
        let second = build_basic_dgraph_relationships(&graph);

        assert_eq!(first, second);
        assert!(first
            .iter()
            .all(|relationship| relationship.relationship_id.starts_with("rel:")));
    }

    #[test]
    fn duplicate_relationships_are_not_emitted() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.flows[0].modules.push("module-ingest".to_string());

        let relationships = build_basic_dgraph_relationships(&graph);
        let contains_count = relationships
            .iter()
            .filter(|relationship| {
                relationship.relation == REL_CONTAINS
                    && relationship.from_id == "flow-local-pipeline"
                    && relationship.to_id == "module-ingest"
            })
            .count();

        assert_eq!(contains_count, 1);
    }

    #[test]
    fn summary_counts_nodes_runtimes_modules_flows_and_relationships_correctly() {
        let graph = sample_local_dgraph("smart-city-campinas");
        let summary = summarize_dgraph(&graph);

        assert_eq!(summary.node_count, 1);
        assert_eq!(summary.runtime_count, 1);
        assert_eq!(summary.module_count, 2);
        assert_eq!(summary.artifact_count, 1);
        assert_eq!(summary.flow_count, 1);
        assert_eq!(summary.relationship_count, graph.relationships.len() as u64);
        assert_eq!(summary.healthy_targets, 0);
        assert_eq!(summary.unknown_targets, 0);
        assert_eq!(summary.degraded_targets, 1);
        assert_eq!(summary.unavailable_targets, 0);
        assert_eq!(summary.unhealthy_targets, 1);
        assert_eq!(summary.pending_deployment_intents, 1);
        assert_eq!(summary.proposed_placements, 1);
        assert_eq!(summary.proposed_reconfigurations, 1);
    }

    #[test]
    fn summary_normalizes_health_without_treating_unknown_as_unhealthy() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        let statuses = [
            "healthy",
            " OK ",
            "RUNNING",
            "degraded",
            " Warning ",
            "unavailable",
            "DOWN",
            "failed",
            "critical",
            "unknown",
            "pending",
            "",
        ];
        graph.health_reports = statuses
            .iter()
            .enumerate()
            .map(|(index, status)| DGraphHealthReport {
                report_id: format!("health-{index}"),
                target_kind: "module".to_string(),
                target_id: "module-ingest".to_string(),
                status: (*status).to_string(),
                reason: None,
                metadata: None,
            })
            .collect();

        let summary = summarize_dgraph(&graph);

        assert_eq!(summary.healthy_targets, 3);
        assert_eq!(summary.unknown_targets, 3);
        assert_eq!(summary.degraded_targets, 2);
        assert_eq!(summary.unavailable_targets, 4);
        assert_eq!(summary.unhealthy_targets, 6);
    }

    #[test]
    fn validation_collects_multiple_errors() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.project_id.clear();
        graph.runtimes[0].node_id = "missing-node".to_string();
        graph.modules[0].artifact_id = Some("missing-artifact".to_string());

        let errors = validate_dgraph(&graph).expect_err("validation should fail");
        assert!(errors.len() >= 3);
        assert!(errors
            .iter()
            .any(|error| error.contains("project_id must not be empty")));
        assert!(errors
            .iter()
            .any(|error| error.contains("missing node 'missing-node'")));
        assert!(errors
            .iter()
            .any(|error| error.contains("missing artifact 'missing-artifact'")));
    }
}
