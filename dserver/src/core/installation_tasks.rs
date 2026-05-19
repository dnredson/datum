use std::collections::HashSet;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const STATUS_PLANNED: &str = "planned";
const STATUS_ACKNOWLEDGED: &str = "acknowledged";
const STATUS_BLOCKED: &str = "blocked";
const STATUS_CANCELLED: &str = "cancelled";
const STATUS_COMPLETED: &str = "completed";
const POLICY_VERSION: &str = "0.1.0";
const POLICY_MANUAL_ONLY: &str = "manual_only";
const POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION: &str = "candidate_for_assisted_execution";
const POLICY_BLOCKED: &str = "blocked";
const POLICY_UNKNOWN: &str = "unknown";
const APPROVAL_MODE_OPERATOR_APPROVED: &str = "operator_approved";
const READINESS_READY: &str = "ready_for_assisted_execution";
const READINESS_NOT_READY: &str = "not_ready";
const READINESS_BLOCKED: &str = "blocked";
const READINESS_UNKNOWN: &str = "unknown";
const APPROVAL_STATUS_APPROVED: &str = "approved";
const APPROVAL_STATUS_MISSING: &str = "missing";
const APPROVAL_STATUS_EXPIRED: &str = "expired";
const APPROVAL_STATUS_NOT_REQUIRED: &str = "not_required";
const APPROVAL_STATUS_INVALID: &str = "invalid";

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
    pub installation_policy: Option<InstallationPolicyDecision>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationTaskApproval {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub approval_id: String,
    pub approved: bool,
    #[serde(default)]
    pub approved_by: Option<String>,
    pub approved_at_utc: String,
    pub approval_mode: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub expires_at_utc: Option<String>,
    #[serde(default)]
    pub installation_policy_snapshot: Option<Value>,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationTaskApprovalList {
    pub project_id: String,
    pub node_id: String,
    pub count: u64,
    pub approvals: Vec<InstallationTaskApproval>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationTaskReadiness {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub readiness: String,
    pub reason: String,
    #[serde(default)]
    pub task_status: Option<String>,
    #[serde(default)]
    pub policy_decision: Option<String>,
    #[serde(default)]
    pub approval_status: Option<String>,
    #[serde(default)]
    pub approval_id: Option<String>,
    #[serde(default)]
    pub approved_at_utc: Option<String>,
    #[serde(default)]
    pub expires_at_utc: Option<String>,
    pub safe_to_auto_execute: bool,
    pub evaluated_at_utc: String,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    #[serde(default)]
    pub approval_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationTaskReadinessList {
    pub project_id: String,
    pub node_id: String,
    pub count: u64,
    pub readiness: Vec<InstallationTaskReadiness>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstallationPolicyDecision {
    pub policy_version: String,
    pub decision: String,
    pub reason: String,
    pub requires_operator: bool,
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub allowed_future_modes: Vec<String>,
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

pub fn annotate_installation_task_policy(task: &mut InstallationTask) {
    task.safe_to_auto_execute = false;

    let terminal_status = matches!(task.status.trim(), STATUS_COMPLETED | STATUS_CANCELLED);
    let policy = if terminal_status {
        InstallationPolicyDecision {
            policy_version: POLICY_VERSION.to_string(),
            decision: POLICY_BLOCKED.to_string(),
            reason: "terminal task status".to_string(),
            requires_operator: task.requires_operator,
            safe_to_auto_execute: false,
            allowed_future_modes: Vec::new(),
        }
    } else {
        match task
            .required_step
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
        {
            "add_local_mapping" => InstallationPolicyDecision {
                policy_version: POLICY_VERSION.to_string(),
                decision: POLICY_MANUAL_ONLY.to_string(),
                reason: "local trusted mapping requires operator confirmation".to_string(),
                requires_operator: task.requires_operator,
                safe_to_auto_execute: false,
                allowed_future_modes: Vec::new(),
            },
            "install_existing_artifact" => InstallationPolicyDecision {
                policy_version: POLICY_VERSION.to_string(),
                decision: POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION.to_string(),
                reason: "artifact installation may be assisted in a future milestone, but is not executable yet".to_string(),
                requires_operator: task.requires_operator,
                safe_to_auto_execute: false,
                allowed_future_modes: vec!["operator_approved".to_string()],
            },
            "enable_runtime" => InstallationPolicyDecision {
                policy_version: POLICY_VERSION.to_string(),
                decision: POLICY_MANUAL_ONLY.to_string(),
                reason: "enabling local runtime requires operator/system administration".to_string(),
                requires_operator: task.requires_operator,
                safe_to_auto_execute: false,
                allowed_future_modes: Vec::new(),
            },
            "investigate_failure" => InstallationPolicyDecision {
                policy_version: POLICY_VERSION.to_string(),
                decision: POLICY_MANUAL_ONLY.to_string(),
                reason: "previous failure requires investigation".to_string(),
                requires_operator: task.requires_operator,
                safe_to_auto_execute: false,
                allowed_future_modes: Vec::new(),
            },
            "define_installation_strategy" => InstallationPolicyDecision {
                policy_version: POLICY_VERSION.to_string(),
                decision: POLICY_MANUAL_ONLY.to_string(),
                reason: "installation strategy is not defined".to_string(),
                requires_operator: task.requires_operator,
                safe_to_auto_execute: false,
                allowed_future_modes: Vec::new(),
            },
            _ => InstallationPolicyDecision {
                policy_version: POLICY_VERSION.to_string(),
                decision: POLICY_UNKNOWN.to_string(),
                reason: "unsupported required step".to_string(),
                requires_operator: task.requires_operator,
                safe_to_auto_execute: false,
                allowed_future_modes: Vec::new(),
            },
        }
    };

    task.installation_policy = Some(policy);
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
    annotate_installation_task_policy(&mut task);
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

pub fn validate_installation_task_can_be_approved(task: &InstallationTask) -> Result<(), String> {
    if task.task_id.trim().is_empty() {
        return Err("task_id must not be empty".to_string());
    }

    if task.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if matches!(task.status.trim(), STATUS_COMPLETED | STATUS_CANCELLED) {
        return Err(format!(
            "installation task cannot be approved in terminal status '{}'",
            task.status
        ));
    }

    if task.safe_to_auto_execute {
        return Err("task.safe_to_auto_execute must remain false".to_string());
    }

    let policy = task
        .installation_policy
        .as_ref()
        .ok_or_else(|| "installation_policy is required for approval".to_string())?;

    if policy.decision != POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION {
        return Err(format!(
            "installation task approval requires installation_policy.decision='{}', got '{}'",
            POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION, policy.decision
        ));
    }

    if policy.safe_to_auto_execute {
        return Err("installation_policy.safe_to_auto_execute must remain false".to_string());
    }

    if !policy
        .allowed_future_modes
        .iter()
        .any(|mode| mode == APPROVAL_MODE_OPERATOR_APPROVED)
    {
        return Err(format!(
            "installation task approval requires installation_policy.allowed_future_modes to include '{}'",
            APPROVAL_MODE_OPERATOR_APPROVED
        ));
    }

    Ok(())
}

pub fn build_installation_task_approval(
    project_id: &str,
    node_id: &str,
    task: &InstallationTask,
    approved_by: Option<String>,
    reason: Option<String>,
    expires_at_utc: Option<String>,
) -> InstallationTaskApproval {
    InstallationTaskApproval {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        task_id: task.task_id.clone(),
        approval_id: format!("approval:{}", Uuid::new_v4()),
        approved: true,
        approved_by,
        approved_at_utc: Utc::now().to_rfc3339(),
        approval_mode: APPROVAL_MODE_OPERATOR_APPROVED.to_string(),
        reason,
        expires_at_utc,
        installation_policy_snapshot: task
            .installation_policy
            .as_ref()
            .and_then(|policy| serde_json::to_value(policy).ok()),
        task_snapshot: serde_json::to_value(task).ok(),
        metadata: None,
    }
}

pub fn evaluate_installation_task_readiness(
    project_id: &str,
    node_id: &str,
    task: &InstallationTask,
    latest_approval: Option<&InstallationTaskApproval>,
) -> InstallationTaskReadiness {
    let mut readiness = InstallationTaskReadiness {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        task_id: task.task_id.clone(),
        readiness: READINESS_UNKNOWN.to_string(),
        reason: "readiness not evaluated".to_string(),
        task_status: Some(task.status.clone()),
        policy_decision: task
            .installation_policy
            .as_ref()
            .map(|policy| policy.decision.clone()),
        approval_status: None,
        approval_id: latest_approval.map(|approval| approval.approval_id.clone()),
        approved_at_utc: latest_approval.map(|approval| approval.approved_at_utc.clone()),
        expires_at_utc: latest_approval.and_then(|approval| approval.expires_at_utc.clone()),
        safe_to_auto_execute: false,
        evaluated_at_utc: Utc::now().to_rfc3339(),
        task_snapshot: serde_json::to_value(task).ok(),
        approval_snapshot: latest_approval.and_then(|approval| serde_json::to_value(approval).ok()),
        metadata: None,
    };

    let status = task.status.trim();
    if matches!(status, STATUS_COMPLETED | STATUS_CANCELLED) {
        readiness.readiness = READINESS_BLOCKED.to_string();
        readiness.reason = "terminal task status".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_NOT_REQUIRED.to_string());
        return readiness;
    }

    if !matches!(status, STATUS_PLANNED | STATUS_ACKNOWLEDGED) {
        readiness.readiness = READINESS_NOT_READY.to_string();
        readiness.reason = "task status is not eligible for assisted execution".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_NOT_REQUIRED.to_string());
        return readiness;
    }

    let Some(policy) = task.installation_policy.as_ref() else {
        readiness.readiness = READINESS_UNKNOWN.to_string();
        readiness.reason = "installation policy missing".to_string();
        return readiness;
    };

    if policy.safe_to_auto_execute {
        readiness.readiness = READINESS_UNKNOWN.to_string();
        readiness.reason = "installation policy safe_to_auto_execute must remain false".to_string();
        return readiness;
    }

    if policy.decision != POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION {
        readiness.readiness = if policy.decision == POLICY_BLOCKED {
            READINESS_BLOCKED.to_string()
        } else {
            READINESS_NOT_READY.to_string()
        };
        readiness.reason = format!(
            "task policy is not {}",
            POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION
        );
        readiness.approval_status = Some(APPROVAL_STATUS_NOT_REQUIRED.to_string());
        return readiness;
    }

    if !policy
        .allowed_future_modes
        .iter()
        .any(|mode| mode == APPROVAL_MODE_OPERATOR_APPROVED)
    {
        readiness.readiness = READINESS_NOT_READY.to_string();
        readiness.reason = "operator_approved mode is not allowed by policy".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_NOT_REQUIRED.to_string());
        return readiness;
    }

    let Some(approval) = latest_approval else {
        readiness.readiness = READINESS_NOT_READY.to_string();
        readiness.reason = "operator approval is required".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_MISSING.to_string());
        return readiness;
    };

    if approval.project_id != project_id
        || approval.node_id != node_id
        || approval.task_id != task.task_id
    {
        readiness.readiness = READINESS_NOT_READY.to_string();
        readiness.reason = "latest approval metadata does not match task identity".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_INVALID.to_string());
        return readiness;
    }

    if !approval.approved {
        readiness.readiness = READINESS_NOT_READY.to_string();
        readiness.reason = "latest approval is not approved".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_INVALID.to_string());
        return readiness;
    }

    if approval.approval_mode.trim() != APPROVAL_MODE_OPERATOR_APPROVED {
        readiness.readiness = READINESS_NOT_READY.to_string();
        readiness.reason = "latest approval mode is not operator_approved".to_string();
        readiness.approval_status = Some(APPROVAL_STATUS_INVALID.to_string());
        return readiness;
    }

    if let Some(expires_at_utc) = approval
        .expires_at_utc
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        match chrono::DateTime::parse_from_rfc3339(expires_at_utc) {
            Ok(expiry) if expiry.with_timezone(&Utc) < Utc::now() => {
                readiness.readiness = READINESS_NOT_READY.to_string();
                readiness.reason = "operator approval has expired".to_string();
                readiness.approval_status = Some(APPROVAL_STATUS_EXPIRED.to_string());
                return readiness;
            }
            Ok(_) => {}
            Err(_) => {
                readiness.readiness = READINESS_NOT_READY.to_string();
                readiness.reason = "approval expiry timestamp is invalid".to_string();
                readiness.approval_status = Some(APPROVAL_STATUS_INVALID.to_string());
                return readiness;
            }
        }
    }

    readiness.readiness = READINESS_READY.to_string();
    readiness.reason = "task has candidate policy and operator approval metadata".to_string();
    readiness.approval_status = Some(APPROVAL_STATUS_APPROVED.to_string());
    readiness
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
                installation_policy: None,
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
        annotate_installation_task_policy, build_installation_task_approval,
        derive_installation_tasks_from_plan, evaluate_installation_task_readiness,
        normalize_installation_task, validate_installation_task,
        validate_installation_task_can_be_approved, validate_installation_task_status_transition,
        InstallationTask,
    };
    use chrono::{Duration, Utc};
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
                installation_policy: None,
                metadata: None,
            },
        )
    }

    fn candidate_task() -> InstallationTask {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        annotate_installation_task_policy(&mut task);
        task
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

    #[test]
    fn add_local_mapping_policy_is_manual_only() {
        let task = base_task();
        let policy = task.installation_policy.expect("policy should exist");

        assert_eq!(policy.decision, "manual_only");
        assert_eq!(
            policy.reason,
            "local trusted mapping requires operator confirmation"
        );
    }

    #[test]
    fn install_existing_artifact_policy_is_candidate_for_assisted_execution() {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "candidate_for_assisted_execution");
        assert_eq!(
            policy.reason,
            "artifact installation may be assisted in a future milestone, but is not executable yet"
        );
        assert_eq!(policy.allowed_future_modes, vec!["operator_approved"]);
    }

    #[test]
    fn enable_runtime_policy_is_manual_only() {
        let mut task = base_task();
        task.required_step = Some("enable_runtime".to_string());
        annotate_installation_task_policy(&mut task);

        assert_eq!(
            task.installation_policy
                .expect("policy should exist")
                .decision,
            "manual_only"
        );
    }

    #[test]
    fn investigate_failure_policy_is_manual_only() {
        let mut task = base_task();
        task.required_step = Some("investigate_failure".to_string());
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "manual_only");
        assert_eq!(policy.reason, "previous failure requires investigation");
    }

    #[test]
    fn define_installation_strategy_policy_is_manual_only() {
        let mut task = base_task();
        task.required_step = Some("define_installation_strategy".to_string());
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "manual_only");
        assert_eq!(policy.reason, "installation strategy is not defined");
    }

    #[test]
    fn unknown_required_step_policy_is_unknown() {
        let mut task = base_task();
        task.required_step = Some("mystery".to_string());
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "unknown");
        assert_eq!(policy.reason, "unsupported required step");
    }

    #[test]
    fn completed_task_policy_is_blocked() {
        let mut task = base_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "blocked");
        assert_eq!(policy.reason, "terminal task status");
    }

    #[test]
    fn cancelled_task_policy_is_blocked() {
        let mut task = base_task();
        task.status = "cancelled".to_string();
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "blocked");
        assert_eq!(policy.reason, "terminal task status");
    }

    #[test]
    fn safe_to_auto_execute_remains_false_for_all_policy_decisions() {
        let mut task = base_task();
        for required_step in [
            "add_local_mapping",
            "install_existing_artifact",
            "enable_runtime",
            "investigate_failure",
            "define_installation_strategy",
            "unknown",
        ] {
            task.required_step = Some(required_step.to_string());
            task.safe_to_auto_execute = true;
            annotate_installation_task_policy(&mut task);

            assert!(!task.safe_to_auto_execute);
            assert_eq!(
                task.installation_policy
                    .as_ref()
                    .expect("policy should exist")
                    .safe_to_auto_execute,
                false
            );
        }
    }

    #[test]
    fn materialized_tasks_include_installation_policy() {
        let tasks = derive_installation_tasks_from_plan(
            "smart-city-campinas",
            None,
            &json!({ "items": [base_installation_item()] }),
        );

        assert!(tasks[0].installation_policy.is_some());
    }

    #[test]
    fn old_task_without_policy_can_be_normalized_without_failing() {
        let mut task = base_task();
        task.installation_policy = None;

        let normalized = normalize_installation_task("smart-city-campinas", "fog-01", task);
        assert!(normalized.installation_policy.is_some());
    }

    #[test]
    fn policy_is_recomputed_after_status_change() {
        let mut task = base_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);

        let policy = task.installation_policy.expect("policy should exist");
        assert_eq!(policy.decision, "blocked");
        assert_eq!(policy.reason, "terminal task status");
    }

    #[test]
    fn candidate_with_operator_approved_can_be_approved() {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        annotate_installation_task_policy(&mut task);

        assert!(validate_installation_task_can_be_approved(&task).is_ok());
    }

    #[test]
    fn manual_only_task_cannot_be_approved() {
        let task = base_task();
        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn blocked_task_cannot_be_approved() {
        let mut task = base_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);

        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn unknown_task_cannot_be_approved() {
        let mut task = base_task();
        task.required_step = Some("mystery".to_string());
        annotate_installation_task_policy(&mut task);

        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn completed_task_cannot_be_approved() {
        let mut task = base_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);

        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn cancelled_task_cannot_be_approved() {
        let mut task = base_task();
        task.status = "cancelled".to_string();
        annotate_installation_task_policy(&mut task);

        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn missing_installation_policy_cannot_be_approved() {
        let mut task = base_task();
        task.installation_policy = None;

        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn missing_operator_approved_mode_cannot_be_approved() {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        annotate_installation_task_policy(&mut task);
        task.installation_policy
            .as_mut()
            .expect("policy should exist")
            .allowed_future_modes = vec!["future_other_mode".to_string()];

        assert!(validate_installation_task_can_be_approved(&task).is_err());
    }

    #[test]
    fn approval_builder_includes_task_and_policy_snapshots() {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        annotate_installation_task_policy(&mut task);

        let approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some("operator".to_string()),
            Some("approved for future assisted execution".to_string()),
            None,
        );

        assert!(approval.task_snapshot.is_some());
        assert!(approval.installation_policy_snapshot.is_some());
        assert_eq!(approval.approval_mode, "operator_approved");
        assert!(approval.approved);
    }

    #[test]
    fn approval_keeps_task_and_policy_safe_to_auto_execute_false() {
        let mut task = base_task();
        task.required_step = Some("install_existing_artifact".to_string());
        annotate_installation_task_policy(&mut task);

        let approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            None,
            None,
            None,
        );

        assert!(!task.safe_to_auto_execute);
        assert_eq!(
            task.installation_policy
                .as_ref()
                .expect("policy should exist")
                .safe_to_auto_execute,
            false
        );
        assert_eq!(
            approval
                .installation_policy_snapshot
                .as_ref()
                .and_then(|value| value.get("safe_to_auto_execute"))
                .and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn candidate_task_with_valid_approval_is_ready() {
        let task = candidate_task();
        let approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some("operator".to_string()),
            Some("approved".to_string()),
            Some((Utc::now() + Duration::hours(1)).to_rfc3339()),
        );

        let readiness = evaluate_installation_task_readiness(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some(&approval),
        );

        assert_eq!(readiness.readiness, "ready_for_assisted_execution");
        assert_eq!(readiness.approval_status.as_deref(), Some("approved"));
        assert_eq!(
            readiness.approval_id.as_deref(),
            Some(approval.approval_id.as_str())
        );
        assert!(!readiness.safe_to_auto_execute);
        assert!(readiness.task_snapshot.is_some());
        assert!(readiness.approval_snapshot.is_some());
    }

    #[test]
    fn candidate_task_with_missing_approval_is_not_ready() {
        let task = candidate_task();

        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert_eq!(readiness.readiness, "not_ready");
        assert_eq!(readiness.approval_status.as_deref(), Some("missing"));
    }

    #[test]
    fn manual_only_task_with_approval_is_not_ready() {
        let task = base_task();
        let approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &candidate_task(),
            Some("operator".to_string()),
            None,
            None,
        );

        let readiness = evaluate_installation_task_readiness(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some(&approval),
        );

        assert_eq!(readiness.readiness, "not_ready");
        assert_eq!(readiness.approval_status.as_deref(), Some("not_required"));
    }

    #[test]
    fn completed_task_is_blocked() {
        let mut task = candidate_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);

        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert_eq!(readiness.readiness, "blocked");
        assert_eq!(readiness.reason, "terminal task status");
    }

    #[test]
    fn cancelled_task_is_blocked() {
        let mut task = candidate_task();
        task.status = "cancelled".to_string();
        annotate_installation_task_policy(&mut task);

        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert_eq!(readiness.readiness, "blocked");
        assert_eq!(readiness.reason, "terminal task status");
    }

    #[test]
    fn missing_policy_is_unknown() {
        let mut task = candidate_task();
        task.installation_policy = None;

        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert_eq!(readiness.readiness, "unknown");
        assert_eq!(readiness.reason, "installation policy missing");
    }

    #[test]
    fn approval_with_wrong_task_id_is_invalid() {
        let task = candidate_task();
        let mut approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some("operator".to_string()),
            None,
            None,
        );
        approval.task_id = "task:other".to_string();

        let readiness = evaluate_installation_task_readiness(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some(&approval),
        );

        assert_eq!(readiness.readiness, "not_ready");
        assert_eq!(readiness.approval_status.as_deref(), Some("invalid"));
    }

    #[test]
    fn approval_false_is_invalid() {
        let task = candidate_task();
        let mut approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some("operator".to_string()),
            None,
            None,
        );
        approval.approved = false;

        let readiness = evaluate_installation_task_readiness(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some(&approval),
        );

        assert_eq!(readiness.readiness, "not_ready");
        assert_eq!(readiness.approval_status.as_deref(), Some("invalid"));
    }

    #[test]
    fn readiness_safe_to_auto_execute_remains_false_when_ready() {
        let task = candidate_task();
        let approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some("operator".to_string()),
            None,
            None,
        );

        let readiness = evaluate_installation_task_readiness(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some(&approval),
        );

        assert!(!readiness.safe_to_auto_execute);
    }
}
