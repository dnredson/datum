use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::{
    local_mapping_preflight::{LocalMappingPreflightItem, LocalMappingPreflightReport},
    local_mapping_recommendations::LocalMappingRecommendation,
};

const PROPOSAL_STATUS_PROPOSED: &str = "proposed";
const PROPOSAL_STATUS_BLOCKED: &str = "blocked";
const PROPOSAL_STATUS_AMBIGUOUS: &str = "ambiguous";
const PROPOSAL_STATUS_NOT_APPLICABLE: &str = "not_applicable";

const PROPOSAL_KIND_ADD_CONTAINER_MAPPING: &str = "add_container_mapping";
const PROPOSAL_KIND_REVIEW_AMBIGUOUS_MAPPING: &str = "review_ambiguous_mapping";
const PROPOSAL_KIND_NO_CHANGE: &str = "no_change";
const PROPOSAL_KIND_UNKNOWN: &str = "unknown";

const PREFLIGHT_STATUS_ALREADY_MAPPED: &str = "already_mapped";
const PREFLIGHT_STATUS_AMBIGUOUS: &str = "ambiguous";
const PREFLIGHT_STATUS_NOT_APPLICABLE: &str = "not_applicable";

const DEFAULT_CONTAINER_PLACEHOLDER: &str = "REVIEW_AND_SET_CONTAINER_NAME";
const CONFIG_SECTION_CONTAINER_MAPPINGS: &str = "dispatch.container_mappings";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalMappingPatchProposal {
    pub project_id: String,
    pub node_id: String,
    pub proposal_id: String,
    pub created_at_utc: String,
    pub proposal_status: String,
    pub proposal_kind: String,
    pub source: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub recommendation_id: Option<String>,
    #[serde(default)]
    pub report_id: Option<String>,
    #[serde(default)]
    pub module_id: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub artifact: Option<String>,
    #[serde(default)]
    pub suggested_mapping_key: Option<String>,
    #[serde(default)]
    pub suggested_container_name: Option<String>,
    #[serde(default)]
    pub suggested_artifact: Option<String>,
    #[serde(default)]
    pub proposed_config_section: Option<String>,
    #[serde(default)]
    pub proposed_config_snippet: Option<String>,
    pub reason: String,
    pub requires_operator: bool,
    pub safe_to_auto_apply: bool,
    pub side_effects_performed: bool,
    #[serde(default)]
    pub recommendation_snapshot: Option<Value>,
    #[serde(default)]
    pub preflight_item_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalMappingPatchProposalList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub proposals: Vec<LocalMappingPatchProposal>,
}

pub fn derive_local_mapping_patch_proposals(
    project_id: &str,
    node_id: &str,
    recommendations: &[LocalMappingRecommendation],
    latest_preflight_report: Option<&LocalMappingPreflightReport>,
) -> Vec<LocalMappingPatchProposal> {
    let mut proposals = recommendations
        .iter()
        .filter(|recommendation| recommendation.node_id == node_id)
        .map(|recommendation| {
            let matching_item = latest_preflight_report
                .and_then(|report| find_matching_preflight_item(report, recommendation));

            build_patch_proposal(
                project_id,
                node_id,
                recommendation,
                latest_preflight_report,
                matching_item,
            )
        })
        .collect::<Vec<_>>();

    proposals.sort_by(|a, b| {
        a.node_id
            .cmp(&b.node_id)
            .then_with(|| a.task_id.cmp(&b.task_id))
            .then_with(|| a.recommendation_id.cmp(&b.recommendation_id))
            .then_with(|| a.proposal_id.cmp(&b.proposal_id))
    });

    proposals
}

