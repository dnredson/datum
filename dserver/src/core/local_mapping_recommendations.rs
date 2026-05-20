use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::core::installation_tasks::InstallationTask;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalMappingRecommendation {
    pub project_id: String,
    pub node_id: String,
    pub recommendation_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub planning_id: Option<String>,
    #[serde(default)]
    pub action_id: Option<String>,
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
    pub recommendation_status: String,
    #[serde(default)]
    pub priority: Option<String>,
    pub reason: String,
    pub source: String,
    pub safe_to_auto_apply: bool,
    pub requires_operator: bool,
    #[serde(default)]
    pub source_task: Option<Value>,
    #[serde(default)]
    pub source_installation_item: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalMappingRecommendationList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub recommendations: Vec<LocalMappingRecommendation>,
}

pub fn derive_local_mapping_recommendations_from_tasks(
    project_id: &str,
    node_id: Option<&str>,
    tasks: &[InstallationTask],
) -> Vec<LocalMappingRecommendation> {
    let mut seen = HashSet::new();
    let mut recommendations = Vec::new();

    for task in tasks {
        if let Some(filter_node_id) = node_id {
            if task.node_id != filter_node_id {
                continue;
            }
        }

        if !task_requires_local_mapping(task) {
            continue;
        }

        let dedupe_key = dedupe_key(task);
        if !seen.insert(dedupe_key) {
            continue;
        }

        recommendations.push(LocalMappingRecommendation {
            project_id: project_id.to_string(),
            node_id: task.node_id.clone(),
            recommendation_id: recommendation_id(project_id, task),
            task_id: Some(task.task_id.clone()),
            installation_id: Some(task.installation_id.clone()),
            planning_id: task.planning_id.clone(),
            action_id: task.action_id.clone(),
            module_id: task.module_id.clone(),
            runtime: task.runtime.clone(),
            artifact: task.artifact.clone(),
            suggested_mapping_key: preferred_mapping_key(task),
            suggested_container_name: None,
            suggested_artifact: task.artifact.clone(),
            recommendation_status: "proposed".to_string(),
            priority: task.priority.clone(),
            reason: recommendation_reason(task),
            source: "installation_task".to_string(),
            safe_to_auto_apply: false,
            requires_operator: true,
            source_task: serde_json::to_value(task).ok(),
            source_installation_item: task.source_installation_item.clone(),
            metadata: recommendation_metadata(task),
        });
    }

    recommendations.sort_by(|a, b| {
        a.node_id
            .cmp(&b.node_id)
            .then_with(|| a.task_id.cmp(&b.task_id))
            .then_with(|| a.installation_id.cmp(&b.installation_id))
            .then_with(|| a.recommendation_id.cmp(&b.recommendation_id))
    });

    recommendations
}

fn task_requires_local_mapping(task: &InstallationTask) -> bool {
    eq_ignore_ascii(task.required_step.as_deref(), "add_local_mapping")
        || eq_ignore_ascii(task.installation_action.as_deref(), "prepare_local_mapping")
        || contains_local_mapping_phrase(task.reason.as_deref())
        || value_mentions_missing_local_mapping(task.source_installation_item.as_ref())
        || value_mentions_missing_local_mapping(task.metadata.as_ref())
}

fn eq_ignore_ascii(value: Option<&str>, expected: &str) -> bool {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.eq_ignore_ascii_case(expected))
        .unwrap_or(false)
}

fn contains_local_mapping_phrase(value: Option<&str>) -> bool {
    let normalized = value.unwrap_or_default().trim().to_ascii_lowercase();
    normalized.contains("local mapping") || normalized.contains("trusted local mapping")
}

fn value_mentions_missing_local_mapping(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(text)) => {
            let normalized = text.trim().to_ascii_lowercase();
            normalized.contains("missing_local_mapping")
                || normalized.contains("trusted local mapping")
                || normalized.contains("local mapping")
        }
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| value_mentions_missing_local_mapping(Some(item))),
        Some(Value::Object(map)) => map
            .values()
            .any(|item| value_mentions_missing_local_mapping(Some(item))),
        _ => false,
    }
}

