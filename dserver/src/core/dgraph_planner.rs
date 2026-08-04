#![allow(dead_code)]

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::core::dgraph::{
    validate_dgraph, DGraph, DGraphModule, DGraphNode, DGraphPlacementDecision,
    DGraphReconfigurationDecision,
};

const PLANNER_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerDryRun {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub planner_version: String,
    #[serde(default)]
    pub generated_at_utc: Option<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub placements: Vec<DGraphPlacementDecision>,
    #[serde(default)]
    pub reconfigurations: Vec<DGraphReconfigurationDecision>,
    #[serde(default)]
    pub blocked: Vec<DGraphPlannerBlockedItem>,
    #[serde(default)]
    pub warnings: Vec<DGraphPlannerWarning>,
    #[serde(default)]
    pub advisory_proposals: Vec<DGraphPlannerAdvisoryProposal>,
    #[serde(default)]
    pub summary: DGraphPlannerSummary,
    #[serde(default)]
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub side_effects_performed: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerBlockedItem {
    #[serde(default)]
    pub item_id: String,
    #[serde(default)]
    pub target_kind: String,
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerWarning {
    #[serde(default)]
    pub warning_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerAdvisoryReason {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerAdvisoryEvidence {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub observed_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerAdvisoryProposal {
    #[serde(default)]
    pub proposal_id: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub reason: DGraphPlannerAdvisoryReason,
    #[serde(default)]
    pub evidence: Vec<DGraphPlannerAdvisoryEvidence>,
    #[serde(default)]
    pub related_flow_ids: Vec<String>,
    #[serde(default)]
    pub related_node_ids: Vec<String>,
    #[serde(default)]
    pub candidate_node_ids: Vec<String>,
    #[serde(default)]
    pub limitations: Vec<String>,
    #[serde(default)]
    pub advisory_only: bool,
    #[serde(default)]
    pub executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DGraphPlannerSummary {
    pub placement_count: u64,
    pub reconfiguration_count: u64,
    pub blocked_count: u64,
    pub warning_count: u64,
    pub candidate_node_count: u64,
    pub unhealthy_node_count: u64,
    pub degraded_flow_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphPlannerDryRunResponse {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub source_snapshot_count: u64,
    #[serde(default)]
    pub materialization_validation_errors: Vec<String>,
    #[serde(default)]
    pub planner: DGraphPlannerDryRun,
    #[serde(default)]
    pub side_effects_performed: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ProjectDGraphPlannerDryRunSummaryResponse {
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub source_snapshot_count: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: DGraphPlannerSummary,
    #[serde(default)]
    pub placement_count: u64,
    #[serde(default)]
    pub reconfiguration_count: u64,
    #[serde(default)]
    pub blocked_count: u64,
    #[serde(default)]
    pub warning_count: u64,
    #[serde(default)]
    pub side_effects_performed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DGraphPlacementPlanResult {
    pub placements: Vec<DGraphPlacementDecision>,
    pub blocked: Vec<DGraphPlannerBlockedItem>,
    pub warnings: Vec<DGraphPlannerWarning>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DGraphReconfigurationPlanResult {
    pub reconfigurations: Vec<DGraphReconfigurationDecision>,
    pub blocked: Vec<DGraphPlannerBlockedItem>,
    pub warnings: Vec<DGraphPlannerWarning>,
}

pub fn is_node_available(graph: &DGraph, node_id: &str) -> bool {
    let Some(node) = graph.nodes.iter().find(|node| node.node_id == node_id) else {
        return false;
    };

    !is_failedish_status(node.status.as_deref())
        && !graph.health_reports.iter().any(|report| {
            report.target_kind.eq_ignore_ascii_case("node")
                && report.target_id == node_id
                && is_failedish_status(Some(report.status.as_str()))
        })
}

pub fn is_node_degraded(graph: &DGraph, node_id: &str) -> bool {
    let node_degraded = graph
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .and_then(|node| node.status.as_deref())
        .map(is_degraded_status)
        .unwrap_or(false);

    node_degraded
        || graph.health_reports.iter().any(|report| {
            report.target_kind.eq_ignore_ascii_case("node")
                && report.target_id == node_id
                && is_degraded_status(report.status.as_str())
        })
}

pub fn candidate_nodes_for_module(graph: &DGraph, module: &DGraphModule) -> Vec<DGraphNode> {
    let available_nodes = graph
        .nodes
        .iter()
        .filter(|node| is_node_available(graph, &node.node_id))
        .cloned()
        .collect::<Vec<_>>();

    if available_nodes.is_empty() {
        return available_nodes;
    }

    let healthy_nodes = available_nodes
        .iter()
        .filter(|node| !is_node_degraded(graph, &node.node_id))
        .cloned()
        .collect::<Vec<_>>();
    let baseline_nodes = if healthy_nodes.is_empty() {
        available_nodes.clone()
    } else {
        healthy_nodes
    };

    match module.runtime.as_deref() {
        Some("container") => {
            let matching = baseline_nodes
                .iter()
                .filter(|node| node_has_runtime_kind(graph, &node.node_id, "container"))
                .cloned()
                .collect::<Vec<_>>();
            if matching.is_empty() {
                baseline_nodes
            } else {
                matching
            }
        }
        Some("wasm") => {
            let matching = baseline_nodes
                .iter()
                .filter(|node| node_supports_wasm(graph, &node.node_id))
                .cloned()
                .collect::<Vec<_>>();
            if matching.is_empty() {
                baseline_nodes
            } else {
                matching
            }
        }
        _ => baseline_nodes,
    }
}

pub fn plan_dgraph_placements(graph: &DGraph) -> Vec<DGraphPlacementDecision> {
    plan_dgraph_placements_with_context(graph).placements
}

pub fn plan_dgraph_reconfigurations(graph: &DGraph) -> Vec<DGraphReconfigurationDecision> {
    plan_dgraph_reconfigurations_with_context(graph).reconfigurations
}

pub fn plan_dgraph_dry_run(graph: &DGraph) -> DGraphPlannerDryRun {
    if let Err(errors) = validate_dgraph(graph) {
        let blocked = errors
            .into_iter()
            .enumerate()
            .map(|(index, reason)| DGraphPlannerBlockedItem {
                item_id: format!("blocked:validation:{}", index + 1),
                target_kind: "graph".to_string(),
                target_id: graph.project_id.clone(),
                reason,
                metadata: None,
            })
            .collect::<Vec<_>>();

        let summary = summarize_dry_run(graph, 0, 0, blocked.len() as u64, 0);

        return DGraphPlannerDryRun {
            project_id: graph.project_id.clone(),
            planner_version: PLANNER_VERSION.to_string(),
            generated_at_utc: Some(Utc::now().to_rfc3339()),
            status: "blocked".to_string(),
            placements: Vec::new(),
            reconfigurations: Vec::new(),
            blocked,
            warnings: Vec::new(),
            advisory_proposals: Vec::new(),
            summary,
            safe_to_auto_execute: false,
            side_effects_performed: false,
            metadata: None,
        };
    }

    let placement_result = plan_dgraph_placements_with_context(graph);
    let reconfiguration_result = plan_dgraph_reconfigurations_with_context(graph);

    let mut blocked = placement_result.blocked;
    blocked.extend(reconfiguration_result.blocked);

    let mut warnings = placement_result.warnings;
    append_unique_warnings(&mut warnings, reconfiguration_result.warnings);
    append_degraded_flow_advisories(graph, &mut warnings);
    let advisory_proposals = build_advisory_proposals(graph);

    let placements = placement_result.placements;
    let reconfigurations = reconfiguration_result.reconfigurations;

    let status = if placements.is_empty() && reconfigurations.is_empty() && blocked.is_empty() {
        "empty"
    } else if !placements.is_empty() || !reconfigurations.is_empty() {
        if blocked.is_empty() {
            "planned"
        } else {
            "partially_blocked"
        }
    } else {
        "blocked"
    };

    let summary = summarize_dry_run(
        graph,
        placements.len() as u64,
        reconfigurations.len() as u64,
        blocked.len() as u64,
        warnings.len() as u64,
    );

    DGraphPlannerDryRun {
        project_id: graph.project_id.clone(),
        planner_version: PLANNER_VERSION.to_string(),
        generated_at_utc: Some(Utc::now().to_rfc3339()),
        status: status.to_string(),
        placements,
        reconfigurations,
        blocked,
        warnings,
        advisory_proposals,
        summary,
        safe_to_auto_execute: false,
        side_effects_performed: false,
        metadata: None,
    }
}

fn build_advisory_proposals(graph: &DGraph) -> Vec<DGraphPlannerAdvisoryProposal> {
    let mut by_id = BTreeMap::new();
    let mut flows = graph.flows.iter().collect::<Vec<_>>();
    flows.sort_by(|left, right| left.flow_id.cmp(&right.flow_id));

    for flow in flows {
        if !is_degraded_status(flow.status.as_deref().unwrap_or("unknown")) {
            continue;
        }
        let observed_flow_status = flow.status.as_deref().unwrap_or("unknown");

        let mut related_nodes = BTreeSet::new();
        let mut candidate_nodes = BTreeSet::new();
        let mut limitations = BTreeSet::new();
        let mut evidence = vec![DGraphPlannerAdvisoryEvidence {
            kind: "flow".to_string(),
            entity_id: flow.flow_id.clone(),
            observed_status: flow.status.clone(),
        }];
        let mut module_ids = flow.modules.iter().collect::<Vec<_>>();
        module_ids.sort();
        module_ids.dedup();

        for module_id in module_ids {
            let Some(module) = graph
                .modules
                .iter()
                .find(|module| module.module_id == *module_id)
            else {
                limitations.insert(format!(
                    "flow references module '{}' that is absent from the materialized graph",
                    module_id
                ));
                continue;
            };
            evidence.push(DGraphPlannerAdvisoryEvidence {
                kind: "module".to_string(),
                entity_id: module.module_id.clone(),
                observed_status: module.observed_state.clone(),
            });

            for placement in graph
                .placements
                .iter()
                .filter(|placement| placement.module_id == module.module_id)
            {
                if graph
                    .nodes
                    .iter()
                    .any(|node| node.node_id == placement.selected_node_id)
                {
                    related_nodes.insert(placement.selected_node_id.clone());
                }
            }
            for candidate in candidate_nodes_for_module(graph, module) {
                candidate_nodes.insert(candidate.node_id);
            }
            for report in graph.health_reports.iter().filter(|report| {
                report.target_kind.eq_ignore_ascii_case("module")
                    && report.target_id == module.module_id
                    && report.status.eq_ignore_ascii_case("unknown")
            }) {
                evidence.push(DGraphPlannerAdvisoryEvidence {
                    kind: "health_report".to_string(),
                    entity_id: report.report_id.clone(),
                    observed_status: Some(report.status.clone()),
                });
                limitations.insert(format!(
                    "module '{}' health is unknown and is not treated as availability evidence",
                    module.module_id
                ));
            }
        }

        for candidate_node_id in &candidate_nodes {
            if node_health_is_unknown_or_unestablished(graph, candidate_node_id) {
                limitations.insert(format!(
                    "candidate node '{}' has unknown or unestablished observed health; availability, suitability, and safety for placement are not established, and its presence in candidate_node_ids is observational only, not a selected destination, recommendation, or execution authorization",
                    candidate_node_id
                ));
            }
        }

        if candidate_nodes.is_empty() {
            limitations.insert(
                "no available candidate nodes are present in current graph evidence".to_string(),
            );
        } else {
            limitations.insert(
                "candidate nodes are observations only; no destination is selected".to_string(),
            );
        }
        limitations.insert(
            "advisory only; it does not authorize placement, recovery, migration, or reconfiguration"
                .to_string(),
        );
        evidence.sort_by(|left, right| {
            (
                left.kind.as_str(),
                left.entity_id.as_str(),
                left.observed_status.as_deref(),
            )
                .cmp(&(
                    right.kind.as_str(),
                    right.entity_id.as_str(),
                    right.observed_status.as_deref(),
                ))
        });
        evidence.dedup();

        let proposal_id = format!("advisory:degraded-flow:{}", flow.flow_id);
        by_id
            .entry(proposal_id.clone())
            .or_insert(DGraphPlannerAdvisoryProposal {
                proposal_id,
                category: "degraded_flow_observation".to_string(),
                summary: format!(
                    "Observed flow '{}' with status '{}' requires operator review",
                    flow.flow_id, observed_flow_status
                ),
                reason: DGraphPlannerAdvisoryReason {
                    code: "degraded_flow_observed".to_string(),
                    message: format!(
                        "flow '{}' is reported with status '{}' in the materialized D-Graph",
                        flow.flow_id, observed_flow_status
                    ),
                },
                evidence,
                related_flow_ids: vec![flow.flow_id.clone()],
                related_node_ids: related_nodes.into_iter().collect(),
                candidate_node_ids: candidate_nodes.into_iter().collect(),
                limitations: limitations.into_iter().collect(),
                advisory_only: true,
                executable: false,
            });
    }

    by_id.into_values().collect()
}

fn node_health_is_unknown_or_unestablished(graph: &DGraph, node_id: &str) -> bool {
    let node_status_is_unknown_or_missing = graph
        .nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .and_then(|node| node.status.as_deref())
        .map(|status| status.eq_ignore_ascii_case("unknown"))
        .unwrap_or(true);

    node_status_is_unknown_or_missing
        || graph.health_reports.iter().any(|report| {
            report.target_kind.eq_ignore_ascii_case("node")
                && report.target_id == node_id
                && report.status.eq_ignore_ascii_case("unknown")
        })
}

pub fn build_project_dgraph_planner_dry_run_response(
    project_id: &str,
    source_snapshot_count: u64,
    materialization_validation_errors: Vec<String>,
    planner: DGraphPlannerDryRun,
) -> ProjectDGraphPlannerDryRunResponse {
    ProjectDGraphPlannerDryRunResponse {
        project_id: project_id.to_string(),
        source_snapshot_count,
        materialization_validation_errors,
        planner,
        side_effects_performed: false,
        metadata: Some(serde_json::json!({
            "planner_mode": "dry_run",
            "execution_performed": false,
        })),
    }
}

impl ProjectDGraphPlannerDryRunResponse {
    pub fn as_summary(&self) -> ProjectDGraphPlannerDryRunSummaryResponse {
        ProjectDGraphPlannerDryRunSummaryResponse {
            project_id: self.project_id.clone(),
            source_snapshot_count: self.source_snapshot_count,
            status: self.planner.status.clone(),
            summary: self.planner.summary.clone(),
            placement_count: self.planner.placements.len() as u64,
            reconfiguration_count: self.planner.reconfigurations.len() as u64,
            blocked_count: self.planner.blocked.len() as u64,
            warning_count: self.planner.warnings.len() as u64,
            side_effects_performed: false,
        }
    }
}

fn plan_dgraph_placements_with_context(graph: &DGraph) -> DGraphPlacementPlanResult {
    let mut result = DGraphPlacementPlanResult::default();

    for module in &graph.modules {
        if !module_needs_placement(module) {
            continue;
        }

        if module_has_healthy_placement(graph, &module.module_id) {
            continue;
        }

        let candidates = candidate_nodes_for_module(graph, module);
        if candidates.is_empty() {
            result.blocked.push(DGraphPlannerBlockedItem {
                item_id: deterministic_blocked_id("module", &module.module_id, "no_candidate"),
                target_kind: "module".to_string(),
                target_id: module.module_id.clone(),
                reason: format!(
                    "no available candidate nodes found for module '{}' with runtime '{}'",
                    module.module_id,
                    module.runtime.as_deref().unwrap_or("unknown")
                ),
                metadata: None,
            });
            continue;
        }

        if module_runtime_mismatch_warning_needed(graph, module, &candidates) {
            result.blocked.push(DGraphPlannerBlockedItem {
                item_id: deterministic_blocked_id(
                    "module",
                    &module.module_id,
                    "runtime_specific_candidate_missing",
                ),
                target_kind: "module".to_string(),
                target_id: module.module_id.clone(),
                reason: format!(
                    "no runtime-specific candidate was found for module '{}' with runtime '{}'; fallback placement remains advisory only",
                    module.module_id,
                    module.runtime.as_deref().unwrap_or("unknown")
                ),
                metadata: None,
            });
            push_warning(
                &mut result.warnings,
                DGraphPlannerWarning {
                    warning_id: deterministic_warning_id("module", &module.module_id, "runtime_fallback"),
                    reason: format!(
                        "module '{}' fell back to general available nodes because no runtime-specific candidate was found",
                        module.module_id
                    ),
                    metadata: None,
                },
            );
        }

        let selected_node = &candidates[0];
        let selected_runtime_id = matching_runtime_id_for_module(graph, selected_node, module);
        let reason = if let Some(runtime_id) = selected_runtime_id.as_deref() {
            format!(
                "selected available node '{}' with matching runtime '{}'",
                selected_node.node_id, runtime_id
            )
        } else {
            format!(
                "selected first available candidate node '{}' for module '{}'",
                selected_node.node_id, module.module_id
            )
        };

        result.placements.push(DGraphPlacementDecision {
            placement_id: deterministic_placement_id(&module.module_id, &selected_node.node_id),
            module_id: module.module_id.clone(),
            selected_node_id: selected_node.node_id.clone(),
            selected_runtime_id,
            reason: Some(reason),
            status: "proposed".to_string(),
            metadata: None,
        });
    }

    result
}

fn plan_dgraph_reconfigurations_with_context(graph: &DGraph) -> DGraphReconfigurationPlanResult {
    let mut result = DGraphReconfigurationPlanResult::default();

    for flow in &graph.flows {
        for module_id in &flow.modules {
            let current_placement = graph
                .placements
                .iter()
                .find(|placement| placement.module_id == *module_id);

            let Some(current_placement) = current_placement else {
                continue;
            };

            if is_node_available(graph, &current_placement.selected_node_id) {
                continue;
            }

            let Some(module) = graph
                .modules
                .iter()
                .find(|module| module.module_id == *module_id)
            else {
                continue;
            };

            let candidates = candidate_nodes_for_module(graph, module)
                .into_iter()
                .filter(|node| node.node_id != current_placement.selected_node_id)
                .collect::<Vec<_>>();

            if candidates.is_empty() {
                result.blocked.push(DGraphPlannerBlockedItem {
                    item_id: deterministic_blocked_id("flow", &flow.flow_id, module_id),
                    target_kind: "flow".to_string(),
                    target_id: flow.flow_id.clone(),
                    reason: format!(
                        "no alternative candidate node found to reconfigure module '{}' away from unavailable node '{}'",
                        module_id, current_placement.selected_node_id
                    ),
                    metadata: None,
                });
                continue;
            }

            if module_runtime_mismatch_warning_needed(graph, module, &candidates) {
                result.blocked.push(DGraphPlannerBlockedItem {
                    item_id: deterministic_blocked_id(
                        "flow",
                        &flow.flow_id,
                        &format!("{}:runtime_specific_candidate_missing", module_id),
                    ),
                    target_kind: "flow".to_string(),
                    target_id: flow.flow_id.clone(),
                    reason: format!(
                        "no runtime-specific reconfiguration candidate was found for module '{}' in flow '{}'; fallback move remains advisory only",
                        module_id, flow.flow_id
                    ),
                    metadata: None,
                });
                push_warning(
                    &mut result.warnings,
                    DGraphPlannerWarning {
                        warning_id: deterministic_warning_id("flow", &flow.flow_id, module_id),
                        reason: format!(
                            "flow '{}' is using runtime fallback candidates for module '{}'",
                            flow.flow_id, module_id
                        ),
                        metadata: None,
                    },
                );
            }

            let selected_node = &candidates[0];
            result.reconfigurations.push(DGraphReconfigurationDecision {
                reconfiguration_id: deterministic_reconfiguration_id(
                    &flow.flow_id,
                    module_id,
                    &current_placement.selected_node_id,
                    &selected_node.node_id,
                ),
                flow_id: Some(flow.flow_id.clone()),
                affected_module_id: Some(module_id.clone()),
                from_node_id: Some(current_placement.selected_node_id.clone()),
                to_node_id: Some(selected_node.node_id.clone()),
                reason: Some(format!(
                    "module '{}' is placed on unavailable node '{}'; propose move to available node '{}'",
                    module_id, current_placement.selected_node_id, selected_node.node_id
                )),
                status: "proposed".to_string(),
                metadata: None,
            });
        }
    }

    result
}

fn summarize_dry_run(
    graph: &DGraph,
    placement_count: u64,
    reconfiguration_count: u64,
    blocked_count: u64,
    warning_count: u64,
) -> DGraphPlannerSummary {
    DGraphPlannerSummary {
        placement_count,
        reconfiguration_count,
        blocked_count,
        warning_count,
        candidate_node_count: graph
            .nodes
            .iter()
            .filter(|node| is_node_available(graph, &node.node_id))
            .count() as u64,
        unhealthy_node_count: graph
            .nodes
            .iter()
            .filter(|node| !is_node_available(graph, &node.node_id))
            .count() as u64,
        degraded_flow_count: graph
            .flows
            .iter()
            .filter(|flow| is_degraded_status(flow.status.as_deref().unwrap_or("unknown")))
            .count() as u64,
    }
}

fn module_needs_placement(module: &DGraphModule) -> bool {
    matches!(module.desired_state.as_deref(), Some("running"))
        || module.desired_state.is_none()
        || matches!(module.observed_state.as_deref(), Some("missing"))
}

fn module_has_healthy_placement(graph: &DGraph, module_id: &str) -> bool {
    graph.placements.iter().any(|placement| {
        placement.module_id == module_id && is_node_available(graph, &placement.selected_node_id)
    })
}

fn module_runtime_mismatch_warning_needed(
    graph: &DGraph,
    module: &DGraphModule,
    candidates: &[DGraphNode],
) -> bool {
    match module.runtime.as_deref() {
        Some("container") => !candidates
            .iter()
            .any(|node| node_has_runtime_kind(graph, &node.node_id, "container")),
        Some("wasm") => !candidates
            .iter()
            .any(|node| node_supports_wasm(graph, &node.node_id)),
        _ => false,
    }
}

fn node_has_runtime_kind(graph: &DGraph, node_id: &str, runtime_kind: &str) -> bool {
    graph.runtimes.iter().any(|runtime| {
        runtime.node_id == node_id
            && runtime.runtime_kind == runtime_kind
            && !is_failedish_status(runtime.status.as_deref())
    })
}

fn node_supports_wasm(graph: &DGraph, node_id: &str) -> bool {
    node_has_runtime_kind(graph, node_id, "wasm")
        || graph.capabilities.iter().any(|capability| {
            capability.node_id.as_deref() == Some(node_id)
                && capability.capability_kind.eq_ignore_ascii_case("wasm")
                && capability.supported
        })
}

fn matching_runtime_id_for_module(
    graph: &DGraph,
    node: &DGraphNode,
    module: &DGraphModule,
) -> Option<String> {
    match module.runtime.as_deref() {
        Some("container") => graph
            .runtimes
            .iter()
            .find(|runtime| {
                runtime.node_id == node.node_id
                    && runtime.runtime_kind == "container"
                    && !is_failedish_status(runtime.status.as_deref())
            })
            .map(|runtime| runtime.runtime_id.clone()),
        Some("wasm") => graph
            .runtimes
            .iter()
            .find(|runtime| {
                runtime.node_id == node.node_id
                    && runtime.runtime_kind == "wasm"
                    && !is_failedish_status(runtime.status.as_deref())
            })
            .map(|runtime| runtime.runtime_id.clone()),
        _ => graph
            .runtimes
            .iter()
            .find(|runtime| {
                runtime.node_id == node.node_id && !is_failedish_status(runtime.status.as_deref())
            })
            .map(|runtime| runtime.runtime_id.clone()),
    }
}

fn is_failedish_status(status: Option<&str>) -> bool {
    matches!(
        status.unwrap_or("").to_ascii_lowercase().as_str(),
        "unavailable" | "down" | "failed" | "critical"
    )
}

fn is_degraded_status(status: &str) -> bool {
    matches!(status.to_ascii_lowercase().as_str(), "degraded" | "warning")
}

fn deterministic_placement_id(module_id: &str, node_id: &str) -> String {
    format!("placement:{}:{}", module_id, node_id)
}

fn deterministic_reconfiguration_id(
    flow_id: &str,
    module_id: &str,
    from_node_id: &str,
    to_node_id: &str,
) -> String {
    format!(
        "reconfiguration:{}:{}:{}:{}",
        flow_id, module_id, from_node_id, to_node_id
    )
}

fn deterministic_blocked_id(target_kind: &str, target_id: &str, suffix: &str) -> String {
    format!("blocked:{}:{}:{}", target_kind, target_id, suffix)
}

fn deterministic_warning_id(target_kind: &str, target_id: &str, suffix: &str) -> String {
    format!("warning:{}:{}:{}", target_kind, target_id, suffix)
}

fn push_warning(warnings: &mut Vec<DGraphPlannerWarning>, warning: DGraphPlannerWarning) {
    if warnings
        .iter()
        .any(|existing| existing.warning_id == warning.warning_id)
    {
        return;
    }
    warnings.push(warning);
}

fn append_unique_warnings(
    warnings: &mut Vec<DGraphPlannerWarning>,
    more: Vec<DGraphPlannerWarning>,
) {
    for warning in more {
        push_warning(warnings, warning);
    }
}

fn append_degraded_flow_advisories(graph: &DGraph, warnings: &mut Vec<DGraphPlannerWarning>) {
    for flow in &graph.flows {
        if !is_degraded_status(flow.status.as_deref().unwrap_or("unknown")) {
            continue;
        }

        push_warning(
            warnings,
            DGraphPlannerWarning {
                warning_id: deterministic_warning_id("flow", &flow.flow_id, "degraded"),
                reason: format!(
                    "flow '{}' is degraded; this metadata-only dry-run remains advisory and does not execute reconfiguration",
                    flow.flow_id
                ),
                metadata: Some(serde_json::json!({
                    "kind": "degraded_flow",
                    "severity": "warning",
                    "target_kind": "flow",
                    "target_id": flow.flow_id,
                    "side_effects_performed": false,
                    "execution_performed": false,
                })),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::dgraph::{
        build_basic_dgraph_relationships, sample_local_dgraph, DGraph, DGraphFlow,
        DGraphHealthReport, DGraphModule, DGraphNode, DGraphPlacementDecision, DGraphRuntime,
    };

    fn sample_two_node_dgraph(project_id: &str) -> DGraph {
        let mut graph = DGraph {
            project_id: project_id.to_string(),
            generated_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
            nodes: vec![
                DGraphNode {
                    node_id: "fog-01".to_string(),
                    display_name: Some("Fog 01".to_string()),
                    node_kind: Some("simulated".to_string()),
                    status: Some("available".to_string()),
                    labels: vec![],
                    metadata: None,
                },
                DGraphNode {
                    node_id: "fog-02".to_string(),
                    display_name: Some("Fog 02".to_string()),
                    node_kind: Some("simulated".to_string()),
                    status: Some("available".to_string()),
                    labels: vec![],
                    metadata: None,
                },
            ],
            runtimes: vec![
                DGraphRuntime {
                    runtime_id: "runtime-container-fog-01".to_string(),
                    node_id: "fog-01".to_string(),
                    runtime_kind: "container".to_string(),
                    status: Some("available".to_string()),
                    metadata: None,
                },
                DGraphRuntime {
                    runtime_id: "runtime-container-fog-02".to_string(),
                    node_id: "fog-02".to_string(),
                    runtime_kind: "container".to_string(),
                    status: Some("available".to_string()),
                    metadata: None,
                },
            ],
            modules: vec![DGraphModule {
                module_id: "module-missing".to_string(),
                module_kind: "service".to_string(),
                desired_state: Some("running".to_string()),
                observed_state: Some("missing".to_string()),
                runtime: Some("container".to_string()),
                artifact_id: None,
                metadata: None,
            }],
            artifacts: vec![],
            flows: vec![DGraphFlow {
                flow_id: "flow-one".to_string(),
                flow_kind: Some("pipeline".to_string()),
                modules: vec!["module-missing".to_string()],
                edges: vec![],
                status: Some("configured".to_string()),
                metadata: None,
            }],
            capabilities: vec![],
            health_reports: vec![],
            deployment_intents: vec![],
            placements: vec![],
            reconfigurations: vec![],
            wasm_modules: vec![],
            wasm_reports: vec![],
            relationships: vec![],
            metadata: None,
        };
        graph.relationships = build_basic_dgraph_relationships(&graph);
        graph
    }

    fn sample_unavailable_node_reconfiguration_graph(project_id: &str) -> DGraph {
        let mut graph = sample_two_node_dgraph(project_id);
        graph.modules[0].module_id = "module-placed".to_string();
        graph.modules[0].observed_state = Some("running".to_string());
        graph.flows[0].modules = vec!["module-placed".to_string()];
        graph.flows[0].status = Some("degraded".to_string());
        graph.placements = vec![DGraphPlacementDecision {
            placement_id: "placement-existing".to_string(),
            module_id: "module-placed".to_string(),
            selected_node_id: "fog-01".to_string(),
            selected_runtime_id: Some("runtime-container-fog-01".to_string()),
            reason: Some("current placement".to_string()),
            status: "selected".to_string(),
            metadata: None,
        }];
        graph.health_reports.push(DGraphHealthReport {
            report_id: "health-fog-01".to_string(),
            target_kind: "node".to_string(),
            target_id: "fog-01".to_string(),
            status: "failed".to_string(),
            reason: Some("node offline".to_string()),
            metadata: None,
        });
        graph.relationships = build_basic_dgraph_relationships(&graph);
        graph
    }

    fn sample_no_candidate_graph(project_id: &str) -> DGraph {
        DGraph {
            project_id: project_id.to_string(),
            generated_at_utc: Some("2026-05-21T00:00:00Z".to_string()),
            nodes: vec![DGraphNode {
                node_id: "fog-01".to_string(),
                display_name: None,
                node_kind: Some("simulated".to_string()),
                status: Some("failed".to_string()),
                labels: vec![],
                metadata: None,
            }],
            runtimes: vec![],
            modules: vec![DGraphModule {
                module_id: "module-no-candidate".to_string(),
                module_kind: "service".to_string(),
                desired_state: Some("running".to_string()),
                observed_state: Some("missing".to_string()),
                runtime: Some("container".to_string()),
                artifact_id: None,
                metadata: None,
            }],
            artifacts: vec![],
            flows: vec![DGraphFlow {
                flow_id: "flow-no-candidate".to_string(),
                flow_kind: Some("pipeline".to_string()),
                modules: vec!["module-no-candidate".to_string()],
                edges: vec![],
                status: Some("degraded".to_string()),
                metadata: None,
            }],
            capabilities: vec![],
            health_reports: vec![],
            deployment_intents: vec![],
            placements: vec![],
            reconfigurations: vec![],
            wasm_modules: vec![],
            wasm_reports: vec![],
            relationships: vec![],
            metadata: None,
        }
    }

    #[test]
    fn valid_graph_with_missing_module_emits_proposed_placement() {
        let graph = sample_two_node_dgraph("smart-city-campinas");
        let placements = plan_dgraph_placements(&graph);

        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].status, "proposed");
        assert_eq!(placements[0].module_id, "module-missing");
    }

    #[test]
    fn placement_selects_available_node() {
        let graph = sample_two_node_dgraph("smart-city-campinas");
        let placements = plan_dgraph_placements(&graph);

        assert!(
            placements[0].selected_node_id == "fog-01"
                || placements[0].selected_node_id == "fog-02"
        );
        assert!(is_node_available(&graph, &placements[0].selected_node_id));
    }

    #[test]
    fn placement_selects_matching_runtime_when_available() {
        let graph = sample_two_node_dgraph("smart-city-campinas");
        let placements = plan_dgraph_placements(&graph);

        assert_eq!(
            placements[0].selected_runtime_id.as_deref(),
            Some("runtime-container-fog-01")
        );
    }

    #[test]
    fn already_placed_module_on_healthy_node_does_not_emit_duplicate_placement() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.modules[0].module_id = "module-healthy".to_string();
        graph.placements.push(DGraphPlacementDecision {
            placement_id: "placement-module-healthy".to_string(),
            module_id: "module-healthy".to_string(),
            selected_node_id: "fog-01".to_string(),
            selected_runtime_id: Some("runtime-container-fog-01".to_string()),
            reason: Some("already placed".to_string()),
            status: "selected".to_string(),
            metadata: None,
        });

        let placements = plan_dgraph_placements(&graph);
        assert!(placements.is_empty());
    }

    #[test]
    fn unavailable_node_is_not_selected() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.nodes[0].status = Some("failed".to_string());

        let placements = plan_dgraph_placements(&graph);
        assert_eq!(placements[0].selected_node_id, "fog-02");
    }

    #[test]
    fn degraded_node_can_be_avoided_when_healthy_alternative_exists() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.nodes[0].status = Some("degraded".to_string());

        let candidates = candidate_nodes_for_module(&graph, &graph.modules[0]);
        assert_eq!(candidates[0].node_id, "fog-02");
    }

    #[test]
    fn no_candidate_produces_blocked_dry_run_item() {
        let graph = sample_no_candidate_graph("smart-city-campinas");
        let dry_run = plan_dgraph_dry_run(&graph);

        assert_eq!(dry_run.status, "blocked");
        assert_eq!(dry_run.blocked.len(), 1);
        assert!(dry_run.blocked[0]
            .reason
            .contains("no available candidate nodes"));
    }

    #[test]
    fn invalid_graph_produces_blocked_dry_run_with_validation_errors() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.project_id.clear();

        let dry_run = plan_dgraph_dry_run(&graph);
        assert_eq!(dry_run.status, "blocked");
        assert!(dry_run
            .blocked
            .iter()
            .any(|item| item.reason.contains("project_id must not be empty")));
    }

    #[test]
    fn flow_with_module_on_failed_node_emits_reconfiguration_proposal() {
        let graph = sample_unavailable_node_reconfiguration_graph("smart-city-campinas");
        let reconfigurations = plan_dgraph_reconfigurations(&graph);

        assert_eq!(reconfigurations.len(), 1);
        assert_eq!(reconfigurations[0].status, "proposed");
        assert_eq!(reconfigurations[0].to_node_id.as_deref(), Some("fog-02"));
    }

    #[test]
    fn flow_reconfiguration_selects_alternative_available_node() {
        let graph = sample_unavailable_node_reconfiguration_graph("smart-city-campinas");
        let dry_run = plan_dgraph_dry_run(&graph);

        assert_eq!(dry_run.reconfigurations.len(), 1);
        assert_eq!(
            dry_run.reconfigurations[0].to_node_id.as_deref(),
            Some("fog-02")
        );
    }

    #[test]
    fn flow_with_no_alternative_produces_blocked_item() {
        let mut graph = sample_unavailable_node_reconfiguration_graph("smart-city-campinas");
        graph.nodes[1].status = Some("failed".to_string());

        let dry_run = plan_dgraph_dry_run(&graph);
        assert!(dry_run
            .blocked
            .iter()
            .any(|item| item.reason.contains("no alternative candidate node found")));
    }

    #[test]
    fn dry_run_status_planned_when_proposals_only() {
        let graph = sample_unavailable_node_reconfiguration_graph("smart-city-campinas");
        let dry_run = plan_dgraph_dry_run(&graph);

        assert_eq!(dry_run.status, "planned");
        assert!(!dry_run.reconfigurations.is_empty());
        assert!(dry_run.blocked.is_empty());
    }

    #[test]
    fn dry_run_status_partially_blocked_when_proposals_and_blocked_items_exist() {
        let mut graph = sample_unavailable_node_reconfiguration_graph("smart-city-campinas");
        graph.modules.push(DGraphModule {
            module_id: "module-no-candidate".to_string(),
            module_kind: "service".to_string(),
            desired_state: Some("running".to_string()),
            observed_state: Some("missing".to_string()),
            runtime: Some("container".to_string()),
            artifact_id: None,
            metadata: None,
        });
        graph.modules[1].runtime = Some("wasm".to_string());
        graph.capabilities.clear();
        graph.relationships = build_basic_dgraph_relationships(&graph);

        let dry_run = plan_dgraph_dry_run(&graph);
        assert_eq!(dry_run.status, "partially_blocked");
        assert!(!dry_run.reconfigurations.is_empty() || !dry_run.placements.is_empty());
        assert!(!dry_run.blocked.is_empty());
    }

    #[test]
    fn dry_run_status_blocked_when_validation_fails() {
        let mut graph = sample_local_dgraph("smart-city-campinas");
        graph.project_id.clear();

        let dry_run = plan_dgraph_dry_run(&graph);
        assert_eq!(dry_run.status, "blocked");
    }

    #[test]
    fn dry_run_status_empty_when_no_action_needed() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.modules[0].observed_state = Some("running".to_string());
        graph.placements.push(DGraphPlacementDecision {
            placement_id: "placement-module-missing".to_string(),
            module_id: "module-missing".to_string(),
            selected_node_id: "fog-01".to_string(),
            selected_runtime_id: Some("runtime-container-fog-01".to_string()),
            reason: Some("already running".to_string()),
            status: "selected".to_string(),
            metadata: None,
        });

        let dry_run = plan_dgraph_dry_run(&graph);
        assert_eq!(dry_run.status, "empty");
    }

    #[test]
    fn side_effects_performed_is_false() {
        let dry_run = plan_dgraph_dry_run(&sample_two_node_dgraph("smart-city-campinas"));
        assert!(!dry_run.side_effects_performed);
    }

    #[test]
    fn summary_counts_are_correct() {
        let dry_run = plan_dgraph_dry_run(&sample_unavailable_node_reconfiguration_graph(
            "smart-city-campinas",
        ));
        assert_eq!(dry_run.summary.reconfiguration_count, 1);
        assert_eq!(dry_run.summary.blocked_count, 0);
        assert_eq!(dry_run.summary.candidate_node_count, 1);
        assert_eq!(dry_run.summary.unhealthy_node_count, 1);
        assert_eq!(dry_run.summary.degraded_flow_count, 1);
        assert_eq!(dry_run.summary.warning_count, 1);
    }

    #[test]
    fn baseline_flow_has_no_degraded_flow_advisory() {
        let graph = sample_two_node_dgraph("smart-city-campinas");
        let dry_run = plan_dgraph_dry_run(&graph);

        assert_eq!(graph.flows.len(), 1);
        assert_eq!(dry_run.summary.degraded_flow_count, 0);
        assert_eq!(dry_run.summary.warning_count, 0);
        assert_eq!(dry_run.summary.blocked_count, 0);
        assert!(dry_run.advisory_proposals.is_empty());
        assert!(!dry_run.safe_to_auto_execute);
        assert!(!dry_run.side_effects_performed);
    }

    #[test]
    fn degraded_flow_emits_structured_advisory_without_reconfiguration_execution() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].flow_id = "flow-chirpstack-to-mqtt-adapter".to_string();
        graph.flows[0].status = Some("degraded".to_string());
        graph.relationships = build_basic_dgraph_relationships(&graph);

        let dry_run = plan_dgraph_dry_run(&graph);

        assert_eq!(graph.flows.len(), 1);
        assert_eq!(dry_run.summary.degraded_flow_count, 1);
        assert!(dry_run.summary.warning_count >= 1);
        assert_eq!(dry_run.status, "planned");
        assert!(dry_run.reconfigurations.is_empty());
        assert_eq!(dry_run.advisory_proposals.len(), 1);
        let proposal = &dry_run.advisory_proposals[0];
        assert_eq!(
            proposal.proposal_id,
            "advisory:degraded-flow:flow-chirpstack-to-mqtt-adapter"
        );
        assert!(proposal.summary.contains("status 'degraded'"));
        assert!(proposal.reason.message.contains("status 'degraded'"));
        assert_eq!(
            proposal.related_flow_ids,
            vec!["flow-chirpstack-to-mqtt-adapter"]
        );
        assert!(proposal.advisory_only);
        assert!(!proposal.executable);
        assert!(!proposal.evidence.is_empty());
        assert!(!dry_run.safe_to_auto_execute);
        assert!(!dry_run.side_effects_performed);
        let warning = dry_run
            .warnings
            .iter()
            .find(|warning| {
                warning.warning_id == "warning:flow:flow-chirpstack-to-mqtt-adapter:degraded"
            })
            .expect("degraded flow advisory should be present");
        assert!(warning.reason.contains("flow-chirpstack-to-mqtt-adapter"));
        assert_eq!(
            warning
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("kind"))
                .and_then(|value| value.as_str()),
            Some("degraded_flow")
        );
        assert_eq!(
            warning
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("execution_performed"))
                .and_then(|value| value.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn warning_flow_preserves_its_observed_status_in_the_advisory() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].status = Some("warning".to_string());

        let proposal = build_advisory_proposals(&graph)
            .pop()
            .expect("warning flow should have an advisory");

        assert_eq!(
            proposal.evidence[0].observed_status.as_deref(),
            Some("warning")
        );
        assert!(proposal.summary.contains("status 'warning'"));
        assert!(proposal.reason.message.contains("status 'warning'"));
        assert!(!proposal.executable);
    }

    #[test]
    fn unavailable_node_behavior_remains_read_only() {
        let dry_run = plan_dgraph_dry_run(&sample_unavailable_node_reconfiguration_graph(
            "smart-city-campinas",
        ));

        assert_eq!(dry_run.summary.candidate_node_count, 1);
        assert_eq!(dry_run.summary.unhealthy_node_count, 1);
        assert!(!dry_run.side_effects_performed);
    }

    #[test]
    fn deterministic_ids_for_placements_reconfigurations_and_blockers() {
        let graph = sample_unavailable_node_reconfiguration_graph("smart-city-campinas");
        let first = plan_dgraph_dry_run(&graph);
        let second = plan_dgraph_dry_run(&graph);

        assert_eq!(
            first
                .reconfigurations
                .first()
                .map(|item| item.reconfiguration_id.clone()),
            second
                .reconfigurations
                .first()
                .map(|item| item.reconfiguration_id.clone())
        );

        let blocked_graph = sample_no_candidate_graph("smart-city-campinas");
        let blocked_first = plan_dgraph_dry_run(&blocked_graph);
        let blocked_second = plan_dgraph_dry_run(&blocked_graph);
        assert_eq!(
            blocked_first.blocked[0].item_id,
            blocked_second.blocked[0].item_id
        );
    }

    #[test]
    fn advisory_proposals_are_sorted_deduplicated_and_reference_graph_entities() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].flow_id = "flow-z".to_string();
        graph.flows[0].status = Some("degraded".to_string());
        let mut second = graph.flows[0].clone();
        second.flow_id = "flow-a".to_string();
        graph.flows.push(second);

        let first = build_advisory_proposals(&graph);
        let second = build_advisory_proposals(&graph);
        let ids = first
            .iter()
            .map(|proposal| proposal.proposal_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "advisory:degraded-flow:flow-a",
                "advisory:degraded-flow:flow-z"
            ]
        );
        assert_eq!(first, second);

        for proposal in &first {
            assert_eq!(proposal.candidate_node_ids, vec!["fog-01", "fog-02"]);
            for flow_id in &proposal.related_flow_ids {
                assert!(graph.flows.iter().any(|flow| &flow.flow_id == flow_id));
            }
            for node_id in &proposal.candidate_node_ids {
                assert!(graph.nodes.iter().any(|node| &node.node_id == node_id));
            }
            for evidence in &proposal.evidence {
                assert!(
                    (evidence.kind == "flow"
                        && graph
                            .flows
                            .iter()
                            .any(|flow| flow.flow_id == evidence.entity_id))
                        || (evidence.kind == "module"
                            && graph
                                .modules
                                .iter()
                                .any(|module| module.module_id == evidence.entity_id))
                );
            }
        }
    }

    #[test]
    fn incomplete_inputs_are_conservative_and_dry_run_does_not_mutate_graph() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].status = Some("degraded".to_string());
        graph.flows[0].modules = vec!["missing-module".to_string()];
        let before = graph.clone();
        let proposals = build_advisory_proposals(&graph);

        assert_eq!(graph, before);
        assert_eq!(proposals.len(), 1);
        assert!(proposals[0].candidate_node_ids.is_empty());
        assert!(proposals[0]
            .limitations
            .iter()
            .any(|item| item.contains("absent from the materialized graph")));
        assert!(proposals[0].advisory_only);
        assert!(!proposals[0].executable);
    }

    #[test]
    fn unknown_health_is_a_limitation_not_availability_evidence() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].status = Some("degraded".to_string());
        graph.health_reports.push(DGraphHealthReport {
            report_id: "health-module-missing".to_string(),
            target_kind: "module".to_string(),
            target_id: "module-missing".to_string(),
            status: "unknown".to_string(),
            reason: None,
            metadata: None,
        });

        let proposal = build_advisory_proposals(&graph)
            .pop()
            .expect("degraded flow should have an advisory");
        assert!(proposal
            .limitations
            .iter()
            .any(|item| item.contains("unknown and is not treated as availability")));
        assert!(proposal.evidence.iter().any(|item| {
            item.kind == "health_report"
                && item.entity_id == "health-module-missing"
                && item.observed_status.as_deref() == Some("unknown")
        }));
        assert!(!proposal.executable);
    }

    #[test]
    fn unknown_candidate_health_is_explicitly_limited_without_selecting_a_destination() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].status = Some("degraded".to_string());
        graph.health_reports.push(DGraphHealthReport {
            report_id: "health-fog-02-unknown".to_string(),
            target_kind: "node".to_string(),
            target_id: "fog-02".to_string(),
            status: "unknown".to_string(),
            reason: None,
            metadata: None,
        });
        let before = graph.clone();

        let first = build_advisory_proposals(&graph);
        let second = build_advisory_proposals(&graph);
        let proposal = first
            .first()
            .expect("degraded flow should have an advisory proposal");
        let unknown_node_limitation = "candidate node 'fog-02' has unknown or unestablished observed health; availability, suitability, and safety for placement are not established, and its presence in candidate_node_ids is observational only, not a selected destination, recommendation, or execution authorization";

        assert_eq!(graph, before);
        assert_eq!(first, second);
        assert_eq!(proposal.related_flow_ids, vec!["flow-one"]);
        assert!(proposal.candidate_node_ids.contains(&"fog-02".to_string()));
        assert!(proposal
            .limitations
            .contains(&unknown_node_limitation.to_string()));
        assert!(proposal
            .limitations
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
        assert!(proposal.advisory_only);
        assert!(!proposal.executable);

        let dry_run = plan_dgraph_dry_run(&graph);
        assert!(!dry_run.safe_to_auto_execute);
        assert!(!dry_run.side_effects_performed);
        assert!(dry_run.reconfigurations.is_empty());
    }

    #[test]
    fn advisory_proposals_serialize_additively_with_safety_flags_false() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.flows[0].status = Some("degraded".to_string());
        let value =
            serde_json::to_value(plan_dgraph_dry_run(&graph)).expect("planner should serialize");

        assert!(value.get("placements").is_some());
        assert!(value.get("warnings").is_some());
        assert!(value.get("advisory_proposals").is_some());
        assert_eq!(value["safe_to_auto_execute"], false);
        assert_eq!(value["side_effects_performed"], false);
    }

    #[test]
    fn project_response_wrapper_keeps_side_effects_false() {
        let planner = plan_dgraph_dry_run(&sample_two_node_dgraph("smart-city-campinas"));
        let response = build_project_dgraph_planner_dry_run_response(
            "smart-city-campinas",
            2,
            Vec::new(),
            planner,
        );

        assert!(!response.side_effects_performed);
        assert!(!response.planner.side_effects_performed);
    }

    #[test]
    fn dry_run_over_empty_graph_keeps_side_effects_false() {
        let graph = DGraph {
            project_id: "smart-city-campinas".to_string(),
            ..DGraph::default()
        };
        let dry_run = plan_dgraph_dry_run(&graph);

        assert_eq!(dry_run.status, "empty");
        assert!(!dry_run.side_effects_performed);
    }

    #[test]
    fn summary_response_counts_items_correctly() {
        let planner = plan_dgraph_dry_run(&sample_unavailable_node_reconfiguration_graph(
            "smart-city-campinas",
        ));
        let response = build_project_dgraph_planner_dry_run_response(
            "smart-city-campinas",
            1,
            vec!["partial materialization".to_string()],
            planner,
        );
        let summary = response.as_summary();

        assert_eq!(summary.project_id, "smart-city-campinas");
        assert_eq!(summary.source_snapshot_count, 1);
        assert_eq!(summary.reconfiguration_count, 1);
        assert_eq!(summary.blocked_count, 0);
        assert_eq!(summary.warning_count, 1);
        assert!(!summary.side_effects_performed);
    }

    #[test]
    fn materialization_validation_errors_are_preserved_in_project_response() {
        let planner = plan_dgraph_dry_run(&sample_two_node_dgraph("smart-city-campinas"));
        let response = build_project_dgraph_planner_dry_run_response(
            "smart-city-campinas",
            2,
            vec!["flow references partial data".to_string()],
            planner,
        );

        assert_eq!(
            response.materialization_validation_errors,
            vec!["flow references partial data".to_string()]
        );
    }

    #[test]
    fn planner_validation_errors_produce_blocked_output() {
        let mut graph = sample_two_node_dgraph("smart-city-campinas");
        graph.project_id.clear();

        let response = build_project_dgraph_planner_dry_run_response(
            "smart-city-campinas",
            1,
            vec!["project_id must not be empty".to_string()],
            plan_dgraph_dry_run(&graph),
        );

        assert_eq!(response.planner.status, "blocked");
        assert!(!response.planner.blocked.is_empty());
        assert!(!response.side_effects_performed);
    }
}