fn build_patch_proposal(
    project_id: &str,
    node_id: &str,
    recommendation: &LocalMappingRecommendation,
    latest_preflight_report: Option<&LocalMappingPreflightReport>,
    matching_item: Option<&LocalMappingPreflightItem>,
) -> LocalMappingPatchProposal {
    let normalized_mapping_key =
        trimmed_non_empty_option(recommendation.suggested_mapping_key.as_deref());
    let normalized_artifact =
        trimmed_non_empty_option(recommendation.suggested_artifact.as_deref())
            .or_else(|| trimmed_non_empty_option(recommendation.artifact.as_deref()));
    let normalized_container_name =
        trimmed_non_empty_option(recommendation.suggested_container_name.as_deref());

    let (proposal_kind, proposal_status, reason, proposed_config_section, proposed_config_snippet) =
        derive_proposal_shape(
            recommendation,
            matching_item,
            normalized_mapping_key.as_deref(),
            normalized_artifact.as_deref(),
            normalized_container_name.as_deref(),
        );

    let recommendation_snapshot = serde_json::to_value(recommendation).ok();
    let preflight_item_snapshot = matching_item.and_then(|item| serde_json::to_value(item).ok());
    let report_id = latest_preflight_report
        .filter(|_| matching_item.is_some())
        .map(|report| report.report_id.clone());
    let metadata = build_metadata(recommendation, matching_item, latest_preflight_report);

    LocalMappingPatchProposal {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        proposal_id: proposal_id_for(project_id, node_id, recommendation),
        created_at_utc: Utc::now().to_rfc3339(),
        proposal_status,
        proposal_kind,
        source: proposal_source(latest_preflight_report, matching_item),
        task_id: recommendation.task_id.clone(),
        recommendation_id: Some(recommendation.recommendation_id.clone()),
        report_id,
        module_id: recommendation.module_id.clone(),
        runtime: recommendation.runtime.clone(),
        artifact: recommendation.artifact.clone(),
        suggested_mapping_key: normalized_mapping_key,
        suggested_container_name: normalized_container_name,
        suggested_artifact: normalized_artifact,
        proposed_config_section,
        proposed_config_snippet,
        reason,
        requires_operator: true,
        safe_to_auto_apply: false,
        side_effects_performed: false,
        recommendation_snapshot,
        preflight_item_snapshot,
        metadata,
    }
}

fn derive_proposal_shape(
    recommendation: &LocalMappingRecommendation,
    matching_item: Option<&LocalMappingPreflightItem>,
    mapping_key: Option<&str>,
    artifact: Option<&str>,
    suggested_container_name: Option<&str>,
) -> (String, String, String, Option<String>, Option<String>) {
    if let Some(item) = matching_item {
        match item.status.trim() {
            PREFLIGHT_STATUS_AMBIGUOUS => {
                return (
                    PROPOSAL_KIND_REVIEW_AMBIGUOUS_MAPPING.to_string(),
                    PROPOSAL_STATUS_AMBIGUOUS.to_string(),
                    item_reason_or_fallback(item, recommendation),
                    None,
                    None,
                );
            }
            PREFLIGHT_STATUS_ALREADY_MAPPED | PREFLIGHT_STATUS_NOT_APPLICABLE => {
                return (
                    PROPOSAL_KIND_NO_CHANGE.to_string(),
                    PROPOSAL_STATUS_NOT_APPLICABLE.to_string(),
                    item_reason_or_fallback(item, recommendation),
                    None,
                    None,
                );
            }
            _ => {}
        }
    }

    if let (Some(mapping_key), Some(artifact)) = (mapping_key, artifact) {
        return (
            PROPOSAL_KIND_ADD_CONTAINER_MAPPING.to_string(),
            PROPOSAL_STATUS_PROPOSED.to_string(),
            matching_item
                .map(|item| item_reason_or_fallback(item, recommendation))
                .unwrap_or_else(|| recommendation_reason_or_default(recommendation)),
            Some(CONFIG_SECTION_CONTAINER_MAPPINGS.to_string()),
            Some(proposed_config_snippet(
                mapping_key,
                artifact,
                suggested_container_name.unwrap_or(DEFAULT_CONTAINER_PLACEHOLDER),
            )),
        );
    }

    (
        PROPOSAL_KIND_UNKNOWN.to_string(),
        PROPOSAL_STATUS_BLOCKED.to_string(),
        blocked_reason(recommendation, mapping_key, artifact),
        None,
        None,
    )
}

