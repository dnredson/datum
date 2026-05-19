use std::collections::HashSet;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const STATUS_PLANNED: &str = "planned";
const STATUS_ACKNOWLEDGED: &str = "acknowledged";
const STATUS_BLOCKED: &str = "blocked";
const STATUS_CANCELLED: &str = "cancelled";
const STATUS_COMPLETED: &str = "completed";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationTask {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub installation_id: String,
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
    pub required_step: Option<String>,
    #[serde(default)]
    pub installation_action: Option<String>,
    pub status: String,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    pub requires_operator: bool,
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub source: Option<String>,
    pub created_at_utc: String,
    pub updated_at_utc: String,
    #[serde(default)]
    pub source_installation_item: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationTaskList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub tasks: Vec<InstallationTask>,
}

fn is_valid_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_PLANNED | STATUS_ACKNOWLEDGED | STATUS_BLOCKED | STATUS_CANCELLED | STATUS_COMPLETED
    )
}

fn derive_installation_id(item: &Value) -> Option<String> {
    if let Some(installation_id) = item.get("installation_id").and_then(Value::as_str) {
        let trimmed = installation_id.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let planning_id = item
        .get("planning_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let action_id = item
        .get("action_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let installation_action = item
        .get("installation_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");

    if let Some(planning_id) = planning_id {
        return Some(format!(
            "installation:{}:{}",
            planning_id, installation_action
        ));
    }

    action_id.map(|action_id| format!("installation:{}:{}", action_id, installation_action))
}

fn derive_node_id(node_id: Option<&str>, item: &Value) -> Option<String> {
    node_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            item.get("node_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            item.get("assigned_node")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            item.get("target_node")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .or_else(|| {
            item.get("fallback_to")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
}

fn task_id_for(project_id: &str, node_id: &str, installation_id: &str) -> String {
    format!(
        "project:{}:node:{}:installation:{}",
        project_id, node_id, installation_id
    )
}

pub fn normalize_installation_task(
    project_id: &str,
    node_id: &str,
    mut task: InstallationTask,
) -> InstallationTask {
    let now = Utc::now().to_rfc3339();

    task.project_id = project_id.to_string();
    task.node_id = node_id.to_string();

    if task.installation_id.trim().is_empty() {
        let fallback = task
            .planning_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| {
                task.action_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| "unknown".to_string());
        task.installation_id = fallback;
    }

    if task.task_id.trim().is_empty() {
        task.task_id = task_id_for(project_id, node_id, &task.installation_id);
    }

    if task.status.trim().is_empty() {
        task.status = STATUS_PLANNED.to_string();
    }

    if task
        .source
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        task.source = Some("installation_plan".to_string());
    }

    if task.created_at_utc.trim().is_empty() {
        task.created_at_utc = now.clone();
    }

    if task.updated_at_utc.trim().is_empty() {
        task.updated_at_utc = task.created_at_utc.clone();
    }

    task.safe_to_auto_execute = false;
    task
}

pub fn validate_installation_task(task: &InstallationTask) -> Result<(), String> {
    if task.project_id.trim().is_empty() {
        return Err("project_id must not be empty".to_string());
    }

    if task.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if task.task_id.trim().is_empty() {
        return Err("task_id must not be empty".to_string());
    }

    if task.installation_id.trim().is_empty() {
        return Err("installation_id must not be empty".to_string());
    }

    if !is_valid_status(task.status.trim()) {
        return Err(format!("status is invalid: {}", task.status));
    }

    if task.created_at_utc.trim().is_empty() {
        return Err("created_at_utc must not be empty".to_string());
    }

    if task.updated_at_utc.trim().is_empty() {
        return Err("updated_at_utc must not be empty".to_string());
    }

    Ok(())
}

pub fn validate_installation_task_status_transition(
    current_status: &str,
    new_status: &str,
) -> Result<(), String> {
    if !is_valid_status(current_status) {
        return Err(format!("current status is invalid: {}", current_status));
    }

    if !is_valid_status(new_status) {
        return Err(format!("new status is invalid: {}", new_status));
    }

    let allowed = matches!(
        (current_status, new_status),
        (STATUS_PLANNED, STATUS_ACKNOWLEDGED)
            | (STATUS_PLANNED, STATUS_BLOCKED)
            | (STATUS_PLANNED, STATUS_CANCELLED)
            | (STATUS_ACKNOWLEDGED, STATUS_BLOCKED)
            | (STATUS_ACKNOWLEDGED, STATUS_CANCELLED)
            | (STATUS_ACKNOWLEDGED, STATUS_COMPLETED)
            | (STATUS_BLOCKED, STATUS_ACKNOWLEDGED)
            | (STATUS_BLOCKED, STATUS_CANCELLED)
    );

    if allowed {
        Ok(())
    } else {
        Err(format!(
            "installation task status transition not allowed: {} -> {}",
            current_status, new_status
        ))
    }
}

pub fn derive_installation_tasks_from_plan(
    project_id: &str,
    node_id: Option<&str>,
    installation_plan: &Value,
) -> Vec<InstallationTask> {
    let items = installation_plan
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut tasks = Vec::new();
    let mut seen = HashSet::new();

    for item in items {
        let Some(derived_node_id) = derive_node_id(node_id, &item) else {
            continue;
        };
        let Some(installation_id) = derive_installation_id(&item) else {
            continue;
        };

        let task_id = task_id_for(project_id, &derived_node_id, &installation_id);
        if !seen.insert(task_id.clone()) {
            continue;
        }

        let requires_operator = item
            .get("requires_operator")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let task = normalize_installation_task(
            project_id,
            &derived_node_id,
            InstallationTask {
                project_id: project_id.to_string(),
                node_id: derived_node_id.clone(),
                task_id,
                installation_id,
                planning_id: item
                    .get("planning_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                action_id: item
                    .get("action_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                module_id: item
                    .get("module_id")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                runtime: item
                    .get("runtime")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                artifact: item
                    .get("artifact")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                required_step: item
                    .get("required_step")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                installation_action: item
                    .get("installation_action")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                status: STATUS_PLANNED.to_string(),
                priority: item
                    .get("priority")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                reason: item
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(ToString::to_string),
                requires_operator,
                safe_to_auto_execute: false,
                source: item
                    .get("source")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
                    .or_else(|| Some("installation_plan".to_string())),
                created_at_utc: String::new(),
                updated_at_utc: String::new(),
                source_installation_item: Some(item),
                metadata: None,
            },
        );

        tasks.push(task);
    }

    tasks
}

#[cfg(test)]
mod tests {
    use super::{
        derive_installation_tasks_from_plan, normalize_installation_task,
        validate_installation_task, validate_installation_task_status_transition, InstallationTask,
    };
    use serde_json::{json, Value};

    fn base_installation_item() -> Value {
        json!({
            "installation_id": "installation:planning:1:prepare_local_mapping",
            "planning_id": "planning:1",
            "action_id": "gap:1",
            "node_id": "fog-01",
            "module_id": "chirpstack-primary",
            "runtime": "container",
            "artifact": "chirpstack/chirpstack:latest",
            "required_step": "add_local_mapping",
            "installation_action": "prepare_local_mapping",
            "status": "planned",
            "priority": "high",
            "reason": "client lacks trusted local mapping for this action",
            "requires_operator": true,
            "safe_to_auto_execute": false,
            "source": "deployment_planning"
        })
    }

    fn base_task() -> InstallationTask {
        normalize_installation_task(
            "smart-city-campinas",
            "fog-01",
            InstallationTask {
                project_id: "smart-city-campinas".to_string(),
                node_id: "fog-01".to_string(),
                task_id: String::new(),
                installation_id: "installation:planning:1:prepare_local_mapping".to_string(),
                planning_id: Some("planning:1".to_string()),
                action_id: Some("gap:1".to_string()),
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
                source: Some("installation_plan".to_string()),
                created_at_utc: String::new(),
                updated_at_utc: String::new(),
                source_installation_item: Some(base_installation_item()),
                metadata: None,
            },
        )
    }

    #[test]
    fn derive_task_from_installation_plan_item() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [base_installation_item()] }),
        );

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].project_id, "smart-city-campinas");
        assert_eq!(tasks[0].node_id, "fog-01");
        assert_eq!(
            tasks[0].installation_action.as_deref(),
            Some("prepare_local_mapping")
        );
    }

    #[test]
    fn task_id_is_deterministic() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [base_installation_item()] }),
        );

        assert_eq!(
            tasks[0].task_id,
            "project:smart-city-campinas:node:fog-01:installation:installation:planning:1:prepare_local_mapping"
        );
    }

    #[test]
    fn duplicate_installation_items_do_not_create_duplicate_tasks() {
        let item = base_installation_item();
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [item.clone(), item] }),
        );

        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn all_derived_tasks_are_planned() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [base_installation_item()] }),
        );

        assert!(tasks.iter().all(|task| task.status == "planned"));
    }

    #[test]
    fn derived_tasks_are_operator_required_and_non_executable() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [base_installation_item()] }),
        );

        assert!(tasks.iter().all(|task| task.requires_operator));
        assert!(tasks.iter().all(|task| !task.safe_to_auto_execute));
    }

    #[test]
    fn source_installation_item_is_preserved() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [base_installation_item()] }),
        );

        assert_eq!(
            tasks[0]
                .source_installation_item
                .as_ref()
                .and_then(|value| value.get("module_id"))
                .and_then(Value::as_str),
            Some("chirpstack-primary")
        );
    }

    #[test]
    fn valid_status_transitions_pass() {
        assert!(validate_installation_task_status_transition("planned", "acknowledged").is_ok());
        assert!(validate_installation_task_status_transition("acknowledged", "completed").is_ok());
        assert!(validate_installation_task_status_transition("blocked", "acknowledged").is_ok());
    }

    #[test]
    fn invalid_transitions_from_completed_and_cancelled_fail() {
        assert!(validate_installation_task_status_transition("completed", "acknowledged").is_err());
        assert!(validate_installation_task_status_transition("cancelled", "planned").is_err());
    }

    #[test]
    fn unknown_status_fails_validation() {
        let mut task = base_task();
        task.status = "explode".to_string();

        assert!(validate_installation_task(&task).is_err());
        assert!(validate_installation_task_status_transition("planned", "explode").is_err());
    }

    #[test]
    fn missing_project_id_node_id_and_task_id_fail_validation() {
        let mut task = base_task();
        task.project_id.clear();
        assert!(validate_installation_task(&task).is_err());

        let mut task = base_task();
        task.node_id.clear();
        assert!(validate_installation_task(&task).is_err());

        let mut task = base_task();
        task.task_id.clear();
        assert!(validate_installation_task(&task).is_err());
    }

    #[test]
    fn empty_installation_plan_derives_empty_tasks() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [] }),
        );

        assert!(tasks.is_empty());
    }
}
