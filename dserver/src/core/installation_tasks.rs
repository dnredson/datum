use std::collections::HashSet;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
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
const REQUEST_STATUS_REQUESTED: &str = "requested";
const EXECUTION_MODE_ASSISTED_OPERATOR_APPROVED: &str = "assisted_operator_approved";
const CLAIM_STATUS_CLAIMED: &str = "claimed";
const CLAIM_STATUS_RELEASED: &str = "released";
const CLAIM_STATUS_EXPIRED: &str = "expired";
const CLAIM_STATUS_COMPLETED: &str = "completed";
const CLAIM_STATUS_CANCELLED: &str = "cancelled";
const ATTEMPT_STATUS_RECORDED: &str = "recorded";
const ATTEMPT_EXECUTION_MODE_NO_OP: &str = "no_op";
const ATTEMPT_EXECUTION_MODE_DRY_RUN: &str = "dry_run";
const ATTEMPT_EXECUTION_RESULT_NO_OP_RECORDED: &str = "no_op_recorded";
const ATTEMPT_EXECUTION_RESULT_DRY_RUN_RECORDED: &str = "dry_run_recorded";
const ATTEMPT_EXECUTION_RESULT_REJECTED_INVALID_REQUEST: &str = "rejected_invalid_request";

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
pub struct AssistedExecutionRequest {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub request_id: String,
    pub request_status: String,
    #[serde(default)]
    pub requested_by: Option<String>,
    pub requested_at_utc: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub readiness_snapshot: Option<Value>,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    #[serde(default)]
    pub approval_snapshot: Option<Value>,
    pub safe_to_auto_execute: bool,
    pub execution_mode: String,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionRequestList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub requests: Vec<AssistedExecutionRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionClaim {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub request_id: String,
    pub claim_id: String,
    pub claim_status: String,
    #[serde(default)]
    pub claimed_by: Option<String>,
    pub claimed_at_utc: String,
    #[serde(default)]
    pub lease_expires_at_utc: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub request_snapshot: Option<Value>,
    #[serde(default)]
    pub readiness_snapshot: Option<Value>,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    pub safe_to_auto_execute: bool,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionClaimList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub claims: Vec<AssistedExecutionClaim>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionAttempt {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub request_id: String,
    pub claim_id: String,
    pub attempt_id: String,
    pub attempt_status: String,
    #[serde(default)]
    pub attempted_by: Option<String>,
    pub attempted_at_utc: String,
    pub execution_mode: String,
    pub execution_result: String,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub claim_snapshot: Option<Value>,
    #[serde(default)]
    pub request_snapshot: Option<Value>,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionAttemptList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub attempts: Vec<AssistedExecutionAttempt>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionStateSummary {
    pub project_id: String,
    pub node_id: String,
    pub task_id: String,
    pub state: String,
    pub reason: String,
    #[serde(default)]
    pub task_status: Option<String>,
    #[serde(default)]
    pub policy_decision: Option<String>,
    #[serde(default)]
    pub approval_status: Option<String>,
    #[serde(default)]
    pub readiness: Option<String>,
    #[serde(default)]
    pub request_status: Option<String>,
    #[serde(default)]
    pub claim_status: Option<String>,
    #[serde(default)]
    pub latest_attempt_status: Option<String>,
    #[serde(default)]
    pub latest_execution_mode: Option<String>,
    #[serde(default)]
    pub latest_execution_result: Option<String>,
    pub safe_to_auto_execute: bool,
    pub side_effects_performed: bool,
    pub evaluated_at_utc: String,
    #[serde(default)]
    pub task_snapshot: Option<Value>,
    #[serde(default)]
    pub approval_snapshot: Option<Value>,
    #[serde(default)]
    pub readiness_snapshot: Option<Value>,
    #[serde(default)]
    pub request_snapshot: Option<Value>,
    #[serde(default)]
    pub claim_snapshot: Option<Value>,
    #[serde(default)]
    pub latest_attempt_snapshot: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionStateSummaryList {
    pub project_id: String,
    #[serde(default)]
    pub node_id: Option<String>,
    pub count: u64,
    pub summaries: Vec<AssistedExecutionStateSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionClaimStatusPatch {
    pub status: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AssistedExecutionAttemptBody {
    pub execution_mode: String,
    #[serde(default)]
    pub attempted_by: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
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

fn json_snapshot<T: Serialize>(value: Option<&T>) -> Option<Value> {
    value.and_then(|value| serde_json::to_value(value).ok())
}

fn is_valid_status(status: &str) -> bool {
    matches!(
        status,
        STATUS_PLANNED | STATUS_ACKNOWLEDGED | STATUS_BLOCKED | STATUS_CANCELLED | STATUS_COMPLETED
    )
}

fn is_valid_assisted_execution_claim_status(status: &str) -> bool {
    matches!(
        status,
        CLAIM_STATUS_CLAIMED
            | CLAIM_STATUS_RELEASED
            | CLAIM_STATUS_EXPIRED
            | CLAIM_STATUS_COMPLETED
            | CLAIM_STATUS_CANCELLED
    )
}

fn is_valid_assisted_execution_attempt_mode(mode: &str) -> bool {
    matches!(
        mode,
        ATTEMPT_EXECUTION_MODE_NO_OP | ATTEMPT_EXECUTION_MODE_DRY_RUN
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

pub fn derive_assisted_execution_state_summary(
    project_id: &str,
    node_id: &str,
    task: Option<&InstallationTask>,
    approval: Option<&InstallationTaskApproval>,
    readiness: Option<&InstallationTaskReadiness>,
    request: Option<&AssistedExecutionRequest>,
    claim: Option<&AssistedExecutionClaim>,
    latest_attempt: Option<&AssistedExecutionAttempt>,
) -> AssistedExecutionStateSummary {
    let evaluated_at_utc = Utc::now().to_rfc3339();
    let task_id = task
        .map(|task| task.task_id.clone())
        .or_else(|| request.map(|request| request.task_id.clone()))
        .or_else(|| claim.map(|claim| claim.task_id.clone()))
        .or_else(|| latest_attempt.map(|attempt| attempt.task_id.clone()))
        .unwrap_or_default();
    let task_status = task.map(|task| task.status.clone());
    let policy_decision = task
        .and_then(|task| task.installation_policy.as_ref())
        .map(|policy| policy.decision.clone());
    let approval_status = readiness
        .and_then(|readiness| readiness.approval_status.clone())
        .or_else(|| {
            approval.map(|approval| {
                if approval.approved {
                    APPROVAL_STATUS_APPROVED.to_string()
                } else {
                    APPROVAL_STATUS_INVALID.to_string()
                }
            })
        });
    let readiness_status = readiness.map(|readiness| readiness.readiness.clone());
    let request_status = request.map(|request| request.request_status.clone());
    let claim_status = claim.map(|claim| claim.claim_status.clone());
    let latest_attempt_status = latest_attempt.map(|attempt| attempt.attempt_status.clone());
    let latest_execution_mode = latest_attempt.map(|attempt| attempt.execution_mode.clone());
    let latest_execution_result = latest_attempt.map(|attempt| attempt.execution_result.clone());
    let side_effects_performed = latest_attempt
        .map(|attempt| attempt.side_effects_performed)
        .unwrap_or(false);
    let metadata = latest_attempt
        .and_then(|attempt| attempt.metadata.clone())
        .or_else(|| claim.and_then(|claim| claim.metadata.clone()))
        .or_else(|| request.and_then(|request| request.metadata.clone()))
        .or_else(|| readiness.and_then(|readiness| readiness.metadata.clone()))
        .or_else(|| approval.and_then(|approval| approval.metadata.clone()))
        .or_else(|| task.and_then(|task| task.metadata.clone()));

    let (state, reason) = if task.is_none() {
        (
            "task_missing".to_string(),
            "installation task is missing".to_string(),
        )
    } else if let Some(attempt) = latest_attempt {
        match attempt.execution_mode.as_str() {
            ATTEMPT_EXECUTION_MODE_NO_OP => (
                "attempt_recorded_no_op".to_string(),
                "latest assisted execution attempt recorded in no_op mode".to_string(),
            ),
            ATTEMPT_EXECUTION_MODE_DRY_RUN => (
                "attempt_recorded_dry_run".to_string(),
                "latest assisted execution attempt recorded in dry_run mode".to_string(),
            ),
            _ => (
                "unknown".to_string(),
                format!(
                    "latest assisted execution attempt recorded with unknown mode '{}'",
                    attempt.execution_mode
                ),
            ),
        }
    } else if let Some(claim) = claim {
        match claim.claim_status.as_str() {
            CLAIM_STATUS_CLAIMED => (
                "claimed".to_string(),
                "assisted execution claim is active".to_string(),
            ),
            CLAIM_STATUS_RELEASED => (
                "claim_released".to_string(),
                "assisted execution claim was released".to_string(),
            ),
            CLAIM_STATUS_COMPLETED | CLAIM_STATUS_CANCELLED | CLAIM_STATUS_EXPIRED => (
                "claim_terminal".to_string(),
                format!(
                    "assisted execution claim is in terminal status '{}'",
                    claim.claim_status
                ),
            ),
            _ => (
                "unknown".to_string(),
                format!(
                    "assisted execution claim has unknown status '{}'",
                    claim.claim_status
                ),
            ),
        }
    } else if request.is_some() {
        (
            "awaiting_claim".to_string(),
            "assisted execution request is recorded and awaiting claim".to_string(),
        )
    } else if let Some(task) = task {
        if matches!(
            task.status.trim(),
            STATUS_COMPLETED | STATUS_CANCELLED | STATUS_BLOCKED
        ) {
            (
                "blocked".to_string(),
                format!("task status '{}' is not eligible", task.status),
            )
        } else {
            match task
                .installation_policy
                .as_ref()
                .map(|policy| policy.decision.as_str())
            {
                Some(POLICY_MANUAL_ONLY) => (
                    "policy_manual_only".to_string(),
                    "installation policy requires manual handling".to_string(),
                ),
                Some(POLICY_UNKNOWN) | Some(POLICY_BLOCKED) | None => (
                    "policy_unknown".to_string(),
                    "installation policy is missing, blocked, or not eligible for assisted execution"
                        .to_string(),
                ),
                Some(POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION) => {
                    if let Some(readiness) = readiness {
                        if readiness.readiness == READINESS_READY {
                            (
                                "awaiting_request".to_string(),
                                "task is ready for assisted execution and awaiting request"
                                    .to_string(),
                            )
                        } else if readiness.approval_status.as_deref()
                            == Some(APPROVAL_STATUS_MISSING)
                        {
                            (
                                "awaiting_approval".to_string(),
                                "candidate task is awaiting operator approval".to_string(),
                            )
                        } else {
                            (
                                "awaiting_readiness".to_string(),
                                format!("task readiness is '{}'", readiness.readiness),
                            )
                        }
                    } else if approval.is_some() {
                        (
                            "awaiting_readiness".to_string(),
                            "operator approval exists but readiness has not been evaluated"
                                .to_string(),
                        )
                    } else {
                        (
                            "awaiting_approval".to_string(),
                            "candidate task is awaiting operator approval".to_string(),
                        )
                    }
                }
                Some(other) => (
                    "unknown".to_string(),
                    format!("installation policy decision '{}' is not recognized", other),
                ),
            }
        }
    } else {
        (
            "unknown".to_string(),
            "assisted execution state is unknown".to_string(),
        )
    };

    AssistedExecutionStateSummary {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        task_id,
        state,
        reason,
        task_status,
        policy_decision,
        approval_status,
        readiness: readiness_status,
        request_status,
        claim_status,
        latest_attempt_status,
        latest_execution_mode,
        latest_execution_result,
        safe_to_auto_execute: false,
        side_effects_performed,
        evaluated_at_utc,
        task_snapshot: json_snapshot(task),
        approval_snapshot: json_snapshot(approval),
        readiness_snapshot: json_snapshot(readiness),
        request_snapshot: json_snapshot(request),
        claim_snapshot: json_snapshot(claim),
        latest_attempt_snapshot: json_snapshot(latest_attempt),
        metadata,
    }
}

pub fn validate_assisted_execution_request_allowed(
    task: &InstallationTask,
    readiness: &InstallationTaskReadiness,
    latest_approval: Option<&InstallationTaskApproval>,
) -> Result<(), String> {
    if task.task_id.trim().is_empty() {
        return Err("task_id must not be empty".to_string());
    }

    if task.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if matches!(task.status.trim(), STATUS_COMPLETED | STATUS_CANCELLED) {
        return Err(format!(
            "assisted execution request is not allowed in terminal task status '{}'",
            task.status
        ));
    }

    if !matches!(task.status.trim(), STATUS_PLANNED | STATUS_ACKNOWLEDGED) {
        return Err("task status is not eligible for assisted execution request".to_string());
    }

    if task.safe_to_auto_execute {
        return Err("task.safe_to_auto_execute must remain false".to_string());
    }

    let policy = task.installation_policy.as_ref().ok_or_else(|| {
        "installation_policy is required for assisted execution request".to_string()
    })?;

    if policy.decision != POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION {
        return Err(format!(
            "assisted execution request requires installation_policy.decision='{}', got '{}'",
            POLICY_CANDIDATE_FOR_ASSISTED_EXECUTION, policy.decision
        ));
    }

    if policy.safe_to_auto_execute {
        return Err("installation_policy.safe_to_auto_execute must remain false".to_string());
    }

    if readiness.readiness != READINESS_READY {
        return Err(format!(
            "assisted execution request requires readiness='{}', got '{}'",
            READINESS_READY, readiness.readiness
        ));
    }

    if readiness.safe_to_auto_execute {
        return Err("readiness.safe_to_auto_execute must remain false".to_string());
    }

    let approval = latest_approval.ok_or_else(|| {
        "latest approved operator approval is required for assisted execution request".to_string()
    })?;

    if !approval.approved {
        return Err("latest approval must be approved".to_string());
    }

    if approval.project_id != task.project_id
        || approval.node_id != task.node_id
        || approval.task_id != task.task_id
    {
        return Err("latest approval metadata does not match task identity".to_string());
    }

    if approval.approval_mode != APPROVAL_MODE_OPERATOR_APPROVED {
        return Err(format!(
            "latest approval must use approval_mode='{}'",
            APPROVAL_MODE_OPERATOR_APPROVED
        ));
    }

    Ok(())
}

pub fn build_assisted_execution_request(
    project_id: &str,
    node_id: &str,
    task: &InstallationTask,
    readiness: &InstallationTaskReadiness,
    latest_approval: Option<&InstallationTaskApproval>,
    requested_by: Option<String>,
    reason: Option<String>,
) -> AssistedExecutionRequest {
    AssistedExecutionRequest {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        task_id: task.task_id.clone(),
        request_id: format!("request:{}", Uuid::new_v4()),
        request_status: REQUEST_STATUS_REQUESTED.to_string(),
        requested_by,
        requested_at_utc: Utc::now().to_rfc3339(),
        reason,
        readiness_snapshot: serde_json::to_value(readiness).ok(),
        task_snapshot: serde_json::to_value(task).ok(),
        approval_snapshot: latest_approval.and_then(|approval| serde_json::to_value(approval).ok()),
        safe_to_auto_execute: false,
        execution_mode: EXECUTION_MODE_ASSISTED_OPERATOR_APPROVED.to_string(),
        metadata: None,
    }
}

pub fn validate_assisted_execution_claim_allowed(
    task: &InstallationTask,
    request: &AssistedExecutionRequest,
    latest_existing_claim: Option<&AssistedExecutionClaim>,
) -> Result<(), String> {
    if task.task_id.trim().is_empty() {
        return Err("task_id must not be empty".to_string());
    }

    if task.node_id.trim().is_empty() {
        return Err("node_id must not be empty".to_string());
    }

    if request.request_status != REQUEST_STATUS_REQUESTED {
        return Err(format!(
            "assisted execution claim requires request_status='{}', got '{}'",
            REQUEST_STATUS_REQUESTED, request.request_status
        ));
    }

    if request.safe_to_auto_execute {
        return Err("request.safe_to_auto_execute must remain false".to_string());
    }

    if !matches!(task.status.trim(), STATUS_PLANNED | STATUS_ACKNOWLEDGED) {
        return Err("task status is not eligible for assisted execution claim".to_string());
    }

    if task.safe_to_auto_execute {
        return Err("task.safe_to_auto_execute must remain false".to_string());
    }

    if request.project_id != task.project_id
        || request.node_id != task.node_id
        || request.task_id != task.task_id
    {
        return Err("assisted execution request metadata does not match task identity".to_string());
    }

    if let Some(claim) = latest_existing_claim {
        match claim.claim_status.as_str() {
            CLAIM_STATUS_CLAIMED => {
                return Err("assisted execution request is already claimed".to_string());
            }
            CLAIM_STATUS_COMPLETED => {
                return Err("assisted execution request already completed".to_string());
            }
            CLAIM_STATUS_RELEASED | CLAIM_STATUS_CANCELLED | CLAIM_STATUS_EXPIRED => {}
            _ => {}
        }
    }

    Ok(())
}

pub fn build_assisted_execution_claim(
    project_id: &str,
    node_id: &str,
    task: &InstallationTask,
    request: &AssistedExecutionRequest,
    claimed_by: Option<String>,
    reason: Option<String>,
    lease_expires_at_utc: Option<String>,
) -> AssistedExecutionClaim {
    AssistedExecutionClaim {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        task_id: task.task_id.clone(),
        request_id: request.request_id.clone(),
        claim_id: format!("claim:{}", Uuid::new_v4()),
        claim_status: CLAIM_STATUS_CLAIMED.to_string(),
        claimed_by,
        claimed_at_utc: Utc::now().to_rfc3339(),
        lease_expires_at_utc,
        reason,
        request_snapshot: serde_json::to_value(request).ok(),
        readiness_snapshot: request.readiness_snapshot.clone(),
        task_snapshot: serde_json::to_value(task).ok(),
        safe_to_auto_execute: false,
        metadata: None,
    }
}

pub fn validate_assisted_execution_claim_status_transition(
    current_status: &str,
    new_status: &str,
) -> Result<(), String> {
    if !is_valid_assisted_execution_claim_status(current_status) {
        return Err(format!(
            "current claim status is invalid: {}",
            current_status
        ));
    }

    if !is_valid_assisted_execution_claim_status(new_status) {
        return Err(format!("new claim status is invalid: {}", new_status));
    }

    let allowed = matches!(
        (current_status, new_status),
        (CLAIM_STATUS_CLAIMED, CLAIM_STATUS_RELEASED)
            | (CLAIM_STATUS_CLAIMED, CLAIM_STATUS_EXPIRED)
            | (CLAIM_STATUS_CLAIMED, CLAIM_STATUS_COMPLETED)
            | (CLAIM_STATUS_CLAIMED, CLAIM_STATUS_CANCELLED)
            | (CLAIM_STATUS_RELEASED, CLAIM_STATUS_CLAIMED)
            | (CLAIM_STATUS_RELEASED, CLAIM_STATUS_CANCELLED)
            | (CLAIM_STATUS_EXPIRED, CLAIM_STATUS_CLAIMED)
            | (CLAIM_STATUS_EXPIRED, CLAIM_STATUS_CANCELLED)
    );

    if allowed {
        Ok(())
    } else if matches!(
        current_status,
        CLAIM_STATUS_COMPLETED | CLAIM_STATUS_CANCELLED
    ) {
        Err(format!(
            "assisted execution claim status transition not allowed from terminal status: {} -> {}",
            current_status, new_status
        ))
    } else {
        Err(format!(
            "assisted execution claim status transition not allowed: {} -> {}",
            current_status, new_status
        ))
    }
}

pub fn apply_assisted_execution_claim_status_patch(
    claim: &AssistedExecutionClaim,
    new_status: &str,
    reason: Option<&str>,
) -> AssistedExecutionClaim {
    let mut updated_claim = claim.clone();
    let now = Utc::now().to_rfc3339();

    updated_claim.claim_status = new_status.trim().to_string();
    updated_claim.safe_to_auto_execute = false;

    let mut metadata = updated_claim.metadata.clone().unwrap_or_else(|| json!({}));
    if let Some(obj) = metadata.as_object_mut() {
        if let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) {
            obj.insert("status_reason".to_string(), json!(reason));
        }
        obj.insert("updated_at_utc".to_string(), json!(now));
    }
    updated_claim.metadata = Some(metadata);

    updated_claim
}

pub fn validate_assisted_execution_attempt_allowed(
    task: &InstallationTask,
    request: &AssistedExecutionRequest,
    claim: &AssistedExecutionClaim,
    execution_mode: &str,
) -> Result<(), String> {
    if !is_valid_assisted_execution_attempt_mode(execution_mode) {
        return Err(format!(
            "assisted execution attempt requires execution_mode to be '{}' or '{}', got '{}'",
            ATTEMPT_EXECUTION_MODE_NO_OP, ATTEMPT_EXECUTION_MODE_DRY_RUN, execution_mode
        ));
    }

    if task.project_id != request.project_id
        || task.project_id != claim.project_id
        || task.node_id != request.node_id
        || task.node_id != claim.node_id
        || task.task_id != request.task_id
        || task.task_id != claim.task_id
        || request.request_id != claim.request_id
    {
        return Err(
            "assisted execution attempt metadata does not match task/request/claim identity"
                .to_string(),
        );
    }

    if request.request_status != REQUEST_STATUS_REQUESTED {
        return Err(format!(
            "assisted execution attempt requires request_status='{}', got '{}'",
            REQUEST_STATUS_REQUESTED, request.request_status
        ));
    }

    if claim.claim_status != CLAIM_STATUS_CLAIMED {
        return Err(format!(
            "assisted execution attempt requires claim_status='{}', got '{}'",
            CLAIM_STATUS_CLAIMED, claim.claim_status
        ));
    }

    if matches!(task.status.trim(), STATUS_COMPLETED | STATUS_CANCELLED) {
        return Err("task status is not eligible for assisted execution attempt".to_string());
    }

    if task.safe_to_auto_execute || request.safe_to_auto_execute || claim.safe_to_auto_execute {
        return Err("safe_to_auto_execute must remain false across task/request/claim".to_string());
    }

    Ok(())
}

pub fn build_assisted_execution_attempt(
    project_id: &str,
    node_id: &str,
    task: &InstallationTask,
    request: &AssistedExecutionRequest,
    claim: &AssistedExecutionClaim,
    execution_mode: &str,
    attempted_by: Option<String>,
    message: Option<String>,
) -> AssistedExecutionAttempt {
    let execution_result = match execution_mode {
        ATTEMPT_EXECUTION_MODE_NO_OP => ATTEMPT_EXECUTION_RESULT_NO_OP_RECORDED,
        ATTEMPT_EXECUTION_MODE_DRY_RUN => ATTEMPT_EXECUTION_RESULT_DRY_RUN_RECORDED,
        _ => ATTEMPT_EXECUTION_RESULT_REJECTED_INVALID_REQUEST,
    };

    AssistedExecutionAttempt {
        project_id: project_id.to_string(),
        node_id: node_id.to_string(),
        task_id: task.task_id.clone(),
        request_id: request.request_id.clone(),
        claim_id: claim.claim_id.clone(),
        attempt_id: format!("attempt:{}", Uuid::new_v4()),
        attempt_status: ATTEMPT_STATUS_RECORDED.to_string(),
        attempted_by,
        attempted_at_utc: Utc::now().to_rfc3339(),
        execution_mode: execution_mode.to_string(),
        execution_result: execution_result.to_string(),
        safe_to_auto_execute: false,
        side_effects_performed: false,
        message,
        claim_snapshot: serde_json::to_value(claim).ok(),
        request_snapshot: serde_json::to_value(request).ok(),
        task_snapshot: serde_json::to_value(task).ok(),
        metadata: None,
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
        annotate_installation_task_policy, apply_assisted_execution_claim_status_patch,
        build_assisted_execution_attempt, build_assisted_execution_claim,
        build_assisted_execution_request, build_installation_task_approval,
        derive_assisted_execution_state_summary, derive_installation_tasks_from_plan,
        evaluate_installation_task_readiness, normalize_installation_task,
        validate_assisted_execution_attempt_allowed, validate_assisted_execution_claim_allowed,
        validate_assisted_execution_claim_status_transition,
        validate_assisted_execution_request_allowed, validate_installation_task,
        validate_installation_task_can_be_approved, validate_installation_task_status_transition,
        AssistedExecutionAttempt, AssistedExecutionClaim, AssistedExecutionRequest,
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

    fn requested_request() -> AssistedExecutionRequest {
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

        build_assisted_execution_request(
            "smart-city-campinas",
            "fog-01",
            &task,
            &readiness,
            Some(&approval),
            Some("operator".to_string()),
            Some("request future assisted execution".to_string()),
        )
    }

    fn existing_claim(claim_status: &str) -> AssistedExecutionClaim {
        let task = candidate_task();
        let request = requested_request();
        existing_claim_for_request(&task, &request, claim_status)
    }

    fn existing_claim_for_request(
        task: &InstallationTask,
        request: &AssistedExecutionRequest,
        claim_status: &str,
    ) -> AssistedExecutionClaim {
        let mut claim = build_assisted_execution_claim(
            "smart-city-campinas",
            "fog-01",
            task,
            request,
            Some("operator".to_string()),
            Some("claim for future assisted execution".to_string()),
            None,
        );
        claim.claim_status = claim_status.to_string();
        claim
    }

    fn recorded_attempt(execution_mode: &str) -> AssistedExecutionAttempt {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");

        build_assisted_execution_attempt(
            "smart-city-campinas",
            "fog-01",
            &task,
            &request,
            &claim,
            execution_mode,
            Some("operator".to_string()),
            Some("recording metadata-only attempt".to_string()),
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

    #[test]
    fn ready_task_with_valid_approval_can_create_request() {
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

        assert!(
            validate_assisted_execution_request_allowed(&task, &readiness, Some(&approval)).is_ok()
        );

        let request = build_assisted_execution_request(
            "smart-city-campinas",
            "fog-01",
            &task,
            &readiness,
            Some(&approval),
            Some("operator".to_string()),
            Some("request future assisted execution".to_string()),
        );

        assert_eq!(request.request_status, "requested");
        assert_eq!(request.execution_mode, "assisted_operator_approved");
        assert!(!request.safe_to_auto_execute);
        assert!(request.readiness_snapshot.is_some());
        assert!(request.task_snapshot.is_some());
        assert!(request.approval_snapshot.is_some());
    }

    #[test]
    fn missing_approval_cannot_create_request() {
        let task = candidate_task();
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert!(validate_assisted_execution_request_allowed(&task, &readiness, None).is_err());
    }

    #[test]
    fn readiness_not_ready_cannot_create_request() {
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

        assert!(
            validate_assisted_execution_request_allowed(&task, &readiness, Some(&approval))
                .is_err()
        );
    }

    #[test]
    fn readiness_blocked_cannot_create_request() {
        let mut task = candidate_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert!(validate_assisted_execution_request_allowed(&task, &readiness, None).is_err());
    }

    #[test]
    fn manual_only_policy_cannot_create_request() {
        let task = base_task();
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert!(validate_assisted_execution_request_allowed(&task, &readiness, None).is_err());
    }

    #[test]
    fn completed_task_cannot_create_request() {
        let mut task = candidate_task();
        task.status = "completed".to_string();
        annotate_installation_task_policy(&mut task);
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

        assert!(
            validate_assisted_execution_request_allowed(&task, &readiness, Some(&approval))
                .is_err()
        );
    }

    #[test]
    fn cancelled_task_cannot_create_request() {
        let mut task = candidate_task();
        task.status = "cancelled".to_string();
        annotate_installation_task_policy(&mut task);
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        assert!(validate_assisted_execution_request_allowed(&task, &readiness, None).is_err());
    }

    #[test]
    fn requested_request_with_no_existing_claim_can_be_claimed() {
        let task = candidate_task();
        let request = requested_request();

        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_ok());
    }

    #[test]
    fn existing_claimed_claim_blocks_new_claim() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim("claimed");

        let err = validate_assisted_execution_claim_allowed(&task, &request, Some(&claim))
            .expect_err("claimed claim should block");
        assert_eq!(err, "assisted execution request is already claimed");
    }

    #[test]
    fn existing_released_cancelled_or_expired_claim_allows_new_claim() {
        let task = candidate_task();
        let request = requested_request();

        for status in ["released", "cancelled", "expired"] {
            let claim = existing_claim(status);
            assert!(
                validate_assisted_execution_claim_allowed(&task, &request, Some(&claim)).is_ok()
            );
        }
    }

    #[test]
    fn existing_completed_claim_blocks_new_claim() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim("completed");

        let err = validate_assisted_execution_claim_allowed(&task, &request, Some(&claim))
            .expect_err("completed claim should block");
        assert_eq!(err, "assisted execution request already completed");
    }

    #[test]
    fn non_requested_request_cannot_be_claimed() {
        let task = candidate_task();
        let mut request = requested_request();
        request.request_status = "cancelled".to_string();

        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_err());
    }

    #[test]
    fn request_safe_to_auto_execute_true_is_rejected_for_claim() {
        let task = candidate_task();
        let mut request = requested_request();
        request.safe_to_auto_execute = true;

        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_err());
    }

    #[test]
    fn task_safe_to_auto_execute_true_is_rejected_for_claim() {
        let mut task = candidate_task();
        let request = requested_request();
        task.safe_to_auto_execute = true;

        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_err());
    }

    #[test]
    fn mismatched_request_identity_is_rejected_for_claim() {
        let task = candidate_task();

        let mut request = requested_request();
        request.project_id = "other-project".to_string();
        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_err());

        let mut request = requested_request();
        request.node_id = "other-node".to_string();
        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_err());

        let mut request = requested_request();
        request.task_id = "other-task".to_string();
        assert!(validate_assisted_execution_claim_allowed(&task, &request, None).is_err());
    }

    #[test]
    fn claim_safe_to_auto_execute_remains_false() {
        let task = candidate_task();
        let request = requested_request();
        let claim = build_assisted_execution_claim(
            "smart-city-campinas",
            "fog-01",
            &task,
            &request,
            Some("operator".to_string()),
            Some("claim".to_string()),
            None,
        );

        assert!(!claim.safe_to_auto_execute);
    }

    #[test]
    fn claim_starts_with_claim_status_claimed() {
        let task = candidate_task();
        let request = requested_request();
        let claim = build_assisted_execution_claim(
            "smart-city-campinas",
            "fog-01",
            &task,
            &request,
            Some("operator".to_string()),
            Some("claim".to_string()),
            None,
        );

        assert_eq!(claim.claim_status, "claimed");
    }

    #[test]
    fn claim_includes_request_and_task_snapshots() {
        let task = candidate_task();
        let request = requested_request();
        let claim = build_assisted_execution_claim(
            "smart-city-campinas",
            "fog-01",
            &task,
            &request,
            Some("operator".to_string()),
            Some("claim".to_string()),
            None,
        );

        assert!(claim.request_snapshot.is_some());
        assert!(claim.task_snapshot.is_some());
        assert!(claim.readiness_snapshot.is_some());
    }

    #[test]
    fn claimed_to_released_passes() {
        assert!(validate_assisted_execution_claim_status_transition("claimed", "released").is_ok());
    }

    #[test]
    fn claimed_to_expired_passes() {
        assert!(validate_assisted_execution_claim_status_transition("claimed", "expired").is_ok());
    }

    #[test]
    fn claimed_to_completed_passes() {
        assert!(
            validate_assisted_execution_claim_status_transition("claimed", "completed").is_ok()
        );
    }

    #[test]
    fn claimed_to_cancelled_passes() {
        assert!(
            validate_assisted_execution_claim_status_transition("claimed", "cancelled").is_ok()
        );
    }

    #[test]
    fn released_to_claimed_passes() {
        assert!(validate_assisted_execution_claim_status_transition("released", "claimed").is_ok());
    }

    #[test]
    fn released_to_cancelled_passes() {
        assert!(
            validate_assisted_execution_claim_status_transition("released", "cancelled").is_ok()
        );
    }

    #[test]
    fn expired_to_claimed_passes() {
        assert!(validate_assisted_execution_claim_status_transition("expired", "claimed").is_ok());
    }

    #[test]
    fn expired_to_cancelled_passes() {
        assert!(
            validate_assisted_execution_claim_status_transition("expired", "cancelled").is_ok()
        );
    }

    #[test]
    fn completed_to_claimed_fails() {
        assert!(
            validate_assisted_execution_claim_status_transition("completed", "claimed").is_err()
        );
    }

    #[test]
    fn cancelled_to_claimed_fails() {
        assert!(
            validate_assisted_execution_claim_status_transition("cancelled", "claimed").is_err()
        );
    }

    #[test]
    fn unknown_current_claim_status_fails() {
        assert!(validate_assisted_execution_claim_status_transition("mystery", "claimed").is_err());
    }

    #[test]
    fn unknown_new_claim_status_fails() {
        assert!(validate_assisted_execution_claim_status_transition("claimed", "mystery").is_err());
    }

    #[test]
    fn claim_status_patch_does_not_change_safe_to_auto_execute() {
        let task = candidate_task();
        let request = requested_request();
        let claim = build_assisted_execution_claim(
            "smart-city-campinas",
            "fog-01",
            &task,
            &request,
            Some("operator".to_string()),
            Some("claim".to_string()),
            None,
        );

        let updated = apply_assisted_execution_claim_status_patch(
            &claim,
            "released",
            Some("operator released claim metadata"),
        );

        assert!(!updated.safe_to_auto_execute);
    }

    #[test]
    fn claim_status_patch_preserves_snapshots() {
        let task = candidate_task();
        let request = requested_request();
        let claim = build_assisted_execution_claim(
            "smart-city-campinas",
            "fog-01",
            &task,
            &request,
            Some("operator".to_string()),
            Some("claim".to_string()),
            None,
        );

        let updated = apply_assisted_execution_claim_status_patch(
            &claim,
            "released",
            Some("operator released claim metadata"),
        );

        assert_eq!(updated.request_snapshot, claim.request_snapshot);
        assert_eq!(updated.readiness_snapshot, claim.readiness_snapshot);
        assert_eq!(updated.task_snapshot, claim.task_snapshot);
        assert_eq!(
            updated
                .metadata
                .as_ref()
                .and_then(|value| value.get("status_reason"))
                .and_then(Value::as_str),
            Some("operator released claim metadata")
        );
        assert!(updated
            .metadata
            .as_ref()
            .and_then(|value| value.get("updated_at_utc"))
            .and_then(Value::as_str)
            .is_some());
    }

    #[test]
    fn claimed_claim_with_requested_request_can_record_no_op_attempt() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_ok()
        );
    }

    #[test]
    fn claimed_claim_with_requested_request_can_record_dry_run_attempt() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "dry_run").is_ok()
        );
    }

    #[test]
    fn non_claimed_claim_is_rejected_for_attempt() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim("released");

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );
    }

    #[test]
    fn completed_task_is_rejected_for_attempt() {
        let mut task = candidate_task();
        task.status = "completed".to_string();
        let request = requested_request();
        let claim = existing_claim("claimed");

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );
    }

    #[test]
    fn cancelled_task_is_rejected_for_attempt() {
        let mut task = candidate_task();
        task.status = "cancelled".to_string();
        let request = requested_request();
        let claim = existing_claim("claimed");

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );
    }

    #[test]
    fn request_status_not_requested_is_rejected_for_attempt() {
        let task = candidate_task();
        let mut request = requested_request();
        let claim = existing_claim("claimed");
        request.request_status = "cancelled".to_string();

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );
    }

    #[test]
    fn safe_to_auto_execute_true_on_task_request_or_claim_is_rejected_for_attempt() {
        let request = requested_request();
        let claim = existing_claim("claimed");
        let mut task = candidate_task();
        task.safe_to_auto_execute = true;
        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );

        let task = candidate_task();
        let mut request = requested_request();
        request.safe_to_auto_execute = true;
        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );

        let task = candidate_task();
        let request = requested_request();
        let mut claim = existing_claim("claimed");
        claim.safe_to_auto_execute = true;
        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "no_op").is_err()
        );
    }

    #[test]
    fn invalid_execution_mode_is_rejected_for_attempt() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim("claimed");

        assert!(
            validate_assisted_execution_attempt_allowed(&task, &request, &claim, "execute")
                .is_err()
        );
    }

    #[test]
    fn attempt_side_effects_performed_remains_false() {
        let attempt = recorded_attempt("no_op");

        assert!(!attempt.side_effects_performed);
    }

    #[test]
    fn attempt_safe_to_auto_execute_remains_false() {
        let attempt = recorded_attempt("no_op");

        assert!(!attempt.safe_to_auto_execute);
    }

    #[test]
    fn attempt_includes_task_request_and_claim_snapshots() {
        let attempt = recorded_attempt("no_op");

        assert!(attempt.task_snapshot.is_some());
        assert!(attempt.request_snapshot.is_some());
        assert!(attempt.claim_snapshot.is_some());
    }

    #[test]
    fn attempt_starts_with_attempt_status_recorded() {
        let attempt = recorded_attempt("no_op");

        assert_eq!(attempt.attempt_status, "recorded");
    }

    #[test]
    fn execution_result_is_no_op_or_dry_run_recorded() {
        let no_op = recorded_attempt("no_op");
        let dry_run = recorded_attempt("dry_run");

        assert_eq!(no_op.execution_result, "no_op_recorded");
        assert_eq!(dry_run.execution_result, "dry_run_recorded");
    }

    #[test]
    fn state_summary_without_approval_is_awaiting_approval() {
        let task = candidate_task();
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            Some(&readiness),
            None,
            None,
            None,
        );

        assert_eq!(summary.state, "awaiting_approval");
    }

    #[test]
    fn state_summary_manual_only_policy_is_policy_manual_only() {
        let task = base_task();
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            Some(&readiness),
            None,
            None,
            None,
        );

        assert_eq!(summary.state, "policy_manual_only");
    }

    #[test]
    fn state_summary_missing_policy_is_policy_unknown() {
        let mut task = candidate_task();
        task.installation_policy = None;
        let readiness =
            evaluate_installation_task_readiness("smart-city-campinas", "fog-01", &task, None);

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            Some(&readiness),
            None,
            None,
            None,
        );

        assert_eq!(summary.state, "policy_unknown");
    }

    #[test]
    fn state_summary_approved_but_not_ready_is_awaiting_readiness() {
        let task = candidate_task();
        let approval = build_installation_task_approval(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some("operator".to_string()),
            None,
            None,
        );
        let mut approval = approval;
        approval.approved = false;
        let readiness = evaluate_installation_task_readiness(
            "smart-city-campinas",
            "fog-01",
            &task,
            Some(&approval),
        );

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            Some(&approval),
            Some(&readiness),
            None,
            None,
            None,
        );

        assert_eq!(summary.state, "awaiting_readiness");
    }

    #[test]
    fn state_summary_ready_but_no_request_is_awaiting_request() {
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

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            Some(&approval),
            Some(&readiness),
            None,
            None,
            None,
        );

        assert_eq!(summary.state, "awaiting_request");
    }

    #[test]
    fn state_summary_request_without_claim_is_awaiting_claim() {
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
        let request = requested_request();

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            Some(&approval),
            Some(&readiness),
            Some(&request),
            None,
            None,
        );

        assert_eq!(summary.state, "awaiting_claim");
    }

    #[test]
    fn state_summary_claimed_without_attempt_is_claimed() {
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
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            Some(&approval),
            Some(&readiness),
            Some(&request),
            Some(&claim),
            None,
        );

        assert_eq!(summary.state, "claimed");
    }

    #[test]
    fn state_summary_released_claim_is_claim_released() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "released");

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            None,
            Some(&request),
            Some(&claim),
            None,
        );

        assert_eq!(summary.state, "claim_released");
    }

    #[test]
    fn state_summary_terminal_claims_are_claim_terminal() {
        let task = candidate_task();
        let request = requested_request();

        for status in ["completed", "cancelled", "expired"] {
            let claim = existing_claim_for_request(&task, &request, status);
            let summary = derive_assisted_execution_state_summary(
                "smart-city-campinas",
                "fog-01",
                Some(&task),
                None,
                None,
                Some(&request),
                Some(&claim),
                None,
            );

            assert_eq!(summary.state, "claim_terminal");
        }
    }

    #[test]
    fn state_summary_latest_no_op_attempt_is_attempt_recorded_no_op() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");
        let attempt = recorded_attempt("no_op");

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            None,
            Some(&request),
            Some(&claim),
            Some(&attempt),
        );

        assert_eq!(summary.state, "attempt_recorded_no_op");
    }

    #[test]
    fn state_summary_latest_dry_run_attempt_is_attempt_recorded_dry_run() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");
        let attempt = recorded_attempt("dry_run");

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            None,
            Some(&request),
            Some(&claim),
            Some(&attempt),
        );

        assert_eq!(summary.state, "attempt_recorded_dry_run");
    }

    #[test]
    fn state_summary_safe_to_auto_execute_remains_false() {
        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&candidate_task()),
            None,
            None,
            None,
            None,
            None,
        );

        assert!(!summary.safe_to_auto_execute);
    }

    #[test]
    fn state_summary_side_effects_performed_remains_false_when_attempt_false() {
        let task = candidate_task();
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");
        let attempt = recorded_attempt("no_op");

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            None,
            None,
            Some(&request),
            Some(&claim),
            Some(&attempt),
        );

        assert!(!summary.side_effects_performed);
    }

    #[test]
    fn state_summary_preserves_snapshots() {
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
        let request = requested_request();
        let claim = existing_claim_for_request(&task, &request, "claimed");
        let attempt = recorded_attempt("no_op");

        let summary = derive_assisted_execution_state_summary(
            "smart-city-campinas",
            "fog-01",
            Some(&task),
            Some(&approval),
            Some(&readiness),
            Some(&request),
            Some(&claim),
            Some(&attempt),
        );

        assert!(summary.task_snapshot.is_some());
        assert!(summary.approval_snapshot.is_some());
        assert!(summary.readiness_snapshot.is_some());
        assert!(summary.request_snapshot.is_some());
        assert!(summary.claim_snapshot.is_some());
        assert!(summary.latest_attempt_snapshot.is_some());
    }
}