fn build_metadata(
    recommendation: &LocalMappingRecommendation,
    matching_item: Option<&LocalMappingPreflightItem>,
    latest_preflight_report: Option<&LocalMappingPreflightReport>,
) -> Option<Value> {
    let mut metadata = serde_json::Map::new();

    if recommendation.safe_to_auto_apply {
        metadata.insert(
            "normalized_safe_to_auto_apply".to_string(),
            Value::Bool(false),
        );
    }

    if !recommendation.requires_operator {
        metadata.insert(
            "normalized_requires_operator".to_string(),
            Value::Bool(true),
        );
    }

    if let Some(report) = latest_preflight_report {
        metadata.insert(
            "preflight_report_status".to_string(),
            Value::String(report.status.clone()),
        );
    }

    if let Some(item) = matching_item {
        metadata.insert(
            "preflight_item_status".to_string(),
            Value::String(item.status.clone()),
        );
    }

    if metadata.is_empty() {
        None
    } else {
        Some(Value::Object(metadata))
    }
}

fn find_matching_preflight_item<'a>(
    report: &'a LocalMappingPreflightReport,
    recommendation: &LocalMappingRecommendation,
) -> Option<&'a LocalMappingPreflightItem> {
    if let Some(task_id) = recommendation
        .task_id
        .as_deref()
        .and_then(trimmed_non_empty)
    {
        if let Some(item) = report
            .items
            .iter()
            .find(|item| option_matches(item.task_id.as_deref(), task_id))
        {
            return Some(item);
        }
    }

    if let Some(module_id) = recommendation
        .module_id
        .as_deref()
        .and_then(trimmed_non_empty)
    {
        if let Some(item) = report
            .items
            .iter()
            .find(|item| option_matches(item.module_id.as_deref(), module_id))
        {
            return Some(item);
        }
    }

    if let Some(artifact) = recommendation
        .artifact
        .as_deref()
        .and_then(trimmed_non_empty)
    {
        if let Some(item) = report
            .items
            .iter()
            .find(|item| option_matches(item.artifact.as_deref(), artifact))
        {
            return Some(item);
        }
    }

    recommendation
        .suggested_mapping_key
        .as_deref()
        .and_then(trimmed_non_empty)
        .and_then(|suggested_mapping_key| {
            report.items.iter().find(|item| {
                option_matches(item.suggested_mapping_key.as_deref(), suggested_mapping_key)
            })
        })
}

fn blocked_reason(
    recommendation: &LocalMappingRecommendation,
    mapping_key: Option<&str>,
    artifact: Option<&str>,
) -> String {
    let mut missing = Vec::new();

    if mapping_key.is_none() {
        missing.push("suggested mapping key");
    }

    if artifact.is_none() {
        missing.push("artifact");
    }

    let missing_summary = if missing.is_empty() {
        "operator review is required before proposing a local mapping patch".to_string()
    } else {
        format!(
            "operator review is required before proposing a local mapping patch because {} is missing",
            missing.join(" and ")
        )
    };

    let recommendation_reason = recommendation_reason_or_default(recommendation);
    format!("{missing_summary}; recommendation context: {recommendation_reason}")
}

fn proposal_source(
    latest_preflight_report: Option<&LocalMappingPreflightReport>,
    matching_item: Option<&LocalMappingPreflightItem>,
) -> String {
    if latest_preflight_report.is_some() && matching_item.is_some() {
        "recommendation_and_preflight".to_string()
    } else {
        "recommendation".to_string()
    }
}

fn proposal_id_for(
    project_id: &str,
    node_id: &str,
    recommendation: &LocalMappingRecommendation,
) -> String {
    if let Some(task_id) = recommendation
        .task_id
        .as_deref()
        .and_then(trimmed_non_empty)
    {
        return format!(
            "project:{}:node:{}:task:{}:local-mapping-patch-proposal",
            project_id, node_id, task_id
        );
    }

    if !recommendation.recommendation_id.trim().is_empty() {
        return format!(
            "project:{}:node:{}:recommendation:{}:local-mapping-patch-proposal",
            project_id, node_id, recommendation.recommendation_id
        );
    }

    format!(
        "project:{}:node:{}:local-mapping-patch-proposal",
        project_id, node_id
    )
}