fn preferred_mapping_key(task: &InstallationTask) -> Option<String> {
    first_non_empty(&[
        task.module_id.as_deref(),
        task.artifact.as_deref(),
        task.action_id.as_deref(),
        Some(task.installation_id.as_str()),
    ])
}

fn first_non_empty(values: &[Option<&str>]) -> Option<String> {
    values
        .iter()
        .copied()
        .flatten()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn recommendation_id(project_id: &str, task: &InstallationTask) -> String {
    if !task.task_id.trim().is_empty() {
        return format!(
            "project:{}:node:{}:task:{}:local-mapping-recommendation",
            project_id, task.node_id, task.task_id
        );
    }

    format!(
        "project:{}:node:{}:installation:{}:local-mapping-recommendation",
        project_id, task.node_id, task.installation_id
    )
}

fn dedupe_key(task: &InstallationTask) -> String {
    if !task.task_id.trim().is_empty() {
        return format!("task:{}", task.task_id);
    }

    format!("installation:{}", task.installation_id)
}

fn recommendation_reason(task: &InstallationTask) -> String {
    task.reason.clone().unwrap_or_else(|| {
        "trusted local mapping is required before assisted execution can proceed".to_string()
    })
}

fn recommendation_metadata(task: &InstallationTask) -> Option<Value> {
    let mut metadata = serde_json::Map::new();

    if let Some(policy) = task.installation_policy.as_ref() {
        metadata.insert(
            "installation_policy".to_string(),
            serde_json::to_value(policy).unwrap_or(Value::Null),
        );
    }

    if let Some(source) = task.source.as_ref() {
        metadata.insert("task_source".to_string(), json!(source));
    }

    if let Some(existing) = task.metadata.as_ref() {
        metadata.insert("task_metadata".to_string(), existing.clone());
    }

    if metadata.is_empty() {
        None
    } else {
        Some(Value::Object(metadata))
    }
}

#[cfg(test)]
mod tests {
    use super::derive_local_mapping_recommendations_from_tasks;
    use crate::core::installation_tasks::{InstallationPolicyDecision, InstallationTask};
    use serde_json::{json, Value};

    fn base_task() -> InstallationTask {
        InstallationTask {
            project_id: "smart-city-campinas".to_string(),
            node_id: "fog-01".to_string(),
            task_id: "task-1".to_string(),
            installation_id: "installation-1".to_string(),
            planning_id: Some("planning-1".to_string()),
            action_id: Some("action-1".to_string()),
            module_id: Some("chirpstack-primary".to_string()),
            runtime: Some("container".to_string()),
            artifact: Some("chirpstack/chirpstack:latest".to_string()),
            required_step: Some("add_local_mapping".to_string()),
            installation_action: Some("prepare_local_mapping".to_string()),
            status: "planned".to_string(),
            priority: Some("high".to_string()),
            reason: Some("client lacks trusted local mapping for this action".to_string()),
            requires_operator: true,
            safe_to_auto_execute: false,
            source: Some("installation_task".to_string()),
            created_at_utc: "2026-05-20T12:00:00Z".to_string(),
            updated_at_utc: "2026-05-20T12:00:00Z".to_string(),
            source_installation_item: Some(json!({
                "module_id": "chirpstack-primary",
                "missing_local_mapping": true
            })),
            installation_policy: Some(InstallationPolicyDecision {
                policy_version: "0.1.0".to_string(),
                decision: "manual_only".to_string(),
                reason: "local mapping requires operator review".to_string(),
                requires_operator: true,
                safe_to_auto_execute: false,
                allowed_future_modes: vec![],
            }),
            metadata: None,
        }
    }

    #[test]
    fn task_with_required_step_add_local_mapping_produces_recommendation() {
        let recommendations = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            None,
            &[base_task()],
        );

        assert_eq!(recommendations.len(), 1);
        assert_eq!(
            recommendations[0].suggested_mapping_key.as_deref(),
            Some("chirpstack-primary")
        );
    }

    #[test]
    fn task_with_installation_action_prepare_local_mapping_produces_recommendation() {
        let mut task = base_task();
        task.required_step = None;

        let recommendations =
            derive_local_mapping_recommendations_from_tasks("smart-city-campinas", None, &[task]);

        assert_eq!(recommendations.len(), 1);
    }

    #[test]
    fn task_with_unrelated_required_step_produces_no_recommendation() {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        task.installation_action = Some("install_artifact".to_string());
        task.reason = Some("artifact needs installation".to_string());
        task.source_installation_item = Some(json!({ "artifact": "x" }));

        let recommendations =
            derive_local_mapping_recommendations_from_tasks("smart-city-campinas", None, &[task]);

        assert!(recommendations.is_empty());
    }

    #[test]
    fn suggested_mapping_key_prefers_module_id() {
        let recommendation = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            None,
            &[base_task()],
        )
        .remove(0);

        assert_eq!(
            recommendation.suggested_mapping_key.as_deref(),
            Some("chirpstack-primary")
        );
    }

    #[test]
    fn suggested_mapping_key_falls_back_to_artifact() {
        let mut task = base_task();
        task.module_id = None;

        let recommendation =
            derive_local_mapping_recommendations_from_tasks("smart-city-campinas", None, &[task])
                .remove(0);

        assert_eq!(
            recommendation.suggested_mapping_key.as_deref(),
            Some("chirpstack/chirpstack:latest")
        );
    }

    #[test]
    fn suggested_mapping_key_falls_back_to_action_id() {
        let mut task = base_task();
        task.module_id = None;
        task.artifact = None;

        let recommendation =
            derive_local_mapping_recommendations_from_tasks("smart-city-campinas", None, &[task])
                .remove(0);

        assert_eq!(
            recommendation.suggested_mapping_key.as_deref(),
            Some("action-1")
        );
    }

    #[test]
    fn safe_to_auto_apply_remains_false() {
        let recommendation = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            None,
            &[base_task()],
        )
        .remove(0);

        assert!(!recommendation.safe_to_auto_apply);
    }

    #[test]
    fn requires_operator_remains_true() {
        let recommendation = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            None,
            &[base_task()],
        )
        .remove(0);

        assert!(recommendation.requires_operator);
    }

    #[test]
    fn recommendations_are_deduplicated_by_task_id_or_installation_id() {
        let mut duplicate = base_task();
        duplicate.reason = Some("duplicate record".to_string());

        let recommendations = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            None,
            &[base_task(), duplicate],
        );

        assert_eq!(recommendations.len(), 1);
    }

    #[test]
    fn source_task_is_preserved() {
        let recommendation = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            None,
            &[base_task()],
        )
        .remove(0);

        assert_eq!(
            recommendation
                .source_task
                .as_ref()
                .and_then(|value| value.get("task_id"))
                .and_then(Value::as_str),
            Some("task-1")
        );
    }

    #[test]
    fn node_id_filtering_works() {
        let mut other_node = base_task();
        other_node.node_id = "fog-02".to_string();
        other_node.task_id = "task-2".to_string();
        other_node.installation_id = "installation-2".to_string();

        let recommendations = derive_local_mapping_recommendations_from_tasks(
            "smart-city-campinas",
            Some("fog-01"),
            &[base_task(), other_node],
        );

        assert_eq!(recommendations.len(), 1);
        assert_eq!(recommendations[0].node_id, "fog-01");
    }

    #[test]
    fn empty_tasks_produce_empty_recommendations() {
        let recommendations =
            derive_local_mapping_recommendations_from_tasks("smart-city-campinas", None, &[]);

        assert!(recommendations.is_empty());
    }
}