fn proposed_config_snippet(
    mapping_key: &str,
    artifact: &str,
    suggested_container_name: &str,
) -> String {
    format!(
        "# Advisory only. Operator review required before any manual config update.\n# Suggested artifact: {}\n[dispatch.container_mappings]\n\"{}\" = \"{}\"",
        artifact, mapping_key, suggested_container_name
    )
}

fn item_reason_or_fallback(
    item: &LocalMappingPreflightItem,
    recommendation: &LocalMappingRecommendation,
) -> String {
    if let Some(reason) = trimmed_non_empty(&item.reason) {
        reason.to_string()
    } else {
        recommendation_reason_or_default(recommendation)
    }
}

fn recommendation_reason_or_default(recommendation: &LocalMappingRecommendation) -> String {
    trimmed_non_empty(&recommendation.reason)
        .unwrap_or("local mapping patch requires operator review")
        .to_string()
}

fn trimmed_non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn trimmed_non_empty_option(value: Option<&str>) -> Option<String> {
    value.and_then(trimmed_non_empty).map(ToString::to_string)
}

fn option_matches(value: Option<&str>, expected: &str) -> bool {
    value.and_then(trimmed_non_empty) == Some(expected)
}

#[cfg(test)]
mod tests {
    use super::derive_local_mapping_patch_proposals;
    use crate::core::{
        local_mapping_preflight::{LocalMappingPreflightItem, LocalMappingPreflightReport},
        local_mapping_recommendations::LocalMappingRecommendation,
    };
    use serde_json::json;

    fn base_recommendation() -> LocalMappingRecommendation {
        LocalMappingRecommendation {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            recommendation_id: "rec-1".to_string(),
            task_id: Some("task-1".to_string()),
            installation_id: Some("installation-1".to_string()),
            planning_id: Some("planning-1".to_string()),
            action_id: Some("action-1".to_string()),
            module_id: Some("chirpstack-primary".to_string()),
            runtime: Some("container".to_string()),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
            suggested_mapping_key: Some("chirpstack-primary".to_string()),
            suggested_container_name: None,
            suggested_artifact: Some("chirpstack/chirpstack:latest".to_string()),
            recommendation_status: "proposed".to_string(),
            priority: Some("high".to_string()),
            reason: "no trusted local mapping found".to_string(),
            source: "installation_task".to_string(),
            safe_to_auto_apply: false,
            requires_operator: true,
            source_task: Some(json!({ "task_id": "task-1" })),
            source_installation_item: None,
            metadata: None,
        }
    }

    fn base_report(items: Vec<LocalMappingPreflightItem>) -> LocalMappingPreflightReport {
        LocalMappingPreflightReport {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            report_id: "report-1".to_string(),
            reported_at_utc: "2026-05-20T12:00:00Z".to_string(),
            source: Some("client".to_string()),
            status: "warning".to_string(),
            items,
            summary: Default::default(),
            safe_to_auto_execute: false,
            side_effects_performed: false,
            metadata: None,
        }
    }

    fn preflight_item(status: &str) -> LocalMappingPreflightItem {
        LocalMappingPreflightItem {
            task_id: Some("task-1".to_string()),
            installation_id: Some("installation-1".to_string()),
            action_id: Some("action-1".to_string()),
            module_id: Some("chirpstack-primary".to_string()),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
            suggested_mapping_key: Some("chirpstack-primary".to_string()),
            status: status.to_string(),
            reason: format!("preflight {}", status),
            ..LocalMappingPreflightItem::default()
        }
    }

    #[test]
    fn recommendation_with_suggested_mapping_key_produces_add_container_mapping_proposal() {
        let proposals = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            None,
        );

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].proposal_kind, "add_container_mapping");
        assert_eq!(proposals[0].proposal_status, "proposed");
        assert_eq!(
            proposals[0].proposed_config_section.as_deref(),
            Some("dispatch.container_mappings")
        );
    }

    #[test]
    fn proposal_safe_to_auto_apply_is_false() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            None,
        )
        .remove(0);

        assert!(!proposal.safe_to_auto_apply);
    }

    #[test]
    fn proposal_side_effects_performed_is_false() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            None,
        )
        .remove(0);

        assert!(!proposal.side_effects_performed);
    }

    #[test]
    fn proposal_requires_operator_is_true() {
        let mut recommendation = base_recommendation();
        recommendation.requires_operator = false;

        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[recommendation],
            None,
        )
        .remove(0);

        assert!(proposal.requires_operator);
    }

    #[test]
    fn missing_container_name_uses_review_placeholder() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            None,
        )
        .remove(0);

        assert_eq!(proposal.suggested_container_name, None);
        assert!(proposal
            .proposed_config_snippet
            .as_deref()
            .unwrap_or_default()
            .contains("REVIEW_AND_SET_CONTAINER_NAME"));
    }

    #[test]
    fn matching_preflight_missing_mapping_preserves_proposed_status() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            Some(&base_report(vec![preflight_item("missing_mapping")])),
        )
        .remove(0);

        assert_eq!(proposal.proposal_kind, "add_container_mapping");
        assert_eq!(proposal.proposal_status, "proposed");
    }

    #[test]
    fn matching_preflight_ambiguous_produces_review_ambiguous_mapping() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            Some(&base_report(vec![preflight_item("ambiguous")])),
        )
        .remove(0);

        assert_eq!(proposal.proposal_kind, "review_ambiguous_mapping");
        assert_eq!(proposal.proposal_status, "ambiguous");
    }

    #[test]
    fn matching_preflight_already_mapped_produces_no_change() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            Some(&base_report(vec![preflight_item("already_mapped")])),
        )
        .remove(0);

        assert_eq!(proposal.proposal_kind, "no_change");
        assert_eq!(proposal.proposal_status, "not_applicable");
    }

    #[test]
    fn insufficient_data_produces_blocked_unknown() {
        let mut recommendation = base_recommendation();
        recommendation.suggested_mapping_key = None;
        recommendation.suggested_artifact = None;
        recommendation.artifact = None;

        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[recommendation],
            None,
        )
        .remove(0);

        assert_eq!(proposal.proposal_kind, "unknown");
        assert_eq!(proposal.proposal_status, "blocked");
    }

    #[test]
    fn matching_by_task_id_works() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            Some(&base_report(vec![preflight_item("missing_mapping")])),
        )
        .remove(0);

        assert_eq!(proposal.report_id.as_deref(), Some("report-1"));
        assert!(proposal.preflight_item_snapshot.is_some());
    }

    #[test]
    fn matching_by_module_id_works() {
        let mut recommendation = base_recommendation();
        recommendation.task_id = Some("different-task".to_string());
        let mut item = preflight_item("missing_mapping");
        item.task_id = Some("other-task".to_string());

        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[recommendation],
            Some(&base_report(vec![item])),
        )
        .remove(0);

        assert_eq!(proposal.source, "recommendation_and_preflight");
    }

    #[test]
    fn matching_by_artifact_works() {
        let mut recommendation = base_recommendation();
        recommendation.task_id = Some("different-task".to_string());
        recommendation.module_id = Some("different-module".to_string());
        let mut item = preflight_item("missing_mapping");
        item.task_id = Some("other-task".to_string());
        item.module_id = Some("other-module".to_string());

        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[recommendation],
            Some(&base_report(vec![item])),
        )
        .remove(0);

        assert_eq!(proposal.report_id.as_deref(), Some("report-1"));
    }

    #[test]
    fn empty_recommendations_produce_empty_proposals() {
        let proposals =
            derive_local_mapping_patch_proposals("smart-city-campinas", "fog-01", &[], None);

        assert!(proposals.is_empty());
    }

    #[test]
    fn proposed_config_snippet_never_claims_automatic_application() {
        let proposal = derive_local_mapping_patch_proposals(
            "smart-city-campinas",
            "fog-01",
            &[base_recommendation()],
            None,
        )
        .remove(0);

        let snippet = proposal.proposed_config_snippet.unwrap_or_default();
        assert!(snippet.contains("Advisory only"));
        assert!(!snippet.to_ascii_lowercase().contains("auto-apply"));
        assert!(!snippet.to_ascii_lowercase().contains("automatically apply"));
    }
}
